package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.sync.ConflictResolver
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncActionType
import io.github.yuanbaobaoo.petallink.sync.executor.ActionResult
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * JVM 冲突协调器：先保全败方副本，再通过持久传输回调提交胜方。
 */
internal class JvmConflictCoordinator(
    private val root: Path,
    private val placeholder: JvmPlaceholderManager,
    private val executeTransfer: suspend (SyncAction) -> ActionResult,
    private val hasActiveUpload: suspend (String) -> Boolean,
    private val zoneId: ZoneId = ZoneId.systemDefault(),
) {

    /**
     * 解决双端修改冲突；本地胜出时云端内容先下载为副本，云端胜出时先移动本地副本。
     */
    suspend fun execute(action: SyncAction): ActionResult {
        val source = safeLocalPath(action.relativePath)
        val localMtime = Files.getLastModifiedTime(source, LinkOption.NOFOLLOW_LINKS).toMillis()
        val cloudMtime = action.cloudFile?.editedTime?.let { Instant.parse(it).toEpochMilli() }
            ?: throw AppError.Data("冲突文件缺少云端 editedTime: ${action.relativePath}")
        val resolution = ConflictResolver.resolve(localMtime, cloudMtime)
        if (resolution.winner == ConflictResolver.ConflictSide.LOCAL && hasActiveUpload(action.relativePath)) {
            return executeTransfer(action.copy(type = SyncActionType.UPLOAD))
        }
        val backup = allocateBackup(source, resolution.loser, resolution.loserTimestampMs)
        return if (resolution.winner == ConflictResolver.ConflictSide.CLOUD) {
            executeCloudWinner(action, source, backup)
        } else {
            executeLocalWinner(action, backup)
        }
    }

    /**
     * 云端删除但本地有修改时，把任意普通文件移动为本地冲突副本。
     */
    suspend fun backupBeforeCloudDelete(relativePath: String) {
        val source = safeLocalPath(relativePath)
        val modifiedAt = Files.getLastModifiedTime(source, LinkOption.NOFOLLOW_LINKS).toMillis()
        val backup = allocateBackup(source, ConflictResolver.ConflictSide.LOCAL, modifiedAt)
        placeholder.moveToConflictCopy(source.toString(), backup.toString())
    }

    /**
     * 云端胜出：移动本地败方后下载原名，失败且原名未生成时恢复本地内容。
     */
    private suspend fun executeCloudWinner(action: SyncAction, source: Path, backup: Path): ActionResult {
        placeholder.moveToConflictCopy(source.toString(), backup.toString())
        val downloaded = executeTransfer(action.copy(type = SyncActionType.DOWNLOAD))
        if (!downloaded.success && !Files.exists(source, LinkOption.NOFOLLOW_LINKS)) {
            placeholder.restoreConflictCopy(backup.toString(), source.toString())
        }
        return downloaded
    }

    /**
     * 本地胜出：先下载云端败方副本，再更新原云端文件。
     */
    private suspend fun executeLocalWinner(action: SyncAction, backup: Path): ActionResult {
        val cloudCopyPath = root.toRealPath().relativize(backup).joinToString("/")
        val downloaded = executeTransfer(
            action.copy(type = SyncActionType.DOWNLOAD, relativePath = cloudCopyPath),
        )
        if (!downloaded.success) return downloaded
        return executeTransfer(action.copy(type = SyncActionType.UPLOAD))
    }

    /**
     * 用败方时间和侧别分配不覆盖现有文件的副本路径。
     */
    private fun allocateBackup(source: Path, side: ConflictResolver.ConflictSide, timestampMs: Long): Path {
        val timestamp = DateTimeFormatter.ofPattern("yyyy-MM-dd HH-mm-ss")
            .withZone(zoneId)
            .format(Instant.ofEpochMilli(timestampMs))
        for (sequence in 0..ConflictResolver.MAX_SEQUENCE) {
            val name = ConflictResolver.copyName(source.fileName.toString(), side, timestamp, sequence)
            val candidate = source.resolveSibling(name)
            if (!Files.exists(candidate, LinkOption.NOFOLLOW_LINKS)) return candidate
        }
        throw AppError.LocalIo("无法分配冲突副本路径")
    }

    /**
     * 校验相对路径并解析为挂载根内的路径。
     */
    private fun safeLocalPath(relativePath: String): Path {
        val relative = Path.of(relativePath)
        if (relative.isAbsolute || relative.none() || relative.any { it.toString() == ".." || it.toString() == "." }) {
            throw AppError.LocalIo("非法同步路径: $relativePath")
        }
        val canonicalRoot = root.toRealPath()
        val target = canonicalRoot.resolve(relative).normalize()
        if (!target.startsWith(canonicalRoot) || target == canonicalRoot) {
            throw AppError.LocalIo("同步路径越界: $relativePath")
        }
        return target
    }
}
