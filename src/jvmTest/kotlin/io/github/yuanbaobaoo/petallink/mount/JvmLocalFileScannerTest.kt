package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.PlatformInode
import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class JvmLocalFileScannerTest {
    @Test
    fun 递归扫描输出路径inode大小mtime类型和state并统一跳过() = runBlocking {
        val root = Files.createTempDirectory("petallink-scan-")
        val attrs = FakeXattrs()
        Files.createDirectories(root.resolve("docs"))
        Files.writeString(root.resolve("docs/a.txt"), "hello")
        Files.createFile(root.resolve("empty-user.txt"))
        val placeholder = Files.createFile(root.resolve("cloud.txt"))
        attrs.set(placeholder.toString(), AppConfig.XATTR_STATE, "placeholder".encodeToByteArray())
        val downloaded = Files.createFile(root.resolve("downloaded.txt"))
        attrs.set(downloaded.toString(), AppConfig.XATTR_STATE, "downloaded".encodeToByteArray())
        Files.writeString(root.resolve(".hwcloud_internal"), "skip")
        Files.writeString(root.resolve("download.tmp"), "skip")
        Files.createDirectories(root.resolve(".Trash"))
        Files.writeString(root.resolve(".Trash/hidden"), "skip")
        runCatching { Files.createSymbolicLink(root.resolve("outside-link"), root.parent) }

        val entries = JvmLocalFileScanner(root, attrs).scan().associateBy { it.relativePath }

        assertEquals(setOf("cloud.txt", "docs", "docs/a.txt", "downloaded.txt", "empty-user.txt"), entries.keys)
        val regular = assertNotNull(entries["docs/a.txt"])
        assertTrue(regular.inode > 0UL)
        assertEquals(5, regular.size)
        assertTrue(regular.mtime > 0)
        assertFalse(regular.isDirectory)
        assertFalse(regular.isPlaceholder)
        assertTrue(assertNotNull(entries["docs"]).isDirectory)
        assertTrue(assertNotNull(entries["cloud.txt"]).isPlaceholder)
        assertEquals(PlaceholderState.PLACEHOLDER, entries["cloud.txt"]?.placeholderState)
        assertFalse(assertNotNull(entries["empty-user.txt"]).isPlaceholder)
        assertFalse(assertNotNull(entries["downloaded.txt"]).isPlaceholder)
    }

    @Test
    fun rename保持inode而copy产生新inode且delete从扫描消失() = runBlocking {
        val root = Files.createTempDirectory("petallink-inode-")
        val scanner = JvmLocalFileScanner(root, FakeXattrs())
        val source = Files.writeString(root.resolve("source.txt"), "content")
        val sourceInode = scanner.scan().single().inode

        val renamed = Files.move(source, root.resolve("renamed.txt"), StandardCopyOption.ATOMIC_MOVE)
        assertEquals(sourceInode, scanner.scan().single().inode)

        Files.copy(renamed, root.resolve("copy.txt"))
        val afterCopy = scanner.scan().associateBy { it.relativePath }
        assertNotEquals(afterCopy["renamed.txt"]?.inode, afterCopy["copy.txt"]?.inode)

        Files.delete(renamed)
        assertEquals(listOf("copy.txt"), scanner.scan().map { it.relativePath })
    }

    @Test
    fun inode读取失败不降级到hash伪身份() {
        val missing = Path.of("/path/that/does/not/exist/petallink")
        assertFailsWith<AppError.LocalIo> { PlatformInode.readInode(missing.toString()) }
    }

    private class FakeXattrs : XattrAccess {
        private val values = mutableMapOf<Pair<String, String>, ByteArray>()
        override fun get(path: String, name: String): ByteArray? = values[path to name]?.copyOf()
        override fun set(path: String, name: String, value: ByteArray) {
            values[path to name] = value.copyOf()
        }
        override fun remove(path: String, name: String) {
            values.remove(path to name)
        }
    }
}
