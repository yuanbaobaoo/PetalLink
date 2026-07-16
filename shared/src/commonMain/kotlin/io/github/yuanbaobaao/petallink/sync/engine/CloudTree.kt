package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.drive.DriveFile

/**
 * 云端树缓存（对标 src/sync/cloud_tree.rs）。
 *
 * 详见 docs/06 §cloud_tree、docs/10 阶段 4 item 18。
 * BFS 遍历 + validate_trusted + 原子 checkpoint + 增量 replay。
 */
data class CloudTreeCache(
    val tree: Map<String, DriveFile>,           // relative_path → DriveFile
    val pathToId: Map<String, String>,          // relative_path → fileId
    val rootFolderId: String?,                  // 根目录 fileId
    val cursor: String?,                        // 增量游标
    val complete: Boolean,                      // 是否完整提交
) {
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
    }

    /** 是否可信（validateTrusted 不抛异常） */
    fun isTrusted(): Boolean = try {
        validateTrusted(); true
    } catch (e: Throwable) {
        false
    }
}

/**
 * 云端树刷新逻辑（对标 refresh_cloud_tree）。
 *
 * BFS 并发 8，retry ≤2，检测根目录 id（最高频 parent，平局 fail closed）。
 */
object CloudTreeRefresh {
    /** BFS 并发数 */
    const val INDEXING_CONCURRENCY = 8

    /** 内部文件前缀（跳过） */
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
