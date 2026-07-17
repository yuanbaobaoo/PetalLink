package io.github.yuanbaobaao.petallink.mount

import io.github.yuanbaobaao.petallink.AppError
import java.nio.file.Files
import java.security.MessageDigest
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotEquals

class JvmFileHasherTest {
    @Test
    fun 使用64KiB流式哈希并在mtime或size变化后重算() = runBlocking {
        val file = Files.createTempFile("petallink-hasher-", ".bin")
        val bytes = ByteArray(JvmFileHasher.BUFFER_SIZE * 2 + 17) { (it % 251).toByte() }
        Files.write(file, bytes)
        val hasher = JvmFileHasher()
        val expected = MessageDigest.getInstance("SHA-256").digest(bytes)
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }
        assertEquals(expected, hasher.sha256(file.toString()))
        assertEquals(expected, hasher.sha256(file.toString()))

        Files.writeString(file, "changed")
        assertNotEquals(expected, hasher.sha256(file.toString()))
    }

    @Test
    fun 拒绝哈希符号链接() = runBlocking {
        val dir = Files.createTempDirectory("petallink-hasher-link-")
        val target = Files.writeString(dir.resolve("target"), "content")
        val link = dir.resolve("link")
        Files.createSymbolicLink(link, target)
        assertFailsWith<AppError.LocalIo> { JvmFileHasher().sha256(link.toString()) }
        Unit
    }
}
