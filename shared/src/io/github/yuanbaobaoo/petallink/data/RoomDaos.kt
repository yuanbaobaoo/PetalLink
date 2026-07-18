package io.github.yuanbaobaoo.petallink.data

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Transaction
import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRecord
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.identity.InodeRecord

/**
 * 同步基线表的 Room DAO。
 */
@Dao
interface SyncItemDao {
    /**
     * 按 `fileId` 覆盖同步基线。
     *
     * @param item 新的完整基线。
     */
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(item: SyncItem)

    /**
     * 批量覆盖同步基线。
     *
     * @param items 新的完整基线集合。
     */
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsertAll(items: List<SyncItem>)

    /**
     * 按 `fileId` 查询基线。
     *
     * @param fileId 云端文件 ID。
     * @return 对应基线，不存在时返回 null。
     */
    @Query("SELECT * FROM sync_items WHERE file_id = :fileId LIMIT 1")
    suspend fun findByFileId(fileId: String): SyncItem?

    /**
     * 按本地路径查询基线。
     *
     * @param localPath 相对挂载目录的路径。
     * @return 对应基线，不存在时返回 null。
     */
    @Query("SELECT * FROM sync_items WHERE local_path = :localPath LIMIT 1")
    suspend fun findByLocalPath(localPath: String): SyncItem?

    /**
     * 更新基线状态与错误信息。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE sync_items
            SET status = :newStatus, error_message = :errorMessage
            WHERE file_id = :fileId AND local_path = :localPath""",
    )
    suspend fun updateStatus(
        fileId: String,
        localPath: String,
        newStatus: Int,
        errorMessage: String?,
    ): Int

    /**
     * 仅在本地快照仍匹配时推进为 cloud-only。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE sync_items
            SET status = 1, local_size = 0, error_message = NULL
            WHERE file_id = :fileId AND local_path = :localPath AND status = 0
              AND local_mtime = :sourceMtime AND local_size = :sourceSize""",
    )
    suspend fun markCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Int

    /**
     * 仅在 cloud-only 快照仍匹配时回滚。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE sync_items
            SET status = 0, local_size = :sourceSize, error_message = NULL
            WHERE file_id = :fileId AND local_path = :localPath AND status = 1
              AND local_mtime = :sourceMtime AND local_size = 0""",
    )
    suspend fun rollbackCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Int

    /**
     * 按 `fileId` 删除基线。
     */
    @Query("DELETE FROM sync_items WHERE file_id = :fileId")
    suspend fun deleteByFileId(fileId: String)

    /**
     * 按本地路径删除基线。
     */
    @Query("DELETE FROM sync_items WHERE local_path = :localPath")
    suspend fun deleteByLocalPath(localPath: String)

    /**
     * 重置异常退出遗留的中间状态。
     */
    @Query("UPDATE sync_items SET status = 0, error_message = NULL WHERE status = 3")
    suspend fun resetStaleStatuses()

    /**
     * 查询全部基线。
     */
    @Query("SELECT * FROM sync_items ORDER BY local_path")
    suspend fun selectAll(): List<SyncItem>

    /**
     * 按路径前缀查询基线。
     */
    @Query("SELECT * FROM sync_items WHERE local_path LIKE :prefix || '%' ESCAPE '\\' ORDER BY local_path")
    suspend fun selectByFolderPrefix(prefix: String): List<SyncItem>

    /**
     * 按状态查询基线。
     */
    @Query("SELECT * FROM sync_items WHERE status = :status ORDER BY local_path")
    suspend fun selectByStatus(status: Int): List<SyncItem>

    /**
     * 返回基线总数。
     */
    @Query("SELECT COUNT(*) FROM sync_items")
    suspend fun countAll(): Long

    /**
     * 返回指定状态的基线数。
     */
    @Query("SELECT COUNT(*) FROM sync_items WHERE status = :status")
    suspend fun countByStatus(status: Int): Long

