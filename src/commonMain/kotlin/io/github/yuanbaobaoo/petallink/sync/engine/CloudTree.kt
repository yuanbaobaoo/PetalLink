package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.drive.ChangeKind
import io.github.yuanbaobaoo.petallink.drive.DriveChange
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import kotlinx.serialization.Serializable

/**
 * 云端树缓存（对标 src/sync/cloud_tree.rs）。
 *
 * 详见 docs/06 §cloud_tree、docs/10 阶段 4 item 18。
 * BFS 遍历 + validate_trusted + 原子 checkpoint + 增量 replay。
 */
@Serializable
data class CloudTreeCache(
    val tree: Map<String, DriveFile>,           // relative_path → DriveFile
    val pathToId: Map<String, String>,          // relative_path → fileId
    val rootFolderId: String?,                  // 根目录 fileId
    val cursor: String?,                        // 增量游标
    val complete: Boolean,                      // 是否完整提交
) {
    companion object {
        /**
         * 由原始映射构造一个可信缓存：补全根 fileId 索引并立即执行 [validateTrusted]。
         */
        fun trusted(
            tree: Map<String, DriveFile>,
            pathToId: Map<String, String>,
            rootFolderId: String?,
            cursor: String,
        ): CloudTreeCache {
            val indexed = pathToId.toMutableMap()
            rootFolderId?.takeIf(String::isNotBlank)?.let { indexed[""] = it }
            return CloudTreeCache(tree.toMap(), indexed, rootFolderId, cursor, complete = true)
                .also(CloudTreeCache::validateTrusted)
        }
    }

    /**
     * 校验可信度（对标 validate_trusted）。
     *
     * 四项检查：
     * 1. complete == true
     * 2. cursor 非空
     * 3. fileId 全局唯一
     * 4. path_to_id 与 tree 双向一致
     *
     * @throws AppError.Data 校验失败
     */
    fun validateTrusted() {
        if (!complete) throw AppError.Data("未完整提交")
        if (cursor.isNullOrBlank()) throw AppError.Data("缺少有效 cursor")

        val seenIds = mutableSetOf<String>()
        for ((path, file) in tree) {
            if (path.isEmpty()) throw AppError.Data("空路径")
            if (file.id.isNullOrBlank()) throw AppError.Data("空 fileId: $path")
            if (!seenIds.add(file.id)) throw AppError.Data("fileId 重复: ${file.id}")
            // 双向一致：path_to_id[path] == file.id
            if (pathToId[path] != file.id) throw AppError.Data("路径索引不一致: $path")
        }
        // 反向：path_to_id 的每项都在 tree 中
        for ((path, fileId) in pathToId) {
            if (path.isEmpty()) {
                if (rootFolderId != fileId) throw AppError.Data("根目录索引不一致")
                continue
            }
            if (tree[path]?.id != fileId) throw AppError.Data("孤立路径索引: $path")
        }
        if (rootFolderId != null && pathToId[""] != rootFolderId) {
            throw AppError.Data("根目录索引缺失")
        }
    }

    /**
     * 是否可信（validateTrusted 不抛异常）
     */
    fun isTrusted(): Boolean = try {
        validateTrusted(); true
    } catch (e: Throwable) {
        false
    }
}

/**
 * 单文件云树 checkpoint 提交门；实现必须保证失败不替换旧版本。
 */
interface CloudTreeCheckpointStore {
    /**
     * 加载并通过可信校验的 checkpoint；不存在或不可信时返回 null。
     */
    suspend fun loadTrusted(): CloudTreeCache?

    /**
     * 原子提交 checkpoint，失败时不得替换已有版本。
     */
    suspend fun persist(checkpoint: CloudTreeCache)

    /**
     * 清理未提交的暂存（tmp/backup），用于放弃当前写入。
     */
    suspend fun discardUncommitted()
}

/**
 * Changes 只在候选 clone 上执行；任一项不可解释时整批失败。
 */
object CloudTreeChanges {
    /**
     * 在缓存副本上应用增量变更并以新游标构造可信缓存；任一变更不可解释都会在构造时抛异常。
     */
    fun apply(cache: CloudTreeCache, changes: List<DriveChange>, finalCursor: String): CloudTreeCache {
        val tree = cache.tree.toMutableMap()
        val pathToId = cache.pathToId.toMutableMap()
        applyToCandidate(tree, pathToId, cache.rootFolderId, changes)
        return CloudTreeCache.trusted(tree, pathToId, cache.rootFolderId, finalCursor)
    }

    /**
     * 直接在候选可变映射上应用变更：按 REMOVED/MODIFIED 分别删除子树或重写节点。
     */
    fun applyToCandidate(
        tree: MutableMap<String, DriveFile>,
        pathToId: MutableMap<String, String>,
        rootFolderId: String?,
        changes: List<DriveChange>,
    ) {
        val idToPath = pathToId.entries.associateTo(mutableMapOf()) { (path, id) -> id to path }
        rootFolderId?.takeIf(String::isNotBlank)?.let { idToPath[it] = "" }

        for (change in changes) {
            when (change.kind) {
                ChangeKind.REMOVED -> removeSubtree(change.fileId, tree, pathToId, idToPath)
                ChangeKind.MODIFIED -> applyModified(change, tree, pathToId, idToPath)
            }
        }
    }

