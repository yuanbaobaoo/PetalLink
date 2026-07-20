package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.github.yuanbaobaoo.petallink.data.ColumnPatch
import io.github.yuanbaobaoo.petallink.data.TransferPatch
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.data.RunningTransferPatch
import io.github.yuanbaobaoo.petallink.drive.ResumeSession
import io.github.yuanbaobaoo.petallink.drive.RetryAfter
import io.github.yuanbaobaoo.petallink.data.repository.TransferRepository
import io.github.yuanbaobaoo.petallink.data.repository.StaleRevisionException
import io.github.yuanbaobaoo.petallink.sync.RetryPolicy
import io.github.yuanbaobaoo.petallink.sync.TransferState
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlin.math.min
import kotlin.random.Random

/**
 * 任务执行后端操作接口（对标 TransferOperations trait）。
 *
 * 由实际的传输实现提供（上传/下载/远端核验）。
 * 详见 docs/06 §TaskRunner contracts。
 */
interface TransferOperations {
    /**
     * 预检（静态检查：路径合法/空间/冲突）
     */
    suspend fun preflight(task: TaskContext): PreflightResult

    /**
     * 执行传输
     */
    suspend fun execute(task: TaskContext, progress: TaskProgressReporter): TaskOutput

    /**
     * 远端核验（VerifyingRemote 状态用）
     */
    suspend fun verifyRemote(task: TaskContext): RemoteVerification

    /**
     * 启动恢复时读取下载断点的磁盘真值（§3.11，对标 recovery.rs:400-424：
     * 以 .tmp 临时文件实际大小为准，而非 DB 中的 resumeOffset）。
     *
     * 返回 null 表示实现无法判定，TaskRunner 回退使用 DB 断点。
     */
    suspend fun durableDownloadOffset(task: TaskContext): Long? = null
}

/**
 * 预检结果
 */
sealed class PreflightResult {
    /**
     * 预检通过：允许进入执行
     */
    object Ok : PreflightResult()

    /**
     * 预检拒绝：携带原因与目标态（如 Failed / RestartRequired）
     */
    data class Reject(val reason: String, val targetState: TransferState) : PreflightResult()
}

/**
 * 任务上下文（对标 RunningTask 快照）
 */
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
    val nextRetryAt: Long? = null,
    val remoteResultFileId: String? = null,
    val sessionUrl: String? = null,
    val serverId: String? = null,
    val uploadId: String? = null,
    val parentFileId: String? = null,
    val operation: Int? = null,
    val sourceMtime: Long? = null,
    val sourceSize: Long? = null,
    val expectedCloudEditedTime: Long? = null,
    val createdAt: Long = 0L,
)

/**
 * 将 [TransferTask] 转为执行上下文 [TaskContext]（字段一一映射）。
 *
 * id 缺失视为数据错误，直接抛异常；其余可空字段原样透传。
 */
fun TransferTask.toTaskContext(): TaskContext = TaskContext(
    id = id ?: error("传输任务缺少 id"),
    fileId = fileId.orEmpty(),
    localPath = localPath.orEmpty(),
    direction = direction,
    state = state,
    stateRevision = stateRevision,
    attempt = attempt,
    bytesTotal = bytesTotal,
    bytesDone = bytesDone,
    nextRetryAt = nextRetryAt,
    remoteResultFileId = remoteResultFileId,
    sessionUrl = sessionUrl,
    serverId = serverId,
    uploadId = uploadId,
    parentFileId = parentFileId,
    operation = operation,
    sourceMtime = sourceMtime,
    sourceSize = sourceSize,
    expectedCloudEditedTime = expectedCloudEditedTime,
    createdAt = createdAt,
)

/**
 * 任务执行输出
 */
data class TaskOutput(
    val disposition: TaskDisposition,
    val cloudFileId: String? = null,
    val bytesTransferred: Long? = null,
    val errorMessage: String? = null,
    /**
     * 429 限流时服务端指定的重试时间（§3.11，原 retry_policy.rs:157-166）；
     * 非空时优先于本地指数退避。
     */
    val retryAfter: RetryAfter? = null,
)

