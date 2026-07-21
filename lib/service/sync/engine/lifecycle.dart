/// 引擎生命周期切片（对齐 Rust `src/sync/engine/lifecycle.rs`）。
///
/// 启动序列：运行态发布 → 启动周期（可延后）→ 发布启动空闲态 →
/// started 置位 → 网络监听 → 本地 watcher（云树扫描完成后才启动）→
/// 云端定时刷新 → backoff 调度器。
/// 停止序列：封活动门 → 发布屏障内置 shutdown → 停 watcher/定时器 →
/// 等周期 owner 静默 → 等全部活动释放。
library;

import 'dart:async';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/service/mount/local_watcher.dart';
import 'package:petal_link/service/sync/engine.dart';

/// 引擎生命周期 mixin。
mixin EngineLifecycle on SyncEngineBase {
  /// 启动引擎（对齐 Rust `start`）。
  @override
  Future<void> start() async {
    if (shutdownFlag) {
      AppLogger.i('同步引擎已停止，拒绝启动');
      return;
    }
    // 先订阅网络转换再跑启动周期，避免丢失缓冲边沿
    _startNetworkListener();

    running = true;
    try {
      await updateRuntimeAndBroadcast((r) {
        r.isRunning = true;
      });
    } catch (e) {
      await restoreIdleRuntimeAfterError();
      rethrow;
    }

    // 启动周期：可恢复失败/离线时延后（请求已保留）
    var startupDeferred = false;
    try {
      await runSyncCycle('startup-resume');
    } catch (e) {
      if (!isOnline() || SyncEngineBase.isRecoverableCycleError(e)) {
        AppLogger.w('启动周期延后（等待网络恢复）: $e');
        startupDeferred = true;
      } else {
        rethrow;
      }
    }
    ensureCycleActive();

    // 发布启动空闲态（在任何可入队新周期的来源启用之前）
    await updateRuntimeAndBroadcast((r) {
      r.isRunning = false;
      r.isIndexing = false;
      r.syncPhase = null;
    });
    started = true;

    // 云树扫描完成后才启动本地监视器
    ensureCycleActive();
    await _startWatcher();
    ensureCycleActive();
    _startCloudRefreshTimer();
    _startBackoffScheduler();

    if (startupDeferred && isOnline() && cycle.hasPending()) {
      scheduleBackgroundDrain();
    }
    AppLogger.i('同步引擎已启动');
  }

  /// 停止引擎（对齐 Rust `shutdown`）。
  @override
  Future<void> shutdown() async {
    final w = watcher;
    watcher = null;
    activity.close();
    await shutdownSync();
    if (w != null) await w.stop();
    // 等当前周期 owner 静默（结算收敛前替代引擎不会启动）
    final release = await cycle.lockOwner();
    release();
    // 等关闭屏障前已登记的全部活动释放
    await activity.waitIdle();
    final late = watcher;
    watcher = null;
    if (late != null) await late.stop();
    AppLogger.i('同步引擎已停止');
  }

  /// 同步停止语义（对齐 Rust `shutdown_sync`）：
  /// 发布屏障内置 shutdown 标志，保证返回后旧引擎的后续发布只能失败。
  @override
  Future<void> shutdownSync() async {
    activity.close();
    await statusAggregator.lockPublication(() async {
      shutdownFlag = true;
    });
    started = false;
    running = false;
    if (!shutdownCtrl.isClosed) shutdownCtrl.add(true);
    cloudRefreshTimer?.cancel();
    cloudRefreshTimer = null;
    backoffTimer?.cancel();
    backoffTimer = null;
    await netSub?.cancel();
    netSub = null;
    await watcherSub?.cancel();
    watcherSub = null;
    final w = watcher;
    watcher = null;
    if (w != null) await w.stop();
    notifyBackoffScheduleChanged();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 各触发源
  // ═══════════════════════════════════════════════════════════════════

  /// 网络转换监听：在线边沿 → 恢复周期。
  void _startNetworkListener() {
    final transitions = netTransitions;
    if (transitions == null) return;
    netSub = transitions.listen((transition) {
      if (transition == NetworkTransition.online && !shutdownFlag) {
        requestCycleBackground('network-recovery');
      }
    });
  }

  /// 启动本地 watcher（变更批/Lagged 均触发补偿重扫）。
  Future<void> _startWatcher() async {
    final m = mount;
    if (m == null || shutdownFlag) return;
    final w = LocalWatcher(
      mountDir: m.mountDir,
      skipPatterns: skipPatterns,
      debounce: debounce,
    );
    await w.start();
    if (shutdownFlag) {
      await w.stop();
      return;
    }
    watcher = w;
    watcherSub = w.changes.listen(
      (_) {
        if (shutdownFlag) return;
        requestCycleBackground('local-watcher');
      },
      onError: (Object e) => AppLogger.e('本地监视器事件流异常', e),
    );
  }

  /// 云端定时刷新（pollInterval 为零则不启动；用 sleep 语义避免欠债 tick）。
  void _startCloudRefreshTimer() {
    if (pollInterval <= Duration.zero) return;
    cloudRefreshTimer?.cancel();
    cloudRefreshTimer = Timer.periodic(pollInterval, (_) {
      if (shutdownFlag) return;
      if (!isOnline()) {
        AppLogger.d('离线，跳过定时云端刷新');
        return;
      }
      unawaited(runSyncCycle('auto-cloud-refresh').catchError((Object e) {
        AppLogger.w('定时云端刷新失败（下次定时再试）: $e');
      }));
    });
  }

  /// backoff 调度器：到期且在线 → backoff-deadline 周期；
  /// 1s 兜底复查防止横跨 deadline 无通知而永久休眠或热循环。
  void _startBackoffScheduler() {
    if (taskRunner == null) return;
    unawaited(() async {
      while (!shutdownFlag) {
        final runner = taskRunner;
        if (runner == null) return;
        final deadline = await runner.nextBackoffDeadlineMs();
        if (deadline == null) {
          await _waitScheduleChange(null);
          continue;
        }
        final remaining = deadline - nowMs();
        if (remaining > 0) {
          await _waitScheduleChange(Duration(milliseconds: remaining));
          continue;
        }
        if (!isOnline()) {
          await _waitScheduleChange(
              SyncEngineBase.blockedDeadlineRecheckInterval);
          continue;
        }
        try {
          await runSyncCycle('backoff-deadline');
        } catch (e) {
          AppLogger.w('backoff 到期周期失败: $e');
        }
        await _waitScheduleChange(
            SyncEngineBase.blockedDeadlineRecheckInterval);
      }
    }());
  }

  /// 等待调度变化 / shutdown / 超时（轮询实现，粒度 50-200ms）。
  Future<void> _waitScheduleChange(Duration? timeout) async {
    final observed = scheduleRevision;
    final deadlineMs =
        timeout == null ? null : nowMs() + timeout.inMilliseconds;
    final poll = timeout == null
        ? const Duration(milliseconds: 200)
        : const Duration(milliseconds: 50);
    while (!shutdownFlag && scheduleRevision == observed) {
      if (deadlineMs != null && nowMs() >= deadlineMs) return;
      await Future<void>.delayed(poll);
    }
  }
}
