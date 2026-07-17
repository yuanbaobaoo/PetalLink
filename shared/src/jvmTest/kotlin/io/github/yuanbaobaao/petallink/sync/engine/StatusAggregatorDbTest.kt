package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.data.PetalLinkDb
import io.github.yuanbaobaao.petallink.data.SyncItem
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.data.TransferTask
import io.github.yuanbaobaao.petallink.sync.SyncStatus
import io.github.yuanbaobaao.petallink.sync.TransferState
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class StatusAggregatorDbTest {
    @Test
    fun 快照同时包含计数_失败项_运行态和单调revision() = runBlocking {
        val path = Files.createTempDirectory("petallink-status-").resolve("db.sqlite")
        val db = PetalLinkDb(path.toString())
        try {
            db.syncItems.upsert(item("ok", SyncStatus.SYNCED, null))
            db.syncItems.upsert(item("failed", SyncStatus.FAILED, "network"))
            db.transfers.insert(task(TransferDirection.DOWNLOAD))
            db.transfers.insert(task(TransferDirection.DOWNLOAD_UPDATE))
            val aggregator = StatusAggregator()
            val first = aggregator.snapshot(db, RuntimeStatus(isOnline = false))
            val second = aggregator.snapshot(db, RuntimeStatus(isIndexing = true, syncPhase = "bfs"))
            assertEquals(2, first.counts.total)
            assertEquals(1, first.counts.failed)
            assertEquals(2, first.counts.downloading)
            assertEquals("failed", first.failedItems.single().relativePath)
            assertEquals(SyncGlobalStatus.SYNCING, first.global)
            assertEquals(SyncGlobalStatus.INDEXING, second.global)
            assertTrue(second.revision > first.revision)
            assertEquals(second, aggregator.snapshots.value)
        } finally {
            db.close()
        }
    }

    private fun item(path: String, status: Int, error: String?) = SyncItem(
        fileId = "id-$path", localPath = path, parentFolderId = "root", name = path,
        isFolder = false, size = 1, localSize = 1, sha256 = null,
        localMtime = 1, cloudEditedTime = 1, lastSyncTime = 1,
        status = status, errorMessage = error,
    )

    private fun task(direction: TransferDirection) = TransferTask(
        id = null,
        direction = direction,
        fileId = "download-${direction.name}",
        localPath = "/tmp/${direction.name}",
        name = direction.name,
        state = TransferState.Running,
        errorMessage = null,
        createdAt = 1,
    )
}
