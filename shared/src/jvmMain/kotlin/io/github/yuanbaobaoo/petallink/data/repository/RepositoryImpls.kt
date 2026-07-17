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
    private fun Sync_items.toModel() = SyncItem(
        file_id, local_path, parent_folder_id, name, is_folder != 0L, size, local_size,
        sha256, local_mtime, cloud_edited_time, last_sync_time, status.toInt(), error_message,
    )

    private fun upsertNow(item: SyncItem) {
        queries.upsertRow(
            fileId = item.fileId, localPath = item.localPath, parentFolderId = item.parentFolderId,
            name = item.name, isFolder = if (item.isFolder) 1L else 0L, size = item.size,
            localSize = item.localSize, sha256 = item.sha256, localMtime = item.localMtime,
            cloudEditedTime = item.cloudEditedTime, lastSyncTime = item.lastSyncTime,
            status = item.status.toLong(), errorMessage = item.errorMessage,
        )
    }
    override suspend fun upsert(item: SyncItem) = upsertNow(item)
    override suspend fun findByFileId(fileId: String): SyncItem? {
        val rows = queries.selectByFileId(fileId).executeAsList()
        check(rows.size <= 1) { "拒绝歧义基线：fileId=$fileId 命中 ${rows.size} 行" }
        return rows.singleOrNull()?.toModel()
    }
    override suspend fun findByLocalPath(localPath: String) = queries.selectByLocalPath(localPath).executeAsOneOrNull()?.toModel()
    override suspend fun selectAll(): List<SyncItem> = queries.selectAll().executeAsList()
        .filterNot { it.local_path.startsWith(".hwcloud_") }
        .map { it.toModel() }
    override suspend fun selectByFolderPrefix(folderPrefix: String): List<SyncItem> = queries.selectByFolderPrefix(folderPrefix).executeAsList().map { it.toModel() }
    override suspend fun selectByStatus(status: Int): List<SyncItem> = queries.selectByStatus(status.toLong()).executeAsList().map { it.toModel() }
    override suspend fun countAll(): Long = queries.countAll().executeAsOne()
    override suspend fun countByStatus(status: Int): Long = queries.countByStatus(status.toLong()).executeAsOne()
    override suspend fun updateStatus(fileId: String, localPath: String, newStatus: Int, errorMsg: String?) =
        queries.updateStatus(newStatus.toLong(), errorMsg, fileId, localPath)
    override suspend fun casMarkCloudOnly(fileId: String, localPath: String, sourceMtime: Long, sourceSize: Long): Boolean =
        queries.transactionWithResult {
            queries.casMarkCloudOnly(fileId, localPath, sourceMtime, sourceSize)
            queries.selectChanges().executeAsOne() == 1L
        }
    override suspend fun casRollbackCloudOnly(fileId: String, localPath: String, sourceMtime: Long, sourceSize: Long): Boolean =
        queries.transactionWithResult {
            queries.casRollbackCloudOnly(sourceSize, fileId, localPath, sourceMtime)
            queries.selectChanges().executeAsOne() == 1L
        }
    override suspend fun deleteByFileId(fileId: String) = queries.deleteByFileId(fileId)
    override suspend fun deleteByLocalPath(localPath: String) = queries.deleteByLocalPath(localPath)
    override suspend fun replaceSubtree(oldRoot: String, replacements: List<SyncItem>) {
        queries.transaction {
            val prefix = "$oldRoot/"
            queries.selectAll().executeAsList()
                .filter { it.local_path == oldRoot || it.local_path.startsWith(prefix) }
                .forEach { queries.deleteByLocalPath(it.local_path) }
            replacements.forEach(::upsertNow)
        }
    }
    override suspend fun updateSubtreeStatus(root: String, newStatus: Int, errorMsg: String?) {
        queries.transaction {
            val prefix = "$root/"
            queries.selectAll().executeAsList()
                .filter { it.local_path == root || it.local_path.startsWith(prefix) }
                .forEach { queries.updateStatus(newStatus.toLong(), errorMsg, it.file_id, it.local_path) }
        }
    }
    override suspend fun resetStaleStatuses() = queries.resetStaleStatuses()
}

class TransferRepositoryImpl(private val queries: Transfer_queueQueries) : TransferRepository {
    private fun <T> ColumnPatch<T>.resolve(current: T?): T? = when (this) {
        ColumnPatch.Keep -> current
        is ColumnPatch.Set -> value
        ColumnPatch.Clear -> null
    }

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
    override suspend fun findById(id: Long) = queries.selectById(id).executeAsOneOrNull()?.toModel()
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
    override suspend fun updateRunningProgress(id: Long, expectedRevision: Long, bytesDone: Long): Boolean {
        return queries.transactionWithResult {
            queries.updateRunningProgress(bytesDone, id, expectedRevision)
            queries.selectChanges().executeAsOne() == 1L
        }
    }
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
    override suspend fun selectByState(state: TransferState) = queries.selectByState(state.ordinal.toLong()).executeAsList().map { it.toModel() }
    override suspend fun selectAll() = queries.selectAll().executeAsList().map { it.toModel() }
    override suspend fun pruneHistory(keepCount: Int) = queries.pruneHistory(keepCount.toLong())
    override suspend fun clearHistory(includeCompleted: Boolean, includeFailed: Boolean) {
        if (includeCompleted) queries.deleteCompleted()
        if (includeFailed) queries.deleteFailed()
    }
    override suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long =
        queries.countByStateAndDirection(state.ordinal.toLong(), direction.toLong()).executeAsOne()
    override suspend fun countByState(state: TransferState): Long =
        queries.countByState(state.ordinal.toLong()).executeAsOne()
}

class InodeMapRepositoryImpl(private val queries: Local_inode_mapQueries) : InodeMapRepository {
    private fun io.github.yuanbaobaoo.petallink.data.Local_inode_map.toModel() =
        InodeRecord(inode.toULong(), relative_path, file_id, scanned_at)

    override suspend fun lookup(inode: ULong): InodeRecord? {
        val row = queries.lookupByInode(inode.toLong()).executeAsOneOrNull() ?: return null
        return row.toModel()
    }
    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long) =
        queries.upsert(inode.toLong(), relativePath, fileId, scannedAt)
    override suspend fun delete(inode: ULong) = queries.deleteByInode(inode.toLong())
    override suspend fun selectAll(): List<InodeRecord> = queries.selectAll().executeAsList().map { it.toModel() }
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

class FreeUpStagingRepositoryImpl(private val queries: Free_up_stagingQueries) : FreeUpStagingRepository {
    override suspend fun insert(record: FreeUpStagingRecord) =
        queries.insertRow(record.stagingName, record.relativePath, record.fileId, record.sourceMtime, record.sourceSize, record.createdAt)
    override suspend fun findByName(stagingName: String): FreeUpStagingRecord? {
        val row = queries.selectByName(stagingName).executeAsOneOrNull() ?: return null
        return FreeUpStagingRecord(row.staging_name, row.relative_path, row.file_id, row.source_mtime, row.source_size, row.created_at)
    }
    override suspend fun findAll(): List<FreeUpStagingRecord> =
        queries.selectAll().executeAsList().map { FreeUpStagingRecord(it.staging_name, it.relative_path, it.file_id, it.source_mtime, it.source_size, it.created_at) }
    override suspend fun deleteByName(stagingName: String) = queries.deleteByName(stagingName)
}
