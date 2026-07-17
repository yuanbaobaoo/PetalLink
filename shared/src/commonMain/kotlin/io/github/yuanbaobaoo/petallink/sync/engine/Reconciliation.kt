package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.sync.isFolder

/**
 * 完整的 Reconciliation 模块（对标 src/sync/engine/reconciliation.rs
 * + engine/cache.rs apply_changes_to_candidate + path_recovery.rs rekey_candidate_subtree）。
 *
 * 包含：id_to_path 双向索引、rekey 子路径重映射、增量 changes replay、
 *       DB 基线协调（三层关卡在 inode 方案后简化）。
 * 详见 docs/06 §reconciliation、docs/11 §4.4。
 */
object Reconciliation {

    /**
     * 构建 id_to_path 反向索引（fileId → relative_path）。
     * 用于增量 changes replay 中快速查找文件的当前路径。
     *
     * @param rootFolderId 根目录 fileId（映射到 ""）
     */
    fun buildIdToPath(pathToId: Map<String, String>, rootFolderId: String? = null): MutableMap<String, String> {
        val idToPath = mutableMapOf<String, String>()
        for ((path, fid) in pathToId) idToPath[fid] = path
        if (!rootFolderId.isNullOrBlank()) idToPath[rootFolderId] = ""
        return idToPath
    }

    /**
     * 重映射子路径（对标 rekey_candidate_subtree）。
     *
     * 当云端树中某 fileId 的路径发生变化（改名/移动）时，
     * 把所有以 oldPath 为前缀的条目重新映射到 newPath 前缀。
     *
     * 检查：
     * 1. oldPath 在 tree 中必须存在（至少一条以 oldPath 为前缀的条目）
     * 2. 目标路径不能与未移动的条目冲突
     * 3. 不能移动到自己的子树内
     */
    fun rekeySubtree(
        tree: MutableMap<String, io.github.yuanbaobaoo.petallink.drive.DriveFile>,
        pathToId: MutableMap<String, String>,
        idToPath: MutableMap<String, String>,
        oldRoot: String,
        newRoot: String,
    ) {
        // 1. 收集所有 oldRoot 前缀的路径
        val oldPaths = tree.keys.filter { it == oldRoot || it.startsWith("$oldRoot/") }.toList()
        require(oldPaths.isNotEmpty()) { "oldRoot 在 tree 中不存在: $oldRoot" }
        require(!newRoot.startsWith("$oldRoot/")) { "不能移动到自己的子树内: $newRoot ⊆ $oldRoot" }

        // 2. 检查目标冲突
        val moves: List<Pair<String, String>> = oldPaths.map { old ->
            val suffix = if (old == oldRoot) "" else old.removePrefix(oldRoot).removePrefix("/")
            val newPath = if (suffix.isEmpty()) newRoot else "$newRoot/$suffix"
            old to newPath
        }
        val movedSet = moves.map { it.first }.toSet()
        for ((_, newPath) in moves) {
            val existing = tree[newPath]
            require(existing == null || newPath in movedSet) {
                "目标路径冲突: $newPath 已存在且不在移动集合中"
            }
        }

        // 3. 删除旧路径 → 写入新路径
        for ((oldPath, newPath) in moves) {
            val file = tree.remove(oldPath) ?: continue
            tree[newPath] = file
            val fid = pathToId.remove(oldPath) ?: continue
            pathToId[newPath] = fid
            idToPath.put(fid, newPath)
        }
    }

    /**
     * 应用增量 changes 到候选云树（对标 apply_changes_to_candidate）。
     *
     * @param changes 从 ChangesApi 获取的增量变更列表
     * @param tree 现有云树（会被原地修改）
     * @param pathToId path→fileId 映射（会被原地修改）
     * @param rootFolderId 根目录 id
     */
    fun applyChangesToCandidate(
        changes: List<io.github.yuanbaobaoo.petallink.drive.DriveChange>,
        tree: MutableMap<String, io.github.yuanbaobaoo.petallink.drive.DriveFile>,
        pathToId: MutableMap<String, String>,
        rootFolderId: String?,
    ) {
        val idToPath = buildIdToPath(pathToId, rootFolderId)

        for (change in changes) {
            val fileId = change.fileId

            when (change.kind) {
                io.github.yuanbaobaoo.petallink.drive.ChangeKind.REMOVED -> {
                    // 从 tree + pathToId + idToPath 中删除该 fileId 及其子路径
                    val relPath = idToPath[fileId] ?: continue
                    require(relPath.isNotEmpty()) { "不能删除根目录" }
                    val prefix = "$relPath/"
                    val toRemove = tree.keys.filter { it == relPath || it.startsWith(prefix) }
                    for (p in toRemove) {
                        tree.remove(p)
                        pathToId.remove(p)?.let { idToPath.remove(it) }
                    }
                }

                io.github.yuanbaobaoo.petallink.drive.ChangeKind.MODIFIED -> {
                    val file = change.file ?: throw AppError.Remote(0, "MODIFIED change 缺少 file: $fileId")
                    val name = file.name ?: throw AppError.Remote(0, "change file 缺少 name: $fileId")
                    validatePathSegmentFull(name)

                    // 获取父目录路径
                    val parents = file.parent ?: throw AppError.Remote(0, "change file 缺少 parent: $fileId")
                    val parentPath = idToPath[parents] ?: throw AppError.Remote(0, "change file 的父目录 $parents 不在云树中")
                    val desiredPath = if (parentPath.isEmpty()) name else "$parentPath/$name"

                    // 检查是否重命名/移动
                    val existingPath = idToPath[fileId]
                    if (existingPath != null && existingPath != desiredPath) {
                        require(existingPath.isNotEmpty()) { "不能移动根目录" }
                        rekeySubtree(tree as MutableMap, pathToId as MutableMap, idToPath as MutableMap, existingPath, desiredPath)
                    } else if (pathToId[desiredPath] != null && pathToId[desiredPath] != fileId) {
                        throw AppError.Remote(0, "目标路径冲突: $desiredPath 已被 ${pathToId[desiredPath]} 占用")
                    }

                    // 写新值
                    tree[desiredPath] = file
                    pathToId[desiredPath] = fileId
                    idToPath[fileId] = desiredPath
                }
            }
        }
    }

    /**
     * 校验路径段（含错误信息前缀）
     */
    private fun validatePathSegmentFull(segment: String) {
        require(segment.isNotEmpty()) { "文件名为空" }
        require('.' !in segment || (segment != "." && segment != "..")) { "文件名为目录引用: $segment" }
        require('/' !in segment) { "文件名含斜杠: $segment" }
    }
}
