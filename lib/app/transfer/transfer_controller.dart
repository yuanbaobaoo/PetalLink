import 'dart:async';

import 'package:get/get.dart';

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';

/// 传输 UI 状态
///
/// 对标 CMP TransferViewModel + Vue transfer store（docs/08 §2.4）。
class TransferUIState {
  /// 全部传输任务列表
  final List<TransferTask> tasks;

  /// 活跃任务数（running + verifyingRemote）
  final int activeCount;

  /// 已完成任务数
  final int completedCount;

  /// 失败任务数
  final int failedCount;

  const TransferUIState({
    this.tasks = const [],
    this.activeCount = 0,
    this.completedCount = 0,
    this.failedCount = 0,
  });

  /// 初始状态
  factory TransferUIState.initial() => const TransferUIState();

  /// 是否有活跃任务
  bool get hasActiveTasks => activeCount > 0;

  /// 深拷贝并替换指定字段
  TransferUIState copyWith({
    List<TransferTask>? tasks,
    int? activeCount,
    int? completedCount,
    int? failedCount,
  }) {
    return TransferUIState(
      tasks: tasks ?? this.tasks,
      activeCount: activeCount ?? this.activeCount,
      completedCount: completedCount ?? this.completedCount,
      failedCount: failedCount ?? this.failedCount,
    );
  }
}

/// 传输控制器 — 全局传输任务队列管理
///
/// 对标 CMP TransferViewModel（ViewModels.kt）与 Vue transfer store（docs/08 §2.4）。
///
/// 核心机制：
/// - **两重乱序保护**（对标 Vue transfer.loadAll）：
///   1. 请求 ID（requestId）：过期请求丢弃
///   2. Per-task state_revision 比对：同 task 旧 revision 回写保护
/// - 从 TransferService 加载持久化任务列表
/// - CAS 更新：每个任务更新检查 stateRevision
class TransferController extends GetxController {
  final TransferService _transferService = Get.find<TransferService>();

  /// 持久化传输执行器（快照流 + 命令面；由 GlobalBinding 注册）
  final TaskRunner _taskRunner = Get.find<TaskRunner>();

  /// 传输 UI 状态（响应式）
  final Rx<TransferUIState> state = TransferUIState.initial().obs;

  /// 上传失败通知流（透传 TaskRunner，供 UI 弹 toast）
  Stream<UploadFailureNotice> get uploadFailures => _taskRunner.uploadFailures;

  /// 请求 ID（单调递增，乱序保护第 1 重）
  int _nextLoadRequest = 0;

  /// 最后接受的请求 ID
  int _lastAppliedLoadRequest = -1;

  /// 已应用的队列快照版本（revision 防乱序）
  int _appliedSnapshotRevision = 0;

  /// TaskRunner 快照订阅
  StreamSubscription<TransferQueueSnapshot>? _snapshotSub;

  @override
  void onInit() {
    super.onInit();
    // 初始化时加载一次
    refreshTasks();
    // 补偿晚订阅：先应用启动前最近一次快照，再持续订阅 revision 快照流
    final last = _taskRunner.lastSnapshot;
    if (last != null) _applySnapshot(last);
    _snapshotSub = _taskRunner.snapshots.listen(_applySnapshot);
  }

  @override
  void onClose() {
    _snapshotSub?.cancel();
    super.onClose();
  }

