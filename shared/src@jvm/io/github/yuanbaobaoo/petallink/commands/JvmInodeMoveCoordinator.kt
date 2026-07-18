package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.mount.LocalFileEntry
import io.github.yuanbaobaoo.petallink.sync.DbBaselineEntry
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache
import io.github.yuanbaobaoo.petallink.sync.identity.DetectedMove
import io.github.yuanbaobaoo.petallink.sync.identity.LocalMoveActionReconciler
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.time.Instant

/**
 * JVM inode 身份协调器：识别可信移动，并在远端提交后事务结算路径子树和 inode 映射。
 */
internal class JvmInodeMoveCoordinator(
    private val db: PetalLinkDb,
    private val nowMs: () -> Long = System::currentTimeMillis,
) {

    /**
     * 根据持久 inode 映射识别本地重命名，仅接受可由基线和可信云树共同印证的记录。
     */
    suspend fun detect(
        entries: List<LocalFileEntry>,
        baselines: Map<String, DbBaselineEntry>,
        cloud: CloudTreeCache,
    ): List<DetectedMove> {
        val records = db.inodeMap.selectAll().associateBy { it.inode }
        val detected = entries.mapNotNull { entry ->
            val old = records[entry.inode] ?: return@mapNotNull null
            if (old.relativePath == entry.relativePath) return@mapNotNull null
            val baseline = baselines[old.relativePath] ?: return@mapNotNull null
            val remote = cloud.tree[old.relativePath] ?: return@mapNotNull null
            if (baseline.fileId != old.fileId || remote.id != old.fileId) return@mapNotNull null
            DetectedMove(entry.inode, old.fileId, old.relativePath, entry.relativePath)
        }
        return LocalMoveActionReconciler.collapseNested(detected)
    }

    /**
     * 在动作与基线都提交后刷新 inode 身份映射，并清理本轮未见记录。
     */
    suspend fun refresh(entries: List<LocalFileEntry>) {
        val baselines = db.syncItems.selectAll().associateBy { it.localPath }
        val scannedAt = nowMs()
        for (entry in entries) {
            val baseline = baselines[entry.relativePath] ?: continue
            db.inodeMap.upsert(entry.inode, entry.relativePath, baseline.fileId, scannedAt)
        }
        db.inodeMap.purgeMissing(entries.mapTo(mutableSetOf()) { it.inode })
    }

    /**
     * 在单个事务中把移动目录及全部后代基线替换为新路径。
     */
    suspend fun settleSubtree(root: Path, cloud: CloudTreeCache, action: SyncAction) {
        val fileId = action.fileId ?: throw AppError.Data("云端移动缺少 fileId")
        val oldRoot = db.syncItems.findByFileId(fileId)?.localPath
            ?: throw AppError.Data("云端移动缺少旧基线: $fileId")
        val oldPrefix = "$oldRoot/"
        val oldItems = db.syncItems.selectAll().filter {
            it.localPath == oldRoot || it.localPath.startsWith(oldPrefix)
        }
        if (oldItems.isEmpty()) throw AppError.Data("云端移动基线子树为空: $oldRoot")
        val replacements = oldItems.map { item -> replacement(root, cloud, action, oldRoot, item) }
        db.syncItems.replaceSubtree(oldRoot, replacements)
    }

    /**
     * 根据移动后的本地路径和可信云树构造一条成功基线。
     */
    private fun replacement(
        root: Path,
        cloud: CloudTreeCache,
        action: SyncAction,
        oldRoot: String,
        item: SyncItem,
    ): SyncItem {
        val suffix = item.localPath.removePrefix(oldRoot)
        val newPath = action.relativePath + suffix
        val remote = cloud.tree[newPath] ?: throw AppError.Data("移动后云树缺少路径: $newPath")
        val localPath = safeLocalPath(root, newPath)
        val exists = Files.exists(localPath, LinkOption.NOFOLLOW_LINKS)
        return item.copy(
            localPath = newPath,
            parentFolderId = remote.singleParentOrNull,
            name = remote.name ?: newPath.substringAfterLast('/'),
            isFolder = Files.isDirectory(localPath, LinkOption.NOFOLLOW_LINKS),
            size = remote.sizeBytes,
            localSize = if (exists && Files.isRegularFile(localPath, LinkOption.NOFOLLOW_LINKS)) {
                Files.size(localPath)
            } else null,
            sha256 = remote.contentHash,
            localMtime = if (exists) Files.getLastModifiedTime(localPath, LinkOption.NOFOLLOW_LINKS).toMillis() else null,
            cloudEditedTime = remote.editedTime?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() },
            lastSyncTime = nowMs(),
            status = SyncStatus.SYNCED,
            errorMessage = null,
        )
    }

    /**
     * 校验相对路径并解析为挂载根内的路径。
     */
    private fun safeLocalPath(root: Path, relativePath: String): Path {
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
