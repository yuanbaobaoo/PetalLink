package io.github.yuanbaobaoo.petallink.data.repository

import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.Sync_items
import io.github.yuanbaobaoo.petallink.data.Sync_itemsQueries
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.data.TransferPatch
import io.github.yuanbaobaoo.petallink.data.ColumnPatch
import io.github.yuanbaobaoo.petallink.data.RunningTransferPatch
import io.github.yuanbaobaoo.petallink.data.Transfer_queue
import io.github.yuanbaobaoo.petallink.data.Transfer_queueQueries
import io.github.yuanbaobaoo.petallink.data.Free_up_stagingQueries
import io.github.yuanbaobaoo.petallink.data.Local_inode_mapQueries
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.identity.InodeRecord

// SQLDelight 按参数在 SQL 中出现顺序生成方法签名，故调用时须匹配该顺序。

class SyncItemRepositoryImpl(private val queries: Sync_itemsQueries) : SyncItemRepository {
    /**
     * 将数据库行转换为 SyncItem 模型。
     */
    private fun Sync_items.toModel() = SyncItem(
        file_id, local_path, parent_folder_id, name, is_folder != 0L, size, local_size,
        sha256, local_mtime, cloud_edited_time, last_sync_time, status.toInt(), error_message,
    )

    /**
     * 以当前时间戳执行 upsert 写入。
     */
    private fun upsertNow(item: SyncItem) {
        queries.upsertRow(
            fileId = item.fileId, localPath = item.localPath, parentFolderId = item.parentFolderId,
            name = item.name, isFolder = if (item.isFolder) 1L else 0L, size = item.size,
            localSize = item.localSize, sha256 = item.sha256, localMtime = item.localMtime,
            cloudEditedTime = item.cloudEditedTime, lastSyncTime = item.lastSyncTime,
            status = item.status.toLong(), errorMessage = item.errorMessage,
        )
    }

    /**
     * 插入或更新一条同步项记录。
     */
    override suspend fun upsert(item: SyncItem) = upsertNow(item)

    /**
     * 按 fileId 查询单条同步项；命中多行则视为基线歧义并拒绝。
     */
    override suspend fun findByFileId(fileId: String): SyncItem? {
        val rows = queries.selectByFileId(fileId).executeAsList()
        check(rows.size <= 1) { "拒绝歧义基线：fileId=$fileId 命中 ${rows.size} 行" }
        return rows.singleOrNull()?.toModel()
    }

    /**
     * 按本地路径查询单条同步项。
     */
    override suspend fun findByLocalPath(localPath: String) = queries.selectByLocalPath(localPath).executeAsOneOrNull()?.toModel()

    /**
     * 查询全部同步项，过滤华为云临时文件（.hwcloud_ 前缀）。
     */
    override suspend fun selectAll(): List<SyncItem> = queries.selectAll().executeAsList()
        .filterNot { it.local_path.startsWith(".hwcloud_") }
        .map { it.toModel() }

    /**
     * 按本地路径前缀查询子树下的所有同步项。
     */
    override suspend fun selectByFolderPrefix(folderPrefix: String): List<SyncItem> = queries.selectByFolderPrefix(folderPrefix).executeAsList().map { it.toModel() }

    /**
     * 按状态码查询同步项列表。
     */
    override suspend fun selectByStatus(status: Int): List<SyncItem> = queries.selectByStatus(status.toLong()).executeAsList().map { it.toModel() }

    /**
     * 统计同步项总数。
     */
    override suspend fun countAll(): Long = queries.countAll().executeAsOne()

    /**
     * 按状态码统计同步项数量。
     */
    override suspend fun countByStatus(status: Int): Long = queries.countByStatus(status.toLong()).executeAsOne()

    /**
     * 更新指定同步项的状态与错误信息。
     */
    override suspend fun updateStatus(fileId: String, localPath: String, newStatus: Int, errorMsg: String?) =
        queries.updateStatus(newStatus.toLong(), errorMsg, fileId, localPath)

    /**
     * 基于 mtime/size 的 CAS 原子标记为云端独占（仅本地已删除），成功返回 true。
     */
    override suspend fun casMarkCloudOnly(fileId: String, localPath: String, sourceMtime: Long, sourceSize: Long): Boolean =
        queries.transactionWithResult {
            queries.casMarkCloudOnly(fileId, localPath, sourceMtime, sourceSize)
            queries.selectChanges().executeAsOne() == 1L
        }

