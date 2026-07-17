package io.github.yuanbaobaao.petallink.mount

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.config.AppConfig
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class JvmPlaceholderManagerTest {
    @Test
    fun 无state的用户空文件绝不转占位也不删除() = runBlocking {
        val root = Files.createTempDirectory("petallink-placeholder-user-")
        val xattrs = FakeXattrs()
        val manager = JvmPlaceholderManager(root, xattrs)
        val userFile = Files.createFile(root.resolve(".gitkeep"))

        assertFalse(manager.createPlaceholderIfNeeded(".gitkeep"))
        assertFalse(manager.isPlaceholder(userFile.toString()))
        manager.deleteLocal(userFile.toString())
        assertTrue(Files.exists(userFile))
        assertNull(xattrs.get(userFile.toString(), AppConfig.XATTR_STATE))
    }

    @Test
    fun 创建占位只写state和32字节FinderInfo且downloaded清灰标() = runBlocking {
        val root = Files.createTempDirectory("petallink-placeholder-create-")
        val xattrs = FakeXattrs()
        val manager = JvmPlaceholderManager(root, xattrs)

        assertTrue(manager.createPlaceholderIfNeeded("folder/cloud.txt"))
        val path = root.resolve("folder/cloud.txt")
        assertEquals(0, Files.size(path))
        assertTrue(manager.isPlaceholder(path.toString()))
        assertEquals("placeholder", xattrs.get(path.toString(), AppConfig.XATTR_STATE)?.decodeToString())
        val finder = assertNotNull(xattrs.get(path.toString(), AppConfig.XATTR_FINDER_INFO))
        assertEquals(32, finder.size)
        assertEquals(0x02, finder[9].toInt())

        manager.markDownloaded(path.toString())
        assertFalse(manager.isPlaceholder(path.toString()))
        assertEquals("downloaded", xattrs.get(path.toString(), AppConfig.XATTR_STATE)?.decodeToString())
        assertNull(xattrs.get(path.toString(), AppConfig.XATTR_FINDER_INFO))
    }

    @Test
    fun FinderInfo读改写保留其他字节且清灰后不误删() = runBlocking {
        val root = Files.createTempDirectory("petallink-finder-info-")
        val xattrs = FakeXattrs()
        val manager = JvmPlaceholderManager(root, xattrs)
        val path = Files.writeString(root.resolve("a.txt"), "a")
        val original = ByteArray(32).also { it[0] = 7 }
        xattrs.set(path.toString(), AppConfig.XATTR_FINDER_INFO, original)

        manager.setFinderGreyLabel(path.toString(), true)
        manager.setFinderGreyLabel(path.toString(), false)

        val after = assertNotNull(xattrs.get(path.toString(), AppConfig.XATTR_FINDER_INFO))
        assertEquals(7, after[0])
        assertEquals(0, after[9])
    }

    @Test
    fun 已修改占位符先备份且清理备份state() = runBlocking {
        val root = Files.createTempDirectory("petallink-placeholder-backup-")
        val xattrs = FakeXattrs(moveAware = true)
        val manager = JvmPlaceholderManager(root, xattrs, nowMs = { 1234 })
        manager.createPlaceholderStrict("notes.txt")
        val source = root.resolve("notes.txt")
        Files.writeString(source, "user edit")

        val backupPath = assertNotNull(manager.backupModifiedPlaceholder(source.toString()))
        val backup = java.nio.file.Path.of(backupPath)
        assertFalse(Files.exists(source))
        assertEquals("user edit", Files.readString(backup))
        assertTrue(backup.fileName.toString().startsWith("notes.local-1234"))
        assertNull(xattrs.get(backup.toString(), AppConfig.XATTR_STATE))
    }

    @Test
    fun 严格创建拒绝覆盖和路径逃逸() = runBlocking {
        val root = Files.createTempDirectory("petallink-placeholder-strict-")
        val manager = JvmPlaceholderManager(root, FakeXattrs())
        Files.writeString(root.resolve("user.txt"), "content")
        assertFailsWith<AppError.LocalIo> { manager.createPlaceholderStrict("user.txt") }
        assertFailsWith<AppError.LocalIo> { manager.createPlaceholderStrict("../escape.txt") }
        assertEquals("content", Files.readString(root.resolve("user.txt")))
    }

    @Test
    fun 占位空文件可删除() = runBlocking {
        val root = Files.createTempDirectory("petallink-placeholder-delete-")
        val manager = JvmPlaceholderManager(root, FakeXattrs())
        manager.createPlaceholderStrict("cloud.txt")
        val path = root.resolve("cloud.txt")
        manager.deleteLocal(path.toString())
        assertFalse(Files.exists(path))
    }

    private class FakeXattrs(private val moveAware: Boolean = false) : XattrAccess {
        private val values = mutableMapOf<Pair<String, String>, ByteArray>()
        override fun get(path: String, name: String): ByteArray? {
            values[key(path, name)]?.let { return it.copyOf() }
            if (moveAware) {
                val sameFile = values.keys.firstOrNull { (oldPath, oldName) ->
                    oldName == name && runCatching { Files.isSameFile(java.nio.file.Path.of(oldPath), java.nio.file.Path.of(path)) }.getOrDefault(false)
                }
                sameFile?.let { return values[it]?.copyOf() }
            }
            return null
        }
        override fun set(path: String, name: String, value: ByteArray) {
            values[key(path, name)] = value.copyOf()
        }
        override fun remove(path: String, name: String) {
            values.remove(key(path, name))
            if (moveAware) {
                values.keys.filter { it.second == name && it.first != path }.forEach { key ->
                    if (!Files.exists(java.nio.file.Path.of(key.first))) values.remove(key)
                }
            }
        }

        private fun key(path: String, name: String): Pair<String, String> {
            val normalized = java.nio.file.Path.of(path).toAbsolutePath().normalize()
            val canonical = if (Files.exists(normalized)) normalized.toRealPath() else normalized
            return canonical.toString() to name
        }
    }
}
