package io.github.yuanbaobaoo.petallink.data.repository

/**
 * 释放空间暂存记录（对标 docs/11 §3.2 free_up_staging 表）
 *
 * 替代 XATTR_FREE_UP_RELATIVE_PATH，恢复记录走 DB 事务。
 */
data class FreeUpStagingRecord(
    val stagingName: String,   // 暂存文件名（如 .hwcloud_freeup-xxxx）
    val relativePath: String,  // 原始相对路径
    val fileId: String,        // 云端文件 ID
    val sourceMtime: Long?,    // 原文件 mtime（回滚恢复用）
    val sourceSize: Long?,     // 原文件大小（回滚恢复用）
    val createdAt: Long,       // 创建时间戳（ms）
)

/**
 * 释放空间暂存仓库接口。
 */
interface FreeUpStagingRepository {
    /** 记录一条暂存（事务内） */
    suspend fun insert(record: FreeUpStagingRecord)

    /** 按暂存文件名查询 */
    suspend fun findByName(stagingName: String): FreeUpStagingRecord?

    /** 查询全部暂存记录（启动恢复用） */
    suspend fun findAll(): List<FreeUpStagingRecord>

    /** 删除一条暂存记录（恢复完成后清理） */
    suspend fun deleteByName(stagingName: String)
}