    /**
     * 在一个事务中替换路径子树。
     */
    @Transaction
    suspend fun replaceSubtree(oldRoot: String, replacements: List<SyncItem>) {
        val prefix = "$oldRoot/"
        selectAll()
            .filter { it.localPath == oldRoot || it.localPath.startsWith(prefix) }
            .forEach { deleteByLocalPath(it.localPath) }
        upsertAll(replacements)
    }

    /**
     * 在一个事务中更新路径子树状态。
     */
    @Transaction
    suspend fun updateSubtreeStatus(root: String, newStatus: Int, errorMessage: String?) {
        val prefix = "$root/"
        selectAll()
            .filter { it.localPath == root || it.localPath.startsWith(prefix) }
            .forEach { updateStatus(it.fileId, it.localPath, newStatus, errorMessage) }
    }
}

/**
 * 持久化传输队列的 Room DAO。
 */
@Dao
interface TransferTaskDao {
    /**
     * 插入传输任务。
     *
     * @return SQLite 生成的任务 ID。
     */
    @Insert
    suspend fun insert(task: TransferTask): Long

    /**
     * 按 ID 查询任务。
     */
    @Query("SELECT * FROM transfer_queue WHERE id = :id LIMIT 1")
    suspend fun findById(id: Long): TransferTask?

    /**
     * 以 revision CAS 切换任务状态。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE transfer_queue
            SET state = :newState,
                state_revision = state_revision + 1,
                attempt_count = :attemptCount,
                error_message = :errorMessage,
                finished_at = :finishedAt
            WHERE id = :id AND state_revision = :expectedRevision""",
    )
    suspend fun transitionState(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        attemptCount: Int,
        errorMessage: String?,
        finishedAt: Long?,
    ): Int

    /**
     * 以 revision CAS 切换状态并提交完整字段补丁。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE transfer_queue
            SET state = :newState,
                state_revision = state_revision + 1,
                error_kind = :errorKind,
                error_message = :errorMessage,
                next_retry_at = :nextRetryAt,
                finished_at = :finishedAt,
                remote_result_file_id = :remoteResultFileId,
                server_id = :serverId,
                upload_id = :uploadId,
                session_url = :sessionUrl,
                transferred = :transferred,
                resume_offset = :resumeOffset,
                attempt_count = :attemptCount
            WHERE id = :id AND state_revision = :expectedRevision""",
    )
    suspend fun transitionWithPatch(
        id: Long,
        expectedRevision: Long,
        newState: TransferState,
        errorKind: Int?,
        errorMessage: String?,
        nextRetryAt: Long?,
        finishedAt: Long?,
        remoteResultFileId: String?,
        serverId: String?,
        uploadId: String?,
        sessionUrl: String?,
        transferred: Long,
        resumeOffset: Long,
        attemptCount: Int,
    ): Int

    /**
     * 更新同一 Running revision 内的进度。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE transfer_queue
            SET transferred = :bytesDone, resume_offset = :bytesDone
            WHERE id = :id AND state = :runningState AND state_revision = :expectedRevision""",
    )
    suspend fun updateRunningProgress(
        id: Long,
        expectedRevision: Long,
        bytesDone: Long,
        runningState: TransferState,
    ): Int

    /**
     * 更新同一 Running revision 内的断点上下文。
     *
     * @return 受影响行数。
     */
    @Query(
        """UPDATE transfer_queue
            SET transferred = :transferred,
                resume_offset = :resumeOffset,
                server_id = :serverId,
                upload_id = :uploadId,
                session_url = :sessionUrl
            WHERE id = :id AND state = :runningState AND state_revision = :expectedRevision""",
    )
    suspend fun updateRunningTransfer(
        id: Long,
        expectedRevision: Long,
        transferred: Long,
        resumeOffset: Long,
        serverId: String?,
        uploadId: String?,
        sessionUrl: String?,
        runningState: TransferState,
    ): Int

    /**
     * 按状态查询任务。
     */
    @Query("SELECT * FROM transfer_queue WHERE state = :state")
    suspend fun selectByState(state: TransferState): List<TransferTask>

    /**
     * 查询全部任务并按新旧顺序排列。
     */
    @Query("SELECT * FROM transfer_queue ORDER BY created_at DESC, id DESC")
    suspend fun selectAll(): List<TransferTask>

