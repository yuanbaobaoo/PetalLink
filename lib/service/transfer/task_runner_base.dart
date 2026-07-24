// ignore_for_file: prefer_initializing_formals — 公开命名参数映射 protected 字段

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/task_runner_preflight.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';

/// 持久化传输任务执行器基类（对齐 Rust `src/sync/task_runner.rs` + `task_runner/`）。
///
/// 拆分自原 `task_runner.dart`：跨 mixin 共享的字段、helper 与生命周期/命令面/
/// pump loop/持久化/发布原语集中在基类；admission/execution/recovery 三大区域
/// 由各 mixin（`on TaskRunnerBase`）提供。被 mixin 调用的基类成员去掉 `_` 前缀
/// 并标 `@protected`（Dart `mixin on` 可访问基类非 private 成员）。
abstract class TaskRunnerBase {
  /// 任务持久化与 CAS 状态迁移
  @protected
  final TransferService transferService;

  /// 操作执行适配层
  @protected
  final TaskOperations operations;

  /// 在线判定（生产接 NetGuard.isOnline）
  @protected
  final bool Function() isOnline;

  /// 稳定网络转换流（生产接 NetGuard.transitions，可空）
  @protected
  final Stream<NetworkTransition>? netTransitions;

  /// 请求级网络失败边沿上报（生产接 NetGuard.reportRequestNetworkFailure，可空）
  @protected
  final void Function()? onRequestNetworkFailure;

  /// 当前毫秒时钟（测试注入）
  @protected
  final int Function() nowMs;

  /// 退避抖动毫秒（对齐 Rust runner 默认 0）
  @protected
  final int Function() jitterMs;

  /// 并发槽位提供（AppConfig.concurrency；clamp 1-20）
  @protected
  final Future<int> Function()? concurrencyProvider;

  /// 挂载根提供（静态前置校验用；未配置时校验按 Rust 语义拒绝执行）
  @protected
  final String? Function() mountRootProvider;

  /// 0 字节占位符判定（下载静态校验用，可空 = 保守视为非占位）
  @protected
  final Future<bool> Function(String path)? isPlaceholder;

  /// 调度 tick 周期（退避/核验到期粒度）
  @protected
  final Duration tickInterval;

  /// 单个任务允许的最大自动重试次数（对齐 Rust MAX_AUTOMATIC_ATTEMPTS）
  @protected
  final int maxAttempts;

  /// 进度持久化节流间隔（对齐 Rust PROGRESS_THROTTLE_MS）
  static const int progressThrottleMs = 500;

  /// VerifyingRemote 首次核验延迟（对齐 Rust settle_error 的 3s）
  static const int verifyInitialDelayMs = 3000;

  /// 核验结果仍歧义时的再核间隔（对齐 Rust 的 60s）
  static const int verifyAmbiguousDelayMs = 60000;

  /// 核验通道暂不可用时的再核间隔（对齐 Rust 的 15s）
  static const int verifyUnavailableDelayMs = 15000;

