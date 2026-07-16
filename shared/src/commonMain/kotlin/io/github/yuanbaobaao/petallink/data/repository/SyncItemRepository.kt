package io.github.yuanbaobaao.petallink.data.repository

import io.github.yuanbaobaao.petallink.data.SyncItem

/**
 * sync_items 仓库接口（对标 docs/04 §6）
 *
 * CAS 乐观锁：状态变更带 state_revision，受影响行数 == 0 即冲突。
 */
interface SyncItemRepository {
    /** 新增基线记录，返回自增 id */
    suspend fun insert(item: SyncItem): Long

    /** 按 id 查询 */
    suspend fun findById(id: Long): SyncItem?

    /** 按 fileId 查询 */
    suspend fun findByFileId(fileId: String): SyncItem?

    /** 按本地路径查询 */
    suspend fun findByLocalPath(localPath: String): SyncItem?

    /**
     * CAS 状态更新。
     * @param expectedRevision 预期的 state_revision（乐观锁）
     * @return true 成功；false 表示 revision 已变（CAS 冲突）
     */
    suspend fun casUpdateStatus(
        id: Long,
        expectedRevision: Long,
        newStatus: Int,
        errorMsg: String?,
    ): Boolean

    /**
     * CAS 更新 etag（同步基线刷新）。
     * @return true 成功；false CAS 冲突
     */
    suspend fun casUpdateEtag(id: Long, expectedRevision: Long, etag: String): Boolean

    /** 按 fileId 删除 */
    suspend fun deleteByFileId(fileId: String)

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
