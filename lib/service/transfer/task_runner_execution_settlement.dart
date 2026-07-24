import 'dart:io';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/sync/user_messages.dart';
import 'package:petal_link/service/transfer/retry_policy.dart';
import 'package:petal_link/service/transfer/task_runner_base.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/task_runner_preflight.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/types/enums.dart';

/// 执行主链与结果结算（对齐 Rust `run_expected` + `settlement.rs`）。
///
/// 通过 `on TaskRunnerBase` 访问基类的 protected 字段与 helper
/// （`operations` / `transferService` / `isOnline` / `nowMs` / `jitterMs` /
/// `maxAttempts` / `onRequestNetworkFailure` / `syncHooks` / `transition` /
/// `transitionFailure` / `persistPreflightRejection` / `promoteRestartToVerifying` /
/// `hasPersistedRemoteResult` / `validateStatic` / `progressCallbacks` /
/// `publishSnapshot` / `completeOutcome` / `failOutcome` / `dispositionForState`）。
mixin TaskRunnerExecutionSettlement on TaskRunnerBase {
  // ═══════════════════════════════════════════════════════════════════
  // 执行主链（对齐 Rust run_expected）
  // ═══════════════════════════════════════════════════════════════════

  /// 执行单个可运行任务（Pending/WaitingForNetwork/BackingOff）。
  ///
  /// [runBackendPreflight] 对齐 Rust `run_expected` 同名参数：
  /// 手动 retry 已完成一次后端前置校验，链路内不再重复执行。
  @override
  Future<void> runExpected(
    TransferTask current, {
    bool runBackendPreflight = true,
  }) async {
    final state = current.state;
    if (state != TransferState.pending &&
        state != TransferState.waitingForNetwork &&
        state != TransferState.backingOff) {
      AppLogger.w('任务 ${current.id} 状态 ${state.name} 不可执行');
      failOutcome(current.id,
          AppError.generic('任务状态 ${state.name} 不可执行'));
      await publishSnapshot();
      return;
    }
    if (state == TransferState.backingOff && current.nextRetryAt == null) {
      await persistPreflightRejection(
        current,
        const PreflightFailure.validation('退避任务缺少 next_retry_at，拒绝立即重放'),
      );
      failOutcome(current.id,
          AppError.generic('退避任务缺少 next_retry_at，拒绝立即重放'));
      return;
    }
    // 静态前置校验
    try {
      await validateStatic(current);
    } on PreflightFailure catch (failure) {
      await persistPreflightRejection(current, failure);
      if (failure.target == TransferState.failed) {
        failOutcome(current.id, AppError.generic(failure.message));
      } else {
        completeOutcome(
            current.id,
            TaskExecutionOutcome(
                disposition: dispositionForState(failure.target)));
      }
      return;
    }
    // 在线门控（离线：Pending → WaitingForNetwork；其余停留）
    if (!isOnline()) {
      if (state == TransferState.pending) {
        await transitionFailure(
          current,
          TransferState.waitingForNetwork,
          TransferErrorKind.network,
          '网络不可用，等待恢复',
        );
        completeOutcome(
            current.id,
            const TaskExecutionOutcome(
                disposition: TaskDisposition.waitingForNetwork));
      } else {
        completeOutcome(
            current.id,
            TaskExecutionOutcome(
                disposition: dispositionForState(state)));
        await publishSnapshot();
      }
      return;
    }
    // 退避到期检查（对齐 Rust notify_rejection：早退也发布一次快照）
    if (state == TransferState.backingOff &&
        (current.nextRetryAt ?? 0) > nowMs()) {
      completeOutcome(
          current.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.backingOff));
      await publishSnapshot();
      return;
    }
    // 后端前置校验
    if (runBackendPreflight) {
      try {
        await operations.preflight(current);
      } on BackendPreflightFailure catch (failure) {
        await persistPreflightRejection(
          current,
          PreflightFailure(
            target: failure.target,
            kind: failure.kind,
            message: failure.message,
          ),
        );
        if (failure.target == TransferState.failed) {
          failOutcome(current.id, AppError.generic(failure.message));
        } else {
          completeOutcome(
              current.id,
              TaskExecutionOutcome(
                  disposition: dispositionForState(failure.target)));
        }
        return;
      }
    }
    // Running 仲裁（同路径排他 + 歧义重启提升）
    final running = await transitionToRunningOrBlock(current);
    if (running == null) {
      completeOutcome(
          current.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.blockedByActiveIntent));
      return;
    }

    // 执行传输
    final progress = progressCallbacks(running);
    try {
      final outcome = await operations.execute(running, progress);
      // 对齐 progress.ensure_current：任务被并发推进后忽略过期回调结果
      if (!await ensureCurrent(running)) {
        failOutcome(running.id, AppError.generic('传输任务状态已变化'));
        return;
      }
      // 对齐 Rust execution.rs：结算可能把任务落库为
      // VerifyingRemote/RestartRequired，必须按落库状态修正 outcome 再
      // 上报，否则引擎会按原始 completed 误结算基线
      final corrected = await settleOutcome(running, outcome);
      if (corrected == null) {
        // 非法成功核验目标（对齐 Rust 返回 Err）
        failOutcome(running.id, AppError.generic('非法成功核验目标状态'));
        return;
      }
      completeOutcome(running.id, corrected);
    } on TaskRestartRequired catch (e) {
      await transitionFailure(
        running,
        TransferState.restartRequired,
        TransferErrorKind.localChanged,
        e.message,
      );
      completeOutcome(
          running.id,
          const TaskExecutionOutcome(
              disposition: TaskDisposition.restartRequired));
    } on TaskAppError catch (e) {
      await settleError(running, e.error);
    } catch (e) {
      await settleError(running, AppError.generic('$e'));
    }
  }

  /// 确认任务仍指向同一 Running 修订（对齐 Rust `ensure_current`）。
  Future<bool> ensureCurrent(TransferTask running) async {
    final fresh =
        (await transferService.getTaskById(running.id)).unwrapOr(null);
    if (fresh == null ||
        fresh.stateRevision != running.stateRevision ||
        fresh.state != TransferState.running) {
      AppLogger.d('任务 ${running.id} 状态已变化，忽略过期回调');
      return false;
    }
    return true;
  }

  /// Running 仲裁（对齐 Rust `transition_to_running_or_block`）：
  /// 同路径存在 Running/VerifyingRemote 任务时被阻塞；
  /// 同路径含已持久远端结果的 RestartRequired 任务先全部提升为待核验。
  @override
  Future<TransferTask?> transitionToRunningOrBlock(TransferTask current) async {
    final rel = current.relativePath;
    if (rel == null) {
      // 对齐 Rust：直接返回错误，任务停留原态（静态校验已拦截，实际不可达）
      AppLogger.w('任务 ${current.id} Running 仲裁缺少 relative_path');
      await publishSnapshot();
      return null;
    }
    final active = (await transferService.getActiveTasks()).unwrapOr([]);
    var promotedAny = false;
    for (final candidate in active) {
      if (candidate.id == current.id || candidate.relativePath != rel) continue;
      if (candidate.state == TransferState.running ||
          candidate.state == TransferState.verifyingRemote) {
        AppLogger.d('任务 ${current.id} 被同路径活动意图 ${candidate.id} 阻塞');
        return null;
      }
      if (candidate.state == TransferState.restartRequired &&
          hasPersistedRemoteResult(candidate)) {
        await promoteRestartToVerifying(
          candidate,
          message: '远端结果 ID 已存在；Running 仲裁禁止重放并等待核验',
        );
        promotedAny = true;
      }
    }
    // 有歧义重启被提升时，本任务等待核验结果后再调度
    if (promotedAny) return null;
    return transition(
      current,
      TransferState.running,
      const TransferPatch.clearingError(),
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 结算（对齐 Rust settlement.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 按后端执行结果结算（成功核验 / 延迟状态持久化）。
  ///
  /// 返回**按落库状态修正后**的 outcome（对齐 Rust execution.rs 的
  /// `output.disposition` 改写）：任务被迁移为 VerifyingRemote /
  /// RestartRequired 时，上报的 disposition 同步修正，引擎不得按原始
  /// completed 结算。返回 null 表示非法成功核验目标（对齐 Rust Err）。
  Future<TaskExecutionOutcome?> settleOutcome(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    switch (outcome.disposition) {
      case TaskDisposition.completed:
        try {
          await validateSuccessOutcome(running, outcome);
        } on PreflightFailure catch (failure) {
          // 上传且远端已返回资源 ID → 禁止直接重放，进入核验
          final remoteId = outcome.cloudFile?.id;
          final isUpload = running.operation == TransferOperation.create ||
              running.operation == TransferOperation.update;
          final TransferState target;
          if (isUpload && remoteId != null && remoteId.trim().isNotEmpty) {
            target = TransferState.verifyingRemote;
            await transition(
              running,
              TransferState.verifyingRemote,
              TransferPatch(
                errorKind: const SetPatch(TransferErrorKind.remoteAmbiguous),
                errorMessage: SetPatch(simplifySyncError(
                    '${failure.message}；远端已返回资源 ID，禁止直接重放')),
                remoteResultFileId: SetPatch(remoteId),
              ),
            );
          } else {
            // 对齐 Rust execution.rs：finished_at 保持 Keep；
            // 远端已返回资源 ID 时仍持久化 remote_result_file_id
            target = failure.target;
            await transition(
              running,
              failure.target,
              TransferPatch(
                errorKind: SetPatch(failure.kind),
                errorMessage: SetPatch(simplifySyncError(failure.message)),
                remoteResultFileId: remoteId != null && remoteId.trim().isNotEmpty
                    ? SetPatch(remoteId)
                    : const KeepPatch(),
              ),
            );
          }
          // 按落库状态修正 outcome（对齐 Rust output.disposition 改写）
          return switch (target) {
            TransferState.verifyingRemote => TaskExecutionOutcome(
                cloudFile: outcome.cloudFile,
                disposition: TaskDisposition.verifyingRemote,
              ),
            TransferState.restartRequired => TaskExecutionOutcome(
                cloudFile: outcome.cloudFile,
                disposition: TaskDisposition.restartRequired,
              ),
            _ => null, // Failed 等非法成功核验目标：对齐 Rust 返回 Err
          };
        }
        return settleSuccess(running, outcome);
      case TaskDisposition.verifyingRemote:
        await transition(
          running,
          TransferState.verifyingRemote,
          TransferPatch(
            errorKind: const SetPatch(TransferErrorKind.remoteAmbiguous),
            errorMessage:
                const SetPatch('远端写入已返回资源 ID，但完整元数据尚未确认'),
            nextRetryAt: SetPatch(
                nowMs() + TaskRunnerBase.verifyInitialDelayMs),
            remoteResultFileId: outcome.cloudFile != null
                ? SetPatch(outcome.cloudFile!.id)
                : const KeepPatch(),
          ),
        );
        return outcome;
      case TaskDisposition.waitingForNetwork:
        await transition(
          running,
          TransferState.waitingForNetwork,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.network),
            errorMessage: SetPatch('后端请求等待网络恢复'),
          ),
        );
        return outcome;
      case TaskDisposition.restartRequired:
        await transition(
          running,
          TransferState.restartRequired,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.localChanged),
            errorMessage: SetPatch('本地源已变化，需要重新规划'),
          ),
        );
        return outcome;
      case TaskDisposition.pending ||
            TaskDisposition.running ||
            TaskDisposition.blockedByActiveIntent ||
            TaskDisposition.backingOff:
        await settleError(
          running,
          AppError.generic(
              '后端返回缺少可持久化恢复条件的状态 ${outcome.disposition.name}'),
        );
        // settleError 已自行完结 watcher，返回值不再被消费
        return outcome;
    }
  }

  /// 根据错误分类持久化失败或恢复状态（对齐 Rust `settle_error`）。
  Future<void> settleError(TransferTask running, AppError error) async {
    final operation = running.operation;
    if (operation == null) {
      await transitionFailure(
        running,
        TransferState.failed,
        TransferErrorKind.validation,
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
        nowMs: nowMs(),
        jitterMs: jitterMs(),
        authAlreadyReplayed: false,
        maxAttempts: maxAttempts,
      ),
    );
    final attempts =
        running.attemptCount + (classified.consumesRetryBudget ? 1 : 0);
    // 请求级网络失败边沿上报（对齐 Rust engine/publication.rs：
    // 仅「等待计数边沿增加且当前在线」时上报；离线期间由 NetGuard 探测主导）
    if (classified.decision is WaitForNetworkDecision && isOnline()) {
      onRequestNetworkFailure?.call();
    }
    final (state, nextRetryAt) = switch (classified.decision) {
      WaitForNetworkDecision() => (
          TransferState.waitingForNetwork,
          const ClearPatch<int>(),
        ),
      BackoffDecision(:final nextRetryAt) => (
          TransferState.backingOff,
          SetPatch<int>(nextRetryAt),
        ),
      VerifyRemoteDecision() => (
          TransferState.verifyingRemote,
          SetPatch<int>(nowMs() + TaskRunnerBase.verifyInitialDelayMs),
        ),
      // DriveClient 负责唯一一次带认证重放；首次 401 不由 runner 盲目重放
      RefreshAuthDecision() => (
          TransferState.failed,
          const ClearPatch<int>(),
        ),
      FailDecision() => (
          TransferState.failed,
          const ClearPatch<int>(),
        ),
    };
    // 对齐 Rust settlement.rs：技术错误在落库前转换为用户可读提示，
    // 日志仍保留技术原文（见各调用点 AppLogger.w）。
    final userMessage = simplifySyncError('$error');
    final updated = await transition(
      running,
      state,
      TransferPatch(
        errorKind: SetPatch(classified.kind),
        errorMessage: SetPatch(userMessage),
        nextRetryAt: nextRetryAt,
        finishedAt: state == TransferState.failed
            ? SetPatch(nowMs())
            : const ClearPatch(),
        attemptCount: attempts,
      ),
    );
    if (state == TransferState.failed) {
      // 永久失败：sync_items FAILED 回写（仅旧状态白名单覆盖）
      if (updated != null) {
        try {
          await syncHooks?.onTaskFailed(updated, userMessage);
        } catch (e) {
          AppLogger.w('任务 ${running.id} FAILED 基线回写失败（忽略）: $e');
        }
      }
      failOutcome(running.id, error);
    } else {
      completeOutcome(
        running.id,
        TaskExecutionOutcome(disposition: dispositionForState(state)),
      );
    }
  }

  /// 原子完成任务结算（对齐 Rust `settle_success` 的任务行部分；
  /// sync_items 基线结算属引擎任务接缝）。
  Future<TaskExecutionOutcome> settleSuccess(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    final operation = running.operation;
    if (operation == null) {
      await settleError(running, AppError.generic('任务缺少 operation'));
      // settleError 已自行完结 watcher，返回值不再被消费
      return outcome;
    }
    final String? resultFileId = switch (operation) {
      TransferOperation.create ||
      TransferOperation.update =>
        outcome.cloudFile?.id,
      TransferOperation.download ||
      TransferOperation.downloadUpdate =>
        running.fileId,
      _ => outcome.cloudFile?.id ?? running.fileId,
    };
    // 先结算 sync_items 基线（对齐 Rust settle_success 同事务语义的最佳近似：
    // 基线结算失败禁止完成任务行，进入恢复路径）
    final hooks = syncHooks;
    if (hooks != null) {
      try {
        await hooks.onTaskCommitted(running, outcome);
      } catch (e) {
        AppLogger.w('任务 ${running.id} 基线结算失败，进入恢复路径: $e');
        const message = '后端已完成，但本地同步基线结算失败';
        switch (operation) {
          case TransferOperation.create:
          case TransferOperation.update:
            await transition(
              running,
              TransferState.verifyingRemote,
              TransferPatch(
                errorKind: const SetPatch(TransferErrorKind.remoteAmbiguous),
                errorMessage: const SetPatch(message),
                remoteResultFileId: outcome.cloudFile != null
                    ? SetPatch(outcome.cloudFile!.id)
                    : const KeepPatch(),
              ),
            );
            // 按落库状态修正 outcome（对齐 Rust recover_success_settlement_failure）
            return TaskExecutionOutcome(
              cloudFile: outcome.cloudFile,
              disposition: TaskDisposition.verifyingRemote,
            );
          case TransferOperation.download:
          case TransferOperation.downloadUpdate:
            await transition(
              running,
              TransferState.restartRequired,
              const TransferPatch(
                errorKind: SetPatch(TransferErrorKind.unknown),
                errorMessage: SetPatch(message),
              ),
            );
            return TaskExecutionOutcome(
              cloudFile: outcome.cloudFile,
              disposition: TaskDisposition.restartRequired,
            );
          default:
            return outcome;
        }
      }
    }
    final completed = await transition(
      running,
      TransferState.completed,
      TransferPatch.clearingError(
        finishedAt: SetPatch(nowMs()),
        remoteResultFileId: resultFileId != null
            ? SetPatch(resultFileId)
            : const KeepPatch(),
        transferred: running.totalSize,
      ),
    );
    if (completed != null) return outcome;
    // Completed 迁移失败（CAS 冲突或 DB 错误）：
    // 对齐 Rust recover_success_settlement_failure——后端已完成但结算未落地，
    // 上传禁止盲目重放（转远端核验），下载回 planner 重新规划。
    const message = '后端已完成，但本地同步基线结算失败（迁移被拒绝或写入失败）';
    switch (operation) {
      case TransferOperation.create:
      case TransferOperation.update:
        await transition(
          running,
          TransferState.verifyingRemote,
          TransferPatch(
            errorKind: const SetPatch(TransferErrorKind.remoteAmbiguous),
            errorMessage: const SetPatch(message),
            remoteResultFileId: outcome.cloudFile != null
                ? SetPatch(outcome.cloudFile!.id)
                : const KeepPatch(),
          ),
        );
        return TaskExecutionOutcome(
          cloudFile: outcome.cloudFile,
          disposition: TaskDisposition.verifyingRemote,
        );
      case TransferOperation.download:
      case TransferOperation.downloadUpdate:
        await transition(
          running,
          TransferState.restartRequired,
          const TransferPatch(
            errorKind: SetPatch(TransferErrorKind.unknown),
            errorMessage: SetPatch(message),
          ),
        );
        return TaskExecutionOutcome(
          cloudFile: outcome.cloudFile,
          disposition: TaskDisposition.restartRequired,
        );
      default:
        // Flutter 扩展操作：迁移失败时任务滞留 Running，由启动恢复收敛
        AppLogger.w('任务 ${running.id} 完成结算迁移失败，滞留 Running 等待启动恢复');
        return outcome;
    }
  }

  /// 校验完成结果是否可安全结算（对齐 Rust `validate_success_outcome`）。
  Future<void> validateSuccessOutcome(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    final operation = running.operation;
    if (operation == null) {
      throw const PreflightFailure.validation('成功核验缺少 operation');
    }
    switch (operation) {
      case TransferOperation.create:
      case TransferOperation.update:
        final cloud = outcome.cloudFile;
        if (cloud == null) {
          throw const PreflightFailure.remoteAmbiguous('上传结果缺少远端资源');
        }
        if (cloud.id.trim().isEmpty ||
            cloud.name.trim().isEmpty ||
            cloud.name != running.name ||
            cloud.editedTime == null ||
            cloud.size != (running.sourceSize ?? -1) ||
            (operation == TransferOperation.update &&
                running.fileId != cloud.id)) {
          throw const PreflightFailure.remoteAmbiguous('上传结果元数据不完整或大小不一致');
        }
      case TransferOperation.download:
      case TransferOperation.downloadUpdate:
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
      case TransferOperation.delete ||
            TransferOperation.move ||
            TransferOperation.rename ||
            TransferOperation.createFolder:
        // Flutter 扩展操作：files API 已完成写后验证（身份/名称/recycled）
        break;
    }
  }
}