    /**
     * 基于 mtime/size 的 CAS 原子回滚云端独占标记，成功返回 true。
     */
    override suspend fun casRollbackCloudOnly(fileId: String, localPath: String, sourceMtime: Long, sourceSize: Long): Boolean =
        queries.transactionWithResult {
            queries.casRollbackCloudOnly(sourceSize, fileId, localPath, sourceMtime)
            queries.selectChanges().executeAsOne() == 1L
        }

    /**
     * 按 fileId 删除同步项。
     */
    override suspend fun deleteByFileId(fileId: String) = queries.deleteByFileId(fileId)

    /**
     * 按本地路径删除同步项。
     */
    override suspend fun deleteByLocalPath(localPath: String) = queries.deleteByLocalPath(localPath)

    /**
     * 在事务内删除旧子树下所有记录，并写入给定替换集合。
     */
    override suspend fun replaceSubtree(oldRoot: String, replacements: List<SyncItem>) {
        queries.transaction {
            val prefix = "$oldRoot/"
            queries.selectAll().executeAsList()
                .filter { it.local_path == oldRoot || it.local_path.startsWith(prefix) }
                .forEach { queries.deleteByLocalPath(it.local_path) }
            replacements.forEach(::upsertNow)
        }
    }

    /**
     * 在事务内将指定子树下所有同步项的状态与错误信息统一更新。
     */
    override suspend fun updateSubtreeStatus(root: String, newStatus: Int, errorMsg: String?) {
        queries.transaction {
            val prefix = "$root/"
            queries.selectAll().executeAsList()
                .filter { it.local_path == root || it.local_path.startsWith(prefix) }
                .forEach { queries.updateStatus(newStatus.toLong(), errorMsg, it.file_id, it.local_path) }
        }
    }

    /**
     * 重置所有滞留（运行中异常残留）的同步项状态。
     */
    override suspend fun resetStaleStatuses() = queries.resetStaleStatuses()
}

/**
 * transfer_queue 表的仓库实现：负责上传/下载任务的持久化与状态查询。
 */
class TransferRepositoryImpl(private val queries: Transfer_queueQueries) : TransferRepository {
    /**
     * 将列补丁解析为最终值：Keep 用原值，Set 用新值，Clear 置空。
     */
    private fun <T> ColumnPatch<T>.resolve(current: T?): T? = when (this) {
        ColumnPatch.Keep -> current
        is ColumnPatch.Set -> value
        ColumnPatch.Clear -> null
    }

    /**
     * 将数据库行转换为 TransferTask 模型，枚举越界时回退到默认值。
     */
    private fun Transfer_queue.toModel() = TransferTask(
        id = id,
        direction = TransferDirection.entries.getOrElse(direction.toInt()) { TransferDirection.UPLOAD },
        fileId = file_id,
        localPath = local_path,
        name = name,
        totalSize = total_size,
        transferred = transferred,
        state = TransferState.entries.getOrElse(state.toInt()) { TransferState.Pending },
        errorMessage = error_message,
        createdAt = created_at,
        finishedAt = finished_at,
        serverId = server_id,
        uploadId = upload_id,
        resumeOffset = resume_offset,
        sessionUrl = session_url,
        relativePath = relative_path,
        parentFileId = parent_file_id,
        operation = operation?.toInt(),
        sourceMtime = source_mtime,
        sourceSize = source_size,
        expectedCloudEditedTime = expected_cloud_edited_time,
        attemptCount = attempt_count.toInt(),
        nextRetryAt = next_retry_at,
        errorKind = error_kind?.toInt(),
        remoteResultFileId = remote_result_file_id,
        stateRevision = state_revision,
    )

    /**
     * 插入一条传输任务并返回自增主键 id。
     */
    override suspend fun insert(task: TransferTask): Long {
        return queries.transactionWithResult {
            queries.insertRow(
                direction = task.direction.ordinal.toLong(),
                fileId = task.fileId, localPath = task.localPath, name = task.name,
                totalSize = task.totalSize, transferred = task.transferred,
                state = task.state.ordinal.toLong(), errorMessage = task.errorMessage,
                createdAt = task.createdAt, finishedAt = task.finishedAt,
                serverId = task.serverId, uploadId = task.uploadId, resumeOffset = task.resumeOffset,
                sessionUrl = task.sessionUrl, relativePath = task.relativePath,
                parentFileId = task.parentFileId, operation = task.operation?.toLong(),
                sourceMtime = task.sourceMtime, sourceSize = task.sourceSize,
                expectedCloudEditedTime = task.expectedCloudEditedTime,
                attemptCount = task.attemptCount.toLong(), nextRetryAt = task.nextRetryAt,
                errorKind = task.errorKind?.toLong(), remoteResultFileId = task.remoteResultFileId,
                stateRevision = task.stateRevision,
            )
            queries.selectLastInsertId().executeAsOne()
        }
    }

