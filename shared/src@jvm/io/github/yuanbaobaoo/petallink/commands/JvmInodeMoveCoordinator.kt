package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.mount.LocalFileEntry
import io.github.yuanbaobaoo.petallink.sync.DbBaselineEntry
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache
import io.github.yuanbaobaoo.petallink.sync.identity.DetectedMove
import io.github.yuanbaobaoo.petallink.sync.identity.LocalMoveActionReconciler
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
     *
     * 扫描快照来自周期开始前：周期内的确定性记账（下载安装/释放空间产生的新 inode，
     * scannedAt >= [freshSinceMs]）比扫描更新，既不被扫描回写覆盖、也不参与 purge
     * （对标 transfer_operations.rs:380-381）。
     */
    suspend fun refresh(entries: List<LocalFileEntry>, freshSinceMs: Long = Long.MAX_VALUE) {
        val baselines = db.syncItems.selectAll().associateBy { it.localPath }
        val recordsByPath = db.inodeMap.selectAll().associateBy { it.relativePath }
        val scannedAt = nowMs()
        for (entry in entries) {
            val baseline = baselines[entry.relativePath] ?: continue
            val existing = recordsByPath[entry.relativePath]
            if (existing != null && existing.inode != entry.inode && existing.scannedAt >= freshSinceMs) continue
            db.inodeMap.upsert(entry.inode, entry.relativePath, baseline.fileId, scannedAt)
        }
        val keep = entries.mapTo(mutableSetOf()) { it.inode }
        keep += recordsByPath.values.filter { it.scannedAt >= freshSinceMs }.map { it.inode }
        db.inodeMap.purgeMissing(keep)
    }

    /**
     * 在单个事务中把移动目录及全部后代基线替换为新路径。
     */
    suspend fun settleSubtree(cloud: CloudTreeCache, action: SyncAction) {
        val fileId = action.fileId ?: throw AppError.Data("云端移动缺少 fileId")
        val oldRoot = db.syncItems.findByFileId(fileId)?.localPath
            ?: throw AppError.Data("云端移动缺少旧基线: $fileId")
        val oldPrefix = "$oldRoot/"
        val oldItems = db.syncItems.selectAll().filter {
            it.localPath == oldRoot || it.localPath.startsWith(oldPrefix)
        }
        if (oldItems.isEmpty()) throw AppError.Data("云端移动基线子树为空: $oldRoot")
        val replacements = oldItems.map { item -> replacement(cloud, action, oldRoot, item) }
        db.syncItems.replaceSubtree(oldRoot, replacements)
    }

    /**
     * 根据移动后的路径和可信云树构造基线。
     *
     * 结构性移动只结算结构事实（路径/名称/父目录），内容事实（localMtime/localSize/sha256/
     * isFolder/status/errorMessage/lastSyncTime）必须保留最后实际同步的版本——不能把移动前后
     * 的编辑误认为已同步（对标 results.rs:204-233）；紧随的重扫周期负责上传内容差异。
     */
    private fun replacement(
        cloud: CloudTreeCache,
        action: SyncAction,
        oldRoot: String,
        item: SyncItem,
    ): SyncItem {
        val suffix = item.localPath.removePrefix(oldRoot)
        val newPath = action.relativePath + suffix
        val remote = cloud.tree[newPath] ?: throw AppError.Data("移动后云树缺少路径: $newPath")
        return item.copy(
            localPath = newPath,
            parentFolderId = remote.singleParentOrNull,
            name = remote.name ?: newPath.substringAfterLast('/'),
            size = remote.sizeBytes,
            cloudEditedTime = remote.editedTime?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() }
                ?: item.cloudEditedTime,
        )
    }
}
