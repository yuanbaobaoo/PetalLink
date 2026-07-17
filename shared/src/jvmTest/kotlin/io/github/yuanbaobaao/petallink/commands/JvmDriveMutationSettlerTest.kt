package io.github.yuanbaobaao.petallink.commands

import io.github.yuanbaobaao.petallink.config.ConfigStore
import io.github.yuanbaobaao.petallink.config.UserConfig
import io.github.yuanbaobaao.petallink.data.PetalLinkDb
import io.github.yuanbaobaao.petallink.data.SyncItem
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.drive.DriveFile
import io.github.yuanbaobaao.petallink.mount.XattrAccess
import io.github.yuanbaobaao.petallink.sync.SyncStatus
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class JvmDriveMutationSettlerTest {
    @Test
    fun 文件夹重命名会同步迁移本地子树和DB基线() = runBlocking {
        fixture { mount, db, settler ->
            val oldFolder = Files.createDirectory(mount.resolve("old"))
            val child = Files.writeString(oldFolder.resolve("a.txt"), "hello")
            db.syncItems.upsert(item("folder", "old", isFolder = true))
            db.syncItems.upsert(item(
                "child", "old/a.txt", size = 5,
                localSize = 5, localMtime = Files.getLastModifiedTime(child).toMillis(),
            ))

            val plan = assertNotNull(settler.planRename("folder", "new"))
            settler.settlePathChange(
                plan,
                DriveFile(id = "folder", name = "new", parentFolder = listOf("root"), editedTime = "2025-01-01T00:00:00Z"),
            )

            assertFalse(Files.exists(mount.resolve("old")))
            assertEquals("hello", Files.readString(mount.resolve("new/a.txt")))
            assertEquals("new", db.syncItems.findByFileId("folder")?.localPath)
            assertEquals("new/a.txt", db.syncItems.findByFileId("child")?.localPath)
        }
    }

    @Test
    fun 已核验删除会删本地文件保留墓碑并写Completed留痕() = runBlocking {
        fixture { mount, db, settler ->
            val local = Files.writeString(mount.resolve("a.txt"), "hello")
            db.syncItems.upsert(item(
                "cloud-a", "a.txt", size = 5,
                localSize = 5, localMtime = Files.getLastModifiedTime(local).toMillis(),
            ))

            val plan = settler.planDelete("cloud-a")
            settler.settleDelete(plan, "fallback")

            assertFalse(Files.exists(local))
            assertEquals(SyncStatus.DELETED, db.syncItems.findByFileId("cloud-a")?.status)
            val trace = db.transfers.selectAll().single()
            assertEquals(TransferDirection.DELETE, trace.direction)
            assertEquals(4, trace.operation)
            assertEquals(io.github.yuanbaobaao.petallink.sync.TransferState.Completed, trace.state)
        }
    }

    @Test
    fun 文件夹含未纳入基线的用户文件时在远端写入前拒绝删除() = runBlocking {
        fixture { mount, db, settler ->
            val folder = Files.createDirectory(mount.resolve("docs"))
            Files.writeString(folder.resolve("untracked.txt"), "keep")
            db.syncItems.upsert(item("folder", "docs", isFolder = true))

            assertFailsWith<io.github.yuanbaobaao.petallink.AppError.LocalIo> {
                settler.planDelete("folder")
            }
            assertTrue(Files.exists(folder.resolve("untracked.txt")))
        }
    }

    private suspend fun fixture(block: suspend (java.nio.file.Path, PetalLinkDb, JvmDriveMutationSettler) -> Unit) {
        val workspace = Files.createTempDirectory("petallink-drive-settle-")
        val mount = Files.createDirectory(workspace.resolve("mount"))
        val db = PetalLinkDb(workspace.resolve("db.sqlite").toString())
        val config = object : ConfigStore {
            override fun load() = UserConfig(mountDir = mount.toString(), mountConfigured = true)
            override fun save(config: UserConfig) = Unit
        }
        val xattrs = MemoryXattrs()
        try {
            block(mount, db, JvmDriveMutationSettler(config, db, xattrs))
        } finally {
            db.close()
        }
    }

    private fun item(
        id: String,
        path: String,
        isFolder: Boolean = false,
        size: Long = 0,
        localSize: Long? = null,
        localMtime: Long? = null,
    ) = SyncItem(
        fileId = id,
        localPath = path,
        parentFolderId = "root",
        name = path.substringAfterLast('/'),
        isFolder = isFolder,
        size = size,
        localSize = localSize,
        sha256 = null,
        localMtime = localMtime,
        cloudEditedTime = null,
        lastSyncTime = 1,
        status = SyncStatus.SYNCED,
        errorMessage = null,
    )

    private class MemoryXattrs : XattrAccess {
        private val values = mutableMapOf<Pair<String, String>, ByteArray>()
        override fun get(path: String, name: String) = values[path to name]
        override fun set(path: String, name: String, value: ByteArray) { values[path to name] = value }
        override fun remove(path: String, name: String) { values.remove(path to name) }
    }
}
