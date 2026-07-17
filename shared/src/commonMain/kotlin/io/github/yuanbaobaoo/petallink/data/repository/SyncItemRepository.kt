package io.github.yuanbaobaoo.petallink.data.repository

import io.github.yuanbaobaoo.petallink.data.SyncItem

/**
 * sync_items 仓库接口（对标 docs/04 §6）
 *
 * CAS 乐观锁：状态变更带 state_revision，受影响行数 == 0 即冲突。
 */
interface SyncItemRepository {
    /** 按 (fileId, localPath) 覆盖基线。 */
    suspend fun upsert(item: SyncItem)

    /** 按 fileId 查询 */
    suspend fun findByFileId(fileId: String): SyncItem?

    /** 按本地路径查询 */
    suspend fun findByLocalPath(localPath: String): SyncItem?

    suspend fun updateStatus(fileId: String, localPath: String, newStatus: Int, errorMsg: String?)

    /** 仅当完整成功基线快照仍匹配时，把真实内容推进为 cloud-only。 */
    suspend fun casMarkCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Boolean

    /** 释放空间回滚：仅撤销由同一快照产生的 cloud-only 状态。 */
    suspend fun casRollbackCloudOnly(
        fileId: String,
        localPath: String,
        sourceMtime: Long,
        sourceSize: Long,
    ): Boolean

    /** 按 fileId 删除 */
    suspend fun deleteByFileId(fileId: String)

    suspend fun deleteByLocalPath(localPath: String)

    /** 在同一事务中用新路径子树替换旧基线子树。 */
    suspend fun replaceSubtree(oldRoot: String, replacements: List<SyncItem>)

    /** 原子更新路径子树状态。 */
    suspend fun updateSubtreeStatus(root: String, newStatus: Int, errorMsg: String?)

    suspend fun resetStaleStatuses()

    /** 列出全部同步项 */
    suspend fun selectAll(): List<SyncItem>

    /** 按文件夹前缀列出（LPAD 匹配，用于列出某目录下所有可释放文件） */
    suspend fun selectByFolderPrefix(folderPrefix: String): List<SyncItem>

    /** 按状态列出 */
    suspend fun selectByStatus(status: Int): List<SyncItem>

    /** 总数 */
    suspend fun countAll(): Long

    /** 按状态计数 */
    suspend fun countByStatus(status: Int): Long
}
