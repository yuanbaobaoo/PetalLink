package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

/**
 * checkpoint 原子提交的各阶段：tmp 同步、备份就绪、替换完成、父目录同步。
 */
enum class CheckpointCommitStage { TMP_SYNCED, BACKUP_SYNCED, REPLACED, PARENT_SYNCED }

/**
 * JVM/macOS 云树单文件 checkpoint：tmp fsync → bak → rename → parent fsync。
 */
class JvmCloudTreeCheckpointStore(
    private val file: Path,
    private val commitProbe: (CheckpointCommitStage) -> Unit = {},
) : CloudTreeCheckpointStore {
    private val mutex = Mutex()
    private val json = Json { prettyPrint = true; encodeDefaults = true; ignoreUnknownKeys = false }
    private val tmp get() = file.resolveSibling("${file.fileName}.tmp")
    private val backup get() = file.resolveSibling("${file.fileName}.bak")

    /**
     * JVM 实现：先清理残留 tmp，再优先读取主文件、回退到 backup，校验不可信时返回 null。
     */
    override suspend fun loadTrusted(): CloudTreeCache? = mutex.withLock { io {
        Files.deleteIfExists(tmp)
        loadOne(file) ?: loadOne(backup)
    } }

    /**
     * JVM 实现：tmp 写入并 fsync → 硬链接 backup → 原子替换 → 父目录 fsync；
     * 任何阶段失败都尝试回滚到 backup，绝不留下半成品。
     */
    override suspend fun persist(checkpoint: CloudTreeCache): Unit = mutex.withLock { io {
        checkpoint.validateTrusted()
        val parent = file.toAbsolutePath().normalize().parent
            ?: throw AppError.LocalIo("云树 checkpoint 缺少父目录: $file")
        Files.createDirectories(parent)
        Files.deleteIfExists(tmp)
        Files.deleteIfExists(backup)
        val bytes = json.encodeToString(checkpoint).encodeToByteArray()
        FileChannel.open(tmp, StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
            var buffer = ByteBuffer.wrap(bytes)
            while (buffer.hasRemaining()) channel.write(buffer)
            channel.force(true)
        }
        commitProbe(CheckpointCommitStage.TMP_SYNCED)

        val hadPrevious = Files.exists(file)
        try {
            if (hadPrevious) {
                Files.createLink(backup, file)
                syncDirectory(parent)
                commitProbe(CheckpointCommitStage.BACKUP_SYNCED)
            }
            atomicReplace(tmp, file)
            commitProbe(CheckpointCommitStage.REPLACED)
            syncDirectory(parent)
            commitProbe(CheckpointCommitStage.PARENT_SYNCED)
            Files.deleteIfExists(backup)
        } catch (error: Throwable) {
            runCatching {
                if (hadPrevious && Files.exists(backup)) atomicReplace(backup, file)
                else if (!hadPrevious) Files.deleteIfExists(file)
                syncDirectory(parent)
            }
            Files.deleteIfExists(tmp)
            throw AppError.LocalIo("云树 checkpoint 原子提交失败: $file", error)
        }
        Unit
    } }

    /**
     * JVM 实现：删除未提交的 tmp 与 backup 暂存文件。
     */
    override suspend fun discardUncommitted(): Unit = mutex.withLock { io {
        Files.deleteIfExists(tmp)
        Files.deleteIfExists(backup)
        Unit
    } }

    /**
     * 读取并解析单个候选文件，校验通过返回缓存，非普通文件或解析失败返回 null。
     */
    private fun loadOne(candidate: Path): CloudTreeCache? {
        if (!Files.isRegularFile(candidate)) return null
        return runCatching {
            json.decodeFromString<CloudTreeCache>(Files.readString(candidate)).also(CloudTreeCache::validateTrusted)
        }.getOrNull()
    }

    /**
     * 优先用 ATOMIC_MOVE 替换目标，不支持时退化为普通替换移动。
     */
    private fun atomicReplace(source: Path, target: Path) {
        try {
            Files.move(source, target, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(source, target, StandardCopyOption.REPLACE_EXISTING)
        }
    }

    /**
     * 打开目录只读通道并 fsync，确保目录条目变更落盘。
     */
    private fun syncDirectory(directory: Path) {
        FileChannel.open(directory, StandardOpenOption.READ).use { it.force(true) }
    }

    /**
     * 把阻塞 IO 操作切到 Dispatchers.IO 执行。
     */
    private suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }
}
