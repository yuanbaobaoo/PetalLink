/// 引擎重试切片（对齐 Rust `src/sync/engine/retry.rs`）。
library;

import 'dart:async';

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎重试 mixin。
mixin EngineRetry on SyncEngineBase {
  /// 全局重试失败任务（走 RETRY 位周期）。
  @override
  Future<void> retryFailed() => runSyncCycle('retry-failed');

  /// 单任务重试（对齐 Rust `retry_transfer`）：
  /// - prepare 阶段同步执行（拒绝立即反馈；RestartRequired 改排 retry-replan 周期）
  /// - 执行阶段后台进行（不阻塞调用方），outcome 消费对齐 Rust：
  ///   RestartRequired → 重规划；cloudFile → 回插 live 云树 + pathToId；
  ///   错误 → 日志（非吞没），RestartRequired 时改排重规划
  @override
  Future<void> retryTransfer(int taskId) async {
    final guard = beginExternalActivity();
    final runner = requireTaskRunner();
    // prepare 同步等待（对齐 Rust prepare_retry 的错误即时返回语义）
    final prepared = await runner.prepareRetry(taskId);
    if (prepared.isErr) {
      // RestartRequired → 回 planner 重新规划（对齐
      // request_retry_replan_if_restart_required）
      final fresh = await runner.getTask(taskId);
      if (fresh != null && fresh.state == TransferState.restartRequired) {
        AppLogger.i('任务 $taskId 需重新规划，排入 retry-replan 周期');
        requestCycleBackground('retry-replan');
        guard.close();
        return;
      }
      guard.close();
      throw (prepared as Err).error;
    }
    final pending = (prepared as Ok).value;
    // 执行阶段后台进行（对齐 Rust tauri::async_runtime::spawn；
    // activity guard 随后台任务持闭）
    unawaited(() async {
      try {
        final outcome = await runner.runPreparedAndAwait(pending.id);
        if (outcome.disposition == TaskDisposition.restartRequired) {
          requestCycleBackground('retry-replan');
        } else {
          final cloudFile = outcome.cloudFile;
          if (cloudFile != null) {
            // 对齐 Rust：成功后回插 cloud_tree + path_to_id
            final fresh = await runner.getTask(taskId);
            final rel = fresh?.relativePath;
            if (rel != null) {
              cloudIndex.insert(rel, cloudFile);
            }
          }
        }
      } catch (e) {
        // 对齐 Rust：Err 不吞没——RestartRequired 改排重规划，否则记日志
        final fresh = await runner.getTask(taskId);
        if (fresh != null && fresh.state == TransferState.restartRequired) {
          requestCycleBackground('retry-replan');
        } else {
          AppLogger.w('后台重试任务 $taskId 失败: $e');
        }
      } finally {
        notifyBackoffScheduleChanged();
        guard.close();
      }
    }());
  }
}