/**
 * 任务结局（9 种，对标 TaskDisposition）
 */
enum class TaskDisposition {
    /**
     * 完成
     */
    COMPLETED,

    /**
     * 退避重试
     */
    BACKING_OFF,

    /**
     * 等待网络
     */
    WAITING_FOR_NETWORK,

    /**
     * 核验远端
     */
    VERIFYING_REMOTE,

    /**
     * 需重启（可恢复中断）
     */
    RESTART_REQUIRED,

    /**
     * 被同路径任务阻塞
     */
    BLOCKED,

    /**
     * 失败（终态）
     */
    FAILED,

    /**
     * 取消
     */
    CANCELED,

    /**
     * 重新入队
     */
    PENDING,
}

/**
 * 远端核验结果
 */
sealed class RemoteVerification {
    /**
     * 远端已确认提交：携带最终 fileId
     */
    data class Committed(val fileId: String) : RemoteVerification()

    /**
     * 远端未提交：任务需重新规划
     */
    object NotCommitted : RemoteVerification()

    /**
     * 远端结果不明：需稍后重试核验
     */
    object Ambiguous : RemoteVerification()

    /**
     * 校验过程出错：携带错误信息
     */
    data class Err(val message: String) : RemoteVerification()
}

/**
 * 进度上报器（对标 TaskProgressReporter）
 */
