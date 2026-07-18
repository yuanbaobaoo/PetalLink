package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRecord
import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRepository
import io.github.yuanbaobaoo.petallink.data.repository.IllegalTransferTransitionException
import io.github.yuanbaobaoo.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaoo.petallink.data.repository.StaleRevisionException
import io.github.yuanbaobaoo.petallink.data.repository.SyncItemRepository
import io.github.yuanbaobaoo.petallink.data.repository.TransferRepository
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.identity.InodeRecord

/**
 * 基于 Room DAO 的同步基线仓库。
 */
internal class RoomSyncItemRepository(
    private val dao: SyncItemDao,
) : SyncItemRepository {
    override suspend fun upsert(item: SyncItem) = dao.upsert(item)

    override suspend fun findByFileId(fileId: String): SyncItem? = dao.findByFileId(fileId)

    override suspend fun findByLocalPath(localPath: String): SyncItem? = dao.findByLocalPath(localPath)

    override suspend fun updateStatus(fileId: String, localPath: String, newStatus: Int, errorMsg: String?) {
        dao.updateStatus(fileId, localPath, newStatus, errorMsg)
    }

    override suspend fun casMarkCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Boolean = dao.markCloudOnly(fileId, localPath, sourceMtime, sourceSize) == 1

    override suspend fun casRollbackCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Boolean = dao.rollbackCloudOnly(fileId, localPath, sourceMtime, sourceSize) == 1

    override suspend fun deleteByFileId(fileId: String) = dao.deleteByFileId(fileId)

    override suspend fun deleteByLocalPath(localPath: String) = dao.deleteByLocalPath(localPath)

    override suspend fun replaceSubtree(oldRoot: String, replacements: List<SyncItem>) =
        dao.replaceSubtree(oldRoot, replacements)

    override suspend fun updateSubtreeStatus(root: String, newStatus: Int, errorMsg: String?) =
        dao.updateSubtreeStatus(root, newStatus, errorMsg)

    override suspend fun resetStaleStatuses() = dao.resetStaleStatuses()

    override suspend fun selectAll(): List<SyncItem> =
        dao.selectAll().filterNot { it.localPath.startsWith(".hwcloud_") }

    override suspend fun selectByFolderPrefix(folderPrefix: String): List<SyncItem> =
        dao.selectByFolderPrefix(folderPrefix)

    override suspend fun selectByStatus(status: Int): List<SyncItem> = dao.selectByStatus(status)

    override suspend fun countAll(): Long = dao.countAll()

    override suspend fun countByStatus(status: Int): Long = dao.countByStatus(status)
}

/**
 * 基于 Room DAO 的持久化传输仓库。
 */
