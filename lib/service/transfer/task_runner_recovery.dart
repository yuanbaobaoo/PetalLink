import 'dart:async';
import 'dart:io';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart' show tmpPath;
import 'package:petal_link/service/sync/user_messages.dart';
import 'package:petal_link/service/transfer/task_runner_base.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/task_runner_execution_settlement.dart';
import 'package:petal_link/service/transfer/task_runner_preflight.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎恢复接缝（对齐 Rust `recovery.rs`）。
///
/// 通过 `on TaskRunnerBase, TaskRunnerExecutionSettlement` 访问基类与
/// execution_settlement 的成员（`transferService` / `operations` /
/// `isOnline` / `nowMs` / `inFlight` / `activePaths` / `outcomeWatchers` /
/// `track` / `transition` / `transitionFailure` / `patchInState` /
/// `persistPreflightRejection` / `promoteRestartToVerifying` /
/// `hasPersistedRemoteResult` / `hasAmbiguousRemoteWriteResult` /
/// `validateStatic` / `runExpected` / `validateSuccessOutcome` /
/// `settleSuccess`）。
mixin TaskRunnerRecovery
    on TaskRunnerBase, TaskRunnerExecutionSettlement {
  // ═══════════════════════════════════════════════════════════════════
  // 引擎恢复接缝（对齐 Rust recovery.rs 的公开面）
  // ═══════════════════════════════════════════════════════════════════

  /// 每周期批量提升歧义重启（对齐 Rust `promote_ambiguous_restarts`）。
  @override
  Future<int> promoteAmbiguousRestarts() async {
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    var promoted = 0;
    for (final task in active) {
      if (task.state == TransferState.restartRequired &&
          hasAmbiguousRemoteWriteResult(task)) {
        final result = await promoteRestartToVerifying(task);
        if (result != null) promoted++;
      }
    }
    return promoted;
  }

  /// 恢复到期远端核验（对齐 Rust `resume_verifying`）。
  ///
  /// 返回恢复汇总：到达 Completed 的任务数 + 核验确认的云端写入结果
  /// （对齐 Rust `TaskRecoverySummary`；引擎据此把恢复结果提交 live
  /// 云树与 checkpoint）。
  @override
  Future<TaskRecoverySummary> resumeVerifying() async {
    final summary = TaskRecoverySummary();
    if (!isOnline()) return summary;
    final now = nowMs();
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    final jobs = <(TransferTask, Future<TaskExecutionOutcome?>)>[];
    for (final task in active) {
      if (task.state != TransferState.verifyingRemote) continue;
      if (inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && activePaths.contains(rel)) continue;
      final dueAt = task.nextRetryAt;
      if (dueAt != null && dueAt > now) continue;
      final completer = Completer<TaskExecutionOutcome?>();
      track(task, () async {
        try {
          completer.complete(await resumeVerifyingTask(task));
        } catch (e) {
          // 对齐 Rust：单任务失败 warn 后继续处理其他任务
          AppLogger.w('任务 ${task.id} 远端核验恢复失败: $e');
          completer.complete(null);
        }
      });
      jobs.add((task, completer.future));
    }
    for (final (task, future) in jobs) {
      recordRecovered(summary, task, await future);
    }
    return summary;
  }

  /// 恢复等待网络任务（对齐 Rust `resume_waiting`）。
  @override
  Future<TaskRecoverySummary> resumeWaiting() async {
    final summary = TaskRecoverySummary();
    if (!isOnline()) return summary;
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    final jobs = <(TransferTask, Future<TaskExecutionOutcome?>)>[];
    for (final task in active) {
      if (task.state != TransferState.waitingForNetwork) continue;
      if (inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && activePaths.contains(rel)) continue;
      jobs.add((task, scheduleAndObserve(task)));
    }
    for (final (task, future) in jobs) {
      recordRecovered(summary, task, await future);
    }
    return summary;
  }

  /// 恢复到期退避任务（对齐 Rust `resume_due_backoff`）。
  @override
  Future<TaskRecoverySummary> resumeDueBackoff() async {
    final summary = TaskRecoverySummary();
    if (!isOnline()) return summary;
    final now = nowMs();
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    final jobs = <(TransferTask, Future<TaskExecutionOutcome?>)>[];
    for (final task in active) {
      if (task.state != TransferState.backingOff) continue;
      if (inFlight.containsKey(task.id)) continue;
      final rel = task.relativePath;
      if (rel != null && activePaths.contains(rel)) continue;
      final dueAt = task.nextRetryAt;
      if (dueAt != null && dueAt > now) continue;
      jobs.add((task, scheduleAndObserve(task)));
    }
    for (final (task, future) in jobs) {
      recordRecovered(summary, task, await future);
    }
    return summary;
  }

  /// 调度执行并观察 outcome（watcher 模式；异常归一为 null，
  /// 对齐 Rust warn-and-continue）。
  Future<TaskExecutionOutcome?> scheduleAndObserve(TransferTask task) {
    final completer = Completer<TaskExecutionOutcome>();
    outcomeWatchers[task.id] = completer;
    track(task, () => runExpected(task));
    return completer.future.then<TaskExecutionOutcome?>(
      (outcome) => outcome,
      onError: (_) => null,
    ).whenComplete(() => outcomeWatchers.remove(task.id));
  }

  /// 汇总单个恢复任务（对齐 Rust `record_recovered_task`）：
  /// 仅 Completed 计数；携带云端元数据且有相对路径时记录恢复文件。
  void recordRecovered(
    TaskRecoverySummary summary,
    TransferTask task,
    TaskExecutionOutcome? outcome,
  ) {
    if (outcome == null || outcome.disposition != TaskDisposition.completed) {
      return;
    }
    summary.completed++;
    final rel = task.relativePath;
    final file = outcome.cloudFile;
    if (rel != null && file != null) {
      summary.recoveredCloudFiles
          .add(RecoveredCloudFile(relativePath: rel, file: file));
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 远端核验（对齐 Rust recovery.rs resume_verifying_task）
  // ═══════════════════════════════════════════════════════════════════

  /// 核验并结算一个远端结果不确定的任务。
  /// 返回结算后的修正 outcome（Completed 时供恢复汇总记录）；
  /// 未决/歧义/失败返回 null（对齐 Rust resume_verifying_task 的
  /// Option 语义）。
  @override
  Future<TaskExecutionOutcome?> resumeVerifyingTask(TransferTask task) async {
    final RemoteVerification verification;
    try {
      verification = await operations.verifyRemote(task);
    } catch (e) {
      // 核验暂不可用：保留歧义状态，稍后重试
      AppLogger.w('任务 ${task.id} 远端写入核验暂不可用，保留歧义状态: $e');
      await patchInState(
        task,
        TransferState.verifyingRemote,
        TransferPatch(
          errorMessage: SetPatch(simplifySyncError('远端核验暂不可用：$e')),
          nextRetryAt:
              SetPatch(nowMs() + TaskRunnerBase.verifyUnavailableDelayMs),
        ),
      );
      return null;
    }
    switch (verification) {
      case RemoteCommitted(:final file):
        final outcome = TaskExecutionOutcome(
          cloudFile: file,
          disposition: TaskDisposition.completed,
        );
        try {
          await validateSuccessOutcome(task, outcome);
        } on PreflightFailure catch (failure) {
          final patch = TransferPatch(
            errorKind: SetPatch(failure.kind),
            errorMessage: SetPatch(simplifySyncError(
                '远端写入已确认，但结果仍无法安全结算：${failure.message}')),
            nextRetryAt: failure.target == TransferState.verifyingRemote
                ? SetPatch(nowMs() + TaskRunnerBase.verifyAmbiguousDelayMs)
                : const ClearPatch(),
            remoteResultFileId: SetPatch(file.id),
          );
          if (failure.target == TransferState.verifyingRemote) {
            await patchInState(task, TransferState.verifyingRemote, patch);
          } else {
            await transition(task, failure.target, patch);
          }
          return null;
        }
        return settleSuccess(task, outcome);
      case RemoteNotCommitted():
        final sessionExpired =
            task.errorKind == TransferErrorKind.sessionExpired;
        final restart = await transition(
          task,
          TransferState.restartRequired,
          TransferPatch(
            errorKind: SetPatch(
              sessionExpired
                  ? TransferErrorKind.sessionExpired
                  : TransferErrorKind.remoteAmbiguous,
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
        if (restart == null) return null;
        // 转 Pending 后由调度泵重新执行（对齐 Rust 链式 run_expected）
        await transition(
          restart,
          TransferState.pending,
          const TransferPatch.clearingError(),
        );
        return null;
      case RemoteAmbiguous(:final message):
        // 保留会话过期标记，直至确定远端不存在结果
        final kind = task.errorKind == TransferErrorKind.sessionExpired
            ? TransferErrorKind.sessionExpired
            : TransferErrorKind.remoteAmbiguous;
        await patchInState(
          task,
          TransferState.verifyingRemote,
          TransferPatch(
            errorKind: SetPatch(kind),
            errorMessage: SetPatch(message),
            nextRetryAt:
                SetPatch(nowMs() + TaskRunnerBase.verifyAmbiguousDelayMs),
          ),
        );
        return null;
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 崩溃恢复（对齐 Rust recovery.rs recover_startup）
  // ═══════════════════════════════════════════════════════════════════

  /// 启动恢复：歧义重启提升 → 同路径重复收敛 → 中断 Running 行按操作分流。
  @override
  Future<void> recoverStartup() async {
    final active = (await transferService.getActiveTasks()).unwrapOr([]);

    // 1. 含远端结果 ID 的 RestartRequired → VerifyingRemote（promote_ambiguous_restarts）
    for (final task in active) {
      if (task.state == TransferState.restartRequired &&
          hasPersistedRemoteResult(task)) {
        await promoteRestartToVerifying(task);
      }
    }

    // 2. Pending + Running 行按同路径分组收敛（最新一条胜出）
    final tasks = active
        .where((t) =>
            t.state == TransferState.pending || t.state == TransferState.running)
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
            t.state == TransferState.running &&
            (t.operation == TransferOperation.create ||
                t.operation == TransferOperation.update),
      );
      if (hasRunningRemoteWrite) {
        for (final task in samePath) {
          await suppressStartupDuplicate(task);
        }
        continue;
      }
      // 组内已按 created_at 倒序，首条为最新意图
      selected.add(samePath.first);
      for (final task in samePath.skip(1)) {
        await suppressStartupDuplicate(task);
      }
    }

    // 3. 逐任务恢复中断行
    for (final task in selected) {
      try {
        await recoverStartupTask(task);
      } catch (e, st) {
        AppLogger.e('任务 ${task.id} 启动恢复失败，继续处理其他任务', e, st);
      }
    }
  }

  /// 抑制启动期同路径旧任务（对齐 suppress_startup_duplicate）。
  Future<void> suppressStartupDuplicate(TransferTask task) async {
    if (task.state == TransferState.running &&
        (task.operation == TransferOperation.create ||
            task.operation == TransferOperation.update)) {
      await transitionFailure(
        task,
        TransferState.verifyingRemote,
        TransferErrorKind.remoteAmbiguous,
        '启动恢复发现同路径多个活动任务；旧远端写入等待核验',
      );
      return;
    }
    await transitionFailure(
      task,
      TransferState.restartRequired,
      task.state == TransferState.running
          ? TransferErrorKind.sessionExpired
          : TransferErrorKind.localChanged,
      '启动恢复仅保留同路径最新任务，旧任务等待重新规划',
    );
  }

  /// 恢复单个启动期任务行（Pending 交给调度器；Running 按操作分流）。
  Future<void> recoverStartupTask(TransferTask task) async {
    if (task.state == TransferState.pending) return;
    switch (task.operation) {
      case TransferOperation.create:
      case TransferOperation.update:
        await transitionFailure(
          task,
          TransferState.verifyingRemote,
          TransferErrorKind.remoteAmbiguous,
          '进程中断时远端写入结果不确定，等待核验',
        );
      case TransferOperation.download:
      case TransferOperation.downloadUpdate:
        try {
          await validateStatic(task);
        } on PreflightFailure catch (failure) {
          await persistPreflightRejection(task, failure);
          return;
        }
        // 下载断点以磁盘 .tmp 实际大小为准（对齐 recovery.rs durable_offset）
        final durable = await durableDownloadOffset(task);
        final restart = await transitionFailure(
          task,
          TransferState.restartRequired,
          TransferErrorKind.sessionExpired,
          '进程中断，保留已验证下载断点并重新建立 Range 请求',
        );
        if (restart == null) return;
        // 对齐 Rust recovery.rs：Pending 补丁清错误但保留 next_retry_at（Keep）
        await transition(
          restart,
          TransferState.pending,
          TransferPatch(
            errorKind: const ClearPatch(),
            errorMessage: const ClearPatch(),
            finishedAt: const ClearPatch(),
            transferred: durable,
            resumeOffset: durable,
          ),
        );
      case TransferOperation.delete ||
            TransferOperation.move ||
            TransferOperation.rename ||
            TransferOperation.createFolder:
        // Flutter 扩展操作：files API 写后验证保证可安全重放
        final restart = await transitionFailure(
          task,
          TransferState.restartRequired,
          TransferErrorKind.sessionExpired,
          '进程中断，远端写操作重新调度',
        );
        if (restart == null) return;
        await transition(
          restart,
          TransferState.pending,
          const TransferPatch.clearingError(),
        );
      case null:
        await transitionFailure(
          task,
          TransferState.failed,
          TransferErrorKind.validation,
          '中断任务缺少合法 operation',
        );
    }
  }

  /// 读取下载断点的磁盘真值：.tmp 实际大小（不超过 totalSize），缺失为 0。
  Future<int> durableDownloadOffset(TransferTask task) async {
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
}
