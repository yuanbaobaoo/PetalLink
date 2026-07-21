// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart' show tmpPath;
import 'package:petal_link/service/transfer/retry_policy.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/task_runner_preflight.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';

/// 持久化传输任务执行器（对齐 Rust `src/sync/task_runner.rs` + `task_runner/`）。
///
/// 职责：
/// - 驱动 9 态持久化状态机：Pending→Running→(WaitingForNetwork/BackingOff/
///   VerifyingRemote)→Completed/Failed/Canceled；RestartRequired 回 planner；
///   全部状态变迁走 TransferService.transition（CAS + state_revision 递增）
/// - 并发调度：按 created_at FIFO 准入，槽位数 = AppConfig.concurrency（1-20，默认 6）
/// - 网络门控：离线时执行链将任务转入 WaitingForNetwork；恢复在线后
///   按「核验 → 等待 → 到期退避 → 新任务」重新调度；请求级网络失败边沿上报 NetGuard
/// - 退避重试：按 retry_policy 分类决定可重试性、attempt_count、next_retry_at
/// - 崩溃恢复：启动时收敛非终态任务行（Running 上传→核验、下载→断点续跑）
///
/// 引擎编排（planner/云树/基线结算）属后续任务；本类只暴露执行与命令面。
class TaskRunner {
  /// 任务持久化与 CAS 状态迁移
  final TransferService _transferService;

  /// 操作执行适配层
  final TaskOperations _operations;

  /// 在线判定（生产接 NetGuard.isOnline）
  final bool Function() _isOnline;

  /// 稳定网络转换流（生产接 NetGuard.transitions，可空）
  final Stream<NetworkTransition>? _netTransitions;

  /// 请求级网络失败边沿上报（生产接 NetGuard.reportRequestNetworkFailure，可空）
  final void Function()? _onRequestNetworkFailure;

  /// 当前毫秒时钟（测试注入）
  final int Function() _nowMs;

  /// 退避抖动毫秒（对齐 Rust runner 默认 0）
  final int Function() _jitterMs;

  /// 并发槽位提供（AppConfig.concurrency；clamp 1-20）
  final Future<int> Function()? _concurrencyProvider;

  /// 挂载根提供（静态前置校验用；未配置时校验按 Rust 语义拒绝执行）
  final String? Function() _mountRootProvider;

  /// 0 字节占位符判定（下载静态校验用，可空 = 保守视为非占位）
  final Future<bool> Function(String path)? _isPlaceholder;

  /// 调度 tick 周期（退避/核验到期粒度）
  final Duration _tickInterval;

  /// 单个任务允许的最大自动重试次数（对齐 Rust MAX_AUTOMATIC_ATTEMPTS）
  final int _maxAttempts;

  /// 进度持久化节流间隔（对齐 Rust PROGRESS_THROTTLE_MS）
  static const int progressThrottleMs = 500;

  /// VerifyingRemote 首次核验延迟（对齐 Rust settle_error 的 3s）
  static const int verifyInitialDelayMs = 3000;

  /// 核验结果仍歧义时的再核间隔（对齐 Rust 的 60s）
  static const int verifyAmbiguousDelayMs = 60000;

  /// 核验通道暂不可用时的再核间隔（对齐 Rust 的 15s）
  static const int verifyUnavailableDelayMs = 15000;