    /**
     * 删除 fileId 对应节点及其整棵子树（路径等于根或以 "根/" 为前缀）。
     */
    private fun removeSubtree(
        fileId: String,
        tree: MutableMap<String, DriveFile>,
        pathToId: MutableMap<String, String>,
        idToPath: MutableMap<String, String>,
    ) {
        val root = idToPath[fileId] ?: return
        if (root.isEmpty()) throw AppError.Data("Changes 试图删除云盘根目录")
        val prefix = "$root/"
        val removed = tree.keys.filter { it == root || it.startsWith(prefix) }
        for (path in removed) {
            tree.remove(path)
            pathToId.remove(path)?.let(idToPath::remove)
        }
    }

    /**
     * 处理 MODIFIED 变更：校验父目录、计算期望路径，按需移动子树或写入新节点；冲突即抛异常。
     */
    private fun applyModified(
        change: DriveChange,
        tree: MutableMap<String, DriveFile>,
        pathToId: MutableMap<String, String>,
        idToPath: MutableMap<String, String>,
    ) {
        val file = change.file ?: throw AppError.Data("非删除 Change 缺少完整文件: ${change.fileId}")
        if (file.id != change.fileId) throw AppError.Data("Change fileId 与文件 id 不一致")
        val name = file.name ?: throw AppError.Data("Change ${change.fileId} 缺少文件名")
        CloudTreeRefresh.validatePathSegment(name)
        val parentId = file.parentFolder?.singleOrNull() ?: file.parent
            ?: throw AppError.Data("Change ${change.fileId} 缺少唯一 parentFolder")
        if (parentId.isBlank() || parentId == change.fileId) {
            throw AppError.Data("Change ${change.fileId} 的 parentFolder 非法")
        }
        val parentPath = idToPath[parentId]
            ?: throw AppError.Data("Change ${change.fileId} 的 parentFolder $parentId 无法映射")
        val desiredPath = if (parentPath.isEmpty()) name else "$parentPath/$name"
        val existingPath = idToPath[change.fileId]
        if (existingPath != null && existingPath.isEmpty()) {
            throw AppError.Data("Changes 不支持修改云盘根目录")
        }
        if (existingPath != null && existingPath != desiredPath) {
            if (desiredPath.startsWith("$existingPath/")) {
                throw AppError.Data("Change 试图把目录移到自身子树")
            }
            rekeySubtree(tree, pathToId, idToPath, existingPath, desiredPath)
        } else {
            val occupied = pathToId[desiredPath]
            if (occupied != null && occupied != change.fileId) {
                throw AppError.Data("Change 目标路径冲突: $desiredPath")
            }
        }
        tree[desiredPath] = file
        pathToId[desiredPath] = change.fileId
        idToPath[change.fileId] = desiredPath
    }

    /**
     * 把旧根子树整体改键到新根前缀下，目标路径与候选树冲突时抛异常。
     */
    private fun rekeySubtree(
        tree: MutableMap<String, DriveFile>,
        pathToId: MutableMap<String, String>,
        idToPath: MutableMap<String, String>,
        oldRoot: String,
        newRoot: String,
    ) {
        val oldPrefix = "$oldRoot/"
        val paths = tree.keys.filter { it == oldRoot || it.startsWith(oldPrefix) }
        if (paths.isEmpty()) throw AppError.Data("Change 引用的旧路径不在候选树: $oldRoot")
        val movedSet = paths.toSet()
        val targets = paths.associateWith { old -> newRoot + old.removePrefix(oldRoot) }
        targets.values.forEach { target ->
            if (target in tree && target !in movedSet) throw AppError.Data("Change 移动目标已存在: $target")
        }
        val moved = paths.map { old ->
            val file = tree.remove(old) ?: throw AppError.Data("移动时路径消失: $old")
            val id = pathToId.remove(old) ?: throw AppError.Data("移动时索引消失: $old")
            idToPath.remove(id)
            Triple(targets.getValue(old), id, file)
        }
        moved.forEach { (path, id, file) ->
            tree[path] = file
            pathToId[path] = id
            idToPath[id] = path
        }
    }
}

/**
 * 云端树刷新逻辑（对标 refresh_cloud_tree）。
 *
 * BFS 并发 8，retry ≤2，检测根目录 id（最高频 parent，平局 fail closed）。
 */
object CloudTreeRefresh {
    /**
     * BFS 并发数
     */
    const val INDEXING_CONCURRENCY = 8

    /**
     * 内部文件前缀（跳过）
     */
    private const val INTERNAL_PREFIX = ".hwcloud_"

    /**
     * 检测根目录 id（最高频 parent_folder；平局 → null，fail closed）。
     */
    fun detectRootFolderId(files: List<DriveFile>): String? {
        val counts = mutableMapOf<String, Int>()
        for (f in files) {
            val parents = f.parent?.let { listOf(it) } ?: emptyList()
            for (p in parents) {
                if (p.isNotBlank()) counts[p] = (counts[p] ?: 0) + 1
            }
        }
        if (counts.isEmpty()) return null
        val sorted = counts.entries.sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key })
        // 平局 → null
        if (sorted.size >= 2 && sorted[0].value == sorted[1].value) return null
        return sorted.firstOrNull()?.key
    }

    /**
     * 校验路径段合法（防注入/穿越）。
     */
    fun validatePathSegment(name: String) {
        require(name.isNotBlank()) { "文件名为空" }
        require(!name.contains("/")) { "文件名含斜杠: $name" }
        require(name != "." && name != "..") { "文件名为目录引用: $name" }
        require(!name.startsWith(INTERNAL_PREFIX)) { "内部文件前缀: $name" }
    }
}
