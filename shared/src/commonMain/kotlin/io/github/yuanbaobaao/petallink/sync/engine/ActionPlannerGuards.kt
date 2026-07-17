package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import io.github.yuanbaobaao.petallink.sync.SyncSnapshot
import io.github.yuanbaobaao.petallink.sync.isFolder

/** Planner 之后、Executor 之前的目录安全整形。 */
object ActionPlannerGuards {
    fun prepare(
        snapshot: SyncSnapshot,
        planned: List<SyncAction>,
        recentlyDeleted: Set<String> = emptySet(),
    ): List<SyncAction> {
        val actions = planned.toMutableList()
        addRescueFolders(snapshot, actions, recentlyDeleted)
        preserveBackupParents(snapshot, actions)
        dedupeCloudDirectoryDeletes(snapshot, actions)
        dedupeLocalDirectoryDeletes(snapshot, actions)
        return actions.sortedWith(compareBy<SyncAction> { it.relativePath.count { char -> char == '/' } }.thenBy { it.relativePath })
    }

    private fun addRescueFolders(
        snapshot: SyncSnapshot,
        actions: MutableList<SyncAction>,
        recentlyDeleted: Set<String>,
    ) {
        val creators = actions.filter { action ->
            action.type in setOf(
                SyncActionType.UPLOAD,
                SyncActionType.MOVE_IN_CLOUD,
                SyncActionType.BACKUP_BEFORE_CLOUD_DELETE,
                SyncActionType.CREATE_CONFLICT_COPY,
            ) || action.type == SyncActionType.CREATE_FOLDER && action.cloudFile == null
        }
        val existing = actions.mapTo(mutableSetOf()) { it.relativePath }
        val rescue = mutableSetOf<String>()
        for (action in creators) {
            val parts = action.relativePath.split('/')
            for (end in 1 until parts.size) {
                val ancestor = parts.take(end).joinToString("/")
                if (ancestor in existing || ancestor in recentlyDeleted) continue
                if (snapshot.local[ancestor]?.isFolder == true && ancestor !in snapshot.cloud && ancestor in snapshot.db) {
                    rescue += ancestor
                }
            }
        }
        rescue.sortedWith(compareBy<String> { it.count { char -> char == '/' } }.thenBy { it }).forEach { path ->
            actions += SyncAction(
                SyncActionType.CREATE_FOLDER,
                path,
                reason = "云端已删除但内有内容需救援 → 重建目录到云端",
            )
        }
    }

    private fun preserveBackupParents(snapshot: SyncSnapshot, actions: MutableList<SyncAction>) {
        val backupPaths = actions.filter { it.type == SyncActionType.BACKUP_BEFORE_CLOUD_DELETE }.map { it.relativePath }
        actions.removeAll { action ->
            action.type == SyncActionType.DELETE_FROM_LOCAL && snapshot.local[action.relativePath]?.isFolder == true &&
                backupPaths.any { it.startsWith("${action.relativePath}/") }
        }
    }

    private fun dedupeCloudDirectoryDeletes(snapshot: SyncSnapshot, actions: MutableList<SyncAction>) {
        val roots = actions.filter { it.type == SyncActionType.DELETE_FROM_CLOUD }
            .map { it.relativePath }
            .filter { snapshot.cloud[it]?.isFolder() == true }
        actions.removeAll { action ->
            action.type == SyncActionType.DELETE_FROM_CLOUD && roots.any { root -> action.relativePath != root && action.relativePath.startsWith("$root/") }
        }
    }

    private fun dedupeLocalDirectoryDeletes(snapshot: SyncSnapshot, actions: MutableList<SyncAction>) {
        val roots = actions.filter { it.type == SyncActionType.DELETE_FROM_LOCAL }
            .map { it.relativePath }
            .filter { snapshot.local[it]?.isFolder == true }
        actions.removeAll { action ->
            action.type == SyncActionType.DELETE_FROM_LOCAL && roots.any { root -> action.relativePath != root && action.relativePath.startsWith("$root/") }
        }
    }
}