  TaskRunnerBase({
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
  })  : transferService = transferService,
        operations = operations,
        isOnline = isOnline ?? (() => true),
        netTransitions = netTransitions,
        onRequestNetworkFailure = onRequestNetworkFailure,
        nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch),
        jitterMs = jitterMs ?? (() => 0),
        concurrencyProvider = concurrencyProvider,
        mountRootProvider = mountRootProvider ?? (() => null),
        isPlaceholder = isPlaceholder,
        tickInterval = tickInterval,
        maxAttempts = maxAttempts;

  // ═══════════════════════════════════════════════════════════════════
  // 运行状态
  // ═══════════════════════════════════════════════════════════════════

  /// 是否已启动（start 后准入任务；stop 后不再准入）
  @protected
  bool started = false;

  /// 在途任务（taskId → 含结算收尾的执行 future）
  @protected
  final Map<int, Future<void>> inFlight = {};

  /// 在途任务占用的相对路径（同路径排他）
  @protected
  final Set<String> activePaths = {};

  /// 调度泵重入守卫
  @protected
  bool pumping = false;

  /// 泵执行期间又有泵请求
  @protected
  bool pumpAgain = false;

  /// 到期调度定时器
  @protected
  Timer? tickTimer;

  /// 网络转换订阅
  @protected
  StreamSubscription<NetworkTransition>? netSub;

  /// 同步引擎基线结算钩子（引擎接线后注入）
  @protected
  SyncTaskHooks? syncHooks;

  /// 入队仲裁执行结果等待器（taskId → completer）
  @protected
  final Map<int, Completer<TaskExecutionOutcome>> outcomeWatchers = {};

  // ═══════════════════════════════════════════════════════════════════
  // 事件发布
  // ═══════════════════════════════════════════════════════════════════

  /// 队列快照广播（revision 版本化）
  @protected
  final StreamController<TransferQueueSnapshot> snapshotCtrl =
      StreamController<TransferQueueSnapshot>.broadcast();

  /// 上传失败通知广播（对齐 Rust `upload_failed` 事件）
  @protected
  final StreamController<UploadFailureNotice> uploadFailureCtrl =
      StreamController<UploadFailureNotice>.broadcast();

  /// 快照版本号（单调递增）
  @protected
  int snapshotRevision = 0;

  /// 最近一次发布的快照（protected 可写；public getter 见 [lastSnapshot]）
  @protected
  TransferQueueSnapshot? lastSnapshotValue;

  /// 队列快照流（防乱序：consumer 丢弃 revision 倒退的快照）
  Stream<TransferQueueSnapshot> get snapshots => snapshotCtrl.stream;

  /// 上传失败通知流（{name, relativePath, error}）
  Stream<UploadFailureNotice> get uploadFailures => uploadFailureCtrl.stream;

  /// 最近一次发布的快照（晚订阅者补偿用）
  TransferQueueSnapshot? get lastSnapshot => lastSnapshotValue;

  /// 是否已启动
  bool get isStarted => started;

  // ═══════════════════════════════════════════════════════════════════
  // 生命周期
  // ═══════════════════════════════════════════════════════════════════

  /// 启动执行器：崩溃恢复 → 订阅网络转换 → 到期调度 → 首轮泵。
  ///
  /// 引擎启动时机由 sync 引擎任务统一接管；重复调用安全。
  Future<void> start() async {
    if (started) return;
    started = true;
    AppLogger.i('TaskRunner 启动：开始崩溃恢复');
    try {
      await recoverStartup();
    } catch (e, st) {
      AppLogger.e('TaskRunner 启动恢复异常', e, st);
    }
    netSub = netTransitions?.listen(onNetTransition);
    tickTimer = Timer.periodic(tickInterval, (_) => unawaited(debugTick()));
    await publishSnapshot();
    await pumpLoop();
  }

  /// 停止调度：不再准入新任务；在途任务自然完结（对齐 Rust 引擎封门语义）。
  Future<void> stop() async {
    if (!started) return;
    started = false;
    tickTimer?.cancel();
    tickTimer = null;
    await netSub?.cancel();
    netSub = null;
    AppLogger.i('TaskRunner 已停止调度');
  }

  /// 释放事件流（应用退出时调用）。
  Future<void> dispose() async {
    await stop();
    await snapshotCtrl.close();
    await uploadFailureCtrl.close();
  }

  /// 网络转换处理：恢复在线后立即重新调度（核验 → 等待 → 到期退避 → 新任务）。
  @protected
  void onNetTransition(NetworkTransition transition) {
    if (transition == NetworkTransition.online) {
      AppLogger.i('网络恢复在线：重新调度传输队列');
      unawaited(publishSnapshot().then((_) => pumpLoop()));
    } else {
      // 在途任务经 isOnline 钩子自行失败并转入 WaitingForNetwork
      unawaited(publishSnapshot());
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
        task.state != TransferState.pending) {
      await publishSnapshot();
      return Err(const GenericError(
          message: '新传输意图必须是 id=0/revision=0 的 Pending 任务'));
    }
    final result = await transferService.enqueue(task);
    if (result.isErr) return result;
    await publishSnapshot();
    await pumpLoop();
    return result;
  }

  /// 手动重试失败任务（对齐 Rust `prepare_retry` + `retry`）。
  ///
  /// 仅接受 Failed 状态；转 Pending 前先做静态与后端前置校验，
  /// 校验拒绝持久化到目标态（Failed/RestartRequired），不盲目重放。
  Future<AppResult<void>> retry(int taskId) async {
    final prepared = await prepareRetry(taskId);
    if (prepared.isErr) return Err((prepared as Err).error);
    final pending = (prepared as Ok<TransferTask>).value;
    // 对齐 Rust retry 内联 run_expected(pending, run_backend_preflight=false)：
    // 后端前置校验已在 prepareRetry 执行过一次，不再重复。
    track(pending, () => runExpected(pending, runBackendPreflight: false));
    await pumpLoop();
    return const Ok(null);
  }

  /// 重试准备（对齐 Rust `prepare_retry`）：
  /// 加载 → Failed 校验 → 静态/后端前置校验 → revision 复查 → 转 Pending。
  ///
  /// 成功返回 Pending 任务行；调用方随后用 [runPreparedAndAwait]
  /// 驱动执行并等待结果（对齐 Rust `retry_transfer` 的 spawn run_prepared）。
  Future<AppResult<TransferTask>> prepareRetry(int taskId) async {
    final loaded = await transferService.getTaskById(taskId);
    final task = loaded.unwrapOr(null);
    if (task == null || task.state != TransferState.failed) {
      await publishSnapshot();
      return Err(const GenericError(message: '任务不存在或非失败状态'));
    }
    // 静态前置校验
    try {
      await validateStatic(task);
    } on PreflightFailure catch (failure) {
      await persistPreflightRejection(task, failure);
      return Err(GenericError(message: failure.message));
    }
    // 后端前置校验
    try {
      await operations.preflight(task);
    } on BackendPreflightFailure catch (failure) {
      await persistPreflightRejection(
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
    final fresh = (await transferService.getTaskById(taskId)).unwrapOr(null);
    if (fresh == null ||
        fresh.state != TransferState.failed ||
        fresh.stateRevision != task.stateRevision) {
      await publishSnapshot();
      return Err(const GenericError(message: '传输任务状态已变化，请刷新后重试'));
    }
    final pending = await transition(
      fresh,
      TransferState.pending,
      TransferPatch.clearingError(attemptCount: fresh.attemptCount + 1),
    );
    if (pending == null) {
      return Err(const GenericError(message: '传输任务状态已变化，请刷新后重试'));
    }
    // retry 接受的 SYNCING 回写（对齐 Rust accept_retry_after_preflight）
    try {
      await syncHooks?.onRetryAccepted(pending);
    } catch (e) {
      AppLogger.w('任务 $taskId retry 后 SYNCING 回写失败（忽略）: $e');
    }
    return Ok(pending);
  }

  /// 驱动已就绪的 Pending 任务执行并等待 outcome
  /// （对齐 Rust `run_prepared`：执行含 outcome 修正的完整结算）。
  Future<TaskExecutionOutcome> runPreparedAndAwait(int taskId) async {
    final task = (await transferService.getTaskById(taskId)).unwrapOr(null);
    if (task == null || task.state != TransferState.pending) {
      throw AppError.generic('任务 $taskId 不存在或不在 Pending 状态');
    }
    return runAndAwaitOutcome(task);
  }

  /// 是否存在 Pending/Running 任务（对齐 Rust `transfer_has_active` 命令）。
  Future<AppResult<bool>> hasActive() async {
    final result = await transferService.countPendingOrRunning();
    return result.map((count) => count > 0);
  }

  /// 注入同步引擎基线结算钩子（引擎接线；对齐 Rust 引擎对
  /// TaskRunner 的 state sink 绑定）。
  void setSyncHooks(SyncTaskHooks? hooks) {
    syncHooks = hooks;
  }

  // ═══════════════════════════════════════════════════════════════════
  // admission 接缝（实现见 TaskRunnerAdmission）
  // ═══════════════════════════════════════════════════════════════════

  /// 同路径仲裁入队并执行（对齐 Rust `enqueue_and_run`）。
  /// 实现由 [TaskRunnerAdmission] 提供。
  Future<AppResult<EnqueuedTaskOutcome>> enqueueAndRun(TransferTask task);

  /// 阻塞态判定（对齐 Rust `is_path_blocking_state`；
  /// 不含 RestartRequired/Completed/Failed/Canceled）。
  @protected
  bool isPathBlockingState(TransferState state) {
    return state == TransferState.pending ||
        state == TransferState.running ||
        state == TransferState.waitingForNetwork ||
        state == TransferState.backingOff ||
        state == TransferState.verifyingRemote;
  }

  /// 歧义远端写入判定：Create/Update 且已持久化远端结果 ID。
  @protected
  bool hasAmbiguousRemoteWriteResult(TransferTask task) {
    return (task.operation == TransferOperation.create ||
            task.operation == TransferOperation.update) &&
        hasPersistedRemoteResult(task);
  }

  /// 同意图判定（对齐 Rust `same_transfer_intent`）。
  /// 实现由 [TaskRunnerAdmission] 提供。
  @protected
  bool sameTransferIntent(TransferTask left, TransferTask right);

  /// 活动态 → 调度去向（对齐 Rust `active_task_disposition`）。
  @protected
  TaskDisposition dispositionForState(TransferState state) {
    return switch (state) {
      TransferState.pending => TaskDisposition.pending,
      TransferState.running => TaskDisposition.running,
      TransferState.waitingForNetwork => TaskDisposition.waitingForNetwork,
      TransferState.backingOff => TaskDisposition.backingOff,
      TransferState.verifyingRemote => TaskDisposition.verifyingRemote,
      TransferState.restartRequired => TaskDisposition.restartRequired,
      _ => TaskDisposition.completed,
    };
  }

  /// 完结入队仲裁等待器（成功去向）。
  @protected
  void completeOutcome(int taskId, TaskExecutionOutcome outcome) {
    final watcher = outcomeWatchers.remove(taskId);
    if (watcher != null && !watcher.isCompleted) {
      watcher.complete(outcome);
    }
  }

  /// 完结入队仲裁等待器（失败：对齐 Rust settle_error 的 Err 路径）。
  @protected
  void failOutcome(int taskId, Object error) {
    final watcher = outcomeWatchers.remove(taskId);
    if (watcher != null && !watcher.isCompleted) {
      watcher.completeError(error);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // recovery 接缝（实现见 TaskRunnerRecovery）
  // ═══════════════════════════════════════════════════════════════════

  /// 每周期批量提升歧义重启（对齐 Rust `promote_ambiguous_restarts`）。
  Future<int> promoteAmbiguousRestarts();

  /// 恢复到期远端核验（对齐 Rust `resume_verifying`）。
  Future<TaskRecoverySummary> resumeVerifying();

  /// 恢复等待网络任务（对齐 Rust `resume_waiting`）。
  Future<TaskRecoverySummary> resumeWaiting();

  /// 恢复到期退避任务（对齐 Rust `resume_due_backoff`）。
  Future<TaskRecoverySummary> resumeDueBackoff();

  /// 全部 Failed 任务（引擎 RETRY 周期用）。
  Future<List<TransferTask>> getFailedTasks() async {
    return (await transferService.getFailedTasks()).unwrapOr([]);
  }

  /// 按 id 加载任务（引擎单任务重试的状态复核用）。
  Future<TransferTask?> getTask(int taskId) async {
    return (await transferService.getTaskById(taskId)).unwrapOr(null);
  }

  /// 清除已完成任务（对齐 Rust `transfer_clear_completed`）。
  Future<AppResult<int>> clearCompleted() async {
    final result = await transferService.clearCompleted();
    await publishSnapshot();
    return result;
  }

  /// 清除已失败任务（对齐 Rust `transfer_clear_failed`）。
  Future<AppResult<int>> clearFailed() async {
    final result = await transferService.clearFailed();
    await publishSnapshot();
    return result;
  }

  /// 清除已结束任务（对齐 Rust `transfer_clear_finished`：Completed + Failed）。
  Future<AppResult<int>> clearFinished() async {
    final result = await transferService.clearFinished();
    await publishSnapshot();
    return result;
  }

  /// BackingOff/VerifyingRemote 的最小 next_retry_at（backoff 调度器用，
  /// 对齐 Rust `next_backoff_deadline_ms`）。
  Future<int?> nextBackoffDeadlineMs() async {
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    int? min;
    for (final task in active) {
      if (task.state != TransferState.backingOff &&
          task.state != TransferState.verifyingRemote) {
        continue;
      }
      final at = task.nextRetryAt;
      if (at == null) continue;
      if (min == null || at < min) min = at;
    }
    return min;
  }

  /// 启动崩溃恢复（对齐 Rust `recover_startup`）。
  /// 实现由 [TaskRunnerRecovery] 提供；由 [start] 调用。
  @protected
  Future<void> recoverStartup();

  // ═══════════════════════════════════════════════════════════════════
  // 并发调度
  // ═══════════════════════════════════════════════════════════════════

  /// 当前并发槽位数（clamp 1-20，默认 6）。
  @protected
  Future<int> concurrency() async {
    final configured = await concurrencyProvider?.call() ?? 6;
    return configured.clamp(1, 20);
  }

  /// 调度泵：空槽时按「到期核验 → 可执行任务（created_at FIFO）」准入。
  ///
  /// 重入安全：泵执行期间的泵请求合并为补泵轮次，全程在同一 future 内完成。
  @protected
  Future<void> pumpLoop() async {
    if (!started) return;
    if (pumping) {
      pumpAgain = true;
      return;
    }
    pumping = true;
    try {
      do {
        pumpAgain = false;
        while (await admitNext()) {
          // 持续准入直到无槽位或无候选
        }
      } while (pumpAgain && started);
    } finally {
      pumping = false;
    }
  }

  /// 尝试准入一个任务；有任务被拉起返回 true。
  @protected
  Future<bool> admitNext() async {
    final slots = (await concurrency()) - inFlight.length;
    if (slots <= 0) return false;
    final online = isOnline();
    final now = nowMs();
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    bool pathBusy(TransferTask t) =>
        t.relativePath != null && activePaths.contains(t.relativePath);
    bool idle(TransferTask t) => !inFlight.containsKey(t.id);

    // 1. 到期远端核验（对齐 resume_verifying：离线整体跳过；next_retry_at 空视为到期）
    if (online) {
      for (final task in active) {
        if (task.state != TransferState.verifyingRemote) continue;
        if (!idle(task) || pathBusy(task)) continue;
        final dueAt = task.nextRetryAt;
        if (dueAt != null && dueAt > now) continue;
        track(task, () => resumeVerifyingTask(task));
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
        if (task.state != TransferState.waitingForNetwork) continue;
        if (!idle(task) || pathBusy(task)) continue;
        track(task, () => runExpected(task));
        return true;
      }
      for (final task in active) {
        if (task.state != TransferState.backingOff || !due(task)) continue;
        if (!idle(task) || pathBusy(task)) continue;
        track(task, () => runExpected(task));
        return true;
      }
    }
    for (final task in active) {
      if (task.state != TransferState.pending) continue;
      if (!idle(task) || pathBusy(task)) continue;
      // 离线时也准入：执行链按 Rust 语义转 WaitingForNetwork
      track(task, () => runExpected(task));
      return true;
    }
    return false;
  }

  /// 登记在途任务：占用槽位与同路径屏障，收尾后发布快照并补泵。
  @protected
  void track(TransferTask task, Future<void> Function() body) {
    final rel = task.relativePath;
    if (rel != null) activePaths.add(rel);
    final done = Completer<void>();
    inFlight[task.id] = done.future;
    unawaited(() async {
      try {
        await body();
      } catch (e, st) {
        AppLogger.e('任务 ${task.id} 执行未捕获异常', e, st);
      } finally {
        inFlight.remove(task.id);
        if (rel != null) activePaths.remove(rel);
        try {
          await publishSnapshot();
          await pumpLoop();
        } finally {
          done.complete();
        }
      }
    }());
  }

  // ═══════════════════════════════════════════════════════════════════
  // execution 接缝（实现见 TaskRunnerExecutionSettlement）
  // ═══════════════════════════════════════════════════════════════════

  /// 执行单个可运行任务（对齐 Rust `run_expected`）。
  /// 实现由 [TaskRunnerExecutionSettlement] 提供。
  @protected
  Future<void> runExpected(
    TransferTask current, {
    bool runBackendPreflight = true,
  });

  /// 执行任务并等待调度去向（入队仲裁路径专用，对齐 Rust）。
  @protected
  Future<TaskExecutionOutcome> runAndAwaitOutcome(TransferTask task);

  // ═══════════════════════════════════════════════════════════════════
  // 进度报告（对齐 Rust TaskProgressReporter）
  // ═══════════════════════════════════════════════════════════════════

  /// 为 Running 任务构造进度回调（节流持久化 + 顺序化写入 + 修订门禁）。
  @protected
  TaskProgressCallbacks progressCallbacks(TransferTask running) {
    var lastProgressMs = 0;
    // 顺序化进度写，避免并发回调乱序落库
    Future<void> writes = Future<void>.value();

    bool throttle() {
      final now = nowMs();
      if (lastProgressMs != 0 &&
          now - lastProgressMs < TaskRunnerBase.progressThrottleMs) {
        return false;
      }
      lastProgressMs = now;
      return true;
    }

    void enqueueWrite(Future<AppResult<void>> Function() write) {
      writes = writes.then((_) => write()).then((_) => publishSnapshot(),
          onError: (Object e, StackTrace st) {
        AppLogger.d('忽略过期进度回调: $e');
      });
    }

    return TaskProgressCallbacks(
      totalSize: running.totalSize,
      onProgress: (transferred) {
        if (transferred < 0 || transferred > running.totalSize) return;
        if (!throttle()) return;
        enqueueWrite(() => transferService.updateProgress(
              running.id,
              transferred,
              expectedRevision: running.stateRevision,
            ));
      },
      onDownloadProgress: (transferred) {
        if (transferred < 0 || transferred > running.totalSize) return;
        if (!throttle()) return;
        enqueueWrite(() => transferService.updateProgress(
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
        enqueueWrite(() => transferService.updateResumeSession(
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
  @protected
  Future<void> validateStatic(TransferTask task) async {
    await validateStaticTask(
      task,
      mountRoot: mountRootProvider(),
      isPlaceholder: isPlaceholder,
    );
  }

  /// 持久化常规任务状态迁移（CAS：id + from 状态 + revision）。
  @protected
  Future<TransferTask?> transition(
    TransferTask task,
    TransferState to,
    TransferPatch patch,
  ) async {
    final result = await transferService.transition(
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
    await publishSnapshot();
    return updated;
  }

  /// 持久化带错误信息的任务状态迁移（对齐 Rust `transition_failure`）。
  @protected
  Future<TransferTask?> transitionFailure(
    TransferTask task,
    TransferState state,
    TransferErrorKind kind,
    String message,
  ) {
    return transition(
      task,
      state,
      TransferPatch(
        errorKind: SetPatch(kind),
        errorMessage: SetPatch(message),
        finishedAt: state == TransferState.failed
            ? SetPatch(nowMs())
            : const ClearPatch(),
      ),
    );
  }

  /// 生命周期不变时更新错误与重试事实（对齐 Rust `patch_transfer_in_state`）。
  @protected
  Future<TransferTask?> patchInState(
    TransferTask task,
    TransferState expectedState,
    TransferPatch patch,
  ) async {
    final result = await transferService.patchInState(
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
    if (updated != null) await publishSnapshot();
    return updated;
  }

  /// 持久化前置校验拒绝结果（对齐 Rust `persist_preflight_rejection`）。
  @protected
  Future<void> persistPreflightRejection(
    TransferTask task,
    PreflightFailure failure,
  ) async {
    final patch = failure.patch(nowMs: nowMs());
    if (task.state == TransferState.failed &&
        failure.target == TransferState.failed) {
      await patchInState(task, TransferState.failed, patch);
      return;
    }
    await transition(task, failure.target, patch);
  }

  /// 将含歧义远端结果的重启任务提升为待核验（对齐 Rust
  /// `promote_restart_to_verifying`）。
  ///
  /// [message] 默认用启动恢复文案；Running 仲裁提升时传仲裁专用文案
  /// （对齐 Rust admission.rs 的「远端结果 ID 已存在…」）。
  @protected
  Future<TransferTask?> promoteRestartToVerifying(
    TransferTask task, {
    String message = '远端写入已返回资源 ID，禁止重放并等待核验',
  }) {
    return transition(
      task,
      TransferState.verifyingRemote,
      TransferPatch(
        errorKind: const SetPatch(TransferErrorKind.remoteAmbiguous),
        errorMessage: SetPatch(message),
        nextRetryAt: const ClearPatch(),
        finishedAt: const ClearPatch(),
      ),
    );
  }

  /// 判断任务是否保存了非空远程结果 ID。
  @protected
  bool hasPersistedRemoteResult(TransferTask task) {
    final id = task.remoteResultFileId;
    return id != null && id.trim().isNotEmpty;
  }

  // ═══════════════════════════════════════════════════════════════════
  // settlement 接缝（实现见 TaskRunnerExecutionSettlement）
  // ═══════════════════════════════════════════════════════════════════

  /// Running 仲裁（对齐 Rust `transition_to_running_or_block`）。
  /// 实现由 [TaskRunnerExecutionSettlement] 提供。
  @protected
  Future<TransferTask?> transitionToRunningOrBlock(TransferTask current);

  // ═══════════════════════════════════════════════════════════════════
  // 远端核验接缝（实现见 TaskRunnerRecovery）
  // ═══════════════════════════════════════════════════════════════════

  /// 核验并结算一个远端结果不确定的任务（对齐 Rust `resume_verifying_task`）。
  /// 实现由 [TaskRunnerRecovery] 提供。
  @protected
  Future<TaskExecutionOutcome?> resumeVerifyingTask(TransferTask task);

  // ═══════════════════════════════════════════════════════════════════
  // 快照发布（对齐 Rust publication.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 重算持久事实并广播完整队列快照（尽力而为，对齐 notify_best_effort）。
  @protected
  Future<void> publishSnapshot() async {
    try {
      final tasks = (await transferService.getAllTasks()).unwrapOr([]);
      var activeCount = 0;
      for (final task in tasks) {
        if (task.state.isActive) activeCount++;
      }
      final snapshot = TransferQueueSnapshot(
        revision: ++snapshotRevision,
        tasks: tasks,
        activeCount: activeCount,
      );
      lastSnapshotValue = snapshot;
      if (!snapshotCtrl.isClosed) snapshotCtrl.add(snapshot);
    } catch (e) {
      AppLogger.w('任务状态变化后重算权威快照失败: $e');
    }
  }

  /// 发布上传失败通知（对齐 Rust `upload_failed` 事件负载）。
  void publishUploadFailure(UploadFailureNotice notice) {
    if (!uploadFailureCtrl.isClosed) uploadFailureCtrl.add(notice);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 测试钩子
  // ═══════════════════════════════════════════════════════════════════

  /// 测试用：执行一轮调度（到期核验 + 可执行任务准入）。
  @visibleForTesting
  Future<void> debugTick() => pumpLoop();

  /// 测试用：等待全部在途任务（含结算收尾与链式调度）完成。
  @visibleForTesting
  Future<void> get idle async {
    while (inFlight.isNotEmpty) {
      await Future.wait(inFlight.values.toList());
    }
  }
}
