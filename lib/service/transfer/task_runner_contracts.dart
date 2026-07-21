import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/types/enums.dart';

/// TaskRunner 对外合同与结果类型（对齐 Rust `task_runner/contracts.rs`）。

/// 任务执行或准入后的调度去向（对齐 Rust `TaskDisposition`）。
enum TaskDisposition {
  /// 已完成
  completed,

  /// 重新入队等待调度
  pending,

  /// 仍在执行
  running,

  /// 被同路径活动意图阻塞
  blockedByActiveIntent,

  /// 等待网络恢复
  waitingForNetwork,

  /// 退避重试
  backingOff,

  /// 远端核验
  verifyingRemote,

  /// 需重新规划
  restartRequired,
}

/// 为引擎的基线与云树结算保留的后端输出（对齐 Rust `TaskExecutionOutcome`）。
class TaskExecutionOutcome {
  /// 后端校验过的完整云端元数据（上传/远端写操作产出）
  final DriveFile? cloudFile;

  /// 调度去向
  final TaskDisposition disposition;

  const TaskExecutionOutcome({
    this.cloudFile,
    this.disposition = TaskDisposition.completed,
  });
}

/// 对远程写入是否已提交的核验结果（对齐 Rust `RemoteVerification`）。
sealed class RemoteVerification {
  const RemoteVerification();
}

/// 写入已提交：携带经后端校验的完整云端元数据
final class RemoteCommitted extends RemoteVerification {
  /// 已确认的云端文件
  final DriveFile file;

  const RemoteCommitted(this.file);
}

/// 写入未提交：可安全重放
final class RemoteNotCommitted extends RemoteVerification {
  const RemoteNotCommitted();
}

/// 结果仍不确定：携带原因，继续等待核验
final class RemoteAmbiguous extends RemoteVerification {
  /// 不确定原因
  final String message;

  const RemoteAmbiguous(this.message);
}

/// 后端前置校验对目标状态和错误分类的建议（对齐 Rust `BackendPreflightFailure`）。
class BackendPreflightFailure {
  /// 错误分类
  final TransferErrorKind kind;

  /// 拒绝后应迁移到的状态
  final TransferState target;

  /// 用户可读原因
  final String message;

  const BackendPreflightFailure({
    required this.kind,
    required this.target,
    required this.message,
  });

  /// 构造需要重新规划的本地变化错误。
  const BackendPreflightFailure.restartRequired(String message)
      : this(
          kind: TransferErrorKind.LocalChanged,
          target: TransferState.RestartRequired,
          message: message,
        );
}

/// 传输后端执行失败或需重新规划的原因（对齐 Rust `TaskExecutionError`）。
sealed class TaskExecutionError implements Exception {
  const TaskExecutionError();
}

/// 结构化应用错误（进入重试策略分类）
final class TaskAppError extends TaskExecutionError {
  /// 原始应用错误
  final AppError error;

  const TaskAppError(this.error);

  @override
  String toString() => '$error';
}

/// 本地源已变化，需回 planner 重新规划（对齐 Rust
/// `TaskExecutionError::RestartRequired`）
final class TaskRestartRequired extends TaskExecutionError {
  /// 重新规划原因
  final String message;

  const TaskRestartRequired(this.message);

  @override
  String toString() => message;
}

/// 执行期进度回写通道（对齐 Rust `TaskProgressReporter` 的回调面）。
///
/// 由 TaskRunner 按 Running 修订号构造并传入执行器：
/// - [onProgress] 上传可见进度（节流持久化）
/// - [onDownloadProgress] 下载可见进度与断点（节流持久化）
/// - [onResume] 上传会话身份与断点（不节流，必须落库）
class TaskProgressCallbacks {
  /// 上传进度回调：已传输字节数
  final void Function(int transferred) onProgress;

  /// 下载进度回调：已下载字节数（同时持久化 resume_offset）
  final void Function(int transferred) onDownloadProgress;

