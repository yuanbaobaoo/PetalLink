package io.github.yuanbaobaoo.petallink.data.repository

import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.data.TransferPatch
import io.github.yuanbaobaoo.petallink.data.RunningTransferPatch
import io.github.yuanbaobaoo.petallink.sync.TransferState

class StaleRevisionException(taskId: Long, expected: Long) :
    IllegalStateException("传输任务 $taskId 的 revision 已变化（expected=$expected）")

class IllegalTransferTransitionException(taskId: Long, from: TransferState, to: TransferState) :
    IllegalStateException("传输任务 $taskId 非法状态迁移：$from → $to")

/**
 * transfer_queue 仓库接口（对标 docs/04 §6 + docs/06 九态状态机）
 *
 * CAS 状态迁移 + 进度更新（刻意不递增 revision）+ 历史清理。
 */
interface TransferRepository {
    /** 新增传输任务，返回自增 id */
    suspend fun insert(task: TransferTask): Long

    /** 按 id 查询 */
    suspend fun findById(id: Long): TransferTask?

    /**
     * CAS 状态迁移。
     * @param expectedRevision 预期的 state_revision
     * @return true 成功；false CAS 冲突
     */
    suspend fun casTransitionState(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        attempt: Int,
        errorMsg: String?,
    ): Boolean

    /**
     * 校验 revision 与迁移矩阵后，在同一条 UPDATE 中应用完整生命周期补丁。
     * nullable 列严格区分 Keep / Set / Clear。
     */
    suspend fun transition(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        patch: TransferPatch = TransferPatch(),
    ): TransferTask

    /**
     * 进度更新：刻意不递增 state_revision（进度是高频非状态变更）。
     */
    /** @return false 表示任务已经离开 Running，迟到回调被丢弃。 */
    suspend fun updateRunningProgress(id: Long, expectedRevision: Long, bytesDone: Long): Boolean

    /** 持久化服务端确认的 resume offset 与会话身份，不递增 lifecycle revision。 */
    suspend fun updateRunningTransfer(
        id: Long,
        expectedRevision: Long,
        patch: RunningTransferPatch,
    ): Boolean

    /** 按状态查询任务（阶段 4 调度器用） */
    suspend fun selectByState(state: TransferState): List<TransferTask>

    /** 全部任务，按创建时间和 id 倒序。 */
    suspend fun selectAll(): List<TransferTask>

    /**
     * 历史清理：保留最近 [keepCount] 条（对标 prune_transfer_history，默认 100）。
     */
    suspend fun pruneHistory(keepCount: Int)

    /** 精确清理 Completed/Failed；Canceled 作为审计记录保留。 */
    suspend fun clearHistory(includeCompleted: Boolean, includeFailed: Boolean)

    /** 按状态+方向计数 */
    suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long

    /** 按状态计数 */
    suspend fun countByState(state: TransferState): Long
}
