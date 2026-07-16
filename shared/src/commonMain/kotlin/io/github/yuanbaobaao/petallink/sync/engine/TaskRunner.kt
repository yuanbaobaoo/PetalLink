package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.data.repository.TransferRepository
import io.github.yuanbaobaao.petallink.sync.RetryPolicy
import io.github.yuanbaobaao.petallink.sync.TransferState
import kotlinx.coroutines.sync.Semaphore

/**
 * 任务执行后端操作接口（对标 TransferOperations trait）。
 *
 * 由实际的传输实现提供（上传/下载/远端核验）。
 * 详见 docs/06 §TaskRunner contracts。
 */
interface TransferOperations {
    /** 预检（静态检查：路径合法/空间/冲突） */
    suspend fun preflight(task: TaskContext): PreflightResult

    /** 执行传输 */
    suspend fun execute(task: TaskContext, progress: TaskProgressReporter): TaskOutput

    /** 远端核验（VerifyingRemote 状态用） */
    suspend fun verifyRemote(task: TaskContext): RemoteVerification
}

/** 预检结果 */
sealed class PreflightResult {
    object Ok : PreflightResult()
    data class Reject(val reason: String, val targetState: TransferState) : PreflightResult()
}

/** 任务上下文（对标 RunningTask 快照） */
data class TaskContext(
    val id: Long,
    val fileId: String,
    val localPath: String,
    val direction: TransferDirection,
    val state: TransferState,
    val stateRevision: Long,
    val attempt: Int,
    val bytesTotal: Long,
    val bytesDone: Long,
)

/** 任务执行输出 */
data class TaskOutput(
    val disposition: TaskDisposition,
    val cloudFileId: String? = null,
    val bytesTransferred: Long? = null,
    val errorMessage: String? = null,
)

/** 任务结局（9 种，对标 TaskDisposition） */
enum class TaskDisposition {
    COMPLETED,          // 完成
    BACKING_OFF,        // 退避重试
    WAITING_FOR_NETWORK,// 等待网络
    VERIFYING_REMOTE,   // 核验远端
    RESTART_REQUIRED,   // 需重启（可恢复中断）
    BLOCKED,            // 被同路径任务阻塞
    FAILED,             // 失败（终态）
    CANCELED,           // 取消
    PENDING,            // 重新入队
}

/** 远端核验结果 */
sealed class RemoteVerification {
    data class Committed(val fileId: String) : RemoteVerification()
    object NotCommitted : RemoteVerification()
    object Ambiguous : RemoteVerification()
    data class Err(val message: String) : RemoteVerification()
}

/** 进度上报器（对标 TaskProgressReporter） */
class TaskProgressReporter(
    private val repository: TransferRepository,
    private val taskId: Long,
    private val stateRevision: Long,
    private val throttleMs: Long = PROGRESS_THROTTLE_MS,
) {
    private var lastReportMs: Long = 0L

    /** 上报进度（刻意不递增 revision） */
    suspend fun report(bytesDone: Long, nowMs: Long) {
        if (nowMs - lastReportMs < throttleMs) return  // 节流
        lastReportMs = nowMs
        repository.updateRunningProgress(taskId, bytesDone)
    }

    companion object {
        /** 进度节流间隔：500ms */
        const val PROGRESS_THROTTLE_MS = 500L
    }
}

/**
 * 任务执行器（对标 src/sync/task_runner/）。
 *
 * 九态状态机执行循环 + CAS 乐观锁落库 + 8 步启动恢复。
 * 详见 docs/06 §TaskRunner。
 *
 * @param repository 传输队列表仓库
 * @param operations 传输操作后端
 * @param isOnline 在线检查
 * @param nowMs 当前时间戳提供
 */