  TaskRunner({
    required TransferService transferService,
    required TaskOperations operations,
    bool Function()? isOnline,
    Stream<NetworkTransition>? netTransitions,
    void Function()? onRequestNetworkFailure,
    int Function()? nowMs,
    int Function()? jitterMs,
    Future<int> Function()? concurrencyProvider,
    String? Function()? mountRootProvider,
    Future<bool> Function(String path)? isPlaceholder,
    Duration tickInterval = const Duration(seconds: 1),
    int maxAttempts = 5,
  })  : _transferService = transferService,
        _operations = operations,
        _isOnline = isOnline ?? (() => true),
        _netTransitions = netTransitions,
        _onRequestNetworkFailure = onRequestNetworkFailure,
        _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch),
        _jitterMs = jitterMs ?? (() => 0),
        _concurrencyProvider = concurrencyProvider,
        _mountRootProvider = mountRootProvider ?? (() => null),
        _isPlaceholder = isPlaceholder,
        _tickInterval = tickInterval,
        _maxAttempts = maxAttempts;

  // ═══════════════════════════════════════════════════════════════════
  // 运行状态
  // ═══════════════════════════════════════════════════════════════════

  /// 是否已启动（start 后准入任务；stop 后不再准入）
  bool _started = false;

  /// 在途任务（taskId → 含结算收尾的执行 future）
  final Map<int, Future<void>> _inFlight = {};

  /// 在途任务占用的相对路径（同路径排他）
  final Set<String> _activePaths = {};

  /// 调度泵重入守卫
  bool _pumping = false;

  /// 泵执行期间又有泵请求
  bool _pumpAgain = false;

  /// 到期调度定时器
  Timer? _tickTimer;

  /// 网络转换订阅
  StreamSubscription<NetworkTransition>? _netSub;

  /// 同步引擎基线结算钩子（引擎接线后注入）
  SyncTaskHooks? _syncHooks;

  /// 入队仲裁执行结果等待器（taskId → completer）
  final Map<int, Completer<TaskExecutionOutcome>> _outcomeWatchers = {};

  // ═══════════════════════════════════════════════════════════════════
  // 事件发布
  // ═══════════════════════════════════════════════════════════════════

  /// 队列快照广播（revision 版本化）
  final StreamController<TransferQueueSnapshot> _snapshotCtrl =
      StreamController<TransferQueueSnapshot>.broadcast();

  /// 上传失败通知广播（对齐 Rust `upload_failed` 事件）
  final StreamController<UploadFailureNotice> _uploadFailureCtrl =
      StreamController<UploadFailureNotice>.broadcast();

  /// 快照版本号（单调递增）
  int _snapshotRevision = 0;

  /// 最近一次发布的快照
  TransferQueueSnapshot? _lastSnapshot;

  /// 队列快照流（防乱序：consumer 丢弃 revision 倒退的快照）
  Stream<TransferQueueSnapshot> get snapshots => _snapshotCtrl.stream;

  /// 上传失败通知流（{name, relativePath, error}）
  Stream<UploadFailureNotice> get uploadFailures => _uploadFailureCtrl.stream;

  /// 最近一次发布的快照（晚订阅者补偿用）
  TransferQueueSnapshot? get lastSnapshot => _lastSnapshot;

  /// 是否已启动
  bool get isStarted => _started;

  // ═══════════════════════════════════════════════════════════════════
  // 生命周期
  // ═══════════════════════════════════════════════════════════════════

  /// 启动执行器：崩溃恢复 → 订阅网络转换 → 到期调度 → 首轮泵。
  ///
  /// 引擎启动时机由 sync 引擎任务统一接管；重复调用安全。
  Future<void> start() async {
    if (_started) return;
    _started = true;
    AppLogger.i('TaskRunner 启动：开始崩溃恢复');
    try {
      await _recoverStartup();
    } catch (e, st) {
      AppLogger.e('TaskRunner 启动恢复异常', e, st);
    }
    _netSub = _netTransitions?.listen(_onNetTransition);
    _tickTimer = Timer.periodic(_tickInterval, (_) => unawaited(debugTick()));
    await _publishSnapshot();
    await _pumpLoop();
  }

  /// 停止调度：不再准入新任务；在途任务自然完结（对齐 Rust 引擎封门语义）。
  Future<void> stop() async {
    if (!_started) return;
    _started = false;
    _tickTimer?.cancel();
    _tickTimer = null;
    await _netSub?.cancel();
    _netSub = null;
    AppLogger.i('TaskRunner 已停止调度');
  }

  /// 释放事件流（应用退出时调用）。
  Future<void> dispose() async {
    await stop();
    await _snapshotCtrl.close();
    await _uploadFailureCtrl.close();
  }

  /// 网络转换处理：恢复在线后立即重新调度（核验 → 等待 → 到期退避 → 新任务）。
  void _onNetTransition(NetworkTransition transition) {
    if (transition == NetworkTransition.online) {
      AppLogger.i('网络恢复在线：重新调度传输队列');
      unawaited(_publishSnapshot().then((_) => _pumpLoop()));
    } else {
      // 在途任务经 isOnline 钩子自行失败并转入 WaitingForNetwork
      unawaited(_publishSnapshot());
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 命令面
  // ═══════════════════════════════════════════════════════════════════

  /// 入队新传输任务并触发调度。
  ///
  /// 新意图必须是 id=0/revision=0 的 Pending 任务（对齐 Rust `enqueue_and_run`
  /// 的入参校验）；同路径去重与重规划属 planner 职责（后续任务）。
  Future<AppResult<TransferTask>> enqueue(TransferTask task) async {
    if (task.id != 0 ||
        task.stateRevision != 0 ||
        task.state != TransferState.Pending) {
      await _publishSnapshot();
      return Err(const GenericError(
          message: '新传输意图必须是 id=0/revision=0 的 Pending 任务'));
    }
    final result = await _transferService.enqueue(task);
    if (result.isErr) return result;
    await _publishSnapshot();
    await _pumpLoop();
    return result;
  }

  /// 手动重试失败任务（对齐 Rust `prepare_retry` + `retry`）。
  ///
  /// 仅接受 Failed 状态；转 Pending 前先做静态与后端前置校验，
  /// 校验拒绝持久化到目标态（Failed/RestartRequired），不盲目重放。
  Future<AppResult<void>> retry(int taskId) async {
    final loaded = await _transferService.getTaskById(taskId);
    final task = loaded.unwrapOr(null);
    if (task == null || task.state != TransferState.Failed) {
      await _publishSnapshot();
      return Err(const GenericError(message: '任务不存在或非失败状态'));
    }
    // 静态前置校验
    try {
      await _validateStatic(task);
    } on PreflightFailure catch (failure) {
      await _persistPreflightRejection(task, failure);
      return Err(GenericError(message: failure.message));
    }
    // 后端前置校验
    try {
      await _operations.preflight(task);
    } on BackendPreflightFailure catch (failure) {
      await _persistPreflightRejection(
        task,
        PreflightFailure(
          target: failure.target,
          kind: failure.kind,
          message: failure.message,
        ),
      );
      return Err(GenericError(message: failure.message));
    }
    // 接受重试：revision 复查后转 Pending（对齐 accept_retry_after_preflight）
    final fresh = (await _transferService.getTaskById(taskId)).unwrapOr(null);
    if (fresh == null ||
        fresh.state != TransferState.Failed ||
        fresh.stateRevision != task.stateRevision) {
      await _publishSnapshot();
      return Err(const GenericError(message: '传输任务状态已变化，请刷新后重试'));
    }
    final pending = await _transition(
      fresh,
      TransferState.Pending,
      TransferPatch.clearingError(attemptCount: fresh.attemptCount + 1),
    );
    if (pending == null) {
      return Err(const GenericError(message: '传输任务状态已变化，请刷新后重试'));
    }
    // retry 接受的 SYNCING 回写（对齐 Rust accept_retry_after_preflight）
    try {
      await _syncHooks?.onRetryAccepted(pending);
    } catch (e) {
      AppLogger.w('任务 $taskId retry 后 SYNCING 回写失败（忽略）: $e');
    }
    // 对齐 Rust retry 内联 run_expected(pending, run_backend_preflight=false)：
    // 后端前置校验已在上方执行过一次，不再重复（上传稳定性检查不会跑两遍）。
    _track(pending, () => _runExpected(pending, runBackendPreflight: false));
    await _pumpLoop();
    return const Ok(null);
  }

  /// 是否存在 Pending/Running 任务（对齐 Rust `transfer_has_active` 命令）。
  Future<AppResult<bool>> hasActive() async {
    final result = await _transferService.countPendingOrRunning();
    return result.map((count) => count > 0);
  }

  /// 注入同步引擎基线结算钩子（引擎接线；对齐 Rust 引擎对
  /// TaskRunner 的 state sink 绑定）。
  void setSyncHooks(SyncTaskHooks? hooks) {
    _syncHooks = hooks;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 同路径仲裁入队（对齐 Rust task_runner/admission.rs:125-184）
  // ═══════════════════════════════════════════════════════════════════

  /// 入队新传输意图并执行，含同路径仲裁：
  /// 在途写优先 → 歧义重启提升 → 同意图去重 → 重规划 →
  /// 普通 RestartRequired 重规划 → Failed 路径屏障 → 插入新行。
  Future<AppResult<EnqueuedTaskOutcome>> enqueueAndRun(
    TransferTask task,
  ) async {
    if (task.id != 0 ||
        task.stateRevision != 0 ||
        task.state != TransferState.Pending) {
      await _publishSnapshot();
      return Err(const GenericError(
          message: '新传输意图必须是 id=0/revision=0 的 Pending 任务'));
    }
    final rel = task.relativePath;
    if (rel == null) {
      // 无路径任务：不参与仲裁，直接插入
      final inserted = await _transferService.enqueue(task);
      if (inserted.isErr) return Err((inserted as Err).error);
      final stored = (inserted as Ok<TransferTask>).value;
      await _publishSnapshot();
      final outcome = await _runAndAwaitOutcome(stored);
      return Ok(EnqueuedTaskOutcome(taskId: stored.id, outcome: outcome));
    }

    final pathTasks =
        (await _transferService.getTasksByRelativePath(rel)).unwrapOr([]);
    final blocking =
        pathTasks.where((t) => _isPathBlockingState(t.state)).toList();

    // 1. 在途写优先：Running/VerifyingRemote
    final inflight = blocking
        .where((t) =>
            t.state == TransferState.Running ||
            t.state == TransferState.VerifyingRemote)
        .firstOrNull;
    if (inflight != null) {
      if (_sameTransferIntent(inflight, task)) {
        await _pumpLoop();
        return Ok(EnqueuedTaskOutcome(
          taskId: inflight.id,
          outcome: TaskExecutionOutcome(
              disposition: _dispositionForState(inflight.state)),
        ));
      }
      return Ok(EnqueuedTaskOutcome(
        taskId: inflight.id,
        outcome: const TaskExecutionOutcome(
            disposition: TaskDisposition.blockedByActiveIntent),
      ));
    }

    // 2. 歧义重启提升（在 pathTasks 全体上找，不限 blocking）
    final ambiguous = pathTasks
        .where((t) =>
            t.state == TransferState.RestartRequired &&
            _hasAmbiguousRemoteWriteResult(t))
        .firstOrNull;
    if (ambiguous != null) {
      final promoted = await _promoteRestartToVerifying(ambiguous);
      final current = promoted ?? ambiguous;
      return Ok(EnqueuedTaskOutcome(
        taskId: current.id,
        outcome: const TaskExecutionOutcome(
            disposition: TaskDisposition.verifyingRemote),
      ));
    }

    // 3. 同意图去重
    final duplicate =
        blocking.where((t) => _sameTransferIntent(t, task)).firstOrNull;
    if (duplicate != null) {
      await _pumpLoop();
      return Ok(EnqueuedTaskOutcome(
        taskId: duplicate.id,
        outcome: TaskExecutionOutcome(
            disposition: _dispositionForState(duplicate.state)),
      ));
    }

    // 4. 重规划（阻塞中的首个）
    if (blocking.isNotEmpty) {
      final replanned = await _replanTask(blocking.first, task);
      if (replanned == null) {
        return Err(const GenericError(
            message: '任务重规划期间状态已变化，请等待下次同步'));
      }
      final outcome = await _runAndAwaitOutcome(replanned);
      return Ok(EnqueuedTaskOutcome(taskId: replanned.id, outcome: outcome));
    }

    // 5. 普通 RestartRequired 重规划
    final restart = pathTasks
        .where((t) => t.state == TransferState.RestartRequired)
        .firstOrNull;
    if (restart != null) {
      final replanned = await _replanTask(restart, task);
      if (replanned == null) {
        return Err(const GenericError(
            message: '任务重规划期间状态已变化，请等待下次同步'));
      }
      final outcome = await _runAndAwaitOutcome(replanned);
      return Ok(EnqueuedTaskOutcome(taskId: replanned.id, outcome: outcome));
    }

    // 6. Failed 路径屏障（保留可见错误供显式重试）
    final failed = pathTasks
        .where((t) => t.state == TransferState.Failed)
        .firstOrNull;
    if (failed != null) {
      return Ok(EnqueuedTaskOutcome(
        taskId: failed.id,
        outcome: const TaskExecutionOutcome(
            disposition: TaskDisposition.blockedByActiveIntent),
      ));
    }

    // 7. 插入新行并执行
    final inserted = await _transferService.enqueue(task);
    if (inserted.isErr) return Err((inserted as Err).error);
    final stored = (inserted as Ok<TransferTask>).value;
    await _publishSnapshot();
    final outcome = await _runAndAwaitOutcome(stored);
    return Ok(EnqueuedTaskOutcome(taskId: stored.id, outcome: outcome));
  }

  /// 阻塞态判定（对齐 Rust `is_path_blocking_state`；
  /// 不含 RestartRequired/Completed/Failed/Canceled）。
  bool _isPathBlockingState(TransferState state) {
    return state == TransferState.Pending ||
        state == TransferState.Running ||
        state == TransferState.WaitingForNetwork ||
        state == TransferState.BackingOff ||
        state == TransferState.VerifyingRemote;
  }

  /// 歧义远端写入判定：Create/Update 且已持久化远端结果 ID。
  bool _hasAmbiguousRemoteWriteResult(TransferTask task) {
    return (task.operation == TransferOperation.Create ||
            task.operation == TransferOperation.Update) &&
        _hasPersistedRemoteResult(task);
  }

  /// 同意图判定（对齐 Rust `same_transfer_intent`）。
  bool _sameTransferIntent(TransferTask left, TransferTask right) {
    if (left.relativePath != right.relativePath ||
        left.localPath != right.localPath ||
        left.name != right.name ||
        left.direction != right.direction ||
        left.operation != right.operation ||
        left.fileId != right.fileId ||
        left.totalSize != right.totalSize) {
      return false;
    }
    switch (left.operation) {
      case TransferOperation.Create:
      case TransferOperation.Update:
        if (left.parentFileId != right.parentFileId ||
            left.sourceMtime != right.sourceMtime ||
            left.sourceSize != right.sourceSize) {
          return false;
        }
        if (left.operation == TransferOperation.Update &&
            left.expectedCloudEditedTime != right.expectedCloudEditedTime) {
          return false;
        }
        return true;
      case TransferOperation.Download:
      case TransferOperation.DownloadUpdate:
        return left.parentFileId == right.parentFileId &&
            left.expectedCloudEditedTime == right.expectedCloudEditedTime;
      default:
        return false;
    }
  }

  /// 活动态 → 调度去向（对齐 Rust `active_task_disposition`）。
  TaskDisposition _dispositionForState(TransferState state) {
    return switch (state) {
      TransferState.Pending => TaskDisposition.pending,
      TransferState.Running => TaskDisposition.running,
      TransferState.WaitingForNetwork => TaskDisposition.waitingForNetwork,
      TransferState.BackingOff => TaskDisposition.backingOff,
      TransferState.VerifyingRemote => TaskDisposition.verifyingRemote,
      TransferState.RestartRequired => TaskDisposition.restartRequired,
      _ => TaskDisposition.completed,
    };
  }

  /// 重规划任务（对齐 Rust `replan_task`）：
  /// → RestartRequired → Pending（清错误/远端结果）→ 裸 SQL 覆写意图列 →
  /// sync_items SYNCING 回写。
  Future<TransferTask?> _replanTask(
    TransferTask current,
    TransferTask replacement,
  ) async {
    var cur = current;
    if (cur.state != TransferState.RestartRequired) {
      final restarted = await _transition(
        cur,
        TransferState.RestartRequired,
        const TransferPatch(
          errorKind: SetPatch(TransferErrorKind.LocalChanged),
          errorMessage: SetPatch('新的 planner intent 已取代尚未执行的旧任务'),
          nextRetryAt: ClearPatch(),
          finishedAt: ClearPatch(),
        ),
      );
      if (restarted == null) return null;
      cur = restarted;
    }
    final sessionUrl = replacement.sessionUrl;
    final pending = await _transition(
      cur,
      TransferState.Pending,
      TransferPatch(
        errorKind: const ClearPatch(),
        errorMessage: const ClearPatch(),
        nextRetryAt: const ClearPatch(),
        finishedAt: const ClearPatch(),
        remoteResultFileId: const ClearPatch(),
        sessionUrl: sessionUrl != null
            ? SetPatch(sessionUrl)
            : const ClearPatch(),
        transferred: replacement.transferred,
        resumeOffset: replacement.resumeOffset,
        attemptCount: replacement.attemptCount,
      ),
    );
    if (pending == null) return null;
    final result =
        await _transferService.overwriteReplanIntent(pending, replacement);
    if (result.isErr) {
      AppLogger.w('任务 ${pending.id} 重规划覆写失败: ${(result as Err).error}');
      return null;
    }
    final overwritten = (result as Ok<TransferTask?>).value;
    if (overwritten == null) return null;
    // sync_items SYNCING 回写（无旧状态条件）
    try {
      await _syncHooks?.onTaskReplanned(overwritten);
    } catch (e) {
      AppLogger.w('重规划后 SYNCING 回写失败（忽略）: $e');
    }
    await _publishSnapshot();
    return overwritten;
  }

  /// 执行任务并等待调度去向（入队仲裁路径专用）。
  Future<TaskExecutionOutcome> _runAndAwaitOutcome(TransferTask task) async {
    final completer = Completer<TaskExecutionOutcome>();
    _outcomeWatchers[task.id] = completer;
    _track(task, () => _runExpected(task));
    await _pumpLoop();
    try {
      return await completer.future;
    } finally {
      _outcomeWatchers.remove(task.id);
    }
  }

  /// 完结入队仲裁等待器（成功去向）。
  void _completeOutcome(int taskId, TaskExecutionOutcome outcome) {
    final watcher = _outcomeWatchers.remove(taskId);
    if (watcher != null && !watcher.isCompleted) {
      watcher.complete(outcome);
    }
  }

  /// 完结入队仲裁等待器（失败：对齐 Rust settle_error 的 Err 路径）。
  void _failOutcome(int taskId, Object error) {
    final watcher = _outcomeWatchers.remove(taskId);
    if (watcher != null && !watcher.isCompleted) {
      watcher.completeError(error);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 引擎恢复接缝（对齐 Rust recovery.rs 的公开面）
  // ═══════════════════════════════════════════════════════════════════

  /// 每周期批量提升歧义重启（对齐 Rust `promote_ambiguous_restarts`）。
  Future<int> promoteAmbiguousRestarts() async {
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    var promoted = 0;
    for (final task in active) {
      if (task.state == TransferState.RestartRequired &&
          _hasAmbiguousRemoteWriteResult(task)) {
        final result = await _promoteRestartToVerifying(task);
        if (result != null) promoted++;
      }
    }
    return promoted;
  }

  /// 恢复到期远端核验（对齐 Rust `resume_verifying`；返回核验后到达
  /// Completed 的任务数）。
  Future<int> resumeVerifying() async {
    if (!_isOnline()) return 0;
    final now = _nowMs();
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    final scheduled = <TransferTask>[];
    for (final task in active) {
      if (task.state != TransferState.VerifyingRemote) continue;
      if (_inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && _activePaths.contains(rel)) continue;
      final dueAt = task.nextRetryAt;
      if (dueAt != null && dueAt > now) continue;
      _track(task, () => _resumeVerifyingTask(task));
      scheduled.add(task);
    }
    return _awaitAndCountCompleted(scheduled);
  }

  /// 恢复等待网络任务（对齐 Rust `resume_waiting`）。
  Future<int> resumeWaiting() async {
    if (!_isOnline()) return 0;
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    final scheduled = <TransferTask>[];
    for (final task in active) {
      if (task.state != TransferState.WaitingForNetwork) continue;
      if (_inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && _activePaths.contains(rel)) continue;
      _track(task, () => _runExpected(task));
      scheduled.add(task);
    }
    return _awaitAndCountCompleted(scheduled);
  }

  /// 恢复到期退避任务（对齐 Rust `resume_due_backoff`）。
  Future<int> resumeDueBackoff() async {
    if (!_isOnline()) return 0;
    final now = _nowMs();
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    final scheduled = <TransferTask>[];
    for (final task in active) {
      if (task.state != TransferState.BackingOff) continue;
      if (_inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && _activePaths.contains(rel)) continue;
      final dueAt = task.nextRetryAt;
      if (dueAt != null && dueAt > now) continue;
      _track(task, () => _runExpected(task));
      scheduled.add(task);
    }
    return _awaitAndCountCompleted(scheduled);
  }

  /// 等待调度任务收尾并统计到达 Completed 的数量。
  Future<int> _awaitAndCountCompleted(List<TransferTask> scheduled) async {
    if (scheduled.isEmpty) return 0;
    final futures = <Future<void>>[
      for (final task in scheduled)
        if (_inFlight[task.id] != null) _inFlight[task.id]!,
    ];
    if (futures.isNotEmpty) {
      await Future.wait(futures, eagerError: false);
    }
    var completed = 0;
    for (final task in scheduled) {
      final fresh = (await _transferService.getTaskById(task.id))
          .unwrapOr(null);
      if (fresh?.state == TransferState.Completed) completed++;
    }
    return completed;
  }

  /// BackingOff/VerifyingRemote 的最小 next_retry_at（backoff 调度器用，
  /// 对齐 Rust `next_backoff_deadline_ms`）。
  Future<int?> nextBackoffDeadlineMs() async {
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    int? min;
    for (final task in active) {
      if (task.state != TransferState.BackingOff &&
          task.state != TransferState.VerifyingRemote) {
        continue;
      }
      final at = task.nextRetryAt;
      if (at == null) continue;
      if (min == null || at < min) min = at;
    }
    return min;
  }

  /// 全部 Failed 任务（引擎 RETRY 周期用）。
  Future<List<TransferTask>> getFailedTasks() async {
    return (await _transferService.getFailedTasks()).unwrapOr([]);
  }

  /// 按 id 加载任务（引擎单任务重试的状态复核用）。
  Future<TransferTask?> getTask(int taskId) async {
    return (await _transferService.getTaskById(taskId)).unwrapOr(null);
  }

  /// 清除已完成任务（对齐 Rust `transfer_clear_completed`）。
  Future<AppResult<int>> clearCompleted() async {
    final result = await _transferService.clearCompleted();
    await _publishSnapshot();
    return result;
  }

  /// 清除已失败任务（对齐 Rust `transfer_clear_failed`）。
  Future<AppResult<int>> clearFailed() async {
    final result = await _transferService.clearFailed();
    await _publishSnapshot();
    return result;
  }

  /// 清除已结束任务（对齐 Rust `transfer_clear_finished`：Completed + Failed）。
  Future<AppResult<int>> clearFinished() async {
    final result = await _transferService.clearFinished();
    await _publishSnapshot();
    return result;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 并发调度
  // ═══════════════════════════════════════════════════════════════════

  /// 当前并发槽位数（clamp 1-20，默认 6）。
  Future<int> _concurrency() async {
    final configured = await _concurrencyProvider?.call() ?? 6;
    return configured.clamp(1, 20);
  }

  /// 调度泵：空槽时按「到期核验 → 可执行任务（created_at FIFO）」准入。
  ///
  /// 重入安全：泵执行期间的泵请求合并为补泵轮次，全程在同一 future 内完成。
  Future<void> _pumpLoop() async {
    if (!_started) return;
    if (_pumping) {
      _pumpAgain = true;
      return;
    }
    _pumping = true;
    try {
      do {
        _pumpAgain = false;
        while (await _admitNext()) {
          // 持续准入直到无槽位或无候选
        }
      } while (_pumpAgain && _started);
    } finally {
      _pumping = false;
    }
  }

  /// 尝试准入一个任务；有任务被拉起返回 true。
  Future<bool> _admitNext() async {
    final slots = (await _concurrency()) - _inFlight.length;
    if (slots <= 0) return false;
    final online = _isOnline();
    final now = _nowMs();
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    bool pathBusy(TransferTask t) =>
        t.relativePath != null && _activePaths.contains(t.relativePath);
    bool idle(TransferTask t) => !_inFlight.containsKey(t.id);

    // 1. 到期远端核验（对齐 resume_verifying：离线整体跳过；next_retry_at 空视为到期）
    if (online) {
      for (final task in active) {
        if (task.state != TransferState.VerifyingRemote) continue;
        if (!idle(task) || pathBusy(task)) continue;
        final dueAt = task.nextRetryAt;
        if (dueAt != null && dueAt > now) continue;
        _track(task, () => _resumeVerifyingTask(task));
        return true;
      }
    }

    // 2. 可执行任务（对齐 Rust 引擎恢复顺序：
    //    resume_waiting → resume_due_backoff → 新任务；各趟内保持 created_at FIFO）
    bool due(TransferTask t) {
      final dueAt = t.nextRetryAt;
      // next_retry_at 为空视为到期（执行链会按缺校验拒绝重放）
      return dueAt == null || dueAt <= now;
    }

    if (online) {
      for (final task in active) {
        if (task.state != TransferState.WaitingForNetwork) continue;
        if (!idle(task) || pathBusy(task)) continue;
        _track(task, () => _runExpected(task));
        return true;
      }
      for (final task in active) {
        if (task.state != TransferState.BackingOff || !due(task)) continue;
        if (!idle(task) || pathBusy(task)) continue;
        _track(task, () => _runExpected(task));
        return true;
      }
    }
    for (final task in active) {
      if (task.state != TransferState.Pending) continue;
      if (!idle(task) || pathBusy(task)) continue;
      // 离线时也准入：执行链按 Rust 语义转 WaitingForNetwork
      _track(task, () => _runExpected(task));
      return true;
    }
    return false;
  }

  /// 登记在途任务：占用槽位与同路径屏障，收尾后发布快照并补泵。
  void _track(TransferTask task, Future<void> Function() body) {
    final rel = task.relativePath;
    if (rel != null) _activePaths.add(rel);
    final done = Completer<void>();
    _inFlight[task.id] = done.future;
    unawaited(() async {
      try {
        await body();
      } catch (e, st) {
        AppLogger.e('任务 ${task.id} 执行未捕获异常', e, st);
      } finally {
        _inFlight.remove(task.id);
        if (rel != null) _activePaths.remove(rel);
        try {
          await _publishSnapshot();
          await _pumpLoop();
        } finally {
          done.complete();
        }
      }
    }());
  }

  // ═══════════════════════════════════════════════════════════════════
  // 执行主链（对齐 Rust run_expected）
  // ═══════════════════════════════════════════════════════════════════

  /// 执行单个可运行任务（Pending/WaitingForNetwork/BackingOff）。
  ///
  /// [runBackendPreflight] 对齐 Rust `run_expected` 同名参数：
  /// 手动 retry 已完成一次后端前置校验，链路内不再重复执行。
  Future<void> _runExpected(
    TransferTask current, {
    bool runBackendPreflight = true,
  }) async {
    final state = current.state;
    if (state != TransferState.Pending &&
        state != TransferState.WaitingForNetwork &&
        state != TransferState.BackingOff) {
      AppLogger.w('任务 ${current.id} 状态 ${state.name} 不可执行');
      _failOutcome(current.id,
          AppError.generic('任务状态 ${state.name} 不可执行'));
      await _publishSnapshot();
      return;
    }
    if (state == TransferState.BackingOff && current.nextRetryAt == null) {
      await _persistPreflightRejection(
        current,
        const PreflightFailure.validation('退避任务缺少 next_retry_at，拒绝立即重放'),
      );
      _failOutcome(current.id,
          AppError.generic('退避任务缺少 next_retry_at，拒绝立即重放'));
      return;
    }
    // 静态前置校验
    try {
      await _validateStatic(current);
    } on PreflightFailure catch (failure) {
      await _persistPreflightRejection(current, failure);
      if (failure.target == TransferState.Failed) {
        _failOutcome(current.id, AppError.generic(failure.message));
      } else {
        _completeOutcome(
            current.id,
            TaskExecutionOutcome(
                disposition: _dispositionForState(failure.target)));
      }
      return;
    }
    // 在线门控（离线：Pending → WaitingForNetwork；其余停留）
    if (!_isOnline()) {
      if (state == TransferState.Pending) {
        await _transitionFailure(
          current,
          TransferState.WaitingForNetwork,
          TransferErrorKind.Network,
          '网络不可用，等待恢复',
        );
        _completeOutcome(
            current.id,
            const TaskExecutionOutcome(
                disposition: TaskDisposition.waitingForNetwork));
      } else {
        _completeOutcome(
            current.id,
            TaskExecutionOutcome(
                disposition: _dispositionForState(state)));
        await _publishSnapshot();
      }
      return;
    }
    // 退避到期检查（对齐 Rust notify_rejection：早退也发布一次快照）
    if (state == TransferState.BackingOff &&
        (current.nextRetryAt ?? 0) > _nowMs()) {
      _completeOutcome(
          current.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.backingOff));
      await _publishSnapshot();
      return;
    }
    // 后端前置校验
    if (runBackendPreflight) {
      try {
        await _operations.preflight(current);
      } on BackendPreflightFailure catch (failure) {
        await _persistPreflightRejection(
          current,
          PreflightFailure(
            target: failure.target,
            kind: failure.kind,
            message: failure.message,
          ),
        );
        if (failure.target == TransferState.Failed) {
          _failOutcome(current.id, AppError.generic(failure.message));
        } else {
          _completeOutcome(
              current.id,
              TaskExecutionOutcome(
                  disposition: _dispositionForState(failure.target)));
        }
        return;
      }
    }
    // Running 仲裁（同路径排他 + 歧义重启提升）
    final running = await _transitionToRunningOrBlock(current);
    if (running == null) {
      _completeOutcome(
          current.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.blockedByActiveIntent));
      return;
    }

    // 执行传输
    final progress = _progressCallbacks(running);
    try {
      final outcome = await _operations.execute(running, progress);
      // 对齐 progress.ensure_current：任务被并发推进后忽略过期回调结果
      if (!await _ensureCurrent(running)) {
        _failOutcome(running.id, AppError.generic('传输任务状态已变化'));
        return;
      }
      await _settleOutcome(running, outcome);
      _completeOutcome(running.id, outcome);
    } on TaskRestartRequired catch (e) {
      await _transitionFailure(
        running,
        TransferState.RestartRequired,
        TransferErrorKind.LocalChanged,
        e.message,
      );
      _completeOutcome(
          running.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.restartRequired));
    } on TaskAppError catch (e) {
      await _settleError(running, e.error);
    } catch (e) {
      await _settleError(running, AppError.generic('$e'));
    }
  }

  /// 确认任务仍指向同一 Running 修订（对齐 Rust `ensure_current`）。
  Future<bool> _ensureCurrent(TransferTask running) async {
    final fresh =
        (await _transferService.getTaskById(running.id)).unwrapOr(null);
    if (fresh == null ||
        fresh.stateRevision != running.stateRevision ||
        fresh.state != TransferState.Running) {
      AppLogger.d('任务 ${running.id} 状态已变化，忽略过期回调');
      return false;
    }
    return true;
  }

  /// Running 仲裁（对齐 Rust `transition_to_running_or_block`）：
  /// 同路径存在 Running/VerifyingRemote 任务时被阻塞；
  /// 同路径含已持久远端结果的 RestartRequired 任务先全部提升为待核验。
  Future<TransferTask?> _transitionToRunningOrBlock(TransferTask current) async {
    final rel = current.relativePath;
    if (rel == null) {
      // 对齐 Rust：直接返回错误，任务停留原态（静态校验已拦截，实际不可达）
      AppLogger.w('任务 ${current.id} Running 仲裁缺少 relative_path');
      await _publishSnapshot();
      return null;
    }
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);
    var promotedAny = false;
    for (final candidate in active) {
      if (candidate.id == current.id || candidate.relativePath != rel) continue;
      if (candidate.state == TransferState.Running ||
          candidate.state == TransferState.VerifyingRemote) {
        AppLogger.d('任务 ${current.id} 被同路径活动意图 ${candidate.id} 阻塞');
        return null;
      }
      if (candidate.state == TransferState.RestartRequired &&
          _hasPersistedRemoteResult(candidate)) {
        await _promoteRestartToVerifying(
          candidate,
          message: '远端结果 ID 已存在；Running 仲裁禁止重放并等待核验',
        );
        promotedAny = true;
      }
    }
    // 有歧义重启被提升时，本任务等待核验结果后再调度
    if (promotedAny) return null;
    return _transition(
      current,
      TransferState.Running,
      const TransferPatch.clearingError(),
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 结算（对齐 Rust settlement.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 按后端执行结果结算（成功核验 / 延迟状态持久化）。
  Future<void> _settleOutcome(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    switch (outcome.disposition) {
      case TaskDisposition.completed:
        try {
          await _validateSuccessOutcome(running, outcome);
        } on PreflightFailure catch (failure) {
          // 上传且远端已返回资源 ID → 禁止直接重放，进入核验
          final remoteId = outcome.cloudFile?.id;
          final isUpload = running.operation == TransferOperation.Create ||
              running.operation == TransferOperation.Update;
          if (isUpload && remoteId != null && remoteId.trim().isNotEmpty) {
            await _transition(
              running,
              TransferState.VerifyingRemote,
              TransferPatch(
                errorKind: const SetPatch(TransferErrorKind.RemoteAmbiguous),
                errorMessage:
                    SetPatch('${failure.message}；远端已返回资源 ID，禁止直接重放'),
                remoteResultFileId: SetPatch(remoteId),
              ),
            );
          } else {
            // 对齐 Rust execution.rs：finished_at 保持 Keep；
            // 远端已返回资源 ID 时仍持久化 remote_result_file_id
            await _transition(
              running,
              failure.target,
              TransferPatch(
                errorKind: SetPatch(failure.kind),
                errorMessage: SetPatch(failure.message),
                remoteResultFileId: remoteId != null && remoteId.trim().isNotEmpty
                    ? SetPatch(remoteId)
                    : const KeepPatch(),
              ),
            );
          }
          return;
        }
        await _settleSuccess(running, outcome);
      case TaskDisposition.verifyingRemote:
        await _transition(
          running,
          TransferState.VerifyingRemote,
          TransferPatch(
            errorKind: const SetPatch(TransferErrorKind.RemoteAmbiguous),
            errorMessage:
                const SetPatch('远端写入已返回资源 ID，但完整元数据尚未确认'),
            nextRetryAt: SetPatch(_nowMs() + verifyInitialDelayMs),
            remoteResultFileId: outcome.cloudFile != null
                ? SetPatch(outcome.cloudFile!.id)
                : const KeepPatch(),
          ),
        );
      case TaskDisposition.waitingForNetwork:
        await _transition(
          running,
          TransferState.WaitingForNetwork,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.Network),
            errorMessage: SetPatch('后端请求等待网络恢复'),
          ),
        );
      case TaskDisposition.restartRequired:
        await _transition(
          running,
          TransferState.RestartRequired,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.LocalChanged),
            errorMessage: SetPatch('本地源已变化，需要重新规划'),
          ),
        );
      case TaskDisposition.pending ||
            TaskDisposition.running ||
            TaskDisposition.blockedByActiveIntent ||
            TaskDisposition.backingOff:
        await _settleError(
          running,
          AppError.generic(
              '后端返回缺少可持久化恢复条件的状态 ${outcome.disposition.name}'),
        );
    }
  }

  /// 根据错误分类持久化失败或恢复状态（对齐 Rust `settle_error`）。
  Future<void> _settleError(TransferTask running, AppError error) async {
    final operation = running.operation;
    if (operation == null) {
      await _transitionFailure(
        running,
        TransferState.Failed,
        TransferErrorKind.Validation,
        '任务缺少 operation',
      );
      return;
    }
    final classified = classifyTransferError(
      error,
      RecoveryContext(
        operation: operation,
        attemptCount:
            running.attemptCount < 0 ? 0 : running.attemptCount,
        nowMs: _nowMs(),
        jitterMs: _jitterMs(),
        authAlreadyReplayed: false,
        maxAttempts: _maxAttempts,
      ),
    );
    final attempts =
        running.attemptCount + (classified.consumesRetryBudget ? 1 : 0);
    // 请求级网络失败边沿上报（对齐 Rust engine/publication.rs：
    // 仅「等待计数边沿增加且当前在线」时上报；离线期间由 NetGuard 探测主导）
    if (classified.decision is WaitForNetworkDecision && _isOnline()) {
      _onRequestNetworkFailure?.call();
    }
    final (state, nextRetryAt) = switch (classified.decision) {
      WaitForNetworkDecision() => (
          TransferState.WaitingForNetwork,
          const ClearPatch<int>(),
        ),
      BackoffDecision(:final nextRetryAt) => (
          TransferState.BackingOff,
          SetPatch<int>(nextRetryAt),
        ),
      VerifyRemoteDecision() => (
          TransferState.VerifyingRemote,
          SetPatch<int>(_nowMs() + verifyInitialDelayMs),
        ),
      // DriveClient 负责唯一一次带认证重放；首次 401 不由 runner 盲目重放
      RefreshAuthDecision() => (
          TransferState.Failed,
          const ClearPatch<int>(),
        ),
      FailDecision() => (
          TransferState.Failed,
          const ClearPatch<int>(),
        ),
    };
    final updated = await _transition(
      running,
      state,
      TransferPatch(
        errorKind: SetPatch(classified.kind),
        errorMessage: SetPatch('$error'),
        nextRetryAt: nextRetryAt,
        finishedAt: state == TransferState.Failed
            ? SetPatch(_nowMs())
            : const ClearPatch(),
        attemptCount: attempts,
      ),
    );
    if (state == TransferState.Failed) {
      // 永久失败：sync_items FAILED 回写（仅旧状态白名单覆盖）
      if (updated != null) {
        try {
          await _syncHooks?.onTaskFailed(updated, '$error');
        } catch (e) {
          AppLogger.w('任务 ${running.id} FAILED 基线回写失败（忽略）: $e');
        }
      }
      _failOutcome(running.id, error);
    } else {
      _completeOutcome(
        running.id,
        TaskExecutionOutcome(disposition: _dispositionForState(state)),
      );
    }
  }

  /// 原子完成任务结算（对齐 Rust `settle_success` 的任务行部分；
  /// sync_items 基线结算属引擎任务接缝）。
  Future<void> _settleSuccess(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    final operation = running.operation;
    if (operation == null) {
      await _settleError(running, AppError.generic('任务缺少 operation'));
      return;
    }
    final String? resultFileId = switch (operation) {
      TransferOperation.Create ||
      TransferOperation.Update =>
        outcome.cloudFile?.id,
      TransferOperation.Download ||
      TransferOperation.DownloadUpdate =>
        running.fileId,
      _ => outcome.cloudFile?.id ?? running.fileId,
    };
    // 先结算 sync_items 基线（对齐 Rust settle_success 同事务语义的最佳近似：
    // 基线结算失败禁止完成任务行，进入恢复路径）
    final hooks = _syncHooks;
    if (hooks != null) {
      try {
        await hooks.onTaskCommitted(running, outcome);
      } catch (e) {
        AppLogger.w('任务 ${running.id} 基线结算失败，进入恢复路径: $e');
        const message = '后端已完成，但本地同步基线结算失败';
        switch (operation) {
          case TransferOperation.Create:
          case TransferOperation.Update:
            await _transition(
              running,
              TransferState.VerifyingRemote,
              TransferPatch(
                errorKind: const SetPatch(TransferErrorKind.RemoteAmbiguous),
                errorMessage: const SetPatch(message),
                remoteResultFileId: outcome.cloudFile != null
                    ? SetPatch(outcome.cloudFile!.id)
                    : const KeepPatch(),
              ),
            );
          case TransferOperation.Download:
          case TransferOperation.DownloadUpdate:
            await _transition(
              running,
              TransferState.RestartRequired,
              const TransferPatch(
                errorKind: SetPatch(TransferErrorKind.Unknown),
                errorMessage: SetPatch(message),
              ),
            );
          default:
            break;
        }
        return;
      }
    }
    final completed = await _transition(
      running,
      TransferState.Completed,
      TransferPatch.clearingError(
        finishedAt: SetPatch(_nowMs()),
        remoteResultFileId: resultFileId != null
            ? SetPatch(resultFileId)
            : const KeepPatch(),
        transferred: running.totalSize,
      ),
    );
    if (completed != null) return;
    // Completed 迁移失败（CAS 冲突或 DB 错误）：
    // 对齐 Rust recover_success_settlement_failure——后端已完成但结算未落地，
    // 上传禁止盲目重放（转远端核验），下载回 planner 重新规划。
    const message = '后端已完成，但本地同步基线结算失败（迁移被拒绝或写入失败）';
    switch (operation) {
      case TransferOperation.Create:
      case TransferOperation.Update:
        await _transition(
          running,
          TransferState.VerifyingRemote,
          TransferPatch(
            errorKind: const SetPatch(TransferErrorKind.RemoteAmbiguous),
            errorMessage: const SetPatch(message),
            remoteResultFileId: outcome.cloudFile != null
                ? SetPatch(outcome.cloudFile!.id)
                : const KeepPatch(),
          ),
        );
      case TransferOperation.Download:
      case TransferOperation.DownloadUpdate:
        await _transition(
          running,
          TransferState.RestartRequired,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.Unknown),
            errorMessage: SetPatch(message),
          ),
        );
      default:
        // Flutter 扩展操作：迁移失败时任务滞留 Running，由启动恢复收敛
        AppLogger.w('任务 ${running.id} 完成结算迁移失败，滞留 Running 等待启动恢复');
    }
  }

  /// 校验完成结果是否可安全结算（对齐 Rust `validate_success_outcome`）。
  Future<void> _validateSuccessOutcome(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    final operation = running.operation;
    if (operation == null) {
      throw const PreflightFailure.validation('成功核验缺少 operation');
    }
    switch (operation) {
      case TransferOperation.Create:
      case TransferOperation.Update:
        final cloud = outcome.cloudFile;
        if (cloud == null) {
          throw const PreflightFailure.remoteAmbiguous('上传结果缺少远端资源');
        }
        if (cloud.id.trim().isEmpty ||
            cloud.name.trim().isEmpty ||
            cloud.name != running.name ||
            cloud.editedTime == null ||
            cloud.size != (running.sourceSize ?? -1) ||
            (operation == TransferOperation.Update &&
                running.fileId != cloud.id)) {
          throw const PreflightFailure.remoteAmbiguous('上传结果元数据不完整或大小不一致');
        }
      case TransferOperation.Download:
      case TransferOperation.DownloadUpdate:
        final localPath = running.localPath;
        if (localPath == null) {
          throw const PreflightFailure.validation('成功核验缺少本地路径');
        }
        final stat = await FileStat.stat(localPath);
        if (stat.type != FileSystemEntityType.file) {
          throw const PreflightFailure.localChanged('成功核验时下载文件不存在或不是普通文件');
        }
        if (running.expectedCloudEditedTime == null ||
            stat.size != running.totalSize) {
          throw const PreflightFailure.localChanged('下载结果大小或云端版本不匹配');
        }
      case TransferOperation.Delete ||
            TransferOperation.Move ||
            TransferOperation.Rename ||
            TransferOperation.CreateFolder:
        // Flutter 扩展操作：files API 已完成写后验证（身份/名称/recycled）
        break;
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 远端核验（对齐 Rust recovery.rs resume_verifying_task）
  // ═══════════════════════════════════════════════════════════════════

  /// 核验并结算一个远端结果不确定的任务。
  Future<void> _resumeVerifyingTask(TransferTask task) async {
    final RemoteVerification verification;
    try {
      verification = await _operations.verifyRemote(task);
    } catch (e) {
      // 核验暂不可用：保留歧义状态，稍后重试
      AppLogger.w('任务 ${task.id} 远端写入核验暂不可用，保留歧义状态: $e');
      await _patchInState(
        task,
        TransferState.VerifyingRemote,
        TransferPatch(
          errorMessage: SetPatch('远端核验暂不可用：$e'),
          nextRetryAt: SetPatch(_nowMs() + verifyUnavailableDelayMs),
        ),
      );
      return;
    }
    switch (verification) {
      case RemoteCommitted(:final file):
        final outcome = TaskExecutionOutcome(
          cloudFile: file,
          disposition: TaskDisposition.completed,
        );
        try {
          await _validateSuccessOutcome(task, outcome);
        } on PreflightFailure catch (failure) {
          final patch = TransferPatch(
            errorKind: SetPatch(failure.kind),
            errorMessage:
                SetPatch('远端写入已确认，但结果仍无法安全结算：${failure.message}'),
            nextRetryAt: failure.target == TransferState.VerifyingRemote
                ? SetPatch(_nowMs() + verifyAmbiguousDelayMs)
                : const ClearPatch(),
            remoteResultFileId: SetPatch(file.id),
          );
          if (failure.target == TransferState.VerifyingRemote) {
            await _patchInState(task, TransferState.VerifyingRemote, patch);
          } else {
            await _transition(task, failure.target, patch);
          }
          return;
        }
        await _settleSuccess(task, outcome);
      case RemoteNotCommitted():
        final sessionExpired =
            task.errorKind == TransferErrorKind.SessionExpired;
        final restart = await _transition(
          task,
          TransferState.RestartRequired,
          TransferPatch(
            errorKind: SetPatch(
              sessionExpired
                  ? TransferErrorKind.SessionExpired
                  : TransferErrorKind.RemoteAmbiguous,
            ),
            errorMessage: SetPatch(
              sessionExpired
                  ? '远端核验确认写入未提交，已清理失效会话，可以安全新建会话'
                  : '远端核验确认写入未提交，可以安全重放',
            ),
            nextRetryAt: const ClearPatch(),
            finishedAt: const ClearPatch(),
            remoteResultFileId: const ClearPatch(),
            clearUploadSession: sessionExpired,
          ),
        );
        if (restart == null) return;
        // 转 Pending 后由调度泵重新执行（对齐 Rust 链式 run_expected）
        await _transition(
          restart,
          TransferState.Pending,
          const TransferPatch.clearingError(),
        );
      case RemoteAmbiguous(:final message):
        // 保留会话过期标记，直至确定远端不存在结果
        final kind = task.errorKind == TransferErrorKind.SessionExpired
            ? TransferErrorKind.SessionExpired
            : TransferErrorKind.RemoteAmbiguous;
        await _patchInState(
          task,
          TransferState.VerifyingRemote,
          TransferPatch(
            errorKind: SetPatch(kind),
            errorMessage: SetPatch(message),
            nextRetryAt: SetPatch(_nowMs() + verifyAmbiguousDelayMs),
          ),
        );
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 崩溃恢复（对齐 Rust recovery.rs recover_startup）
  // ═══════════════════════════════════════════════════════════════════

  /// 启动恢复：歧义重启提升 → 同路径重复收敛 → 中断 Running 行按操作分流。
  Future<void> _recoverStartup() async {
    final active = (await _transferService.getActiveTasks()).unwrapOr([]);

    // 1. 含远端结果 ID 的 RestartRequired → VerifyingRemote（promote_ambiguous_restarts）
    for (final task in active) {
      if (task.state == TransferState.RestartRequired &&
          _hasPersistedRemoteResult(task)) {
        await _promoteRestartToVerifying(task);
      }
    }

    // 2. Pending + Running 行按同路径分组收敛（最新一条胜出）
    final tasks = active
        .where((t) =>
            t.state == TransferState.Pending || t.state == TransferState.Running)
        .toList()
      ..sort((a, b) {
        final byCreated = b.createdAt.compareTo(a.createdAt);
        return byCreated != 0 ? byCreated : b.id.compareTo(a.id);
      });
    final selected = <TransferTask>[];
    final grouped = <String, List<TransferTask>>{};
    for (final task in tasks) {
      final rel = task.relativePath;
      if (rel == null) {
        selected.add(task);
      } else {
        grouped.putIfAbsent(rel, () => []).add(task);
      }
    }
    for (final samePath in grouped.values) {
      final hasRunningRemoteWrite = samePath.any(
        (t) =>
            t.state == TransferState.Running &&
            (t.operation == TransferOperation.Create ||
                t.operation == TransferOperation.Update),
      );
      if (hasRunningRemoteWrite) {
        for (final task in samePath) {
          await _suppressStartupDuplicate(task);
        }
        continue;
      }
      // 组内已按 created_at 倒序，首条为最新意图
      selected.add(samePath.first);
      for (final task in samePath.skip(1)) {
        await _suppressStartupDuplicate(task);
      }
    }

    // 3. 逐任务恢复中断行
    for (final task in selected) {
      try {
        await _recoverStartupTask(task);
      } catch (e, st) {
        AppLogger.e('任务 ${task.id} 启动恢复失败，继续处理其他任务', e, st);
      }
    }
  }

  /// 抑制启动期同路径旧任务（对齐 suppress_startup_duplicate）。
  Future<void> _suppressStartupDuplicate(TransferTask task) async {
    if (task.state == TransferState.Running &&
        (task.operation == TransferOperation.Create ||
            task.operation == TransferOperation.Update)) {
      await _transitionFailure(
        task,
        TransferState.VerifyingRemote,
        TransferErrorKind.RemoteAmbiguous,
        '启动恢复发现同路径多个活动任务；旧远端写入等待核验',
      );
      return;
    }
    await _transitionFailure(
      task,
      TransferState.RestartRequired,
      task.state == TransferState.Running
          ? TransferErrorKind.SessionExpired
          : TransferErrorKind.LocalChanged,
      '启动恢复仅保留同路径最新任务，旧任务等待重新规划',
    );
  }

  /// 恢复单个启动期任务行（Pending 交给调度器；Running 按操作分流）。
  Future<void> _recoverStartupTask(TransferTask task) async {
    if (task.state == TransferState.Pending) return;
    switch (task.operation) {
      case TransferOperation.Create:
      case TransferOperation.Update:
        await _transitionFailure(
          task,
          TransferState.VerifyingRemote,
          TransferErrorKind.RemoteAmbiguous,
          '进程中断时远端写入结果不确定，等待核验',
        );
      case TransferOperation.Download:
      case TransferOperation.DownloadUpdate:
        try {
          await _validateStatic(task);
        } on PreflightFailure catch (failure) {
          await _persistPreflightRejection(task, failure);
          return;
        }
        // 下载断点以磁盘 .tmp 实际大小为准（对齐 recovery.rs durable_offset）
        final durable = await _durableDownloadOffset(task);
        final restart = await _transitionFailure(
          task,
          TransferState.RestartRequired,
          TransferErrorKind.SessionExpired,
          '进程中断，保留已验证下载断点并重新建立 Range 请求',
        );
        if (restart == null) return;
        // 对齐 Rust recovery.rs：Pending 补丁清错误但保留 next_retry_at（Keep）
        await _transition(
          restart,
          TransferState.Pending,
          TransferPatch(
            errorKind: const ClearPatch(),
            errorMessage: const ClearPatch(),
            finishedAt: const ClearPatch(),
            transferred: durable,
            resumeOffset: durable,
          ),
        );
      case TransferOperation.Delete ||
            TransferOperation.Move ||
            TransferOperation.Rename ||
            TransferOperation.CreateFolder:
        // Flutter 扩展操作：files API 写后验证保证可安全重放
        final restart = await _transitionFailure(
          task,
          TransferState.RestartRequired,
          TransferErrorKind.SessionExpired,
          '进程中断，远端写操作重新调度',
        );
        if (restart == null) return;
        await _transition(
          restart,
          TransferState.Pending,
          const TransferPatch.clearingError(),
        );
      case null:
        await _transitionFailure(
          task,
          TransferState.Failed,
          TransferErrorKind.Validation,
          '中断任务缺少合法 operation',
        );
    }
  }

  /// 读取下载断点的磁盘真值：.tmp 实际大小（不超过 totalSize），缺失为 0。
  Future<int> _durableDownloadOffset(TransferTask task) async {
    final localPath = task.localPath;
    if (localPath == null) return 0;
    try {
      final stat = await FileStat.stat(tmpPath(localPath));
      if (stat.type != FileSystemEntityType.file) return 0;
      return stat.size < 0
          ? 0
          : (stat.size > task.totalSize ? task.totalSize : stat.size);
    } catch (_) {
      return 0;
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 进度报告（对齐 Rust TaskProgressReporter）
  // ═══════════════════════════════════════════════════════════════════

  /// 为 Running 任务构造进度回调（节流持久化 + 顺序化写入 + 修订门禁）。
  TaskProgressCallbacks _progressCallbacks(TransferTask running) {
    var lastProgressMs = 0;
    // 顺序化进度写，避免并发回调乱序落库
    Future<void> writes = Future<void>.value();

    bool throttle() {
      final now = _nowMs();
      if (lastProgressMs != 0 && now - lastProgressMs < progressThrottleMs) {
        return false;
      }
      lastProgressMs = now;
      return true;
    }

    void enqueueWrite(Future<AppResult<void>> Function() write) {
      writes = writes.then((_) => write()).then((_) => _publishSnapshot(),
          onError: (Object e, StackTrace st) {
        AppLogger.d('忽略过期进度回调: $e');
      });
    }

    return TaskProgressCallbacks(
      totalSize: running.totalSize,
      onProgress: (transferred) {
        if (transferred < 0 || transferred > running.totalSize) return;
        if (!throttle()) return;
        enqueueWrite(() => _transferService.updateProgress(
              running.id,
              transferred,
              expectedRevision: running.stateRevision,
            ));
      },
      onDownloadProgress: (transferred) {
        if (transferred < 0 || transferred > running.totalSize) return;
        if (!throttle()) return;
        enqueueWrite(() => _transferService.updateProgress(
              running.id,
              transferred,
              resumeOffset: transferred,
              expectedRevision: running.stateRevision,
            ));
      },
      onResume: (serverId, uploadId, offset, sessionUrl) {
        if (offset < 0 || offset > running.totalSize) return;
        if (offset > 0 && sessionUrl.trim().isEmpty) return;
        // 会话轮换必须立即持久化，不受进度节流影响
        enqueueWrite(() => _transferService.updateResumeSession(
              running.id,
              serverId: serverId,
              uploadId: uploadId,
              resumeOffset: offset,
              sessionUrl: sessionUrl,
              expectedRevision: running.stateRevision,
            ));
      },
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 持久化迁移原语
  // ═══════════════════════════════════════════════════════════════════

  /// 校验任务静态条件（包装模块函数，注入挂载根与占位判定）。
  Future<void> _validateStatic(TransferTask task) async {
    await validateStaticTask(
      task,
      mountRoot: _mountRootProvider(),
      isPlaceholder: _isPlaceholder,
    );
  }

  /// 持久化常规任务状态迁移（CAS：id + from 状态 + revision）。
  Future<TransferTask?> _transition(
    TransferTask task,
    TransferState to,
    TransferPatch patch,
  ) async {
    final result = await _transferService.transition(
      task.id,
      task.state,
      to,
      patch: patch,
      expectedRevision: task.stateRevision,
    );
    if (result.isErr) {
      AppLogger.w('任务 ${task.id} 迁移 ${to.name} 失败: ${(result as Err).error}');
      return null;
    }
    final updated = (result as Ok<TransferTask?>).value;
    if (updated == null) {
      AppLogger.w('任务 ${task.id} 迁移 ${to.name} CAS 冲突（已被并发推进）');
      return null;
    }
    await _publishSnapshot();
    return updated;
  }

  /// 持久化带错误信息的任务状态迁移（对齐 Rust `transition_failure`）。
  Future<TransferTask?> _transitionFailure(
    TransferTask task,
    TransferState state,
    TransferErrorKind kind,
    String message,
  ) {
    return _transition(
      task,
      state,
      TransferPatch(
        errorKind: SetPatch(kind),
        errorMessage: SetPatch(message),
        finishedAt: state == TransferState.Failed
            ? SetPatch(_nowMs())
            : const ClearPatch(),
      ),
    );
  }

  /// 生命周期不变时更新错误与重试事实（对齐 Rust `patch_transfer_in_state`）。
  Future<TransferTask?> _patchInState(
    TransferTask task,
    TransferState expectedState,
    TransferPatch patch,
  ) async {
    final result = await _transferService.patchInState(
      task.id,
      expectedState,
      task.stateRevision,
      patch: patch,
    );
    if (result.isErr) {
      AppLogger.w('任务 ${task.id} 状态内补丁失败: ${(result as Err).error}');
      return null;
    }
    final updated = (result as Ok<TransferTask?>).value;
    if (updated != null) await _publishSnapshot();
    return updated;
  }

  /// 持久化前置校验拒绝结果（对齐 Rust `persist_preflight_rejection`）。
  Future<void> _persistPreflightRejection(
    TransferTask task,
    PreflightFailure failure,
  ) async {
    final patch = failure.patch(nowMs: _nowMs());
    if (task.state == TransferState.Failed &&
        failure.target == TransferState.Failed) {
      await _patchInState(task, TransferState.Failed, patch);
      return;
    }
    await _transition(task, failure.target, patch);
  }

  /// 将含歧义远端结果的重启任务提升为待核验（对齐 Rust
  /// `promote_restart_to_verifying`）。
  ///
  /// [message] 默认用启动恢复文案；Running 仲裁提升时传仲裁专用文案
  /// （对齐 Rust admission.rs 的「远端结果 ID 已存在…」）。
  Future<TransferTask?> _promoteRestartToVerifying(
    TransferTask task, {
    String message = '远端写入已返回资源 ID，禁止重放并等待核验',
  }) {
    return _transition(
      task,
      TransferState.VerifyingRemote,
      TransferPatch(
        errorKind: const SetPatch(TransferErrorKind.RemoteAmbiguous),
        errorMessage: SetPatch(message),
        nextRetryAt: const ClearPatch(),
        finishedAt: const ClearPatch(),
      ),
    );
  }

  /// 判断任务是否保存了非空远程结果 ID。
  bool _hasPersistedRemoteResult(TransferTask task) {
    final id = task.remoteResultFileId;
    return id != null && id.trim().isNotEmpty;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 快照发布（对齐 Rust publication.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 重算持久事实并广播完整队列快照（尽力而为，对齐 notify_best_effort）。
  Future<void> _publishSnapshot() async {
    try {
      final tasks = (await _transferService.getAllTasks()).unwrapOr([]);
      var activeCount = 0;
      for (final task in tasks) {
        if (task.state.isActive) activeCount++;
      }
      final snapshot = TransferQueueSnapshot(
        revision: ++_snapshotRevision,
        tasks: tasks,
        activeCount: activeCount,
      );
      _lastSnapshot = snapshot;
      if (!_snapshotCtrl.isClosed) _snapshotCtrl.add(snapshot);
    } catch (e) {
      AppLogger.w('任务状态变化后重算权威快照失败: $e');
    }
  }

  /// 发布上传失败通知（对齐 Rust `upload_failed` 事件负载）。
  void publishUploadFailure(UploadFailureNotice notice) {
    if (!_uploadFailureCtrl.isClosed) _uploadFailureCtrl.add(notice);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 测试钩子
  // ═══════════════════════════════════════════════════════════════════

  /// 测试用：执行一轮调度（到期核验 + 可执行任务准入）。
  @visibleForTesting
  Future<void> debugTick() => _pumpLoop();

  /// 测试用：等待全部在途任务（含结算收尾与链式调度）完成。
  @visibleForTesting
  Future<void> get idle async {
    while (_inFlight.isNotEmpty) {
      await Future.wait(_inFlight.values.toList());
    }
  }
}
