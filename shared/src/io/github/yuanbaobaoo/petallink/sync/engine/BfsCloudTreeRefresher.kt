package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.drive.ChangesApi
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.isFolder
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * 云端树刷新器接口（对标 cloud_tree.rs + engine/cache.rs）。
 */
interface CloudTreeRefresher {
    /**
     * 全量刷新云端目录树，返回全新构造的可信缓存。
     */
    suspend fun refreshFull(): CloudTreeCache

    /**
     * 基于 [cursor] 增量刷新；游标失效或阈值超限时自动回退到全量。
     */
    suspend fun refreshIncremental(cursor: String): CloudTreeCache
}

/**
 * 云端树 BFS 刷新器实现（对标 src/sync/cloud_tree.rs refresh_cloud_tree）。
 * BFS 并发 8，每层并行请求，retry ≤2。
 */
class BfsCloudTreeRefresher(
    private val filesApi: FilesApi,
    private val changesApi: ChangesApi,
    private val checkpointStore: CloudTreeCheckpointStore,
    private val forcedFullThreshold: Int = 300,
) : CloudTreeRefresher {

    companion object { const val INDEXING_CONCURRENCY = 8 }

    /**
     * BFS 遍历队列节点：记录待列举的文件夹 id、相对路径以及已重试次数。
     */
    private data class BfsNode(val folderId: String?, val path: String, val retries: Int = 0)
    private val refreshMutex = Mutex()
    private var incrementalSinceFull = 0

    /**
     * 全量刷新：互斥锁下执行 buildFull。
     */
    override suspend fun refreshFull(): CloudTreeCache = refreshMutex.withLock { buildFull() }

    /**
     * 增量刷新实现：游标失效、阈值超限或增量失败时回退到全量刷新。
     */
    override suspend fun refreshIncremental(cursor: String): CloudTreeCache = refreshMutex.withLock {
        require(cursor.isNotBlank()) { "增量 cursor 不能为空" }
        if (incrementalSinceFull >= forcedFullThreshold) return@withLock buildFull()
        val current = checkpointStore.loadTrusted()
        if (current == null || current.cursor != cursor) return@withLock buildFull()
        try {
            val (changes, finalCursor) = changesApi.listAllChanges(cursor)
            val candidate = CloudTreeChanges.apply(current, changes, finalCursor)
            checkpointStore.persist(candidate)
            incrementalSinceFull++
            candidate
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Throwable) {
            buildFull()
        }
    }

    /**
     * 全量构造：先取 startCursor，BFS 建候选，再 replay startCursor 起的增量变更并落盘。
     */
    private suspend fun buildFull(): CloudTreeCache {
        val startCursor = changesApi.getStartCursor()
        val candidate = buildBfsCandidate()
        val (changes, finalCursor) = changesApi.listAllChanges(startCursor)
        val replayed = CloudTreeChanges.apply(candidate, changes, finalCursor)
        checkpointStore.persist(replayed)
        incrementalSinceFull = 0
        return replayed
    }

    /**
     * BFS 遍历构建候选云树：每层并发 8 个目录列举，检测根 fileId，遇重复路径/耗尽重试即失败。
     */
    private suspend fun buildBfsCandidate(): CloudTreeCache = coroutineScope {
        val tree = mutableMapOf<String, DriveFile>()
        val pathToId = mutableMapOf<String, String>()
        val visited = mutableSetOf<String>()
        val queue = ArrayDeque<BfsNode>()
        var rootFolderId: String? = null
        queue.add(BfsNode(null, ""))

        while (queue.isNotEmpty()) {
            val batchSize = minOf(INDEXING_CONCURRENCY, queue.size)
            val batch = List(batchSize) { queue.removeFirst() }
            val results = batch.map { async { listFolder(it) } }.awaitAll()

            for ((node, result) in batch.zip(results)) {
                when (result) {
                    is FolderResult.Ok -> {
                        if (node.path.isEmpty() && rootFolderId == null && result.files.isNotEmpty()) {
                            rootFolderId = CloudTreeRefresh.detectRootFolderId(result.files)
                            if (rootFolderId == null) {
                                throw io.github.yuanbaobaoo.petallink.AppError.Data("根目录 parentFolder 最高频平局或缺失，拒绝推断 root ID")
                            }
                        }
                        for (f in result.files) {
                            val name = f.name ?: throw io.github.yuanbaobaoo.petallink.AppError.Data("BFS 文件缺少 name")
                            val id = f.id ?: throw io.github.yuanbaobaoo.petallink.AppError.Data("BFS 文件缺少 id: $name")
                            if (name.startsWith(".hwcloud_")) continue
                            CloudTreeRefresh.validatePathSegment(name)
                            val relPath = if (node.path.isEmpty()) name else "${node.path}/${name}"
                            if (tree.containsKey(relPath) || pathToId.containsValue(id)) {
                                throw io.github.yuanbaobaoo.petallink.AppError.Data("BFS 包含重复路径或 fileId: $relPath / $id")
                            }
                            tree[relPath] = f
                            pathToId[relPath] = id
                            if (f.isFolder() && visited.add(id)) queue.add(BfsNode(id, relPath))
                        }
                    }
                    is FolderResult.Retry -> {
                        if (node.retries < 2) queue.add(node.copy(retries = node.retries + 1))
                        else throw io.github.yuanbaobaoo.petallink.AppError.Remote(0, "云端树刷新不完整：目录 ${node.path} 重试耗尽")
                    }
                }
            }
        }
        CloudTreeCache(tree = tree, pathToId = pathToId, rootFolderId = rootFolderId, cursor = null, complete = false)
    }

    /**
     * 列举单个文件夹文件，成功返回 Ok，其他异常（除取消）转为可重试的 Retry。
     */
    private suspend fun listFolder(node: BfsNode): FolderResult = try {
        FolderResult.Ok(filesApi.listAllFiles(node.folderId))
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (e: Throwable) { FolderResult.Retry }

    /**
     * 单个文件夹列举的结果密封类：成功携带文件列表，失败为可重试分支。
     */
    private sealed class FolderResult {
        /**
         * 文件夹列举成功，携带该文件夹下的文件列表
         */
        data class Ok(val files: List<DriveFile>) : FolderResult()

        /**
         * 列举失败（非取消），可重新入队重试。
         */
        data object Retry : FolderResult()
    }
}
