package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.drive.FilesApi
import io.github.yuanbaobaao.petallink.drive.DriveFile
import io.github.yuanbaobaao.petallink.sync.isFolder
import kotlinx.coroutines.*

/**
 * 云端树刷新器接口（对标 cloud_tree.rs + engine/cache.rs）。
 */
interface CloudTreeRefresher {
    suspend fun refreshFull(): CloudTreeCache
    suspend fun refreshIncremental(cursor: String): CloudTreeCache
}

/**
 * 云端树 BFS 刷新器实现（对标 src/sync/cloud_tree.rs refresh_cloud_tree）。
 * BFS 并发 8，每层并行请求，retry ≤2。
 */
class BfsCloudTreeRefresher(
    private val filesApi: FilesApi,
    private var rootFolderId: String? = null,
) : CloudTreeRefresher {

    companion object { const val INDEXING_CONCURRENCY = 8 }

    private data class BfsNode(val folderId: String, val path: String, val retries: Int = 0)

    override suspend fun refreshFull(): CloudTreeCache = coroutineScope {
        val tree = mutableMapOf<String, DriveFile>()
        val pathToId = mutableMapOf<String, String>()
        val visited = mutableSetOf<String>()
        val queue = ArrayDeque<BfsNode>()
        queue.add(BfsNode("", ""))

        while (queue.isNotEmpty()) {
            val batchSize = minOf(INDEXING_CONCURRENCY, queue.size)
            val batch = List(batchSize) { queue.removeFirst() }
            val results = batch.map { async { listFolder(it) } }.awaitAll()

            for ((node, result) in batch.zip(results)) {
                when (result) {
                    is FolderResult.Ok -> {
                        if (node.path.isEmpty() && rootFolderId == null && result.files.isNotEmpty()) {
                            rootFolderId = CloudTreeRefresh.detectRootFolderId(result.files)
                        }
                        for (f in result.files) {
                            val name = f.name ?: continue
                            if (name.startsWith(".hwcloud_")) continue
                            CloudTreeRefresh.validatePathSegment(name)
                            val relPath = if (node.path.isEmpty()) name else "${node.path}/${name}"
                            tree[relPath] = f; f.id?.let { pathToId[relPath] = it }
                            if (f.isFolder() && f.id != null && visited.add(f.id)) queue.add(BfsNode(f.id, relPath))
                        }
                    }
                    is FolderResult.Retry -> {
                        if (node.retries < 2) queue.add(node.copy(retries = node.retries + 1))
                        else throw io.github.yuanbaobaao.petallink.AppError.Remote(0, "云端树刷新不完整：目录 ${node.path} 重试耗尽")
                    }
                }
            }
        }
        CloudTreeCache(tree = tree, pathToId = pathToId, rootFolderId = rootFolderId, cursor = null, complete = true)
    }

    override suspend fun refreshIncremental(cursor: String): CloudTreeCache = refreshFull()

    private suspend fun listFolder(node: BfsNode): FolderResult = try {
        val parentId = node.folderId.ifEmpty { null }
        FolderResult.Ok(filesApi.listAllFiles(parentId))
    } catch (e: Throwable) { FolderResult.Retry }

    private sealed class FolderResult {
        data class Ok(val files: List<DriveFile>) : FolderResult()
        data object Retry : FolderResult()
    }
}
