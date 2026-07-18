package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.data.repository.StaleRevisionException
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class PetalLinkDbTest {
    @Test
    fun 首次打开Room数据库且二次打开可读取() = runBlocking {
        val path = Files.createTempDirectory("petallink-db-test-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.syncItems.upsert(sampleSyncItem())
        }
        PetalLinkDb(path.toString()).useDb { db ->
            assertEquals("docs/a.txt", db.syncItems.findByFileId("cloud-1")?.localPath)
        }
    }

    @Test
    fun syncItem使用fileId单主键() = runBlocking {
        val path = Files.createTempDirectory("petallink-single-pk-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.syncItems.upsert(sampleSyncItem())
            db.syncItems.upsert(sampleSyncItem().copy(localPath = "docs/renamed.txt"))
            assertEquals(1, db.syncItems.countAll())
            assertEquals("docs/renamed.txt", db.syncItems.findByFileId("cloud-1")?.localPath)
        }
    }

    @Test
    fun CAS只允许一个revision成功且迟到进度不覆盖终态() = runBlocking {
        val path = Files.createTempDirectory("petallink-cas-test-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            val id = db.transfers.insert(sampleTask())
            assertTrue(db.transfers.casTransitionState(id, 0, TransferState.Running, 0, null))
            assertFailsWith<StaleRevisionException> {
                db.transfers.casTransitionState(id, 0, TransferState.Failed, 0, "stale")
            }
            assertTrue(db.transfers.updateRunningProgress(id, 1, 64))
            assertTrue(db.transfers.casTransitionState(id, 1, TransferState.Completed, 0, null))
            assertFalse(db.transfers.updateRunningProgress(id, 1, 128))
            val stored = assertNotNull(db.transfers.findById(id))
            assertEquals(64L, stored.transferred)
            assertEquals(2L, stored.stateRevision)
            assertEquals(TransferState.Completed, stored.state)
        }
    }

    @Test
    fun inodeMap支持lookupUpsert和purgeMissing() = runBlocking {
        val path = Files.createTempDirectory("petallink-inode-db-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.inodeMap.upsert(11UL, "a.txt", "cloud-a", 1)
            db.inodeMap.upsert(22UL, "b.txt", "cloud-b", 1)
            assertEquals("cloud-a", db.inodeMap.lookup(11UL)?.fileId)
            db.inodeMap.purgeMissing(setOf(22UL))
            assertEquals(null, db.inodeMap.lookup(11UL))
            assertEquals(listOf(22UL), db.inodeMap.selectAll().map { it.inode })
            db.inodeMap.purgeMissing(emptySet())
            assertTrue(db.inodeMap.selectAll().isEmpty())
        }
    }

    @Test
    fun 传输历史按Completed和Failed精确清理并保留Canceled() = runBlocking {
        val path = Files.createTempDirectory("petallink-clear-history-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.transfers.insert(sampleTask().copy(state = TransferState.Completed))
            db.transfers.insert(sampleTask().copy(state = TransferState.Failed))
            db.transfers.insert(sampleTask().copy(state = TransferState.Canceled))
            db.transfers.clearHistory(includeCompleted = false, includeFailed = true)
            assertTrue(db.transfers.selectByState(TransferState.Failed).isEmpty())
            assertEquals(1, db.transfers.selectByState(TransferState.Completed).size)
            db.transfers.clearHistory(includeCompleted = true, includeFailed = false)
            assertTrue(db.transfers.selectByState(TransferState.Completed).isEmpty())
            assertEquals(1, db.transfers.selectByState(TransferState.Canceled).size)
        }
    }

    @Test
    fun 传输列表严格按创建时间和id倒序() = runBlocking {
        val path = Files.createTempDirectory("petallink-transfer-order-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.transfers.insert(sampleTask().copy(name = "old", createdAt = 10))
            db.transfers.insert(sampleTask().copy(name = "same-a", createdAt = 20))
            db.transfers.insert(sampleTask().copy(name = "same-b", createdAt = 20))
            assertEquals(listOf("same-b", "same-a", "old"), db.transfers.selectAll().map { it.name })
        }
    }

    @Test
    fun 切换挂载目录会原子清理基线传输和inode状态() = runBlocking {
        val path = Files.createTempDirectory("petallink-clear-mount-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.syncItems.upsert(sampleSyncItem())
            db.transfers.insert(sampleTask())
            db.inodeMap.upsert(11UL, "docs/a.txt", "cloud-1", 1)
            db.clearMountState()
            assertTrue(db.syncItems.selectAll().isEmpty())
            assertTrue(db.transfers.selectAll().isEmpty())
            assertTrue(db.inodeMap.selectAll().isEmpty())
        }
    }

    private fun sampleSyncItem() = SyncItem(
        fileId = "cloud-1",
        localPath = "docs/a.txt",
        parentFolderId = "root",
        name = "a.txt",
        isFolder = false,
        size = 12,
        localSize = 12,
        sha256 = "abc",
        localMtime = 1,
        cloudEditedTime = 2,
        lastSyncTime = 3,
        status = 0,
        errorMessage = null,
    )

    private fun sampleTask() = TransferTask(
        id = null,
        direction = TransferDirection.UPLOAD,
        fileId = "cloud-1",
        localPath = "/tmp/a.txt",
        name = "a.txt",
        totalSize = 128,
        state = TransferState.Pending,
        errorMessage = null,
        createdAt = System.currentTimeMillis(),
        relativePath = "a.txt",
    )

    private inline fun PetalLinkDb.useDb(block: (PetalLinkDb) -> Unit) {
        try { block(this) } finally { close() }
    }

}
