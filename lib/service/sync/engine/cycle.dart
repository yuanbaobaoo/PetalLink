/// 引擎周期编排切片（对齐 Rust `src/sync/engine/cycle.rs`）。
///
/// - 触发源 → CycleRequest 位映射（`cycleRequestForTrigger`）与反向优先级推导
/// - 唯一 owner drain：sticky 位合并；shutdown 丢弃、folderSyncing/离线/
///   云刷新失败/不可信 restore 保留
/// - 单周期阶段序列：运行态发布 → STARTUP 收敛 → 云端刷新（增量优先）→
///   可信门 → 歧义重启提升 → 路径恢复 → 墓碑清理 → 在线恢复 → RETRY →
///   本地扫描 → 规划 → 过滤 → 执行 → 结算
library;

import 'dart:async';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/action_filters.dart';
import 'package:petal_link/service/sync/engine/cache.dart';
import 'package:petal_link/service/sync/engine/coordination.dart';
import 'package:petal_link/service/sync/identity/detect_moves.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎周期编排 mixin。
mixin EngineCycle on SyncEngineBase {
  // ═══════════════════════════════════════════════════════════════════
  // 触发源映射
  // ═══════════════════════════════════════════════════════════════════

  /// 触发源 → 请求位（对齐 Rust `cycle_request_for_trigger`）。
  @override
  CycleRequest cycleRequestForTrigger(String triggeredBy) {
    switch (triggeredBy) {
      case 'manual-refresh':
        return CycleRequest.of(
            [CycleRequest.localRescan, CycleRequest.cloudFull]);
      case 'auto-cloud-refresh':
        return CycleRequest.of(
            [CycleRequest.localRescan, CycleRequest.cloudIncremental]);
      case 'network-recovery':
        return CycleRequest.of([
          CycleRequest.localRescan,
          CycleRequest.cloudIncremental,
          CycleRequest.onlineRecovery,
        ]);
      case 'startup-resume':
        return CycleRequest.of([
          CycleRequest.localRescan,
          CycleRequest.cloudIncremental,
          CycleRequest.onlineRecovery,
          CycleRequest.startup,
        ]);
      case 'retry-failed':
        return CycleRequest.of([
          CycleRequest.localRescan,
          CycleRequest.cloudIncremental,
          CycleRequest.retry,
        ]);
      case 'retry-replan':
        return CycleRequest.of([
          CycleRequest.localRescan,
          CycleRequest.cloudIncremental,
          CycleRequest.replan,
        ]);
      case 'backoff-deadline':
        return CycleRequest.of([
          CycleRequest.localRescan,
          CycleRequest.cloudIncremental,
          CycleRequest.onlineRecovery,
        ]);
      default:
        // 含 local-watcher
        return CycleRequest.of([CycleRequest.localRescan]);
    }
  }

  /// 请求位 → 触发源（反向推导，决定 syncPhase 与恢复行为；
  /// 对齐 Rust `run_coordinated_cycle` 的优先级链）。
  String _triggerForRequest(CycleRequest request) {
    if (request.contains(CycleRequest.startup)) return 'startup-resume';
    if (request.contains(CycleRequest.cloudFull)) return 'manual-refresh';
    if (request.contains(CycleRequest.retry)) return 'retry-failed';
    if (request.contains(CycleRequest.replan)) return 'retry-replan';
    if (request.contains(CycleRequest.onlineRecovery)) {
      return 'network-recovery';
    }
    if (request.contains(CycleRequest.cloudIncremental)) {
      return 'auto-cloud-refresh';
    }
    return 'local-watcher';
  }

  // ═══════════════════════════════════════════════════════════════════
  // 请求入口
  // ═══════════════════════════════════════════════════════════════════

  /// 合并请求并安排后台 drain（watcher/定时器/网络边沿共用入口）。
  @override
  void requestCycleBackground(String triggeredBy) {
    cycle.request(cycleRequestForTrigger(triggeredBy));
    scheduleBackgroundDrain();
  }

  /// 提交请求并等待本序列结算（对齐 Rust `run_sync_cycle`）。
  ///
  /// 离线/门控恢复时抛「已排队」错误；started 之前仅放行 startup-resume。
  @override
  Future<void> runSyncCycle(String triggeredBy) async {
    if (triggeredBy != 'startup-resume' && !started) {
      throw AppError.generic('同步引擎正在启动，请稍后重试');
    }
    final seq = cycle.request(cycleRequestForTrigger(triggeredBy));
    await drainCycleRequestsFor(awaited: seq);
    final result = cycle.resultIfCompleted(seq);
    if (!result.settled) {
      throw AppError.generic('同步请求已排队，等待恢复条件');
    }
    final error = result.error;
    if (error != null) throw error;
  }

  /// 手动全量刷新入口（对齐 Rust `trigger_manual_sync`）：
  /// 成功后置 contentChanged=true 发布。
  @override
  Future<void> triggerManualSync() async {
    cycleObserver('request-manual');
    await runSyncCycle('manual-refresh');
    await updateRuntimeAndBroadcast((r) {
      r.contentChanged = true;
    });
    cycleObserver('manual-cycle-returned');
  }

  /// 后台 drain 调度（backgroundScheduled CAS 唯一性）。
  @override
  void scheduleBackgroundDrain() {
    if (backgroundScheduled) return;
    backgroundScheduled = true;
    unawaited(_backgroundDrainLoop());
  }

  /// 后台 drain 循环：可恢复失败按指数退避；结束后按条件交接下一棒。
  Future<void> _backgroundDrainLoop() async {
    var recoverableFailures = 0;
    var failed = false;
    try {
      while (cycle.hasPending()) {
        if (shutdownFlag || !started) break;
        if (!isOnline()) break;
        if (folderSyncing) break;
        try {
          await drainCycleRequestsFor();
          recoverableFailures = 0;
        } catch (e) {
          if (!SyncEngineBase.isRecoverableCycleError(e)) {
            AppLogger.w('后台周期失败（不可恢复分类）: $e');
            failed = true;
            break;
          }
          recoverableFailures++;
          final delay = SyncEngineBase.recoverableCycleRetryDelay(
              recoverableFailures);
          AppLogger.w('后台周期可恢复失败，${delay.inSeconds}s 后重试: $e');
          await Future<void>.delayed(delay);
        }
      }
    } catch (e, st) {
      AppLogger.e('后台周期 drain 异常', e, st);
    } finally {
      backgroundScheduled = false;
      // 交接：持有期间到达的新请求必须补跑
      if (!failed &&
          started &&
          isOnline() &&
          !folderSyncing &&
          !shutdownFlag &&
          cycle.hasPending()) {
        scheduleBackgroundDrain();
      }
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // drain：唯一 owner 排空
  // ═══════════════════════════════════════════════════════════════════

  /// 唯一 owner 合并排空（对齐 Rust `drain_cycle_requests_for`）。
  @override
  Future<void> drainCycleRequestsFor({int? awaited}) async {
    final release = await cycle.lockOwner();
    try {
      if (awaited != null) {
        final completed = cycle.resultIfCompleted(awaited);
        if (completed.settled) {
          final error = completed.error;
          if (error != null) throw error;
          return;
        }
      }
      syncing = true;
      try {
        while (true) {
          final (request, seq) = cycle.takePendingWithSequence();
          if (request.isEmpty) return;
          // shutdown：请求被丢弃（不 restore）
          if (shutdownFlag) return;
          // 目录同步进行中 / 离线（非 STARTUP）：restore 保留
          if (folderSyncing) {
            cycle.restore(request);
            return;
          }
          if (!request.contains(CycleRequest.startup) && !isOnline()) {
            cycle.restore(request);
            return;
          }
          try {
            await runCoordinatedCycle(request);
            cycle.complete(seq);
          } catch (e) {
            cycle.complete(seq, e);
            await restoreIdleRuntimeAfterError();
            rethrow;
          }
        }
      } finally {
        syncing = false;
      }
    } finally {
      release();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 单周期阶段序列
  // ═══════════════════════════════════════════════════════════════════

  /// 按请求意图执行单个协调周期（对齐 Rust `run_coordinated_cycle`）。
  @override
  Future<void> runCoordinatedCycle(CycleRequest request) async {
    final triggeredBy = _triggerForRequest(request);
    final startup = request.contains(CycleRequest.startup);
    try {
      // 1. 发布运行态（syncPhase 空闲时按触发源设置）
      await updateRuntimeAndBroadcast((r) {
        r.isRunning = true;
        r.syncPhase ??= switch (triggeredBy) {
          'local-watcher' => SyncPhase.syncingLocal,
          'manual-refresh' => SyncPhase.syncingManual,
          'retry-failed' || 'retry-replan' => SyncPhase.syncingRetry,
          'startup-resume' => SyncPhase.syncingStartup,
          _ => null,
        };
      });

      // 2. STARTUP：清理滞留 SYNCING
      if (startup) {
        final guard = beginExternalActivity();
        try {
          await baselineStore.resetStaleStatuses();
        } finally {
          guard.close();
        }
        ensureCycleActive();
      }

      // 3. STARTUP + CLOUD_INCREMENTAL：加载或全量重建云树
      var startupNeedsIncremental = false;
      if (startup && request.contains(CycleRequest.cloudIncremental)) {
        try {
          startupNeedsIncremental = await loadOrRefreshCloudTree();
        } catch (e) {
          AppLogger.w('启动 owner 无法建立可信云端 checkpoint，禁止进入 planner');
          cycle.restore(request);
          rethrow;
        }
        ensureCycleActive();
      }

      // 4. 云端刷新（全量优先于增量）
      if (request.contains(CycleRequest.cloudFull)) {
        cycleObserver('cloud-refresh');
        try {
          await refreshCloudFullForCycle();
        } catch (e) {
          cycle.restore(request);
          rethrow;
        }
      } else if (request.contains(CycleRequest.cloudIncremental) &&
          (!startup || startupNeedsIncremental)) {
        if (!isOnline()) {
          cycle.restore(request);
          if (startup) {
            throw AppError.generic('启动云端追平等待网络恢复');
          }
          return;
        }
        cycleObserver('cloud-refresh');
        try {
          await refreshCloudIncrementalForCycle();
        } catch (e) {
          AppLogger.w('云端刷新失败，完整保留当前周期意图等待补跑');
          cycle.restore(request);
          rethrow;
        }
      }

      // 5. 可信门：不可信时禁止规划
      if (!cloudTreeIsTrusted()) {
        cycle.restore(request);
        AppLogger.w('云端 checkpoint 尚未追平，跳过任务恢复与同步规划');
        if (startup) {
          throw AppError.generic('启动云端 checkpoint 尚未追平，等待恢复');
        }
        return;
      }

      // 6. 歧义重启提升（保存远端结果 ID 的 RestartRequired → 核验态）
      final runner = taskRunner;
      if (runner != null) {
        final promoted = await runner.promoteAmbiguousRestarts();
        if (promoted > 0) {
          AppLogger.w('已将 $promoted 个含远端结果的重规划任务恢复为核验态');
          notifyBackoffScheduleChanged();
        }
      }

      // 7. 远端路径恢复（本地扫描前收敛已提交的远端改名/移动）
      final List<BlockedPathChange> blockedPathChanges;
      final pathGuard = beginExternalActivity();
      try {
        final m = mount;
        if (m == null) {
          throw AppError.config('挂载管理器未配置');
        }
        blockedPathChanges = (await PathRecovery(
          db: db,
          mount: m,
          nowMs: nowMs,
          identity: identity,
        ).recoverVerifiedRemotePathChanges(
          Map<String, DriveFile>.of(cloudIndex.tree),
          (oldPath, newPath) async {
            final g1 = beginExclusivePathActivity(oldPath);
            final g2 = beginExclusivePathActivity(newPath);
            return () {
              g1.close();
              g2.close();
            };
          },
        ))
            .blockedChanges;
      } catch (e) {
        cycle.restore(request);
        rethrow;
      } finally {
        pathGuard.close();
      }

      // 8. 清理 DELETED 墓碑（仅可信）
      ensureCycleActive();
      await purgeDeletedTombstonesIfTrusted(blockedPathChanges);

      // 9. STARTUP：中断传输恢复由 TaskRunner.start() 完成（核验异步结算），
      //    此处提交恢复 checkpoint（空列表 = 无副作用）
      if (startup) {
        await commitRecoveryCheckpoint(const []);
      }

      // 10. ONLINE_RECOVERY：核验 → 等待 → 到期退避（路径恢复已先行）
      if (request.contains(CycleRequest.onlineRecovery)) {
        if (!isOnline()) {
          cycle.restore(request);
          return;
        }
        var completedRecoveries = 0;
        final r = taskRunner;
        if (r != null) {
          // 对齐 Rust cycle.rs：每个恢复阶段的 recovered_cloud_files
          // 都提交 live 云树与 checkpoint（修复前丢弃，云树与基线短暂不一致）
          cycleObserver('verify-remote');
          final verifying = await r.resumeVerifying();
          completedRecoveries += verifying.completed;
          await commitRecoveryCheckpoint(verifying.recoveredCloudFiles);
          ensureCycleActive();
          cycleObserver('resume-waiting');
          final waiting = await r.resumeWaiting();
          completedRecoveries += waiting.completed;
          await commitRecoveryCheckpoint(waiting.recoveredCloudFiles);
          ensureCycleActive();
          cycleObserver('resume-due');
          final due = await r.resumeDueBackoff();
          completedRecoveries += due.completed;
          await commitRecoveryCheckpoint(due.recoveredCloudFiles);
          ensureCycleActive();
        }
        if (completedRecoveries > 0) {
          cycle.request(CycleRequest.of(
              [CycleRequest.localRescan, CycleRequest.cloudIncremental]));
        }
      }

      // 11. RETRY：全局重试（REPLAN 不清理无关失败项）
      if (request.contains(CycleRequest.retry)) {
        final retryGuard = beginExternalActivity();
        try {
          final r = requireTaskRunner();
          final failedTasks = await r.getFailedTasks();
          for (final task in failedTasks) {
            try {
              await r.retry(task.id);
            } catch (e) {
              AppLogger.w('重试任务 ${task.id} 失败（继续其他任务）: $e');
            }
            ensureCycleActive();
          }
          await baselineStore.sweepFailedWithoutFailedTasks();
          await recomputeAndBroadcastState();
        } finally {
          retryGuard.close();
        }
      }

      // 12. 本地扫描 → 规划 → 过滤 → 执行 → 结算
      cycleObserver('local-rescan');
      await runSyncCycleInner(triggeredBy, blockedPathChanges);
    } finally {
      // 周期收尾（无论成败）：运行时仍非空闲则复位
      if (runtime.isRunning || runtime.isIndexing || runtime.syncPhase != null) {
        await restoreIdleRuntimeAfterError();
      }
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 规划/执行/结算内层
  // ═══════════════════════════════════════════════════════════════════

  /// 扫描 → 对账 → 规划 → 过滤 → 执行 → 结算（对齐 Rust
  /// `run_sync_cycle_inner`）。
  @override
  Future<void> runSyncCycleInner(
    String triggeredBy,
    List<BlockedPathChange> blockedPathChanges,
  ) async {
    final local = await scanLocal();
    cycleObserver('local-scan-complete');
    ensureCycleActive();

    final planningGuard = beginExternalActivity();
    final List<SyncAction> actions;
    final Map<String, DriveFile> cloud;
    try {
      cloud = Map<String, DriveFile>.of(cloudIndex.tree);
      var dbSnapshot = await loadDbSnapshot();
      final trusted = cloudTreeIsTrusted();
      if (trusted) {
        await reconcileDbRecords(local, dbSnapshot, blockedPathChanges);
        await reconcileFailedAndPurgeStaleRecords(
            local, cloud, blockedPathChanges);
        dbSnapshot = await loadDbSnapshot();
      } else {
        AppLogger.w('云端 checkpoint 不可信，跳过 DB 对账');
      }

      actions = SyncPlanner().plan(SyncSnapshot(
        local: local,
        cloud: cloud,
        db: dbSnapshot,
        cloudTreeTrusted: trusted,
        isStartupResume: triggeredBy == 'startup-resume',
      ));

      // 过滤链（顺序固定）
      filterSkippedPaths(actions, skipPatterns);
      if (trusted) {
        // inode 移动合并（docs/design/10 §4.3，取代旧 xattr detectRenames）
        applyDetectedMoves(actions, lastScanMoves, cloudIndex);
      }
      final activeTasks = await queryActiveTransfers(await db.database);
      filterActiveTransferActions(actions, activeTasks, dbSnapshot);
      filterAntiOscillation(actions, recentlyDeletedPaths);
      fillParentFileIds(actions, cloudIndex.pathToId);
      addRescueFolderRecreations(
        actions,
        local: local,
        cloud: cloud,
        db: dbSnapshot,
        recentlyDeletedPaths: recentlyDeletedPaths,
        mountDir: mountDir,
      );
      filterBlockedPathChanges(actions, blockedPathChanges);
      final m = mount;
      if (m != null) {
        await validateDeleteFromCloud(actions, m);
      }
      dedupeDirectoryDeletes(actions, cloud);
      dedupeLocalDescendants(actions);
      preserveDirsWithPendingBackups(actions);
    } finally {
      planningGuard.close();
    }

    // 空动作短路
    if (actions.isEmpty) {
      await updateRuntimeAndBroadcast((r) {
        r.editing = 0;
        r.contentChanged = false;
        r.isRunning = false;
        r.isIndexing = false;
        r.syncPhase = null;
        r.lastSyncTime = nowMs();
      });
      return;
    }

    ensureCycleActive();
    final exec = executor;
    if (exec == null) {
      throw AppError.generic('执行器未初始化');
    }
    final results = await executeActionsOrdered(exec, actions);
    ensureCycleActive();

    // 成功的 MoveInCloud 先落入可信 checkpoint，再提交 DB 路径基线
    final recovered = <RecoveredCloudFile>[];
    for (var i = 0; i < actions.length; i++) {
      if (actions[i].actionType == SyncActionType.moveInCloud &&
          results[i].success) {
        final file = results[i].cloudFile ?? actions[i].cloudFile;
        final rel = actions[i].relativePath;
        if (file != null && rel != null) {
          recovered.add(RecoveredCloudFile(relativePath: rel, file: file));
        }
      }
    }
    if (recovered.isNotEmpty) {
      await commitRecoveryCheckpoint(recovered);
    }

    final applyGuard = beginExternalActivity();
    try {
      await applyResults(actions, results);
    } finally {
      applyGuard.close();
    }

    // 成功 MoveInCloud → 立即重扫（版本校验上传并发编辑）
    if (recovered.isNotEmpty) {
      cycle.request(CycleRequest.of([CycleRequest.localRescan]));
    }

    // 结构性变更判定
    const structuralTypes = {
      SyncActionType.upload,
      SyncActionType.download,
      SyncActionType.deleteFromCloud,
      SyncActionType.deleteFromLocal,
      SyncActionType.createFolder,
      SyncActionType.createConflictCopy,
      SyncActionType.createPlaceholder,
      SyncActionType.moveInCloud,
      SyncActionType.backupBeforeCloudDelete,
    };
    var contentChanged = false;
    for (var i = 0; i < actions.length && !contentChanged; i++) {
      if (results[i].success &&
          structuralTypes.contains(actions[i].actionType)) {
        contentChanged = true;
      }
    }
    await updateAndPushState(contentChanged: contentChanged);
  }
}