class TaskProgressReporter(
    private val repository: TransferRepository,
    private val taskId: Long,
    private val stateRevision: Long,
    private val throttleMs: Long = PROGRESS_THROTTLE_MS,
    private val onProgress: suspend (Long) -> Unit = {},
    private val direction: TransferDirection = TransferDirection.UPLOAD,
) {
    private var lastReportMs: Long = 0L
    private val logger = Logger()

    /**
     * 上报进度（刻意不递增 revision）
     */
    suspend fun report(bytesDone: Long, nowMs: Long) {
        if (nowMs - lastReportMs < throttleMs) return  // 节流
        lastReportMs = nowMs
        if (repository.updateRunningProgress(taskId, stateRevision, bytesDone)) {
            onProgress(taskId)
        } else {
            // revision 已过期（任务被其他执行者推进/终结）→ 丢弃过期回调
            logger.debug("sync.executor.transfer_operations") {
                if (direction == TransferDirection.UPLOAD) "忽略过期上传进度回调" else "忽略过期下载进度回调"
            }
        }
    }

    /**
     * 会话轮换与服务端确认 offset 必须立即持久化，不受 UI 进度节流影响。
     */
    suspend fun reportResume(session: ResumeSession, confirmedOffset: Long): Boolean {
        val updated = repository.updateRunningTransfer(
            taskId,
            stateRevision,
            RunningTransferPatch(
                transferred = confirmedOffset,
                resumeOffset = confirmedOffset,
                serverId = ColumnPatch.Set(session.serverId),
                uploadId = ColumnPatch.Set(session.uploadId),
                sessionUrl = ColumnPatch.Set(session.sessionUrl),
            ),
        )
        if (updated) onProgress(taskId)
        return updated
    }

    companion object {
        /**
         * 进度节流间隔：500ms
         */
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
    private val jitterMs: () -> Long = { Random.nextLong(0L, 1_000L) },
    maxConcurrentTransfers: Int = AppConfig.MAX_CONCURRENT_TRANSFERS,
    private val onNetworkFailure: () -> Unit = {},
    private val onTaskChanged: suspend (Long) -> Unit = {},
) {
    // CAS 并发控制：同一路径同时只允许一个任务 Running
    private val runningSemaphore = Semaphore(maxConcurrentTransfers.coerceIn(1, 20))
    private val logger = Logger()

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
                transitionFailure(task, TransferState.WaitingForNetwork, task.attempt, null, null)
                return TaskDisposition.WAITING_FOR_NETWORK
            }
            return TaskDisposition.WAITING_FOR_NETWORK
        }

        // 3. BackingOff 退避检查
        if (task.state == TransferState.BackingOff && (task.nextRetryAt ?: Long.MAX_VALUE) > nowMs()) {
            return TaskDisposition.BACKING_OFF
        }

        return runningSemaphore.withPermit { runAdmitted(task) }
    }

    /**
     * 取得并发许可后的执行主链：预检 → CAS 转 Running → 执行传输 → settle。
     */
    private suspend fun runAdmitted(task: TaskContext): TaskDisposition {
        // 4. 预检
        val preflight = try {
            operations.preflight(task)
        } catch (error: AppError) {
            return settleError(task, error)
        }
        when (preflight) {
            is PreflightResult.Reject -> {
                transitionFailure(task, preflight.targetState, task.attempt, preflight.reason, null)
                return when (preflight.targetState) {
                    TransferState.RestartRequired -> TaskDisposition.RESTART_REQUIRED
                    TransferState.Failed -> TaskDisposition.FAILED
                    else -> TaskDisposition.BACKING_OFF
                }
            }
            PreflightResult.Ok -> { /* 继续 */ }
        }

        // 5. CAS 转 Running
        val running = try {
            transition(
                task, TransferState.Running,
                TransferPatch(
                    errorKind = ColumnPatch.Clear,
                    errorMessage = ColumnPatch.Clear,
                    nextRetryAt = ColumnPatch.Clear,
                    finishedAt = ColumnPatch.Clear,
                ),
            )
        } catch (_: StaleRevisionException) {
            return TaskDisposition.BLOCKED
        }

        // 6. 执行传输
        val progress = TaskProgressReporter(
            repository,
            running.id,
            running.stateRevision,
            onProgress = onTaskChanged,
            direction = running.direction,
        )
        return try {
            val output = operations.execute(running, progress)
            settle(running, output)
        } catch (e: AppError) {
            settleError(running, e)
        }
    }

    /**
     * 结算成功（对标 settle_success）。
     * 验证输出 → Completed（CAS）；若需远端核验 → VerifyingRemote。
     */
    private suspend fun settle(task: TaskContext, output: TaskOutput): TaskDisposition {
        return when (output.disposition) {
            TaskDisposition.COMPLETED -> {
                transition(
                    task, TransferState.Completed,
                    TransferPatch(
                        errorKind = ColumnPatch.Clear,
                        errorMessage = ColumnPatch.Clear,
                        nextRetryAt = ColumnPatch.Clear,
                        finishedAt = ColumnPatch.Set(nowMs()),
                        remoteResultFileId = output.cloudFileId?.let(ColumnPatch<String>::Set)
                            ?: ColumnPatch.Keep,
                        transferred = output.bytesTransferred,
                        resumeOffset = output.bytesTransferred,
                    ),
                )
                TaskDisposition.COMPLETED
            }
            TaskDisposition.VERIFYING_REMOTE -> {
                transition(
                    task, TransferState.VerifyingRemote,
                    TransferPatch(
                        errorMessage = output.errorMessage.patchOrClear(),
                        nextRetryAt = ColumnPatch.Set(nowMs() + 3_000L),
                        remoteResultFileId = output.cloudFileId?.let(ColumnPatch<String>::Set)
                            ?: ColumnPatch.Keep,
                        transferred = output.bytesTransferred,
                        resumeOffset = output.bytesTransferred,
                    ),
                )
                TaskDisposition.VERIFYING_REMOTE
            }
            TaskDisposition.BACKING_OFF -> {
                val newAttempt = task.attempt + 1
                if (newAttempt >= AppConfig.MAX_AUTOMATIC_ATTEMPTS) {
                    transitionFailure(task, TransferState.Failed, newAttempt, output.errorMessage, null)
                    TaskDisposition.FAILED
                } else {
                    val baseAttempt = (newAttempt - 1).coerceAtLeast(0)
                    // §3.11（原 retry_policy.rs:157-166）：429 优先服务端 Retry-After，缺省回退指数退避
                    val deadline = output.retryAfter?.nextRetryAt(nowMs())
                        ?: (nowMs() + RetryPolicy.backoff(baseAttempt, jitterMs()).inWholeMilliseconds)
                    transitionFailure(task, TransferState.BackingOff, newAttempt, output.errorMessage, deadline)
                    TaskDisposition.BACKING_OFF
                }
            }
            TaskDisposition.WAITING_FOR_NETWORK -> {
                transitionFailure(task, TransferState.WaitingForNetwork, task.attempt, output.errorMessage, null)
                TaskDisposition.WAITING_FOR_NETWORK
            }
            TaskDisposition.RESTART_REQUIRED -> {
                transitionFailure(task, TransferState.RestartRequired, task.attempt, output.errorMessage, null)
                TaskDisposition.RESTART_REQUIRED
            }
            TaskDisposition.FAILED -> {
                transitionFailure(task, TransferState.Failed, task.attempt, output.errorMessage, null)
                TaskDisposition.FAILED
            }
            else -> {
                // 缺少可持久化恢复条件 → 失败
                transitionFailure(task, TransferState.Failed, task.attempt, "后端返回缺少可持久化恢复条件的状态", null)
                TaskDisposition.FAILED
            }
        }
    }

    /**
     * 结算错误（对标 settle_error → classify_transfer_error）。
     * 按错误类型决定：WaitForNetwork / Backoff / VerifyRemote / Fail。
     *
     * §3.3（原 retry_policy.rs:62-68,97-140,167-177）：
     * - 写操作（上传/删除）网络错误按「请求是否可能已送达」分流：可能送达 → VerifyingRemote；
     * - 500/502/503/504 退避预算耗尽且写入可能已送达 → VerifyingRemote；
     * - upload_session_expired（RemoteAmbiguous）→ VerifyingRemote。
     */
    private suspend fun settleError(task: TaskContext, error: AppError): TaskDisposition {
        val modifiesRemote = task.direction == TransferDirection.UPLOAD ||
            task.direction == TransferDirection.DELETE
        val budgetExhausted = task.attempt + 1 >= AppConfig.MAX_AUTOMATIC_ATTEMPTS
        val output = when (RetryPolicy.classifyTransferError(error, modifiesRemote, budgetExhausted)) {
            RetryPolicy.RecoveryDecision.VERIFY_REMOTE -> {
                logger.warn("sync.task_runner.recovery") {
                    "写入结果不确定，转远端核验而非盲目重放 task_id=${task.id} error=${error.message}"
                }
                TaskOutput(TaskDisposition.VERIFYING_REMOTE, errorMessage = error.message)
            }
            RetryPolicy.RecoveryDecision.WAIT_FOR_NETWORK -> TaskOutput(
                TaskDisposition.WAITING_FOR_NETWORK, errorMessage = error.message
            ).also { onNetworkFailure() }
            RetryPolicy.RecoveryDecision.BACKOFF -> {
                val status = (error as? AppError.Remote)?.status ?: 0
                TaskOutput(
                    TaskDisposition.BACKING_OFF,
                    errorMessage = if (status == 429) "限流 429" else "服务端错误 $status",
                )
            }
            RetryPolicy.RecoveryDecision.FAIL -> when (error.kind) {
                AppError.ErrorKind.AUTH -> TaskOutput(
                    TaskDisposition.FAILED, errorMessage = "鉴权失败: ${error.message}"
                )
                AppError.ErrorKind.REMOTE -> TaskOutput(
                    TaskDisposition.FAILED,
                    errorMessage = "远端错误 ${(error as? AppError.Remote)?.status ?: 0}",
                )
                else -> TaskOutput(TaskDisposition.FAILED, errorMessage = error.message)
            }
        }
        return settle(task, output)
    }

    /**
     * 构造失败态迁移 patch 并委托 [transition]：清/写错误信息、设退避时间，终态补 finishedAt。
     */
    private suspend fun transitionFailure(
        task: TaskContext,
        newState: TransferState,
        attempt: Int,
        errorMsg: String?,
        nextRetryAt: Long?,
    ): TaskContext = transition(
        task, newState,
        TransferPatch(
            errorMessage = errorMsg.patchOrClear(),
            nextRetryAt = nextRetryAt?.let(ColumnPatch<Long>::Set) ?: ColumnPatch.Clear,
            finishedAt = if (TransferState.isTerminal(newState)) ColumnPatch.Set(nowMs()) else ColumnPatch.Clear,
            attemptCount = attempt,
        ),
    )

    /**
     * 校验状态迁移合法后，通过仓库 CAS 乐观锁落库并返回更新后的任务上下文。
     */
    private suspend fun transition(task: TaskContext, newState: TransferState, patch: TransferPatch): TaskContext {
        check(TransferState.canTransition(task.state, newState)) {
            "非法状态迁移：${task.state} → $newState"
        }
        val updated = repository.transition(task.id, task.stateRevision, newState, patch).toContext()
        onTaskChanged(task.id)
        return updated
    }

    /**
     * 把可空错误信息映射为 Set（非空）或 Clear（空）。
     */
    private fun String?.patchOrClear(): ColumnPatch<String> =
        this?.let(ColumnPatch<String>::Set) ?: ColumnPatch.Clear

    /**
     * 把持久层 TransferTask 转换为执行上下文 TaskContext。
     */
    private fun TransferTask.toContext(): TaskContext = toTaskContext()

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
        // 步骤 1: Running 写操作先进入远端核验；下载只从持久断点重建请求。
        recoverInterruptedRunning()

        // 步骤 2: load_or_refresh_cloud_tree（由调用方在进入本方法前完成）

        // 步骤 3: refresh_cloud_full / incremental（在 engine 层做）

        // 步骤 4: cloud_tree_is_trusted gate（在 engine 层做）

        // 步骤 5: promote_ambiguous_restarts（RestartRequired + 有 remote_result_file_id → VerifyingRemote）
        promoteAmbiguousRestarts()

        // 步骤 6: recover_verified_remote_path_changes + purge_deleted_tombstones
        // 依赖可信 cloud tree，由运行时在进入本方法前完成。

        // 步骤 7: recover_interrupted_transfers + commit_recovery_checkpoint
        recoverInterrupted()
        val completed = repository.selectByState(TransferState.Completed).size
        val waitingNetwork = repository.selectByState(TransferState.WaitingForNetwork).size
        val verifyingRemote = repository.selectByState(TransferState.VerifyingRemote).size
        val failed = repository.selectByState(TransferState.Failed).size
        logger.info("sync.engine.lifecycle") {
            "中断传输已通过统一 TaskRunner 恢复 completed=$completed" +
                " waiting_network=$waitingNetwork verifying_remote=$verifyingRemote failed=$failed"
        }

        // 步骤 8: run_sync_cycle_inner（在 engine 层做：rescan + plan + execute）
    }

    /**
     * 步骤 1：恢复进程中断时仍处于 Running 的任务——写操作转 VerifyingRemote，下载保留断点转 Pending。
     */
    private suspend fun recoverInterruptedRunning() {
        // 启动期同路径重复任务收敛（对标 recovery.rs:296-336）：每组只恢复最新一条，其余抑制
        val running = repository.selectByState(TransferState.Running)
        val selected = running.filter { it.relativePath == null }.toMutableList()
        running.filter { it.relativePath != null }
            .groupBy { it.relativePath!! }
            .forEach { (_, samePath) ->
                val newestFirst = samePath.sortedWith(
                    compareByDescending<TransferTask> { it.createdAt }.thenByDescending { it.id ?: 0L },
                )
                selected += newestFirst.first()
                for (duplicate in newestFirst.drop(1)) suppressStartupDuplicate(duplicate)
            }
        for (task in selected) {
            val ctx = task.toContext()
            if (task.direction == TransferDirection.UPLOAD || task.direction == TransferDirection.DELETE) {
                transition(
                    ctx, TransferState.VerifyingRemote,
                    TransferPatch(
                        errorMessage = ColumnPatch.Set("进程中断时远端写入结果不确定，等待核验"),
                        nextRetryAt = ColumnPatch.Set(nowMs()),
                    ),
                )
            } else if (task.direction == TransferDirection.DOWNLOAD || task.direction == TransferDirection.DOWNLOAD_UPDATE) {
                val restart = transitionFailure(
                    ctx, TransferState.RestartRequired, ctx.attempt,
                    "进程中断，保留已验证下载断点并重新建立 Range 请求", null,
                )
                // §3.11（原 recovery.rs:400-424）：下载断点以磁盘 .tmp 实际大小为准；
                // 实现无法判定时回退 DB 断点。
                val durable = operations.durableDownloadOffset(ctx)
                val resumedTransferred = durable ?: min(task.transferred, task.totalSize)
                val resumedOffset = durable ?: min(task.resumeOffset, task.totalSize)
                transition(
                    restart, TransferState.Pending,
                    TransferPatch(
                        errorKind = ColumnPatch.Clear,
                        errorMessage = ColumnPatch.Clear,
                        nextRetryAt = ColumnPatch.Clear,
                        finishedAt = ColumnPatch.Clear,
                        transferred = resumedTransferred,
                        resumeOffset = resumedOffset,
                    ),
                )
            } else {
                transitionFailure(ctx, TransferState.Failed, ctx.attempt, "中断任务不支持自动恢复", null)
            }
        }
    }

    /**
     * 抑制启动期同路径重复任务：写操作转远端核验，下载转重新规划（对标 suppress_startup_duplicate）。
     */
    private suspend fun suppressStartupDuplicate(task: TransferTask) {
        val ctx = task.toContext()
        if (task.direction == TransferDirection.UPLOAD || task.direction == TransferDirection.DELETE) {
            transition(
                ctx, TransferState.VerifyingRemote,
                TransferPatch(
                    errorMessage = ColumnPatch.Set("启动恢复：同路径重复任务已收敛，等待核验"),
                    nextRetryAt = ColumnPatch.Set(nowMs()),
                ),
            )
        } else {
            transitionFailure(ctx, TransferState.RestartRequired, ctx.attempt, "启动恢复：同路径重复任务已收敛", null)
        }
    }

    /**
     * 步骤 5：提升歧义重启（RestartRequired → VerifyingRemote）
     */
    private suspend fun promoteAmbiguousRestarts() {
        val restarts = repository.selectByState(TransferState.RestartRequired)
        val promotable = restarts.filter { !it.remoteResultFileId.isNullOrBlank() }
        for (task in promotable) {
            transition(
                task.toContext(), TransferState.VerifyingRemote,
                TransferPatch(nextRetryAt = ColumnPatch.Set(nowMs()), finishedAt = ColumnPatch.Clear),
            )
        }
        if (promotable.isNotEmpty()) {
            logger.info("sync.engine.cycle") { "已将含远端结果的重规划任务恢复为核验态 promoted=${promotable.size}" }
        }
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

    /**
     * resume_verifying：核验 VerifyingRemote 任务
     * §3.11（原 recovery.rs:33-35）：离线时整体跳过核验。
     */
    private suspend fun resumeVerifying() {
        if (!isOnline()) {
            logger.debug("sync.task_runner.recovery") { "离线，跳过远端核验任务恢复" }
            return
        }
        val verifying = repository.selectByState(TransferState.VerifyingRemote)
        for (task in verifying) {
            if ((task.nextRetryAt ?: Long.MIN_VALUE) > nowMs()) continue
            val ctx = task.toContext()
            when (val result = operations.verifyRemote(ctx)) {
                is RemoteVerification.Committed -> {
                    settle(ctx, TaskOutput(TaskDisposition.COMPLETED, cloudFileId = result.fileId))
                }
                RemoteVerification.NotCommitted -> {
                    transitionFailure(ctx, TransferState.RestartRequired, ctx.attempt, "远端未提交；等待显式重规划", null)
                }
                RemoteVerification.Ambiguous -> {
                    transition(
                        ctx, TransferState.VerifyingRemote,
                        TransferPatch(nextRetryAt = ColumnPatch.Set(nowMs() + 60_000L)),
                    )
                }
                is RemoteVerification.Err -> {
                    logger.warn("sync.task_runner.recovery") { "远端写入核验暂不可用，保留歧义状态 task_id=${ctx.id} error=${result.message}" }
                    transition(
                        ctx, TransferState.VerifyingRemote,
                        TransferPatch(
                            errorMessage = ColumnPatch.Set(result.message),
                            nextRetryAt = ColumnPatch.Set(nowMs() + 15_000L),
                        ),
                    )
                }
            }
        }
    }

    /**
     * resume_waiting：在线时恢复 WaitingForNetwork 任务
     */
    private suspend fun resumeWaiting() {
        if (!isOnline()) return
        val waiting = repository.selectByState(TransferState.WaitingForNetwork)
        for (task in waiting) {
            runExpected(task.toContext())
        }
    }

    /**
     * resume_due_backoff：退避到期恢复 BackingOff 任务
     */
    private suspend fun resumeDueBackoff() {
        val backing = repository.selectByState(TransferState.BackingOff)
        for (task in backing.filter { (it.nextRetryAt ?: Long.MAX_VALUE) <= nowMs() }) {
            runExpected(task.toContext())
        }
    }

    /**
     * Failed/RestartRequired 只有显式重试才会重新规划；歧义远端结果仍优先核验。
     *
     * §3.11（原 admission.rs:218-236 prepare_retry）：转 Pending 前先做预检，
     * 预检拒绝则持久化到目标态（Failed/RestartRequired），不再盲目重放。
     */
    suspend fun retryExplicit(taskId: Long): TaskDisposition {
        val task = repository.findById(taskId) ?: return TaskDisposition.FAILED
        if (task.state != TransferState.Failed && task.state != TransferState.RestartRequired) {
            return TaskDisposition.BLOCKED
        }
        // 只有 Create/Update/Download/DownloadUpdate 能安全重放现有任务。
        if (task.operation !in 0..3) return TaskDisposition.BLOCKED
        if (task.state == TransferState.RestartRequired && !task.remoteResultFileId.isNullOrBlank()) {
            transition(
                task.toContext(), TransferState.VerifyingRemote,
                TransferPatch(nextRetryAt = ColumnPatch.Set(nowMs()), finishedAt = ColumnPatch.Clear),
            )
            return TaskDisposition.VERIFYING_REMOTE
        }
        val ctx = task.toContext()
        // 预检（对标 prepare_retry 的 validate_static + 后端预检）
        val preflight = try {
            operations.preflight(ctx)
        } catch (error: AppError) {
            logger.warn("sync.task_runner.recovery") { "显式重试预检异常，保持原状态 task_id=$taskId error=${error.message}" }
            return TaskDisposition.FAILED
        }
        if (preflight is PreflightResult.Reject) {
            try {
                transitionFailure(ctx, preflight.targetState, ctx.attempt, preflight.reason, null)
            } catch (_: StaleRevisionException) {
                return TaskDisposition.BLOCKED
            }
            return when (preflight.targetState) {
                TransferState.RestartRequired -> TaskDisposition.RESTART_REQUIRED
                TransferState.Failed -> TaskDisposition.FAILED
                else -> TaskDisposition.BLOCKED
            }
        }
        val pending = try {
            transition(
                ctx, TransferState.Pending,
                TransferPatch(
                    errorKind = ColumnPatch.Clear,
                    errorMessage = ColumnPatch.Clear,
                    nextRetryAt = ColumnPatch.Clear,
                    finishedAt = ColumnPatch.Clear,
                    attemptCount = 0,
                ),
            )
        } catch (_: StaleRevisionException) {
            // 对标 accept_retry_after_preflight：预检后状态已变化则拒绝
            return TaskDisposition.BLOCKED
        }
        return runExpected(pending)
    }
}
