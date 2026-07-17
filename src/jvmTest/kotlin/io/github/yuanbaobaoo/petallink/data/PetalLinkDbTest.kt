package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.data.repository.StaleRevisionException
import java.nio.file.Files
import java.sql.Connection
import java.sql.DriverManager
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class PetalLinkDbTest {
    @Test
    fun 首次打开创建v6终态表且二次打开可读取() = runBlocking {
        val path = Files.createTempDirectory("petallink-db-test-").resolve("petal_link.db")
        PetalLinkDb(path.toString()).useDb { db ->
            db.syncItems.upsert(sampleSyncItem())
        }
        PetalLinkDb(path.toString()).useDb { db ->
            assertEquals("docs/a.txt", db.syncItems.findByFileId("cloud-1")?.localPath)
        }
        inspect(path.toString()) { connection ->
            assertEquals(6, userVersion(connection))
            assertTrue(tableExists(connection, "local_inode_map"))
            assertTrue(columnExists(connection, "transfer_queue", "remote_result_file_id"))
        }
    }

    @Test
    fun v2到v5数据库都能逐级升级到v6() {
        for (version in 2..5) {
            val path = Files.createTempDirectory("petallink-v$version-").resolve("fixture.db")
            createLegacyFixture(path.toString(), version)
            PetalLinkDb(path.toString()).close()
            inspect(path.toString()) { connection ->
                assertEquals(6, userVersion(connection), "v$version fixture")
                assertTrue(columnExists(connection, "sync_items", "local_size"))
                assertTrue(columnExists(connection, "transfer_queue", "state_revision"))
                assertTrue(tableExists(connection, "free_up_staging"))
            }
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

    private fun createLegacyFixture(path: String, version: Int) {
        inspect(path) { connection ->
            connection.createStatement().use { statement ->
                statement.execute(
                    """CREATE TABLE sync_items (
                        file_id TEXT NOT NULL, local_path TEXT NOT NULL, parent_folder_id TEXT,
                        name TEXT NOT NULL, is_folder INTEGER NOT NULL DEFAULT 0,
                        size INTEGER NOT NULL DEFAULT 0, sha256 TEXT, local_mtime INTEGER,
                        cloud_edited_time INTEGER, last_sync_time INTEGER,
                        status INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                        PRIMARY KEY(file_id, local_path)
                    )""".trimIndent(),
                )
                statement.execute(
                    """CREATE TABLE transfer_queue (
                        id INTEGER PRIMARY KEY AUTOINCREMENT, direction INTEGER NOT NULL,
                        file_id TEXT, local_path TEXT, name TEXT NOT NULL,
                        total_size INTEGER NOT NULL DEFAULT 0, transferred INTEGER NOT NULL DEFAULT 0,
                        state INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                        created_at INTEGER NOT NULL, finished_at INTEGER
                    )""".trimIndent(),
                )
                if (version >= 2) {
                    statement.execute("ALTER TABLE transfer_queue ADD COLUMN server_id TEXT")
                    statement.execute("ALTER TABLE transfer_queue ADD COLUMN upload_id TEXT")
                    statement.execute("ALTER TABLE transfer_queue ADD COLUMN resume_offset INTEGER NOT NULL DEFAULT 0")
                }
                if (version >= 3) statement.execute("ALTER TABLE sync_items ADD COLUMN local_size INTEGER")
                if (version >= 4) statement.execute("ALTER TABLE transfer_queue ADD COLUMN session_url TEXT")
                if (version >= 5) {
                    listOf(
                        "relative_path TEXT", "parent_file_id TEXT", "operation INTEGER",
                        "source_mtime INTEGER", "source_size INTEGER", "expected_cloud_edited_time INTEGER",
                        "attempt_count INTEGER NOT NULL DEFAULT 0", "next_retry_at INTEGER",
                        "error_kind INTEGER", "remote_result_file_id TEXT",
                        "state_revision INTEGER NOT NULL DEFAULT 0",
                    ).forEach { statement.execute("ALTER TABLE transfer_queue ADD COLUMN $it") }
                }
                statement.execute("PRAGMA user_version = $version")
            }
        }
    }

    private inline fun inspect(path: String, block: (Connection) -> Unit) {
        DriverManager.getConnection("jdbc:sqlite:$path").use(block)
    }

    private fun userVersion(connection: Connection): Int = connection.createStatement().use { statement ->
        statement.executeQuery("PRAGMA user_version").use { it.getInt(1) }
    }

    private fun tableExists(connection: Connection, table: String): Boolean =
        connection.prepareStatement("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?").use { query ->
            query.setString(1, table)
            query.executeQuery().use { it.next() }
        }

    private fun columnExists(connection: Connection, table: String, column: String): Boolean =
        connection.createStatement().use { statement ->
            statement.executeQuery("PRAGMA table_info($table)").use { rows ->
                while (rows.next()) if (rows.getString("name") == column) return@use true
                false
            }
        }
}
