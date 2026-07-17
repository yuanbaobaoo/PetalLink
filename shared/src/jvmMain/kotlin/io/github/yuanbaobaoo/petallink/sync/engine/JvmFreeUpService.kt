package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRecord
import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.mount.PlaceholderManager
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.TransferState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.time.Instant
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * 远端释放空间核验快照，用于比对云端副本是否仍存在且大小/版本一致。
 */
data class RemoteFreeUpSnapshot(
    val fileId: String,
    val size: Long,
    val editedTimeMillis: Long?,
    val deletedOrRecycled: Boolean,
)

/**
 * 远端释放核验接口，确认释放前云端副本仍存在且身份一致
 */
fun interface FreeUpRemoteVerifier {
    /**
     * 核验远端 fileId 仍存在且大小/版本一致，返回远端快照。
     */
    suspend fun verify(fileId: String): RemoteFreeUpSnapshot
}

/**
 * 基于 [FilesApi] 的远端释放核验实现：读取远端元数据并确认文件未被删除或回收。
 */
class FilesApiFreeUpVerifier(private val filesApi: FilesApi) : FreeUpRemoteVerifier {
    /**
     * 基于 FilesApi 的核验实现：读取远端文件元数据并确认未被删除或回收。
     */
    override suspend fun verify(fileId: String): RemoteFreeUpSnapshot {
        val file = filesApi.getFile(fileId)
        return RemoteFreeUpSnapshot(
            fileId = file.id ?: "",
            size = file.size?.toLongOrNull() ?: throw AppError.Data("远端 size 缺失或非法"),
            editedTimeMillis = file.editedTime?.let { Instant.parse(it).toEpochMilli() },
            deletedOrRecycled = filesApi.verifyDeleted(fileId),
        )
    }
}

/**
 * 释放空间的 write-ahead staging 实现；任何失败都优先恢复用户真实内容。
 */