internal class RoomTransferRepository(
    private val dao: TransferTaskDao,
) : TransferRepository {
    private fun <T> ColumnPatch<T>.resolve(current: T?): T? = when (this) {
        ColumnPatch.Keep -> current
        is ColumnPatch.Set -> value
        ColumnPatch.Clear -> null
    }

    override suspend fun insert(task: TransferTask): Long = dao.insert(task)

    override suspend fun findById(id: Long): TransferTask? = dao.findById(id)

    override suspend fun casTransitionState(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        attempt: Int,
        errorMsg: String?,
    ): Boolean {
        val current = findById(id) ?: throw StaleRevisionException(id, expectedRevision)
        validateTransition(current, expectedRevision, newState)
        val finishedAt = if (TransferState.isTerminal(newState)) databaseCurrentTimeMillis() else null
        val changed = dao.transitionState(
            id = id,
            expectedRevision = expectedRevision,
            newState = newState,
            attemptCount = attempt,
            errorMessage = errorMsg,
            finishedAt = finishedAt,
        )
        if (changed != 1) throw StaleRevisionException(id, expectedRevision)
        return true
    }

    override suspend fun transition(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        patch: TransferPatch,
    ): TransferTask {
        val current = findById(id) ?: throw StaleRevisionException(id, expectedRevision)
        validateTransition(current, expectedRevision, newState)
        val changed = dao.transitionWithPatch(
            id = id,
            expectedRevision = expectedRevision,
            newState = newState,
            errorKind = patch.errorKind.resolve(current.errorKind),
            errorMessage = patch.errorMessage.resolve(current.errorMessage),
            nextRetryAt = patch.nextRetryAt.resolve(current.nextRetryAt),
            finishedAt = patch.finishedAt.resolve(current.finishedAt),
            remoteResultFileId = patch.remoteResultFileId.resolve(current.remoteResultFileId),
            serverId = patch.serverId.resolve(current.serverId),
            uploadId = patch.uploadId.resolve(current.uploadId),
            sessionUrl = patch.sessionUrl.resolve(current.sessionUrl),
            transferred = patch.transferred ?: current.transferred,
            resumeOffset = patch.resumeOffset ?: current.resumeOffset,
            attemptCount = patch.attemptCount ?: current.attemptCount,
        )
        if (changed != 1) throw StaleRevisionException(id, expectedRevision)
        return findById(id) ?: throw StaleRevisionException(id, expectedRevision + 1)
    }

    override suspend fun updateRunningProgress(id: Long, expectedRevision: Long, bytesDone: Long): Boolean =
        dao.updateRunningProgress(id, expectedRevision, bytesDone, TransferState.Running) == 1

    override suspend fun updateRunningTransfer(
        id: Long,
        expectedRevision: Long,
        patch: RunningTransferPatch,
    ): Boolean {
        val current = findById(id) ?: return false
        if (current.state != TransferState.Running || current.stateRevision != expectedRevision) return false
        return dao.updateRunningTransfer(
            id = id,
            expectedRevision = expectedRevision,
            transferred = patch.transferred ?: current.transferred,
            resumeOffset = patch.resumeOffset ?: current.resumeOffset,
            serverId = patch.serverId.resolve(current.serverId),
            uploadId = patch.uploadId.resolve(current.uploadId),
            sessionUrl = patch.sessionUrl.resolve(current.sessionUrl),
            runningState = TransferState.Running,
        ) == 1
    }

    override suspend fun selectByState(state: TransferState): List<TransferTask> = dao.selectByState(state)

    override suspend fun selectAll(): List<TransferTask> = dao.selectAll()

    override suspend fun pruneHistory(keepCount: Int) {
        dao.pruneHistory(
            keepCount = keepCount,
            completed = TransferState.Completed,
            failed = TransferState.Failed,
            canceled = TransferState.Canceled,
        )
    }

    override suspend fun clearHistory(includeCompleted: Boolean, includeFailed: Boolean) {
        if (includeCompleted) dao.deleteByState(TransferState.Completed)
        if (includeFailed) dao.deleteByState(TransferState.Failed)
    }

    override suspend fun countByStateAndDirection(state: TransferState, direction: Int): Long {
        val transferDirection = TransferDirection.entries.getOrNull(direction) ?: return 0
        return dao.countByStateAndDirection(state, transferDirection)
    }

    override suspend fun countByState(state: TransferState): Long = dao.countByState(state)

    private fun validateTransition(
        current: TransferTask,
        expectedRevision: Long,
        newState: TransferState,
    ) {
        if (current.stateRevision != expectedRevision) {
            throw StaleRevisionException(requireNotNull(current.id), expectedRevision)
        }
        if (!TransferState.canTransition(current.state, newState)) {
            throw IllegalTransferTransitionException(requireNotNull(current.id), current.state, newState)
        }
    }
}

/**
 * 基于 Room DAO 的 inode 身份仓库。
 */
internal class RoomInodeMapRepository(
    private val dao: InodeMapDao,
) : InodeMapRepository {
    override suspend fun lookup(inode: ULong): InodeRecord? = dao.lookup(inode)

    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long) =
        dao.upsert(InodeRecord(inode, relativePath, fileId, scannedAt))

    override suspend fun delete(inode: ULong) = dao.delete(inode)

    override suspend fun selectAll(): List<InodeRecord> = dao.selectAll()

    override suspend fun purgeMissing(seenInodes: Set<ULong>) {
        if (seenInodes.isEmpty()) dao.deleteAll() else dao.purgeMissing(seenInodes.toList())
    }
}

/**
 * 基于 Room DAO 的释放空间暂存仓库。
 */
internal class RoomFreeUpStagingRepository(
    private val dao: FreeUpStagingDao,
) : FreeUpStagingRepository {
    override suspend fun insert(record: FreeUpStagingRecord) = dao.insert(record)

    override suspend fun findByName(stagingName: String): FreeUpStagingRecord? = dao.findByName(stagingName)

    override suspend fun findAll(): List<FreeUpStagingRecord> = dao.findAll()

    override suspend fun deleteByName(stagingName: String) = dao.deleteByName(stagingName)
}
