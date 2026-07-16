package io.github.yuanbaobaao.petallink.data.repository

import io.github.yuanbaobaao.petallink.sync.identity.InodeRecord

/**
 * inode 身份映射仓库接口（docs/11 §4.1）
 *
 * 实现由 macosMain 的 SQLDelight 提供。所有身份查询走此接口——只读 DB 操作。
 */
interface InodeMapRepository {
    /** 查询某 inode 对应的云端身份 */
    suspend fun lookup(inode: ULong): InodeRecord?

    /** 下载/释放空间完成后主动更新映射（确定性记账） */
    suspend fun upsert(inode: ULong, relativePath: String, fileId: String, scannedAt: Long)

    /** 删除某 inode 记录 */
    suspend fun delete(inode: ULong)
}