    /**
     * 删除已完成任务。
     */
    @Query("DELETE FROM transfer_queue WHERE state = :state")
    suspend fun deleteByState(state: TransferState)

    /**
     * 仅保留最近指定数量的终态任务。
     */
    @Query(
        """DELETE FROM transfer_queue
            WHERE state IN (:completed, :failed, :canceled)
              AND id NOT IN (
                SELECT id FROM transfer_queue
                WHERE state IN (:completed, :failed, :canceled)
                ORDER BY COALESCE(finished_at, created_at) DESC
                LIMIT :keepCount
              )""",
    )
    suspend fun pruneHistory(
        keepCount: Int,
        completed: TransferState,
        failed: TransferState,
        canceled: TransferState,
    )

    /**
     * 按状态和方向统计任务。
     */
    @Query("SELECT COUNT(*) FROM transfer_queue WHERE state = :state AND direction = :direction")
    suspend fun countByStateAndDirection(state: TransferState, direction: TransferDirection): Long

    /**
     * 按状态统计任务。
     */
    @Query("SELECT COUNT(*) FROM transfer_queue WHERE state = :state")
    suspend fun countByState(state: TransferState): Long
}

/**
 * inode 身份映射表的 Room DAO。
 */
@Dao
interface InodeMapDao {
    /**
     * 按 inode 查询身份。
     */
    @Query("SELECT * FROM local_inode_map WHERE inode = :inode LIMIT 1")
    suspend fun lookup(inode: ULong): InodeRecord?

    /**
     * 覆盖 inode 身份。
     */
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun upsert(record: InodeRecord)

    /**
     * 删除指定 inode。
     */
    @Query("DELETE FROM local_inode_map WHERE inode = :inode")
    suspend fun delete(inode: ULong)

    /**
     * 查询全部 inode 身份。
     */
    @Query("SELECT * FROM local_inode_map ORDER BY inode")
    suspend fun selectAll(): List<InodeRecord>

    /**
     * 删除本轮扫描未见到的 inode。
     */
    @Query("DELETE FROM local_inode_map WHERE inode NOT IN (:seenInodes)")
    suspend fun purgeMissing(seenInodes: List<ULong>)

    /**
     * 清空 inode 身份。
     */
    @Query("DELETE FROM local_inode_map")
    suspend fun deleteAll()
}

/**
 * 释放空间暂存表的 Room DAO。
 */
@Dao
interface FreeUpStagingDao {
    /**
     * 覆盖暂存记录。
     */
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(record: FreeUpStagingRecord)

    /**
     * 按暂存名查询记录。
     */
    @Query("SELECT * FROM free_up_staging WHERE staging_name = :stagingName LIMIT 1")
    suspend fun findByName(stagingName: String): FreeUpStagingRecord?

    /**
     * 查询全部暂存记录。
     */
    @Query("SELECT * FROM free_up_staging ORDER BY created_at, staging_name")
    suspend fun findAll(): List<FreeUpStagingRecord>

    /**
     * 删除指定暂存记录。
     */
    @Query("DELETE FROM free_up_staging WHERE staging_name = :stagingName")
    suspend fun deleteByName(stagingName: String)
}

/**
 * 数据库整体维护操作的 Room DAO。
 */
@Dao
interface DatabaseMaintenanceDao {
    /**
     * 清空所有挂载与传输状态。
     */
    @Transaction
    suspend fun clearMountState() {
        deleteTransfers()
        deleteSyncItems()
        deleteInodeMap()
        deleteFreeUpStaging()
    }

    /**
     * 清空传输任务。
     */
    @Query("DELETE FROM transfer_queue")
    suspend fun deleteTransfers()

    /**
     * 清空同步基线。
     */
    @Query("DELETE FROM sync_items")
    suspend fun deleteSyncItems()

    /**
     * 清空 inode 身份。
     */
    @Query("DELETE FROM local_inode_map")
    suspend fun deleteInodeMap()

    /**
     * 清空释放空间暂存记录。
     */
    @Query("DELETE FROM free_up_staging")
    suspend fun deleteFreeUpStaging()
}
