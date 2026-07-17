package io.github.yuanbaobaao.petallink.sync.executor

import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import io.github.yuanbaobaao.petallink.drive.DriveFile
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
    val cloudFile: DriveFile? = null,
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
        initialPathToId: Map<String, String> = emptyMap(),
        onFolderCreated: (suspend (SyncAction, ActionResult) -> Unit)? = null,
    ): List<ActionResult> {
        val n = actions.size
        val results = arrayOfNulls<ActionResult>(n)
        val resolved = actions.toMutableList()
        val pathToId = initialPathToId.toMutableMap()

        // 阶段 1：本地新建目录（顺序，父先于子）
        val folderIdx = actions.mapIndexedNotNull { idx, a ->
            if (a.type == SyncActionType.CREATE_FOLDER && a.cloudFile == null) idx else null
        }.sortedBy { idx -> actions[idx].relativePath.count { it == '/' } }

        for (i in folderIdx) {
            resolved[i] = fillParentFileId(resolved[i], pathToId)
            val res = if (requiresMissingParent(resolved[i])) {
                ActionResult(success = false, errorMessage = "嵌套目录缺少已提交的云端父目录")
            } else {
                executeAll(listOf(resolved[i])).first()
            }
            results[i] = res
            val createdId = res.cloudFile?.id ?: res.cloudFileId
            if (res.success && !createdId.isNullOrBlank()) pathToId[resolved[i].relativePath] = createdId
            if (res.success && onFolderCreated != null) {
                onFolderCreated(resolved[i], res)
            }
        }

        // 阶段 2：剩余动作并发
        val otherIdx = (0 until n).filter { results[it] == null }
        val executableIdx = mutableListOf<Int>()
        val otherActions = otherIdx.mapNotNull { idx ->
            resolved[idx] = fillParentFileId(resolved[idx], pathToId)
            if (requiresMissingParent(resolved[idx])) {
                results[idx] = ActionResult(success = false, errorMessage = "嵌套动作缺少已提交的云端父目录")
                null
            } else {
                executableIdx += idx
                resolved[idx]
            }
        }
        if (otherActions.isNotEmpty()) {
            val otherResults = executeAll(otherActions)
            for ((k, idx) in executableIdx.withIndex()) {
                results[idx] = otherResults[k]
            }
        }

        return results.map { it ?: ActionResult(success = false, errorMessage = "未执行") }
    }

    private fun fillParentFileId(action: SyncAction, pathToId: Map<String, String>): SyncAction {
        if (action.parentFileId != null) return action
        val parentPath = action.relativePath.substringBeforeLast('/', missingDelimiterValue = "")
        return action.copy(parentFileId = pathToId[parentPath])
    }

    private fun requiresMissingParent(action: SyncAction): Boolean =
        '/' in action.relativePath && action.parentFileId.isNullOrBlank() &&
            action.type in setOf(SyncActionType.CREATE_FOLDER, SyncActionType.UPLOAD, SyncActionType.MOVE_IN_CLOUD)
}
