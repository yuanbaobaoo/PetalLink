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

class JvmTransferFileStore(
    private val json: Json = Json { ignoreUnknownKeys = false; encodeDefaults = true },
) : TransferFileStore {
    override suspend fun exists(path: String) = withContext(Dispatchers.IO) {
        Files.exists(Path.of(path), LinkOption.NOFOLLOW_LINKS)
    }

    override suspend fun size(path: String) = withContext(Dispatchers.IO) { Files.size(Path.of(path)) }

    override suspend fun snapshot(path: String) = withContext(Dispatchers.IO) {
        val value = Path.of(path)
        LocalSourceSnapshot(Files.size(value), Files.getLastModifiedTime(value, LinkOption.NOFOLLOW_LINKS).toMillis())
    }

    override suspend fun readAll(path: String, maxBytes: Long): ByteArray = withContext(Dispatchers.IO) {
        val value = Path.of(path)
        val length = Files.size(value)
        require(length <= maxBytes) { "文件超过内存上传上限" }
        Files.readAllBytes(value)
    }

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

    override suspend fun sha256(path: String, bufferSize: Int): String = withContext(Dispatchers.IO) {
        sha256File(Path.of(path), bufferSize)
    }

    override suspend fun readResumeMetadata(destination: String): DownloadResumeMetadata? = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.metadataPath(destination))
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) return@withContext null
        runCatching { json.decodeFromString<DownloadResumeMetadata>(Files.readString(path)) }.getOrNull()
    }

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

    override suspend fun deleteResumeMetadata(destination: String) = withContext(Dispatchers.IO) {
        Files.deleteIfExists(Path.of(TransferFileStore.metadataPath(destination)))
        Files.deleteIfExists(Path.of("$destination.download-meta-write.tmp"))
        Unit
    }

    override suspend fun writeTemp(destination: String, offset: Long, bytes: ByteArray, truncate: Boolean) = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.tempPath(destination))
        path.parent?.let(Files::createDirectories)
        RandomAccessFile(path.toFile(), "rw").use { file ->
            if (truncate) file.setLength(0)
            file.seek(offset)
            file.write(bytes)
        }
    }

    override suspend fun tempSize(destination: String): Long? = withContext(Dispatchers.IO) {
        val path = Path.of(TransferFileStore.tempPath(destination))
        if (Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) Files.size(path) else null
    }

    override suspend fun deleteTemp(destination: String) = withContext(Dispatchers.IO) {
        Files.deleteIfExists(Path.of(TransferFileStore.tempPath(destination)))
        Unit
    }

    override suspend fun sha256Temp(destination: String, bufferSize: Int): String = withContext(Dispatchers.IO) {
        sha256File(Path.of(TransferFileStore.tempPath(destination)), bufferSize)
    }

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

    override suspend fun fsyncTemp(destination: String) = withContext(Dispatchers.IO) {
        FileChannel.open(Path.of(TransferFileStore.tempPath(destination)), StandardOpenOption.WRITE).use { it.force(true) }
    }

    override suspend fun installTemp(destination: String) = withContext(Dispatchers.IO) {
        val target = Path.of(destination)
        val temp = Path.of(TransferFileStore.tempPath(destination))
        atomicMove(temp, target)
        fsyncDirectory(target.parent)
    }

    private fun atomicMove(from: Path, to: Path) {
        try {
            Files.move(from, to, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
            Files.move(from, to, StandardCopyOption.REPLACE_EXISTING)
        }
    }

    private fun fsyncDirectory(directory: Path?) {
        if (directory == null) return
        runCatching { FileChannel.open(directory, StandardOpenOption.READ).use { it.force(true) } }
    }
}
