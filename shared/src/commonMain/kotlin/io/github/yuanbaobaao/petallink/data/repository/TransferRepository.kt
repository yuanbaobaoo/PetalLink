package io.github.yuanbaobaao.petallink.data.repository

import io.github.yuanbaobaao.petallink.data.TransferTask
import io.github.yuanbaobaao.petallink.sync.TransferState

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
     * 进度更新：刻意不递增 state_revision（进度是高频非状态变更）。
     */
    suspend fun updateRunningProgress(id: Long, bytesDone: Long)

    /** 按状态查询任务（阶段 4 调度器用） */
    suspend fun selectByState(state: TransferState): List<TransferTask>

    /**
     * 历史清理：保留最近 [keepCount] 条（对标 prune_transfer_history，默认 100）。
     */
    suspend fun pruneHistory(keepCount: Int)

    /** 按状态+方向计数 */
    suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long

    /** 按状态计数 */
    suspend fun countByState(state: TransferState): Long
}
