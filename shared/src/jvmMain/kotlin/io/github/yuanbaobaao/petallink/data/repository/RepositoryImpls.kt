package io.github.yuanbaobaao.petallink.data.repository

import io.github.yuanbaobaao.petallink.data.SyncItem
import io.github.yuanbaobaao.petallink.data.Sync_items
import io.github.yuanbaobaao.petallink.data.Sync_itemsQueries
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.data.TransferTask
import io.github.yuanbaobaao.petallink.data.Transfer_queue
import io.github.yuanbaobaao.petallink.data.Transfer_queueQueries
import io.github.yuanbaobaao.petallink.data.Free_up_stagingQueries
import io.github.yuanbaobaao.petallink.data.Local_inode_mapQueries
import io.github.yuanbaobaao.petallink.sync.TransferState
import io.github.yuanbaobaao.petallink.sync.identity.InodeRecord

// SQLDelight 按参数在 SQL 中出现顺序生成方法签名，故调用时须匹配该顺序。

class SyncItemRepositoryImpl(private val queries: Sync_itemsQueries) : SyncItemRepository {
    private fun Sync_items.toModel() = SyncItem(
        id, file_id, local_path, parent_file_id, is_folder != 0L,
        size, mtime, etag, sync_status.toInt(), state_revision, last_error,
    )

    override suspend fun insert(item: SyncItem): Long {
        queries.insertRow(
            fileId = item.fileId, localPath = item.localPath, parentFileId = item.parentFileId,
            isFolder = if (item.isFolder) 1L else 0L, size = item.size, mtime = item.mtime,
            etag = item.etag, syncStatus = item.syncStatus.toLong(), stateRevision = item.stateRevision,
        )
        return queries.selectByFileId(item.fileId).executeAsOne().id
    }
    override suspend fun findById(id: Long) = queries.selectById(id).executeAsOneOrNull()?.toModel()
    override suspend fun findByFileId(fileId: String) = queries.selectByFileId(fileId).executeAsOneOrNull()?.toModel()
    override suspend fun findByLocalPath(localPath: String) = queries.selectByLocalPath(localPath).executeAsOneOrNull()?.toModel()
    override suspend fun selectAll(): List<SyncItem> = queries.selectAll().executeAsList().map { it.toModel() }
    override suspend fun selectByFolderPrefix(folderPrefix: String): List<SyncItem> = queries.selectByFolderPrefix(folderPrefix).executeAsList().map { it.toModel() }
    override suspend fun selectByStatus(status: Int): List<SyncItem> = queries.selectByStatus(status.toLong()).executeAsList().map { it.toModel() }
    override suspend fun countAll(): Long = queries.countAll().executeAsOne()
    override suspend fun countByStatus(status: Int): Long = queries.countByStatus(status.toLong()).executeAsOne()
    override suspend fun casUpdateStatus(id: Long, expectedRevision: Long, newStatus: Int, errorMsg: String?): Boolean {
        // SQLDelight 生成顺序: (newStatus, errorMsg, id, expectedRevision)
        queries.casUpdateStatus(newStatus.toLong(), errorMsg, id, expectedRevision)
        val after = queries.selectById(id).executeAsOneOrNull()?.state_revision
        return after != null && after == expectedRevision + 1L
    }
    override suspend fun casUpdateEtag(id: Long, expectedRevision: Long, etag: String): Boolean {
        // SQLDelight 生成顺序: (etag, id, expectedRevision)
        queries.casUpdateEtag(etag, id, expectedRevision)
        val after = queries.selectById(id).executeAsOneOrNull()?.state_revision
        return after != null && after == expectedRevision + 1L
    }
    override suspend fun deleteByFileId(fileId: String) = queries.deleteByFileId(fileId)
}

class TransferRepositoryImpl(private val queries: Transfer_queueQueries) : TransferRepository {
    private fun Transfer_queue.toModel() = TransferTask(
        id, file_id, local_path,
        if (direction == 0L) TransferDirection.UPLOAD else TransferDirection.DOWNLOAD,
        TransferState.entries.getOrElse(state.toInt()) { TransferState.Pending },
        state_revision, attempt.toInt(), bytes_total, bytes_done, error_message, upload_session_url,
    )

    override suspend fun insert(task: TransferTask): Long {
        queries.insertRow(
            fileId = task.fileId, localPath = task.localPath,
            direction = if (task.direction == TransferDirection.UPLOAD) 0L else 1L,
            state = task.state.ordinal.toLong(), stateRevision = task.stateRevision,
            attempt = task.attempt.toLong(), bytesTotal = task.bytesTotal, bytesDone = task.bytesDone,
            errorMessage = task.errorMessage, uploadSessionUrl = task.uploadSessionUrl,
            createdAt = System.currentTimeMillis(), updatedAt = System.currentTimeMillis(),
        )
        return queries.selectByFileId(task.fileId).executeAsOne().id
    }
    override suspend fun findById(id: Long) = queries.selectById(id).executeAsOneOrNull()?.toModel()
    override suspend fun casTransitionState(id: Long, expectedRevision: Long, newState: TransferState, attempt: Int, errorMsg: String?): Boolean {
        // SQLDelight 生成顺序: (newState, attempt, errorMsg, updatedAt, id, expectedRevision)
        queries.casTransitionState(newState.ordinal.toLong(), attempt.toLong(), errorMsg, System.currentTimeMillis(), id, expectedRevision)
        val after = queries.selectById(id).executeAsOneOrNull()?.state_revision
        return after != null && after == expectedRevision + 1L
    }
    override suspend fun updateRunningProgress(id: Long, bytesDone: Long) {
        // SQLDelight 生成顺序: (bytesDone, updatedAt, id)
        queries.updateRunningProgress(bytesDone, System.currentTimeMillis(), id)
    }
    override suspend fun selectByState(state: TransferState) = queries.selectByState(state.ordinal.toLong()).executeAsList().map { it.toModel() }
    override suspend fun pruneHistory(keepCount: Int) = queries.pruneHistory(keepCount.toLong())
    override suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long =
        queries.countByStateAndDirection(state.ordinal.toLong(), direction.toLong()).executeAsOne()
    override suspend fun countByState(state: TransferState): Long =
        queries.countByState(state.ordinal.toLong()).executeAsOne()
}

class InodeMapRepositoryImpl(private val queries: Local_inode_mapQueries) : InodeMapRepository {
    override suspend fun lookup(inode: ULong): InodeRecord? {
        val row = queries.lookupByInode(inode.toLong()).executeAsOneOrNull() ?: return null
        return InodeRecord(row.inode.toULong(), row.relative_path, row.file_id, row.scanned_at)
    }
    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long) =
        queries.upsert(inode.toLong(), relativePath, fileId, scannedAt)
    override suspend fun delete(inode: ULong) = queries.deleteByInode(inode.toLong())
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
