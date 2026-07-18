package io.github.yuanbaobaoo.petallink.data.repository

import io.github.yuanbaobaoo.petallink.sync.identity.InodeRecord

/**
 * inode 身份映射仓库接口（docs/11 §4.1）
 *
 * 实现由 `commonMain` 的 Room DAO 提供。所有身份查询走此接口。
 */
interface InodeMapRepository {
    /**
     * 查询某 inode 对应的云端身份
     */
    suspend fun lookup(inode: ULong): InodeRecord?

    /**
     * 下载/释放空间完成后主动更新映射（确定性记账）
     */
    suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long)

    /**
     * 删除某 inode 记录
     */
    suspend fun delete(inode: ULong)

    /**
     * 返回当前全部映射，供扫描对账和诊断使用。
     */
    suspend fun selectAll(): List<InodeRecord>

    /**
     * 扫描结束后删除本轮未见到的 inode 映射。
     */
    suspend fun purgeMissing(seenInodes: Set<ULong>)
}
