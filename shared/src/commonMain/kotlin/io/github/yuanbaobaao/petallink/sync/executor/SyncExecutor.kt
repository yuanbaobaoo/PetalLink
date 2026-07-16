package io.github.yuanbaobaao.petallink.sync.executor

import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

/**
 * 同步动作执行结果（对标 ActionResult）。
 */
data class ActionResult(
    val success: Boolean,
    val deferred: Boolean = false,   // 延迟（被 shutdown/活动门拒绝）
    val cloudFileId: String? = null,
    val errorMessage: String? = null,
)

/**
 * 同步执行器（对标 src/sync/executor/）。
 *
 * 详见 docs/06 §executor、docs/10 阶段 4 item 22。
 * - 并发 6（Semaphore）
 * - 两阶段目录优先（CreateFolder 顺序 → fill_parent_file_ids → 并发剩余）
 * - prune_transfer_history（保留 100 条）
 *
 * @param concurrency 并发数（默认 6，范围 1-20）
 * @param executeOne 单个动作执行回调（由引擎注入实际传输逻辑）
 */
class SyncExecutor(
    private val concurrency: Int = AppConfig.MAX_CONCURRENT_TRANSFERS,
    private val executeOne: suspend (SyncAction) -> ActionResult,
) {
    private val semaphore = Semaphore(concurrency.coerceAtLeast(1))

    /**
     * 执行全部动作（并发，对标 execute_all）。
     * 保持原始顺序返回结果。
     */
    suspend fun executeAll(actions: List<SyncAction>): List<ActionResult> = coroutineScope {
        val indexed = actions.mapIndexed { idx, action ->
            async {
                semaphore.withPermit {
                    try {
                        executeOne(action)
                    } catch (e: Throwable) {
                        ActionResult(success = false, errorMessage = e.message)
                    }
                }
            } to idx
        }
        // 按原始顺序返回
        indexed.map { it.first.await() }
    }

    /**
     * 两阶段有序执行（对标 execute_actions_ordered）。
     *
     * 阶段 1：CreateFolder（cloud_file==null）顺序执行，深度升序（父目录先）。
     * 阶段 2：剩余动作并发执行。
     *
     * @param actions 待执行动作
     * @param onFolderCreated 目录创建成功回调（回填 path_to_id）
     */
    suspend fun executeActionsOrdered(
        actions: List<SyncAction>,
        onFolderCreated: (suspend (SyncAction, ActionResult) -> Unit)? = null,
    ): List<ActionResult> {
        val n = actions.size
        val results = arrayOfNulls<ActionResult>(n)

        // 阶段 1：本地新建目录（顺序，父先于子）
        val folderIdx = actions.mapIndexedNotNull { idx, a ->
            if (a.type == SyncActionType.CREATE_FOLDER && a.cloudFile == null) idx else null
        }.sortedBy { idx -> actions[idx].relativePath.count { it == '/' } }

        for (i in folderIdx) {
            val res = executeAll(listOf(actions[i])).first()
            results[i] = res
            if (res.success && onFolderCreated != null) {
                onFolderCreated(actions[i], res)
            }
        }

        // 阶段 2：剩余动作并发
        val otherIdx = (0 until n).filter { results[it] == null }
        val otherActions = otherIdx.map { actions[it] }
        if (otherActions.isNotEmpty()) {
            val otherResults = executeAll(otherActions)
            for ((k, idx) in otherIdx.withIndex()) {
                results[idx] = otherResults[k]
            }
        }

        return results.map { it ?: ActionResult(success = false, errorMessage = "未执行") }
    }
}