class JvmFreeUpService(
    mountRoot: Path,
    private val appPaths: AppPaths,
    private val db: PetalLinkDb,
    private val placeholder: PlaceholderManager,
    private val remote: FreeUpRemoteVerifier,
    private val nowMs: () -> Long = System::currentTimeMillis,
) {
    private val root = mountRoot.toAbsolutePath().normalize()
    private val leases = ConcurrentHashMap<String, Mutex>()

    /**
     * 静态安全检查（不改动本地内容）：本地已同步、可信云树含同 fileId 且非占位符时返回 "safe"，
     * 其余任意异常或条件不满足统一返回 "not_synced"。
     */
    suspend fun checkSafe(relativePath: String, fileId: String): String = try {
        val path = safeRelative(relativePath)
        val baseline = db.syncItems.findByFileId(fileId)
        val checkpoint = checkpointStore().loadTrusted()
        when {
            baseline == null || baseline.localPath != relativePath || baseline.status != SyncStatus.SYNCED -> "not_synced"
            checkpoint?.pathToId?.get(relativePath) != fileId -> "not_in_cloud"
            !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS) -> "not_synced"
            placeholder.isPlaceholder(path.toString()) -> "not_synced"
            else -> "safe"
        }
    } catch (_: Throwable) {
        "not_synced"
    }

    /**
     * 释放单个文件空间：对该路径加租约后执行 write-ahead 流程，返回释放字节数。
     * 失败时优先自动恢复原内容；释放完成后内容转为占位符。
     */
    suspend fun freeOne(relativePath: String, fileId: String, expectedSize: Long): Long {
        require(expectedSize >= 0) { "释放空间 size 不能为负" }
        val lease = leases.computeIfAbsent(relativePath) { Mutex() }
        return try {
            lease.withLock { freeOneOwned(relativePath, fileId, expectedSize) }
        } finally {
            leases.remove(relativePath, lease)
        }
    }

    /**
     * 持租约的实际释放逻辑：本地/云树/远端三重核验通过后用 write-ahead 把真实内容替换为占位符。
     * 任何阶段失败都会尽力将内容回滚到原路径，绝不覆盖原路径上的新用户文件。
     */
    private suspend fun freeOneOwned(relativePath: String, fileId: String, expectedSize: Long): Long {
        val target = safeRelative(relativePath)
        val checkpoint = checkpointStore().loadTrusted()
            ?: throw AppError.Data("云端索引不可信，拒绝释放本地唯一副本")
        checkpoint.validateTrusted()

        val first = localSnapshot(target)
        if (placeholder.isPlaceholder(target.toString())) throw AppError.LocalIo("目标已经是占位符")
        if (first.size != expectedSize) throw AppError.Conflict("待释放文件大小已变化")
        val baseline = requireBaseline(fileId, relativePath, first)
        requireNoActiveTransfer(fileId, relativePath)
        if (checkpoint.pathToId[relativePath] != fileId) throw AppError.Data("可信云树中不存在同一 fileId")

        val verified = remote.verify(fileId)
        if (verified.fileId != fileId || verified.size != expectedSize || verified.deletedOrRecycled ||
            baseline.cloudEditedTime == null || verified.editedTimeMillis != baseline.cloudEditedTime
        ) {
            throw AppError.Conflict("远端副本不存在、已回收、大小或版本与成功基线不一致")
        }

        val second = localSnapshot(target)
        if (second != first) throw AppError.Conflict("远端核验期间本地文件已变化")
        val secondBaseline = requireBaseline(fileId, relativePath, second)
        if (secondBaseline != baseline) throw AppError.Conflict("远端核验期间同步基线已变化")
        requireNoActiveTransfer(fileId, relativePath)

        val staging = allocateStaging(target)
        val stagingRelative = root.relativize(staging).toString()
        val record = FreeUpStagingRecord(
            stagingName = stagingRelative,
            relativePath = relativePath,
            fileId = fileId,
            sourceMtime = first.mtime,
            sourceSize = first.size,
            createdAt = nowMs(),
        )

        // write-ahead：记录先于 rename。崩溃时“记录存在、staging 不存在”可安全判为未开始。
        db.freeUpStaging.insert(record)
        var moved = false
        var baselineCommitted = false
        try {
            fsyncFile(target)
            Files.move(target, staging, StandardCopyOption.ATOMIC_MOVE)
            moved = true
            fsyncDirectory(target.parent)

            placeholder.createPlaceholderStrict(relativePath)
            db.inodeMap.upsert(readInode(target), relativePath, fileId, nowMs())
            baselineCommitted = db.syncItems.casMarkCloudOnly(
                fileId, relativePath, first.mtime, first.size,
            )
            if (!baselineCommitted) throw AppError.Data("释放空间后基线发生并发变化")

            Files.delete(staging)
            fsyncDirectory(staging.parent)
            db.freeUpStaging.deleteByName(stagingRelative)
            return first.size
        } catch (error: Throwable) {
            val restored = runCatching {
                if (moved && Files.exists(staging, LinkOption.NOFOLLOW_LINKS)) {
                    if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) {
                        if (!placeholder.isPlaceholder(target.toString())) {
                            throw AppError.Conflict("原路径出现新的用户文件，旧内容保留在 $staging")
                        }
                        Files.delete(target)
                    }
                    Files.move(staging, target, StandardCopyOption.ATOMIC_MOVE)
                    fsyncDirectory(target.parent)
                    db.inodeMap.upsert(readInode(target), relativePath, fileId, nowMs())
                }
            }
            if (baselineCommitted && restored.isSuccess) {
                db.syncItems.casRollbackCloudOnly(fileId, relativePath, first.mtime, first.size)
            }
            if (restored.isSuccess) db.freeUpStaging.deleteByName(stagingRelative)
            if (restored.isFailure) {
                throw AppError.LocalIo("释放空间失败且自动恢复未完成；旧内容保留于 $staging", restored.exceptionOrNull())
            }
            throw when (error) {
                is AppError -> error
                else -> AppError.LocalIo("释放空间失败，真实内容已恢复", error)
            }
        }
    }

    /**
     * 启动时收敛 write-ahead 记录；绝不覆盖原路径上的新用户文件。
     */
    suspend fun recoverInterrupted(): Int {
        var recovered = 0
        for (record in db.freeUpStaging.findAll()) {
            val target = runCatching { safeRelative(record.relativePath) }.getOrNull() ?: continue
            val staging = runCatching { safeRelative(record.stagingName) }.getOrNull() ?: continue
            val stagingExists = Files.isRegularFile(staging, LinkOption.NOFOLLOW_LINKS)
            if (!stagingExists) {
                db.freeUpStaging.deleteByName(record.stagingName)
                continue
            }
            if (Files.exists(target, LinkOption.NOFOLLOW_LINKS) && !placeholder.isPlaceholder(target.toString())) {
                continue
            }
            if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) Files.delete(target)
            Files.move(staging, target, StandardCopyOption.ATOMIC_MOVE)
            fsyncDirectory(target.parent)
            db.inodeMap.upsert(readInode(target), record.relativePath, record.fileId, nowMs())
            if (record.sourceMtime != null && record.sourceSize != null) {
                db.syncItems.casRollbackCloudOnly(
                    record.fileId, record.relativePath, record.sourceMtime, record.sourceSize,
                )
            }
            db.freeUpStaging.deleteByName(record.stagingName)
            recovered++
        }
        return recovered
    }

    /**
     * 校验存在匹配的成功同步基线，且大小、修改时间与本地快照完全一致。
     */
    private suspend fun requireBaseline(fileId: String, relativePath: String, snapshot: FileSnapshot): SyncItem {
        val baseline = db.syncItems.findByFileId(fileId)
            ?: throw AppError.Data("找不到成功同步基线")
        if (baseline.localPath != relativePath || baseline.status != SyncStatus.SYNCED ||
            baseline.localMtime != snapshot.mtime || baseline.localSize != snapshot.size ||
            baseline.size != snapshot.size
        ) throw AppError.Conflict("本地内容与最后成功同步基线不一致")
        return baseline
    }

    /**
     * 检查同 fileId 或同路径上不存在活动传输任务，否则拒绝释放。
     */
    private suspend fun requireNoActiveTransfer(fileId: String, relativePath: String) {
        val activeStates = listOf(
            TransferState.Pending, TransferState.Running, TransferState.WaitingForNetwork,
            TransferState.BackingOff, TransferState.VerifyingRemote, TransferState.RestartRequired,
        )
        val tasks = mutableListOf<io.github.yuanbaobaoo.petallink.data.TransferTask>()
        for (state in activeStates) tasks += db.transfers.selectByState(state)
        val active = tasks.any {
            it.fileId == fileId || it.relativePath == relativePath || it.localPath == relativePath
        }
        if (active) throw AppError.Conflict("该文件存在活动传输任务")
    }

    /**
     * 把挂载相对路径规范化为绝对路径，拒绝绝对路径、".." 穿越与符号链接父目录。
     */
    private fun safeRelative(relativePath: String): Path {
        val relative = Path.of(relativePath)
        if (relativePath.isBlank() || relative.isAbsolute || relative.any { it.toString() == ".." }) {
            throw AppError.LocalIo("非法挂载相对路径: $relativePath")
        }
        val target = root.resolve(relative).normalize()
        if (!target.startsWith(root)) throw AppError.LocalIo("路径越出挂载目录")
        var current: Path? = target.parent
        while (current != null && current.startsWith(root)) {
            if (Files.isSymbolicLink(current)) throw AppError.LocalIo("路径包含符号链接: $current")
            if (current == root) break
            current = current.parent
        }
        return target
    }

    /**
     * 取本地文件的大小、修改时间和 inode 快照；非普通文件或符号链接均视为非法。
     */
    private fun localSnapshot(path: Path): FileSnapshot {
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(path)) {
            throw AppError.LocalIo("待释放目标不是普通文件")
        }
        return FileSnapshot(
            Files.size(path),
            Files.getLastModifiedTime(path, LinkOption.NOFOLLOW_LINKS).toMillis(),
            readInode(path),
        )
    }

    /**
     * 在目标同目录下分配一个不存在的 staging 文件路径，最多重试 16 次。
     */
    private fun allocateStaging(target: Path): Path {
        repeat(16) {
            val candidate = target.parent.resolve(".hwcloud_freeup-${UUID.randomUUID()}")
            if (!Files.exists(candidate, LinkOption.NOFOLLOW_LINKS)) return candidate
        }
        throw AppError.LocalIo("无法分配释放空间 staging 路径")
    }

    /**
     * 创建指向当前挂载根的 checkpoint 存储实例。
     */
    private fun checkpointStore() = JvmCloudTreeCheckpointStore(appPaths.cloudTreeCheckpoint(root))

    /**
     * 对单个文件执行 fsync，确保数据与元数据落盘。
     */
    private fun fsyncFile(path: Path) {
        FileChannel.open(path, StandardOpenOption.READ).use { it.force(true) }
    }

    /**
     * 对目录执行 fsync，确保目录条目变更落盘。
     */
    private fun fsyncDirectory(path: Path?) {
        if (path == null) return
        FileChannel.open(path, StandardOpenOption.READ).use { it.force(true) }
    }

    /**
     * 读取文件的 unix inode，用于 inodeMap 重映射。
     */
    private fun readInode(path: Path): ULong =
        (Files.getAttribute(path, "unix:ino", LinkOption.NOFOLLOW_LINKS) as Number).toLong().toULong()

    /**
     * 本地文件快照：大小、修改时间与 inode，用于释放前后比对与 inode 重映射。
     */
    private data class FileSnapshot(val size: Long, val mtime: Long, val inode: ULong)
}