  /// 上传会话回调：serverId、uploadId、已确认偏移、sessionUrl
  final void Function(
    String serverId,
    String uploadId,
    int offset,
    String sessionUrl,
  ) onResume;

  /// 任务总大小（字节），供执行器做边界换算
  final int totalSize;

  const TaskProgressCallbacks({
    required this.onProgress,
    required this.onDownloadProgress,
    required this.onResume,
    required this.totalSize,
  });
}

/// TaskRunner 执行传输所需的后端能力（对齐 Rust `TransferOperations` trait）。
///
/// 由操作执行适配层实现（生产：`DriveTaskOperations`；测试：fake）。
abstract class TaskOperations {
  /// 在任务进入 Running 前执行本地与远程静态校验。
  ///
  /// 默认通过；拒绝时抛 [BackendPreflightFailure] 携带目标状态与错误分类。
  Future<void> preflight(TransferTask task) async {}

  /// 执行传输并通过 [progress] 持久进度。
  ///
  /// 失败抛 [TaskAppError]；本地源变化需重新规划抛 [TaskRestartRequired]。
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  );

  /// 核实响应不确定的远程写入是否真实提交。
  ///
  /// 默认不支持核验（对齐 Rust 默认实现的 Ambiguous 语义由子类按需覆盖）。
  Future<RemoteVerification> verifyRemote(TransferTask task) async {
    return const RemoteAmbiguous('当前后端不支持远端结果核验');
  }
}

/// 入队仲裁结果（对齐 Rust `EnqueuedTaskOutcome`）。
class EnqueuedTaskOutcome {
  /// 承载该意图的任务 id（去重/重规划/屏障时可能不是新插入的行）
  final int taskId;

  /// 执行或准入后的调度去向
  final TaskExecutionOutcome outcome;

  const EnqueuedTaskOutcome({required this.taskId, required this.outcome});
}

/// TaskRunner 向同步引擎暴露的基线结算钩子（对齐 Rust
/// `settlement.rs` / `persistence.rs` 的 sync_items 回写点）。
///
/// 生产实现：`SyncBaselineStore`；钩子在任务行迁移完成后调用，
/// 失败语义对齐 Rust（onTaskCommitted 抛错 → 进入恢复路径，禁止完成）。
abstract class SyncTaskHooks {
  /// 任务完成后的 sync_items 成功基线结算 + xattr 回写
  Future<void> onTaskCommitted(
    TransferTask running,
    TaskExecutionOutcome outcome,
  );

  /// 任务永久失败后的 sync_items FAILED 标记
  Future<void> onTaskFailed(TransferTask failed, String message);

  /// retry 接受后的 SYNCING 回写（旧状态须为 FAILED）
  Future<void> onRetryAccepted(TransferTask pending);

  /// replan 接受后的 SYNCING 回写（无旧状态条件）
  Future<void> onTaskReplanned(TransferTask task);
}

/// 上传失败通知（对齐 Rust `upload_failed` 事件负载）。
class UploadFailureNotice {
  /// 文件名
  final String name;

  /// 相对挂载根路径（可为空）
  final String? relativePath;

  /// 用户可读错误
  final String error;

  const UploadFailureNotice({
    required this.name,
    this.relativePath,
    required this.error,
  });
}

/// 队列快照（revision 版本化，防乱序；供 transfer_controller 订阅）。
///
/// 每次接受或拒绝任务变更后重新构建并发布完整权威快照，
/// 对齐 Rust `TaskStateSink::recompute_and_broadcast` 的发布语义。
class TransferQueueSnapshot {
  /// 单调递增快照版本
  final int revision;

  /// 全部传输任务（created_at 倒序，与 TransferService.getAllTasks 一致）
  final List<TransferTask> tasks;

  /// 活跃任务数（Running + VerifyingRemote，占用传输槽位）
  final int activeCount;

  const TransferQueueSnapshot({
    required this.revision,
    required this.tasks,
    required this.activeCount,
  });
}
