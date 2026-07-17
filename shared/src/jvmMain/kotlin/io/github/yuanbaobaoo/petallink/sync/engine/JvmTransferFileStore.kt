package io.github.yuanbaobaoo.petallink.sync.engine

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.security.MessageDigest

/**
 * 传输文件存储的 JVM 实现，基于 NIO 文件 API 提供快照、分片读写、SHA-256、断点续传与原子安装。
 */
class JvmTransferFileStore(
    private val json: Json = Json { ignoreUnknownKeys = false; encodeDefaults = true },
) : TransferFileStore {
    /**
     * JVM 实现：用 Files.exists 不跟随符号链接判断文件存在性。
     */
    override suspend fun exists(path: String) = withContext(Dispatchers.IO) {
        Files.exists(Path.of(path), LinkOption.NOFOLLOW_LINKS)
    }

    /**
     * JVM 实现：用 Files.size 读取字节大小。
     */
    override suspend fun size(path: String) = withContext(Dispatchers.IO) { Files.size(Path.of(path)) }

    /**
     * JVM 实现：取文件大小与最后修改时间构造源文件快照。
     */
    override suspend fun snapshot(path: String) = withContext(Dispatchers.IO) {
        val value = Path.of(path)
        LocalSourceSnapshot(Files.size(value), Files.getLastModifiedTime(value, LinkOption.NOFOLLOW_LINKS).toMillis())
    }

    /**
     * JVM 实现：校验未越限后用 Files.readAllBytes 一次性读入内存。
     */
    override suspend fun readAll(path: String, maxBytes: Long): ByteArray = withContext(Dispatchers.IO) {
        val value = Path.of(path)
        val length = Files.size(value)
        require(length <= maxBytes) { "文件超过内存上传上限" }
        Files.readAllBytes(value)
    }

    /**
     * JVM 实现：用 FileChannel 定位 offset 后读入缓冲区返回分片。
     */
    override suspend fun readRange(path: String, offset: Long, maxBytes: Int): ByteArray = withContext(Dispatchers.IO) {
        require(offset >= 0 && maxBytes > 0)
        FileChannel.open(Path.of(path), StandardOpenOption.READ).use { channel ->
            channel.position(offset)
            val buffer = ByteBuffer.allocate(maxBytes)
            while (buffer.hasRemaining() && channel.read(buffer) > 0) Unit
            buffer.flip()
            ByteArray(buffer.remaining()).also(buffer::get)
        }
    }

    /**
     * JVM 实现：委托 sha256File 以 MessageDigest 流式计算。
     */
    override suspend fun sha256(path: String, bufferSize: Int): String = withContext(Dispatchers.IO) {
        sha256File(Path.of(path), bufferSize)
    }

    /**
     * JVM 实现：读取并解析续传元数据文件，解析失败返回 null。
     */
    override suspend fun readResumeMetadata(destination: String): DownloadResumeMetadata? = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.metadataPath(destination))
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) return@withContext null
        runCatching { json.decodeFromString<DownloadResumeMetadata>(Files.readString(path)) }.getOrNull()
    }

    /**
     * JVM 实现：写到暂存文件、fsync 后同目录原子替换并同步父目录。
     */
    override suspend fun writeResumeMetadata(destination: String, metadata: DownloadResumeMetadata) = withContext(Dispatchers.IO) {
        val target = Path.of(TransferFileStore.metadataPath(destination))
        target.parent?.let(Files::createDirectories)
        val staging = Path.of("$destination.download-meta-write.tmp")
        FileChannel.open(
            staging,
            StandardOpenOption.CREATE,
            StandardOpenOption.TRUNCATE_EXISTING,
            StandardOpenOption.WRITE,
        ).use { channel ->
            val bytes = json.encodeToString(DownloadResumeMetadata.serializer(), metadata).encodeToByteArray()
            var buffer = ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) channel.write(buffer)
            channel.force(true)
        }
        atomicMove(staging, target)
        fsyncDirectory(target.parent)
    }

    /**
     * JVM 实现：删除元数据文件及其写入暂存文件。
     */
    override suspend fun deleteResumeMetadata(destination: String) = withContext(Dispatchers.IO) {
        Files.deleteIfExists(Path.of(TransferFileStore.metadataPath(destination)))
        Files.deleteIfExists(Path.of("$destination.download-meta-write.tmp"))
        Unit
    }

    /**
     * JVM 实现：用 RandomAccessFile 在 offset 处写分片，按需截断文件。
     */
    override suspend fun writeTemp(destination: String, offset: Long, bytes: ByteArray, truncate: Boolean) = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.tempPath(destination))
        path.parent?.let(Files::createDirectories)
        RandomAccessFile(path.toFile(), "rw").use { file ->
            if (truncate) file.setLength(0)
            file.seek(offset)
            file.write(bytes)
        }
    }

    /**
     * JVM 实现：返回暂存文件大小，非普通文件时返回 null。
     */
    override suspend fun tempSize(destination: String): Long? = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.tempPath(destination))
        if (Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) Files.size(path) else null
    }

    /**
     * JVM 实现：删除暂存文件，不存在视为成功。
     */
    override suspend fun deleteTemp(destination: String) = withContext(Dispatchers.IO) {
        Files.deleteIfExists(Path.of(TransferFileStore.tempPath(destination)))
        Unit
    }

    /**
     * JVM 实现：委托 sha256File 对暂存文件计算摘要。
     */
    override suspend fun sha256Temp(destination: String, bufferSize: Int): String = withContext(Dispatchers.IO) {
        sha256File(Path.of(TransferFileStore.tempPath(destination)), bufferSize)
    }

    /**
     * 用流式 MessageDigest 计算文件 SHA-256，返回十六进制字符串。
     */
    private fun sha256File(path: Path, bufferSize: Int): String {
        val digest = MessageDigest.getInstance("SHA-256")
        Files.newInputStream(path).buffered(bufferSize).use { input ->
            val buffer = ByteArray(bufferSize)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (count > 0) digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /**
     * JVM 实现：用 FileChannel.force(true) 将暂存文件数据与元数据落盘。
     */
    override suspend fun fsyncTemp(destination: String) = withContext(Dispatchers.IO) {
        FileChannel.open(Path.of(TransferFileStore.tempPath(destination)), StandardOpenOption.WRITE).use { it.force(true) }
    }

    /**
     * JVM 实现：原子将暂存文件替换到目标路径并同步父目录。
     */
    override suspend fun installTemp(destination: String) = withContext(Dispatchers.IO) {
        val target = Path.of(destination)
        val temp = Path.of(TransferFileStore.tempPath(destination))
        atomicMove(temp, target)
        fsyncDirectory(target.parent)
    }

    /**
     * 优先使用 ATOMIC_MOVE 替换式移动；不支持原子移动时退化为普通替换移动。
     */
    private fun atomicMove(from: Path, to: Path) {
        try {
            Files.move(from, to, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
            Files.move(from, to, StandardCopyOption.REPLACE_EXISTING)
        }
    }

    /**
     * 打开目录的只读通道并 fsync，确保目录条目变更落盘；失败被忽略。
     */
    private fun fsyncDirectory(directory: Path?) {
        if (directory == null) return
        runCatching { FileChannel.open(directory, StandardOpenOption.READ).use { it.force(true) } }
    }
}
