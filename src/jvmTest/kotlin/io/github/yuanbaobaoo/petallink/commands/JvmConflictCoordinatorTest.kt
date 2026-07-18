package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.mount.XattrAccess
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncActionType
import io.github.yuanbaobaoo.petallink.sync.executor.ActionResult
import java.nio.file.Files
import java.nio.file.attribute.FileTime
import java.time.Instant
import kotlin.io.path.createTempDirectory
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * JVM 冲突文件保全与回滚测试。
 */
class JvmConflictCoordinatorTest {

    @Test
    fun 云端胜出时先保留本地副本再覆盖原名() = withRoot { root ->
        val source = root.resolve("doc.txt")
        Files.write(source, "local".encodeToByteArray())
        Files.setLastModifiedTime(source, FileTime.fromMillis(100_000L))
        val coordinator = coordinator(root) { action ->
            assertEquals(SyncActionType.DOWNLOAD, action.type)
            Files.write(source, "cloud".encodeToByteArray())
            ActionResult(true)
        }

        assertTrue(coordinator.execute(action(150_000L)).success)

        assertContentEquals("cloud".encodeToByteArray(), Files.readAllBytes(source))
        val backup = Files.list(root).use { paths ->
            paths.filter { it.fileName.toString().contains("本地副本") }.findFirst().orElseThrow()
        }
        assertContentEquals("local".encodeToByteArray(), Files.readAllBytes(backup))
    }

    @Test
    fun 云端下载失败时恢复本地原文件() = withRoot { root ->
        val source = root.resolve("doc.txt")
        Files.write(source, "local".encodeToByteArray())
        Files.setLastModifiedTime(source, FileTime.fromMillis(100_000L))
        val coordinator = coordinator(root) { ActionResult(false, errorMessage = "offline") }

        assertFalse(coordinator.execute(action(150_000L)).success)

        assertContentEquals("local".encodeToByteArray(), Files.readAllBytes(source))
        assertEquals(1L, Files.list(root).use { it.count() })
    }

    @Test
    fun 本地胜出时先下载云端副本再上传原名() = withRoot { root ->
        val source = root.resolve("doc.txt")
        Files.write(source, "local".encodeToByteArray())
        Files.setLastModifiedTime(source, FileTime.fromMillis(200_001L))
        val order = mutableListOf<SyncActionType>()
        val coordinator = coordinator(root) { transfer ->
            order += transfer.type
            if (transfer.type == SyncActionType.DOWNLOAD) {
                Files.write(root.resolve(transfer.relativePath), "cloud".encodeToByteArray())
            }
            ActionResult(true)
        }

        assertTrue(coordinator.execute(action(100_000L)).success)

        assertEquals(listOf(SyncActionType.DOWNLOAD, SyncActionType.UPLOAD), order)
        assertContentEquals("local".encodeToByteArray(), Files.readAllBytes(source))
        assertTrue(Files.list(root).use { paths -> paths.anyMatch { it.fileName.toString().contains("云端副本") } })
    }

    private fun coordinator(
        root: java.nio.file.Path,
        execute: suspend (SyncAction) -> ActionResult,
    ) = JvmConflictCoordinator(
        root,
        JvmPlaceholderManager(root, MemoryXattrs()),
        executeTransfer = execute,
        hasActiveUpload = { false },
        zoneId = java.time.ZoneOffset.UTC,
    )

    private fun action(cloudMtime: Long) = SyncAction(
        type = SyncActionType.CREATE_CONFLICT_COPY,
        relativePath = "doc.txt",
        fileId = "file-1",
        cloudFile = DriveFile(
            id = "file-1",
            name = "doc.txt",
            editedTime = Instant.ofEpochMilli(cloudMtime).toString(),
        ),
        reason = "test",
    )

    private fun withRoot(block: suspend (java.nio.file.Path) -> Unit) = runBlocking {
        val root = createTempDirectory("petallink-conflict-")
        try {
            block(root)
        } finally {
            root.toFile().deleteRecursively()
        }
    }

    private class MemoryXattrs : XattrAccess {
        private val values = mutableMapOf<Pair<String, String>, ByteArray>()

        override fun get(path: String, name: String): ByteArray? = values[path to name]

        override fun set(path: String, name: String, value: ByteArray) {
            values[path to name] = value
        }

        override fun remove(path: String, name: String) {
            values.remove(path to name)
        }
    }
}
