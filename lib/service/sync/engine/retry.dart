/// 引擎重试切片（对齐 Rust `src/sync/engine/retry.rs`）。
library;

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎重试 mixin。
mixin EngineRetry on SyncEngineBase {
  /// 全局重试失败任务（走 RETRY 位周期）。
  @override
  Future<void> retryFailed() => runSyncCycle('retry-failed');

  /// 单任务重试（对齐 Rust `retry_transfer`）：
  /// prepare_retry 被拒且任务落为 RestartRequired 时改排 retry-replan 周期。
  @override
  Future<void> retryTransfer(int taskId) async {
    final guard = beginExternalActivity();
    try {
      final runner = requireTaskRunner();
      try {
        await runner.retry(taskId);
        notifyBackoffScheduleChanged();
      } catch (e) {
        // RestartRequired → 回 planner 重新规划（对齐
        // request_retry_replan_if_restart_required）
        final fresh = await runner.getTask(taskId);
        if (fresh != null && fresh.state == TransferState.RestartRequired) {
          AppLogger.i('任务 $taskId 需重新规划，排入 retry-replan 周期');
          requestCycleBackground('retry-replan');
          return;
        }
        rethrow;
      }
    } finally {
      guard.close();
    }
  }
}
