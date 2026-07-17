package io.github.yuanbaobaao.petallink.mount

import io.github.yuanbaobaao.petallink.AppError
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** 64 KiB 流式 SHA-256，以 normalized path + mtime + size 作缓存合同。 */
class JvmFileHasher : FileHasher {
    private val cache = ConcurrentHashMap<String, HashCacheEntry>()
    private val pathLocks = ConcurrentHashMap<String, Mutex>()

    override suspend fun sha256(absolutePath: String): String {
        val path = Path.of(absolutePath).toAbsolutePath().normalize()
        val key = path.toString()
        return pathLocks.computeIfAbsent(key) { Mutex() }.withLock {
            withContext(Dispatchers.IO) {
                try {
                    val before = attributes(path)
                    cache[key]?.takeIf {
                        it.mtime == before.lastModifiedTime().toMillis() && it.size == before.size()
                    }?.let { return@withContext it.sha256 }

                    val digest = MessageDigest.getInstance("SHA-256")
                    Files.newInputStream(path).use { input ->
                        val buffer = ByteArray(BUFFER_SIZE)
                        while (true) {
                            val read = input.read(buffer)
                            if (read < 0) break
                            if (read > 0) digest.update(buffer, 0, read)
                        }
                    }
                    val after = attributes(path)
                    if (before.size() != after.size() ||
                        before.lastModifiedTime() != after.lastModifiedTime() ||
                        before.fileKey() != after.fileKey()
                    ) {
                        throw AppError.LocalIo("文件在哈希期间发生变化: $path")
                    }
                    val hash = digest.digest().joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }
                    cache[key] = HashCacheEntry(after.lastModifiedTime().toMillis(), after.size(), hash)
                    hash
                } catch (error: AppError) {
                    throw error
                } catch (error: Throwable) {
                    throw AppError.LocalIo("计算 SHA-256 失败: $path", error)
                }
            }
        }
    }

    fun invalidate(absolutePath: String) {
        cache.remove(Path.of(absolutePath).toAbsolutePath().normalize().toString())
    }

    fun clear() {
        cache.clear()
        pathLocks.clear()
    }

    private fun attributes(path: Path): BasicFileAttributes {
        if (Files.isSymbolicLink(path)) throw AppError.LocalIo("拒绝哈希符号链接: $path")
        val attrs = Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        if (!attrs.isRegularFile) throw AppError.LocalIo("待哈希路径不是普通文件: $path")
        return attrs
    }

    companion object { const val BUFFER_SIZE = 64 * 1024 }
}