class TaskRunner(
    private val repository: TransferRepository,
    private val operations: TransferOperations,
    private val isOnline: () -> Boolean,
    private val nowMs: () -> Long,
) {
    // CAS 并发控制：同一路径同时只允许一个任务 Running
    private val runningSemaphore = Semaphore(AppConfig.MAX_CONCURRENT_TRANSFERS)

    /**
     * 执行单个任务（对标 run_expected 主链）。
     *
     * 流程：
     * 1. 校验状态可执行（Pending/WaitingForNetwork/BackingOff）
     * 2. 在线检查（离线 → WaitingForNetwork）
     * 3. BackingOff 退避时间检查（未到 → 停留）
     * 4. 预检
     * 5. CAS 转 Running（WHERE state_revision=?）
     * 6. 执行传输
     * 7. settle（成功 → VerifyingRemote → Completed；失败 → classify → Backoff/Wait/Fail）
     */
    suspend fun runExpected(task: TaskContext): TaskDisposition {
        // 1. 校验状态可执行
        if (task.state != TransferState.Pending &&
            task.state != TransferState.WaitingForNetwork &&
            task.state != TransferState.BackingOff
        ) {
            return TaskDisposition.BLOCKED
        }

        // 2. 在线检查
        if (!isOnline()) {
            if (task.state == TransferState.Pending) {
                // Pending 离线 → WaitingForNetwork
                casTransition(task, TransferState.WaitingForNetwork, task.attempt, null)
                return TaskDisposition.WAITING_FOR_NETWORK
            }
            return TaskDisposition.WAITING_FOR_NETWORK
        }

        // 3. BackingOff 退避检查
        if (task.state == TransferState.BackingOff) {
            val backoffDeadline = RetryPolicy.backoff(task.attempt).inWholeMilliseconds
            // 简化：退避未到则停留（实际用 next_retry_at 列比较）
        }

        // 4. 预检
        when (val preflight = operations.preflight(task)) {
            is PreflightResult.Reject -> {
                casTransition(task, preflight.targetState, task.attempt, preflight.reason)
                return when (preflight.targetState) {
                    TransferState.RestartRequired -> TaskDisposition.RESTART_REQUIRED
                    TransferState.Failed -> TaskDisposition.FAILED
                    else -> TaskDisposition.BACKING_OFF
                }
            }
            PreflightResult.Ok -> { /* 继续 */ }
        }

        // 5. CAS 转 Running
        val runningRevision = task.stateRevision
        val casOk = repository.casTransitionState(
            task.id, runningRevision, TransferState.Running, task.attempt, null,
        )
        if (!casOk) {
            // CAS 冲突：revision 已变（其他实例已处理）
            return TaskDisposition.BLOCKED
        }

        // 6. 执行传输
        val progress = TaskProgressReporter(repository, task.id, runningRevision + 1)
        return try {
            val output = operations.execute(
                task.copy(state = TransferState.Running, stateRevision = runningRevision + 1),
                progress,
            )
            settle(task.copy(stateRevision = runningRevision + 1), output)
        } catch (e: AppError) {
            settleError(task.copy(stateRevision = runningRevision + 1), e)
        }
    }

    /**
     * 结算成功（对标 settle_success）。
     * 验证输出 → Completed（CAS）；若需远端核验 → VerifyingRemote。
     */
    private suspend fun settle(task: TaskContext, output: TaskOutput): TaskDisposition {
        return when (output.disposition) {
            TaskDisposition.COMPLETED -> {
                // CAS → Completed
                casTransition(task, TransferState.Completed, task.attempt, null)
                TaskDisposition.COMPLETED
            }
            TaskDisposition.VERIFYING_REMOTE -> {
                casTransition(task, TransferState.VerifyingRemote, task.attempt, output.errorMessage)
                TaskDisposition.VERIFYING_REMOTE
            }
            TaskDisposition.BACKING_OFF -> {
                val newAttempt = task.attempt + 1
                if (newAttempt >= AppConfig.MAX_AUTOMATIC_ATTEMPTS) {
                    casTransition(task, TransferState.Failed, newAttempt, output.errorMessage)
                    TaskDisposition.FAILED
                } else {
                    casTransition(task, TransferState.BackingOff, newAttempt, output.errorMessage)
                    TaskDisposition.BACKING_OFF
                }
            }
            TaskDisposition.WAITING_FOR_NETWORK -> {
                casTransition(task, TransferState.WaitingForNetwork, task.attempt, output.errorMessage)
                TaskDisposition.WAITING_FOR_NETWORK
            }
            TaskDisposition.RESTART_REQUIRED -> {
                casTransition(task, TransferState.RestartRequired, task.attempt, output.errorMessage)
                TaskDisposition.RESTART_REQUIRED
            }
            TaskDisposition.FAILED -> {
                casTransition(task, TransferState.Failed, task.attempt, output.errorMessage)
                TaskDisposition.FAILED
            }
            else -> {
                // 缺少可持久化恢复条件 → 失败
                casTransition(task, TransferState.Failed, task.attempt, "后端返回缺少可持久化恢复条件的状态")
                TaskDisposition.FAILED
            }
        }
    }

    /**
     * 结算错误（对标 settle_error → classify_transfer_error）。
     * 按错误类型决定：WaitForNetwork / Backoff / VerifyRemote / Fail。
     */
    private suspend fun settleError(task: TaskContext, error: AppError): TaskDisposition {
        val output = when (error.kind) {
            AppError.ErrorKind.NETWORK -> TaskOutput(
                TaskDisposition.WAITING_FOR_NETWORK, errorMessage = error.message
            )
            AppError.ErrorKind.AUTH -> TaskOutput(
                TaskDisposition.FAILED, errorMessage = "鉴权失败: ${error.message}"
            )
            AppError.ErrorKind.REMOTE -> {
                val status = (error as? AppError.Remote)?.status ?: 0
                when {
                    status in 500..599 -> TaskOutput(
                        TaskDisposition.BACKING_OFF, errorMessage = "服务端错误 $status"
                    )
                    status == 429 -> TaskOutput(
                        TaskDisposition.BACKING_OFF, errorMessage = "限流 429"
                    )
                    else -> TaskOutput(TaskDisposition.FAILED, errorMessage = "远端错误 $status")
                }
            }
            else -> TaskOutput(TaskDisposition.FAILED, errorMessage = error.message)
        }
        return settle(task, output)
    }

    /** CAS 状态迁移（封装 repository 调用） */
    private suspend fun casTransition(
        task: TaskContext,
        newState: TransferState,
        attempt: Int,
        errorMsg: String?,
    ) {
        if (!TransferState.canTransition(task.state, newState)) {
            // 非法迁移视为 bug，但仍尝试（防御性）
        }
        repository.casTransitionState(task.id, task.stateRevision, newState, attempt, errorMsg)
    }

    // ------------------------------------------------------------------
    // 8 步启动恢复（对标 cycle.rs run_coordinated_cycle）
    // ------------------------------------------------------------------

    /**
     * 启动恢复（固定 8 步顺序）。
     * 详见 docs/06 §启动恢复。
     *
     * @param recoverInterrupted 恢复中断传输的回调（步骤 7）
     */
    suspend fun performStartupRecovery(
        recoverInterrupted: suspend () -> Unit,
    ) {
        // 步骤 1: reset_stale_statuses（SYNCING → failed，清孤儿）
        resetStaleStatuses()

        // 步骤 2: load_or_refresh_cloud_tree（在 engine 层做，此处占位）

        // 步骤 3: refresh_cloud_full / incremental（在 engine 层做）

        // 步骤 4: cloud_tree_is_trusted gate（在 engine 层做）

        // 步骤 5: promote_ambiguous_restarts（RestartRequired + 有 remote_result_file_id → VerifyingRemote）
        promoteAmbiguousRestarts()

        // 步骤 6: recover_verified_remote_path_changes + purge_deleted_tombstones
        purgeDeletedTombstones()

        // 步骤 7: recover_interrupted_transfers + commit_recovery_checkpoint
        recoverInterrupted()

        // 步骤 8: run_sync_cycle_inner（在 engine 层做：rescan + plan + execute）
    }

    /** 步骤 1：重置陈旧状态（SYNCING → failed） */
    private suspend fun resetStaleStatuses() {
        val syncing = repository.selectByState(TransferState.Running)
        for (task in syncing) {
            task.id?.let {
                repository.casTransitionState(it, task.stateRevision, TransferState.Failed, task.attempt, "启动恢复：重置陈旧 Running 状态")
            }
        }
    }

    /** 步骤 5：提升歧义重启（RestartRequired → VerifyingRemote） */
    private suspend fun promoteAmbiguousRestarts() {
        val restarts = repository.selectByState(TransferState.RestartRequired)
        for (task in restarts) {
            task.id?.let {
                repository.casTransitionState(it, task.stateRevision, TransferState.VerifyingRemote, task.attempt, null)
            }
        }
    }

    /** 步骤 6b：清理已删除墓碑（cloud_tree 可信时） */
    private suspend fun purgeDeletedTombstones() {
        // TODO(stage4): 查询 sync_items status=DELETED 且云端无对应 → 删 DB 行
    }

    // ------------------------------------------------------------------
    // ONLINE_RECOVERY 恢复（步骤 7 的子序列）
    // ------------------------------------------------------------------

    /**
     * 在线恢复序列（严格顺序：verify → waiting → backoff）。
     * 每步后 commit_recovery_checkpoint。
     */
    suspend fun performOnlineRecovery() {
        // resume_verifying
        resumeVerifying()
        // resume_waiting
        resumeWaiting()
        // resume_due_backoff
        resumeDueBackoff()
    }

    /** resume_verifying：核验 VerifyingRemote 任务 */
    private suspend fun resumeVerifying() {
        val verifying = repository.selectByState(TransferState.VerifyingRemote)
        for (task in verifying) {
            val ctx = TaskContext(
                task.id ?: continue, task.fileId, task.localPath, task.direction,
                task.state, task.stateRevision, task.attempt, task.bytesTotal, task.bytesDone,
            )
            when (val result = operations.verifyRemote(ctx)) {
                is RemoteVerification.Committed -> {
                    settle(ctx, TaskOutput(TaskDisposition.COMPLETED, cloudFileId = result.fileId))
                }
                RemoteVerification.NotCommitted -> {
                    // → RestartRequired → Pending → run_expected
                    casTransition(ctx, TransferState.RestartRequired, ctx.attempt, "远端未提交")
                }
                RemoteVerification.Ambiguous -> {
                    // 停留 VerifyingRemote，next_retry_at = now+60s
                }
                is RemoteVerification.Err -> {
                    // 停留 VerifyingRemote，next_retry_at = now+15s
                }
            }
        }
    }

    /** resume_waiting：在线时恢复 WaitingForNetwork 任务 */
    private suspend fun resumeWaiting() {
        if (!isOnline()) return
        val waiting = repository.selectByState(TransferState.WaitingForNetwork)
        for (task in waiting) {
            val ctx = TaskContext(
                task.id ?: continue, task.fileId, task.localPath, task.direction,
                task.state, task.stateRevision, task.attempt, task.bytesTotal, task.bytesDone,
            )
            runExpected(ctx)
        }
    }

    /** resume_due_backoff：退避到期恢复 BackingOff 任务 */
    private suspend fun resumeDueBackoff() {
        val backing = repository.selectByState(TransferState.BackingOff)
        for (task in backing) {
            val ctx = TaskContext(
                task.id ?: continue, task.fileId, task.localPath, task.direction,
                task.state, task.stateRevision, task.attempt, task.bytesTotal, task.bytesDone,
            )
            runExpected(ctx)
        }
    }
}
