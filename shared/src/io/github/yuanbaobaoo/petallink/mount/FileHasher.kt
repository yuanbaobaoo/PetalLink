package io.github.yuanbaobaoo.petallink.mount

/**
 * 文件哈希接口（对标 src/mount/file_hasher.rs）。
 *
 * SHA256 流式哈希，64KB buffer，含 mtime/size 缓存。
 * 实现由 macosMain 提供（用平台 crypto）。
 *
 * 详见 docs/10 阶段 3 item 16。
 */
interface FileHasher {
    /**
     * 计算文件 SHA256（小写十六进制）。
     * @param absolutePath 文件绝对路径
     * @return 64 字符十六进制哈希
     */
    suspend fun sha256(absolutePath: String): String
}

/**
 * 哈希缓存项（mtime + size 变化时重新计算）。
 */
data class HashCacheEntry(
    val mtime: Long,
    val size: Long,
    val sha256: String,
)