    /**
     * 按 id 查询单条传输任务。
     */
    override suspend fun findById(id: Long) = queries.selectById(id).executeAsOneOrNull()?.toModel()

    /**
     * 校验状态机与版本号后，以 CAS 原子切换到目标状态并记录尝试次数和错误，终态自动写入 finishedAt。
     */
    override suspend fun casTransitionState(id: Long, expectedRevision: Long, newState: TransferState, attempt: Int, errorMsg: String?): Boolean {
        val current = findById(id) ?: throw StaleRevisionException(id, expectedRevision)
        if (current.stateRevision != expectedRevision) throw StaleRevisionException(id, expectedRevision)
        if (!TransferState.canTransition(current.state, newState)) {
            throw IllegalTransferTransitionException(id, current.state, newState)
        }
        val finishedAt = if (TransferState.isTerminal(newState)) System.currentTimeMillis() else null
        val changed = queries.transactionWithResult {
            queries.casTransitionState(newState.ordinal.toLong(), attempt.toLong(), errorMsg, finishedAt, id, expectedRevision)
            queries.selectChanges().executeAsOne()
        }
        if (changed != 1L) throw StaleRevisionException(id, expectedRevision)
        return true
    }

    /**
     * 校验状态机与版本号后，以 CAS 原子切换状态并应用字段补丁，返回更新后的任务。
     */
    override suspend fun transition(id: Long, expectedRevision: Long, newState: TransferState, patch: TransferPatch): TransferTask {
        val current = findById(id) ?: throw StaleRevisionException(id, expectedRevision)
        if (current.stateRevision != expectedRevision) throw StaleRevisionException(id, expectedRevision)
        if (!TransferState.canTransition(current.state, newState)) {
            throw IllegalTransferTransitionException(id, current.state, newState)
        }
        val changed = queries.transactionWithResult {
            queries.transitionWithPatch(
                newState = newState.ordinal.toLong(),
                errorKind = patch.errorKind.resolve(current.errorKind)?.toLong(),
                errorMessage = patch.errorMessage.resolve(current.errorMessage),
                nextRetryAt = patch.nextRetryAt.resolve(current.nextRetryAt),
                finishedAt = patch.finishedAt.resolve(current.finishedAt),
                remoteResultFileId = patch.remoteResultFileId.resolve(current.remoteResultFileId),
                serverId = patch.serverId.resolve(current.serverId),
                uploadId = patch.uploadId.resolve(current.uploadId),
                sessionUrl = patch.sessionUrl.resolve(current.sessionUrl),
                transferred = patch.transferred ?: current.transferred,
                resumeOffset = patch.resumeOffset ?: current.resumeOffset,
                attemptCount = (patch.attemptCount ?: current.attemptCount).toLong(),
                id = id, expectedRevision = expectedRevision,
            )
            queries.selectChanges().executeAsOne()
        }
        if (changed != 1L) throw StaleRevisionException(id, expectedRevision)
        return findById(id) ?: throw StaleRevisionException(id, expectedRevision + 1)
    }

    /**
     * 以 CAS 原子更新运行中任务的已传输字节数，成功返回 true。
     */
    override suspend fun updateRunningProgress(id: Long, expectedRevision: Long, bytesDone: Long): Boolean {
        return queries.transactionWithResult {
            queries.updateRunningProgress(bytesDone, id, expectedRevision)
            queries.selectChanges().executeAsOne() == 1L
        }
    }

    /**
     * 仅当任务处于 Running 且版本号匹配时，原子更新运行态字段（传输字节、断点、会话信息等）。
     */
    override suspend fun updateRunningTransfer(id: Long, expectedRevision: Long, patch: RunningTransferPatch): Boolean {
        val current = findById(id) ?: return false
        if (current.state != TransferState.Running || current.stateRevision != expectedRevision) return false
        return queries.transactionWithResult {
            queries.updateRunningTransfer(
                transferred = patch.transferred ?: current.transferred,
                resumeOffset = patch.resumeOffset ?: current.resumeOffset,
                serverId = patch.serverId.resolve(current.serverId),
                uploadId = patch.uploadId.resolve(current.uploadId),
                sessionUrl = patch.sessionUrl.resolve(current.sessionUrl),
                id = id,
                expectedRevision = expectedRevision,
            )
            queries.selectChanges().executeAsOne() == 1L
        }
    }

