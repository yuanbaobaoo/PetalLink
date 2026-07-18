package io.github.yuanbaobaoo.petallink.data.repository

import androidx.room.ColumnInfo
import androidx.room.Entity
import androidx.room.PrimaryKey

/**
 * 释放空间暂存记录（对标 docs/11 §3.2 free_up_staging 表）
 *
 * 替代 XATTR_FREE_UP_RELATIVE_PATH，恢复记录走 DB 事务。
 */
@Entity(tableName = "free_up_staging")
data class FreeUpStagingRecord(
    @PrimaryKey
    @ColumnInfo(name = "staging_name")
    val stagingName: String,

    @ColumnInfo(name = "relative_path")
    val relativePath: String,

    @ColumnInfo(name = "file_id")
    val fileId: String,

    @ColumnInfo(name = "source_mtime")
    val sourceMtime: Long?,

    @ColumnInfo(name = "source_size")
    val sourceSize: Long?,

    @ColumnInfo(name = "created_at")
    val createdAt: Long,
)

/**
 * 释放空间暂存仓库接口。
 */
interface FreeUpStagingRepository {
    /**
     * 记录一条暂存（事务内）
     */
    suspend fun insert(record: FreeUpStagingRecord)

    /**
     * 按暂存文件名查询
     */
    suspend fun findByName(stagingName: String): FreeUpStagingRecord?

    /**
     * 查询全部暂存记录（启动恢复用）
     */
    suspend fun findAll(): List<FreeUpStagingRecord>

    /**
     * 删除一条暂存记录（恢复完成后清理）
     */
    suspend fun deleteByName(stagingName: String)
}
