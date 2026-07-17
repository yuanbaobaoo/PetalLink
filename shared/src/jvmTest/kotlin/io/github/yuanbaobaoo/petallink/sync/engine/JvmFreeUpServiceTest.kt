package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRecord
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.mount.PlaceholderManager
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import kotlinx.coroutines.runBlocking
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class JvmFreeUpServiceTest {
    @Test
    fun 完整流程把真实内容替换为占位并提交cloudOnly基线() = environment { env ->
        val bytes = "precious".encodeToByteArray()
        val target = env.createBaseline("docs/a.txt", "f1", bytes, edited = 1_000L)
        env.persistCloud("docs/a.txt", "f1", bytes.size.toLong())
        val service = env.service(remote = matchingRemote("f1", bytes.size.toLong(), 1_000L))

        assertEquals(bytes.size.toLong(), service.freeOne("docs/a.txt", "f1", bytes.size.toLong()))

        assertTrue(env.placeholder.isPlaceholder(target.toString()))
        assertEquals(0L, Files.size(target))
        val baseline = env.db.syncItems.findByFileId("f1")!!
        assertEquals(SyncStatus.CLOUD_ONLY, baseline.status)
        assertEquals(0L, baseline.localSize)
        val placeholderInode = (Files.getAttribute(target, "unix:ino") as Number).toLong().toULong()
        assertEquals("f1", env.db.inodeMap.lookup(placeholderInode)?.fileId)
        assertTrue(env.db.freeUpStaging.findAll().isEmpty())
        assertTrue(Files.list(target.parent).use { stream -> stream.noneMatch { it.fileName.toString().startsWith(".hwcloud_freeup-") } })
    }

    @Test
    fun 远端核验期间本地变化必须在staging前拒绝() = environment { env ->
        val target = env.createBaseline("a.txt", "f1", "before".encodeToByteArray(), edited = 2_000L)
        env.persistCloud("a.txt", "f1", 6L)
        val service = env.service(remote = FreeUpRemoteVerifier {
            Files.writeString(target, "changed")
            RemoteFreeUpSnapshot("f1", 6L, 2_000L, false)
        })

        assertFailsWith<AppError.Conflict> { service.freeOne("a.txt", "f1", 6L) }
        assertEquals("changed", Files.readString(target))
        assertTrue(env.db.freeUpStaging.findAll().isEmpty())
        assertEquals(SyncStatus.SYNCED, env.db.syncItems.findByFileId("f1")?.status)
    }

    @Test
    fun 占位创建失败时恢复原文件并清理writeAhead记录() = environment { env ->
        val bytes = "restore-me".encodeToByteArray()
        val target = env.createBaseline("a.txt", "f1", bytes, edited = 3_000L)
        env.persistCloud("a.txt", "f1", bytes.size.toLong())
        env.placeholder.failCreate = true
        val service = env.service(remote = matchingRemote("f1", bytes.size.toLong(), 3_000L))

        assertFailsWith<AppError.LocalIo> { service.freeOne("a.txt", "f1", bytes.size.toLong()) }
        assertContentEquals(bytes, Files.readAllBytes(target))
        assertFalse(env.placeholder.isPlaceholder(target.toString()))
        assertTrue(env.db.freeUpStaging.findAll().isEmpty())
        assertEquals(SyncStatus.SYNCED, env.db.syncItems.findByFileId("f1")?.status)
    }

    @Test
    fun 启动恢复还原staging且绝不覆盖新用户文件() = environment { env ->
        val original = env.createBaseline("a.txt", "f1", "old".encodeToByteArray(), edited = 4_000L)
        val mtime = Files.getLastModifiedTime(original).toMillis()
        val staging = env.mount.resolve(".hwcloud_freeup-crash")
        Files.move(original, staging)
        env.placeholder.createPlaceholderStrict("a.txt")
        env.db.syncItems.casMarkCloudOnly("f1", "a.txt", mtime, 3L)
        env.db.freeUpStaging.insert(
            FreeUpStagingRecord(".hwcloud_freeup-crash", "a.txt", "f1", mtime, 3L, 1L),
        )
        val service = env.service(remote = matchingRemote("f1", 3L, 4_000L))

        assertEquals(1, service.recoverInterrupted())
        assertEquals("old", Files.readString(original))
        val restoredInode = (Files.getAttribute(original, "unix:ino") as Number).toLong().toULong()
        assertEquals("f1", env.db.inodeMap.lookup(restoredInode)?.fileId)
        assertEquals(SyncStatus.SYNCED, env.db.syncItems.findByFileId("f1")?.status)
        assertTrue(env.db.freeUpStaging.findAll().isEmpty())

        val collisionStaging = env.mount.resolve(".hwcloud_freeup-collision")
        Files.writeString(collisionStaging, "older")
        Files.writeString(original, "new-user-file")
        env.placeholder.placeholders.remove(original.toAbsolutePath().normalize().toString())
        env.db.freeUpStaging.insert(
            FreeUpStagingRecord(".hwcloud_freeup-collision", "a.txt", "f1", mtime, 3L, 2L),
        )
        assertEquals(0, service.recoverInterrupted())
        assertEquals("new-user-file", Files.readString(original))
        assertEquals("older", Files.readString(collisionStaging))
        assertEquals(1, env.db.freeUpStaging.findAll().size)
    }

    private fun matchingRemote(fileId: String, size: Long, edited: Long) = FreeUpRemoteVerifier {
        RemoteFreeUpSnapshot(fileId, size, edited, false)
    }

    private fun environment(block: suspend (Environment) -> Unit) = runBlocking {
        val root = createTempDirectory("petallink-freeup-")
        val mount = root.resolve("mount")
        Files.createDirectories(mount)
        val data = root.resolve("data")
        val db = PetalLinkDb(data.resolve("state.db").toString())
        val env = Environment(root, mount, AppPaths(data), db, FakePlaceholder(mount))
        try {
            block(env)
        } finally {
            db.close()
            root.toFile().deleteRecursively()
        }
    }

    private data class Environment(
        val root: Path,
        val mount: Path,
        val paths: AppPaths,
        val db: PetalLinkDb,
        val placeholder: FakePlaceholder,
    ) {
        suspend fun createBaseline(relative: String, fileId: String, content: ByteArray, edited: Long): Path {
            val target = mount.resolve(relative)
            Files.createDirectories(target.parent)
            Files.write(target, content)
            val mtime = Files.getLastModifiedTime(target).toMillis()
            db.syncItems.upsert(
                SyncItem(
                    fileId, relative, "root", target.fileName.toString(), false,
                    content.size.toLong(), content.size.toLong(), null, mtime, edited, 1L,
                    SyncStatus.SYNCED, null,
                ),
            )
            return target
        }

        suspend fun persistCloud(relative: String, fileId: String, size: Long) {
            val file = DriveFile(id = fileId, name = relative.substringAfterLast('/'), size = size.toString())
            JvmCloudTreeCheckpointStore(paths.cloudTreeCheckpoint(mount)).persist(
                CloudTreeCache.trusted(mapOf(relative to file), mapOf(relative to fileId), "root", "cursor"),
            )
        }

        fun service(remote: FreeUpRemoteVerifier) =
            JvmFreeUpService(mount, paths, db, placeholder, remote, nowMs = { 10L })
    }

    private class FakePlaceholder(private val root: Path) : PlaceholderManager {
        val placeholders = mutableSetOf<String>()
        var failCreate = false

        override suspend fun createPlaceholderIfNeeded(relativePath: String): Boolean {
            val target = root.resolve(relativePath)
            if (Files.exists(target)) return false
            createPlaceholderStrict(relativePath)
            return true
        }

        override suspend fun createPlaceholderStrict(relativePath: String) {
            if (failCreate) throw AppError.LocalIo("injected placeholder failure")
            val target = root.resolve(relativePath).toAbsolutePath().normalize()
            Files.createDirectories(target.parent)
            Files.createFile(target)
            placeholders += target.toString()
        }

        override suspend fun isPlaceholder(absolutePath: String): Boolean =
            Path.of(absolutePath).toAbsolutePath().normalize().toString() in placeholders

        override suspend fun markDownloaded(absolutePath: String) { placeholders.remove(Path.of(absolutePath).toAbsolutePath().normalize().toString()) }
        override suspend fun setFinderGreyLabel(absolutePath: String, on: Boolean) = Unit
        override suspend fun deleteLocal(absolutePath: String) { Files.deleteIfExists(Path.of(absolutePath)) }
        override suspend fun backupModifiedPlaceholder(absolutePath: String): String? = null
    }
}
