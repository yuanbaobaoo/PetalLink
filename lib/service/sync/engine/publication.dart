/// 引擎状态发布切片（对齐 Rust `src/sync/engine/publication.rs`）。
///
/// 唯一发布通道 [updateRuntimeAndBroadcast]：
/// 发布屏障内 → shutdown 检查 → 应用运行时更新 → DB 聚合并分配 revision →
/// 替换内存快照 → 广播。无时间节流：每次变更发布完整快照。
library;

import 'dart:async';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/status_aggregator.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎状态发布 mixin。
mixin EnginePublication on SyncEngineBase {
  /// 当前权威快照。
  @override
  SyncGlobalState currentState() => state;

  /// 快照广播流（best-effort；慢消费者由 revision 防乱序兜底）。
  @override
  Stream<SyncGlobalState> stateReceiver() => stateCtrl.stream;

  /// 唯一发布通道（对齐 Rust `update_runtime_and_broadcast`）。
  ///
  /// 聚合失败时仅把运行时字段保留在内存（保生命周期门准确）并抛错。
  @override
  Future<SyncGlobalState> updateRuntimeAndBroadcast(
    void Function(RuntimeStatus runtime) update,
  ) {
    return statusAggregator.lockPublication(() async {
      if (shutdownFlag) {
        throw AppError.generic('同步引擎已停止，拒绝发布状态');
      }
      update(runtime);
      try {
        final db = await this.db.database;
        final snapshot = await statusAggregator.snapshot(db, runtime);
        state = snapshot;
        if (!stateCtrl.isClosed) stateCtrl.add(snapshot);
        return snapshot;
      } catch (e) {
        AppLogger.w('状态聚合失败，仅保留运行时字段: $e');
        rethrow;
      }
    });
  }

  /// 从 DB 重新聚合并广播完整快照（对齐 Rust
  /// `recompute_and_broadcast_state`）。
  @override
  Future<SyncGlobalState> recomputeAndBroadcastState() {
    return updateRuntimeAndBroadcast((_) {});
  }

  /// 出错后恢复空闲运行时（isRunning=false, isIndexing=false,
  /// syncPhase=null；吞错，对齐 Rust `restore_idle_runtime_after_error`）。
  @override
  Future<void> restoreIdleRuntimeAfterError() async {
    try {
      await updateRuntimeAndBroadcast((r) {
        r.isRunning = false;
        r.isIndexing = false;
        r.syncPhase = null;
      });
    } catch (_) {
      // 尽力恢复
    }
  }

  /// 周期收尾发布（对齐 Rust `update_and_push_state`）：
  /// contentChanged + lastSyncTime=now + 空闲复位。
  @override
  Future<void> updateAndPushState({required bool contentChanged}) async {
    await updateRuntimeAndBroadcast((r) {
      r.contentChanged = contentChanged;
      r.lastSyncTime = nowMs();
      r.isRunning = false;
      r.isIndexing = false;
      r.syncPhase = null;
    });
  }

  /// 尽力重算并广播当前状态（容错包装，对齐 Rust
  /// `push_live_transfer_state`）。
  @override
  Future<void> pushLiveTransferState() async {
    try {
      await recomputeAndBroadcastState();
    } catch (e) {
      AppLogger.d('实时状态发布失败（忽略）: $e');
    }
  }

  /// 清除传输历史并广播（对齐 Rust
  /// `clear_transfer_history_and_broadcast`）。
  @override
  Future<SyncGlobalState> clearTransferHistoryAndBroadcast({
    required bool includeCompleted,
    required bool includeFailed,
  }) async {
    final db = await this.db.database;
    final states = <int>[
      if (includeCompleted) TransferState.completed.code,
      if (includeFailed) TransferState.failed.code,
    ];
    if (states.isNotEmpty) {
      final placeholders = states.map((_) => '?').join(',');
      await db.rawDelete(
        'DELETE FROM transfer_queue WHERE state IN ($placeholders)',
        states,
      );
    }
    return recomputeAndBroadcastState();
  }
}