  /// 应用 TaskRunner 权威快照（丢弃 revision 倒退的过期快照）。
  void _applySnapshot(TransferQueueSnapshot snapshot) {
    if (snapshot.revision <= _appliedSnapshotRevision) return;
    _appliedSnapshotRevision = snapshot.revision;
    _updateTasks(snapshot.tasks);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 任务加载（两重乱序保护）
  // ═══════════════════════════════════════════════════════════════════

  /// 刷新任务列表（从 TransferService 加载）
  ///
  /// 两重乱序保护（对标 Vue transfer.loadAll）：
  /// 1. requestId：过期请求直接丢弃
  /// 2. Per-task state_revision：保留同 ID 中 revision 更大的版本
  Future<void> refreshTasks() async {
    final requestId = ++_nextLoadRequest;

    final result = await _transferService.getAllTasks();
    if (result.isErr) {
      AppLogger.e('refreshTasks 失败: ${(result as Err).error}');
      return;
    }

    final tasks = (result as Ok<List<TransferTask>>).value;

    // 第 1 重：过期请求丢弃（IPC 失败≠空队列，保留最后成功快照）
    if (requestId < _lastAppliedLoadRequest) {
      AppLogger.d('refreshTasks 过期请求丢弃: $requestId < $_lastAppliedLoadRequest');
      return;
    }

    // 第 2 重：Per-task state_revision 比对
    // 如果当前已有同 ID 任务且其 stateRevision 更大 → 保留当前版本
    final currentRevisions = <int, int>{};
    for (final t in state.value.tasks) {
      currentRevisions[t.id] = t.stateRevision;
    }

    // 检查是否有回退：任一已加载任务的 revision 小于当前 revision → 拒绝全量
    final hasStale = tasks.any((t) {
      final currentRev = currentRevisions[t.id];
      return currentRev != null && t.stateRevision < currentRev;
    });

    if (hasStale) {
      AppLogger.d('refreshTasks CAS 拒绝：存在 revision 回退');
      return;
    }

    _lastAppliedLoadRequest = requestId;

    // 统计各状态计数
    int active = 0, completed = 0, failed = 0;
    for (final t in tasks) {
      if (t.state.isActive) {
        active++;
      }
      if (t.state == TransferState.completed) {
        completed++;
      }
      if (t.state == TransferState.failed) {
        failed++;
      }
    }

    state.value = TransferUIState(
      tasks: tasks,
      activeCount: active,
      completedCount: completed,
      failedCount: failed,
    );

    AppLogger.d('refreshTasks: ${tasks.length} 条任务 (活跃:$active 完成:$completed 失败:$failed)');
  }

  /// 更新单个任务进度（CAS：revision 比对）
  ///
  /// 对标 CMP TransferViewModel.updateProgress()。
  /// 仅当 incoming revision >= 当前同 ID 任务的 stateRevision 时更新。
  void updateTaskProgress(int taskId, int bytesDone, int revision) {
    final current = state.value.tasks;
    final updated = current.map((t) {
      if (t.id == taskId && revision >= t.stateRevision) {
        return t.copyWith(
          transferred: bytesDone,
          stateRevision: revision,
        );
      }
      return t;
    }).toList();

    if (updated != current) {
      _updateTasks(updated);
    }
  }

  /// 更新单个任务状态（CAS：revision 比对）
  void updateTaskStatus(int taskId, TransferState newState,
      {String? error, int? revision}) {
    final rev = revision ?? DateTime.now().millisecondsSinceEpoch;
    final current = state.value.tasks;
    final updated = current.map((t) {
      if (t.id == taskId && rev >= t.stateRevision) {
        return t.copyWith(
          state: newState,
          errorMessage: error ?? t.errorMessage,
          stateRevision: rev,
        );
      }
      return t;
    }).toList();

    if (updated != current) {
      _updateTasks(updated);
    }
  }

  /// 内部：更新任务列表并重算计数
  void _updateTasks(List<TransferTask> tasks) {
    int active = 0, completed = 0, failed = 0;
    for (final t in tasks) {
      if (t.state.isActive) active++;
      if (t.state == TransferState.completed) completed++;
      if (t.state == TransferState.failed) failed++;
    }

    state.value = state.value.copyWith(
      tasks: tasks,
      activeCount: active,
      completedCount: completed,
      failedCount: failed,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 命令面（对齐 Rust commands/transfer.rs，经 TaskRunner 走 CAS + 快照广播）
  // ═══════════════════════════════════════════════════════════════════

  /// 手动重试失败任务（前置校验 + revision 复查后转 Pending）
  Future<AppResult<void>> retry(int taskId) => _taskRunner.retry(taskId);

  /// 是否存在 Pending/Running 任务（对齐 transfer_has_active）
  Future<AppResult<bool>> hasActive() => _taskRunner.hasActive();

  /// 清除已完成的任务（调用 TaskRunner.clearCompleted，快照流自动刷新 UI）
  Future<void> clearCompleted() async {
    final result = await _taskRunner.clearCompleted();
    if (result.isOk) {
      final count = (result as Ok<int>).value;
      AppLogger.i('清除已完成任务: $count 条');
    } else {
      AppLogger.e('清除已完成任务失败: ${(result as Err).error}');
    }
  }

  /// 清除已失败的任务
  Future<void> clearFailed() async {
    final result = await _taskRunner.clearFailed();
    if (result.isOk) {
      final count = (result as Ok<int>).value;
      AppLogger.i('清除已失败任务: $count 条');
    } else {
      AppLogger.e('清除已失败任务失败: ${(result as Err).error}');
    }
  }

  /// 清除已结束的任务（Completed + Failed）
  Future<void> clearFinished() async {
    final result = await _taskRunner.clearFinished();
    if (result.isOk) {
      final count = (result as Ok<int>).value;
      AppLogger.i('清除已结束任务: $count 条');
    } else {
      AppLogger.e('清除已结束任务失败: ${(result as Err).error}');
    }
  }
}
