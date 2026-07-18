package io.github.yuanbaobaoo.petallink.sync.identity

import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncActionType
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache

/**
 * 把 inode 检测到的本地路径变化合并为最小的云端移动动作集合。
 */
object LocalMoveActionReconciler {

    /**
     * 移除目录整体移动下的子项移动，并替换 planner 生成的删除/上传动作。
     */
    fun reconcile(
        planned: List<SyncAction>,
        detected: List<DetectedMove>,
        cloud: CloudTreeCache,
    ): List<SyncAction> {
        if (detected.isEmpty()) return planned
        val moves = collapseNested(detected)
        val removedRoots = moves.flatMap { listOf(it.oldRelativePath, it.newRelativePath) }
        val retained = planned.filterNot { action ->
            removedRoots.any { root -> action.relativePath == root || action.relativePath.startsWith("$root/") }
        }.toMutableList()
        for (move in moves) {
            val remote = cloud.tree[move.oldRelativePath] ?: continue
            val parentPath = move.newRelativePath.substringBeforeLast('/', "")
            retained += SyncAction(
                type = SyncActionType.MOVE_IN_CLOUD,
                relativePath = move.newRelativePath,
                fileId = move.fileId,
                cloudFile = remote,
                reason = "inode 未变且路径变化 → 云端移动",
                parentFileId = cloud.pathToId[parentPath] ?: cloud.rootFolderId,
            )
        }
        return retained
    }

    /**
     * 目录整体移动时只保留最上层移动，后代路径由云端子树移动一并完成。
     */
    fun collapseNested(moves: List<DetectedMove>): List<DetectedMove> {
        val accepted = mutableListOf<DetectedMove>()
        for (move in moves.sortedBy { it.oldRelativePath.count { char -> char == '/' } }) {
            val nested = accepted.any { parent ->
                move.oldRelativePath.startsWith("${parent.oldRelativePath}/") &&
                    move.newRelativePath == parent.newRelativePath +
                    move.oldRelativePath.removePrefix(parent.oldRelativePath)
            }
            if (!nested) accepted += move
        }
        return accepted
    }
}
