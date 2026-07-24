import 'dart:async';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/task_runner_base.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/types/enums.dart';

/// 同路径仲裁入队（对齐 Rust `task_runner/admission.rs`）。
///
/// 通过 `on TaskRunnerBase` 访问基类的 protected 字段与持久化/发布 helper
/// （`transferService` / `transition` / `publishSnapshot` /
/// `pumpLoop` / `track` / `promoteRestartToVerifying` /
/// `hasPersistedRemoteResult` / `dispositionForState` /
/// `isPathBlockingState` / `hasAmbiguousRemoteWriteResult` /
/// `outcomeWatchers` / `completeOutcome` / `syncHooks` / `runExpected`）。
mixin TaskRunnerAdmission on TaskRunnerBase {
  // ═══════════════════════════════════════════════════════════════════
  // 同路径仲裁入队（对齐 Rust task_runner/admission.rs:125-184）
  // ═══════════════════════════════════════════════════════════════════

  /// 入队新传输意图并执行，含同路径仲裁：
  /// 在途写优先 → 歧义重启提升 → 同意图去重 → 重规划 →
  /// 普通 RestartRequired 重规划 → Failed 路径屏障 → 插入新行。
  @override
  Future<AppResult<EnqueuedTaskOutcome>> enqueueAndRun(
    TransferTask task,
  ) async {
    if (task.id != 0 ||
        task.stateRevision != 0 ||
        task.state != TransferState.pending) {
      await publishSnapshot();
      return Err(const GenericError(
          message: '新传输意图必须是 id=0/revision=0 的 Pending 任务'));
    }
    final rel = task.relativePath;
    if (rel == null) {
      // 无路径任务：不参与仲裁，直接插入
      final inserted = await transferService.enqueue(task);
      if (inserted.isErr) return Err((inserted as Err).error);
      final stored = (inserted as Ok<TransferTask>).value;
      await publishSnapshot();
      final outcome = await runAndAwaitOutcome(stored);
      return Ok(EnqueuedTaskOutcome(taskId: stored.id, outcome: outcome));
    }

    final pathTasks =
        (await transferService.getTasksByRelativePath(rel)).unwrapOr([]);
    final blocking =
        pathTasks.where((t) => isPathBlockingState(t.state)).toList();

    // 1. 在途写优先：Running/VerifyingRemote
    final inflight = blocking
        .where((t) =>
            t.state == TransferState.running ||
            t.state == TransferState.verifyingRemote)
        .firstOrNull;
    if (inflight != null) {
      if (sameTransferIntent(inflight, task)) {
        await pumpLoop();
        return Ok(EnqueuedTaskOutcome(
          taskId: inflight.id,
          outcome: TaskExecutionOutcome(
              disposition: dispositionForState(inflight.state)),
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
            t.state == TransferState.restartRequired &&
            hasAmbiguousRemoteWriteResult(t))
        .firstOrNull;
    if (ambiguous != null) {
      final promoted = await promoteRestartToVerifying(ambiguous);
      final current = promoted ?? ambiguous;
      return Ok(EnqueuedTaskOutcome(
        taskId: current.id,
        outcome: const TaskExecutionOutcome(
            disposition: TaskDisposition.verifyingRemote),
      ));
    }

    // 3. 同意图去重
    final duplicate =
        blocking.where((t) => sameTransferIntent(t, task)).firstOrNull;
    if (duplicate != null) {
      await pumpLoop();
      return Ok(EnqueuedTaskOutcome(
        taskId: duplicate.id,
        outcome: TaskExecutionOutcome(
            disposition: dispositionForState(duplicate.state)),
      ));
    }

    // 4. 重规划（阻塞中的首个）
    if (blocking.isNotEmpty) {
      final replanned = await replanTask(blocking.first, task);
      if (replanned == null) {
        return Err(const GenericError(
            message: '任务重规划期间状态已变化，请等待下次同步'));
      }
      final outcome = await runAndAwaitOutcome(replanned);
      return Ok(EnqueuedTaskOutcome(taskId: replanned.id, outcome: outcome));
    }

    // 5. 普通 RestartRequired 重规划
    final restart = pathTasks
        .where((t) => t.state == TransferState.restartRequired)
        .firstOrNull;
    if (restart != null) {
      final replanned = await replanTask(restart, task);
      if (replanned == null) {
        return Err(const GenericError(
            message: '任务重规划期间状态已变化，请等待下次同步'));
      }
      final outcome = await runAndAwaitOutcome(replanned);
      return Ok(EnqueuedTaskOutcome(taskId: replanned.id, outcome: outcome));
    }

    // 6. Failed 路径屏障（保留可见错误供显式重试）
    final failed = pathTasks
        .where((t) => t.state == TransferState.failed)
        .firstOrNull;
    if (failed != null) {
      return Ok(EnqueuedTaskOutcome(
        taskId: failed.id,
        outcome: const TaskExecutionOutcome(
            disposition: TaskDisposition.blockedByActiveIntent),
      ));
    }

    // 7. 插入新行并执行
    final inserted = await transferService.enqueue(task);
    if (inserted.isErr) return Err((inserted as Err).error);
    final stored = (inserted as Ok<TransferTask>).value;
    await publishSnapshot();
    final outcome = await runAndAwaitOutcome(stored);
    return Ok(EnqueuedTaskOutcome(taskId: stored.id, outcome: outcome));
  }

  /// 同意图判定（对齐 Rust `same_transfer_intent`）。
  @override
  bool sameTransferIntent(TransferTask left, TransferTask right) {
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
      case TransferOperation.create:
      case TransferOperation.update:
        if (left.parentFileId != right.parentFileId ||
            left.sourceMtime != right.sourceMtime ||
            left.sourceSize != right.sourceSize) {
          return false;
        }
        if (left.operation == TransferOperation.update &&
            left.expectedCloudEditedTime != right.expectedCloudEditedTime) {
          return false;
        }
        return true;
      case TransferOperation.download:
      case TransferOperation.downloadUpdate:
        return left.parentFileId == right.parentFileId &&
            left.expectedCloudEditedTime == right.expectedCloudEditedTime;
      default:
        return false;
    }
  }

  /// 重规划任务（对齐 Rust `replan_task`）：
  /// → RestartRequired → Pending（清错误/远端结果）→ 裸 SQL 覆写意图列 →
  /// sync_items SYNCING 回写。
  Future<TransferTask?> replanTask(
    TransferTask current,
    TransferTask replacement,
  ) async {
    var cur = current;
    if (cur.state != TransferState.restartRequired) {
      final restarted = await transition(
        cur,
        TransferState.restartRequired,
        const TransferPatch(
          errorKind: SetPatch(TransferErrorKind.localChanged),
          errorMessage: SetPatch('新的 planner intent 已取代尚未执行的旧任务'),
          nextRetryAt: ClearPatch(),
          finishedAt: ClearPatch(),
        ),
      );
      if (restarted == null) return null;
      cur = restarted;
    }
    final sessionUrl = replacement.sessionUrl;
    final pending = await transition(
      cur,
      TransferState.pending,
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
        await transferService.overwriteReplanIntent(pending, replacement);
    if (result.isErr) {
      AppLogger.w('任务 ${pending.id} 重规划覆写失败: ${(result as Err).error}');
      return null;
    }
    final overwritten = (result as Ok<TransferTask?>).value;
    if (overwritten == null) return null;
    // sync_items SYNCING 回写（无旧状态条件）
    try {
      await syncHooks?.onTaskReplanned(overwritten);
    } catch (e) {
      AppLogger.w('重规划后 SYNCING 回写失败（忽略）: $e');
    }
    await publishSnapshot();
    return overwritten;
  }

  /// 执行任务并等待调度去向（入队仲裁路径专用）。
  @override
  Future<TaskExecutionOutcome> runAndAwaitOutcome(TransferTask task) async {
    final completer = Completer<TaskExecutionOutcome>();
    outcomeWatchers[task.id] = completer;
    track(task, () => runExpected(task));
    await pumpLoop();
    try {
      return await completer.future;
    } finally {
      outcomeWatchers.remove(task.id);
    }
  }
}