    /**
     * 按状态查询传输任务列表。
     */
    override suspend fun selectByState(state: TransferState) = queries.selectByState(state.ordinal.toLong()).executeAsList().map { it.toModel() }

    /**
     * 查询全部传输任务。
     */
    override suspend fun selectAll() = queries.selectAll().executeAsList().map { it.toModel() }

    /**
     * 仅保留最近 keepCount 条历史记录，清理多余旧数据。
     */
    override suspend fun pruneHistory(keepCount: Int) = queries.pruneHistory(keepCount.toLong())

    /**
     * 按需清除已完成和/或已失败的传输历史。
     */
    override suspend fun clearHistory(includeCompleted: Boolean, includeFailed: Boolean) {
        if (includeCompleted) queries.deleteCompleted()
        if (includeFailed) queries.deleteFailed()
    }

    /**
     * 按状态与方向统计传输任务数量。
     */
    override suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long =
        queries.countByStateAndDirection(state.ordinal.toLong(), direction.toLong()).executeAsOne()

    /**
     * 按状态统计传输任务数量。
     */
    override suspend fun countByState(state: TransferState): Long =
        queries.countByState(state.ordinal.toLong()).executeAsOne()
}

/**
 * local_inode_map 表的仓库实现：维护 inode 到相对路径/fileId 的映射查询。
 */
class InodeMapRepositoryImpl(private val queries: Local_inode_mapQueries) : InodeMapRepository {
    /**
     * 将数据库行转换为 InodeRecord 模型。
     */
    private fun io.github.yuanbaobaoo.petallink.data.Local_inode_map.toModel() =
        InodeRecord(inode.toULong(), relative_path, file_id, scanned_at)

    /**
     * 按 inode 查询映射记录。
     */
    override suspend fun lookup(inode: ULong): InodeRecord? {
        val row = queries.lookupByInode(inode.toLong()).executeAsOneOrNull() ?: return null
        return row.toModel()
    }

    /**
     * 插入或更新 inode 到相对路径、fileId 的映射。
     */
    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long) =
        queries.upsert(inode.toLong(), relativePath, fileId, scannedAt)

    /**
     * 按 inode 删除映射记录。
     */
    override suspend fun delete(inode: ULong) = queries.deleteByInode(inode.toLong())

    /**
     * 查询全部 inode 映射记录。
     */
    override suspend fun selectAll(): List<InodeRecord> = queries.selectAll().executeAsList().map { it.toModel() }

    /**
     * 在事务内清除未出现在本次扫描集合中的 inode 映射；集合为空时清空整表。
     */
    override suspend fun purgeMissing(seenInodes: Set<ULong>) {
        queries.transaction {
            if (seenInodes.isEmpty()) {
                queries.deleteAll()
            } else {
                queries.selectAll().executeAsList().forEach { row ->
                    if (row.inode.toULong() !in seenInodes) queries.deleteByInode(row.inode)
                }
            }
        }
    }
}

/**
 * free_up_staging 表的仓库实现：管理释放空间的写前暂存（WAL）记录的增删查。
 */
class FreeUpStagingRepositoryImpl(private val queries: Free_up_stagingQueries) : FreeUpStagingRepository {
    /**
     * 插入一条释放暂存记录。
     */
    override suspend fun insert(record: FreeUpStagingRecord) =
        queries.insertRow(record.stagingName, record.relativePath, record.fileId, record.sourceMtime, record.sourceSize, record.createdAt)

    /**
     * 按暂存名查询单条释放暂存记录。
     */
    override suspend fun findByName(stagingName: String): FreeUpStagingRecord? {
        val row = queries.selectByName(stagingName).executeAsOneOrNull() ?: return null
        return FreeUpStagingRecord(row.staging_name, row.relative_path, row.file_id, row.source_mtime, row.source_size, row.created_at)
    }

    /**
     * 查询全部释放暂存记录。
     */
    override suspend fun findAll(): List<FreeUpStagingRecord> =
        queries.selectAll().executeAsList().map { FreeUpStagingRecord(it.staging_name, it.relative_path, it.file_id, it.source_mtime, it.source_size, it.created_at) }

    /**
     * 按暂存名删除释放暂存记录。
     */
    override suspend fun deleteByName(stagingName: String) = queries.deleteByName(stagingName)
}
