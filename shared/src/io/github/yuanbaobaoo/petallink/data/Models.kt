package io.github.yuanbaobaoo.petallink.data

import androidx.room.ColumnInfo
import androidx.room.Entity
import androidx.room.Ignore
import androidx.room.Index
import androidx.room.PrimaryKey
import io.github.yuanbaobaoo.petallink.sync.TransferState

/**
 * 数据模型（对标原项目 src/data/ 与 src/sync/state.rs）
 *
 * 详见 docs/04-数据模型与持久化.md。
 */

/**
 * 同步项：云端↔本地的基线记录
 */
@Entity(
    tableName = "sync_items",
    indices = [
        Index(value = ["local_path"], unique = true),
        Index(value = ["parent_folder_id"]),
        Index(value = ["status"]),
    ],
)
data class SyncItem(
    @PrimaryKey
    @ColumnInfo(name = "file_id")
    val fileId: String,

    @ColumnInfo(name = "local_path")
    val localPath: String,

    @ColumnInfo(name = "parent_folder_id")
    val parentFolderId: String?,

    val name: String,

    @ColumnInfo(name = "is_folder", defaultValue = "0")
    val isFolder: Boolean,

    @ColumnInfo(defaultValue = "0")
    val size: Long,

    @ColumnInfo(name = "local_size")
    val localSize: Long?,

    val sha256: String?,

    @ColumnInfo(name = "local_mtime")
    val localMtime: Long?,

    @ColumnInfo(name = "cloud_edited_time")
    val cloudEditedTime: Long?,

    @ColumnInfo(name = "last_sync_time")
    val lastSyncTime: Long?,

    @ColumnInfo(defaultValue = "0")
    val status: Int,

    @ColumnInfo(name = "error_message")
    val errorMessage: String?,
) {
    // 兼容现有 UI/状态聚合命名，存储语义以终态 schema 为准。
    @get:Ignore
    val parentFileId: String? get() = parentFolderId

    @get:Ignore
    val mtime: Long get() = localMtime ?: 0L

    @get:Ignore
    val syncStatus: Int get() = status

    @get:Ignore
    val lastError: String? get() = errorMessage
}

/**
 * 传输任务（对应 transfer_queue 表）
 */
@Entity(
    tableName = "transfer_queue",
    indices = [
        Index(value = ["state"]),
        Index(value = ["direction"]),
        Index(value = ["file_id"]),
        Index(value = ["relative_path"]),
        Index(value = ["next_retry_at"]),
    ],
)
data class TransferTask(
    @PrimaryKey(autoGenerate = true)
    val id: Long?,

    val direction: TransferDirection,

    @ColumnInfo(name = "file_id")
    val fileId: String?,

    @ColumnInfo(name = "local_path")
    val localPath: String?,

    val name: String,

    @ColumnInfo(name = "total_size", defaultValue = "0")
    val totalSize: Long = 0L,

    @ColumnInfo(defaultValue = "0")
    val transferred: Long = 0L,

    val state: TransferState,

    @ColumnInfo(name = "error_message")
    val errorMessage: String?,

    @ColumnInfo(name = "created_at")
    val createdAt: Long,

    @ColumnInfo(name = "finished_at")
    val finishedAt: Long? = null,

    @ColumnInfo(name = "server_id")
    val serverId: String? = null,

    @ColumnInfo(name = "upload_id")
    val uploadId: String? = null,

    @ColumnInfo(name = "resume_offset", defaultValue = "0")
    val resumeOffset: Long = 0L,

    @ColumnInfo(name = "session_url")
    val sessionUrl: String? = null,

    @ColumnInfo(name = "relative_path")
    val relativePath: String? = null,

    @ColumnInfo(name = "parent_file_id")
    val parentFileId: String? = null,

    val operation: Int? = null,

    @ColumnInfo(name = "source_mtime")
    val sourceMtime: Long? = null,

    @ColumnInfo(name = "source_size")
    val sourceSize: Long? = null,

    @ColumnInfo(name = "expected_cloud_edited_time")
    val expectedCloudEditedTime: Long? = null,

    @ColumnInfo(name = "attempt_count", defaultValue = "0")
    val attemptCount: Int = 0,

    @ColumnInfo(name = "next_retry_at")
    val nextRetryAt: Long? = null,

    @ColumnInfo(name = "error_kind")
    val errorKind: Int? = null,

    @ColumnInfo(name = "remote_result_file_id")
    val remoteResultFileId: String? = null,

    @ColumnInfo(name = "state_revision", defaultValue = "0")
    val stateRevision: Long = 0L,
) {
    @get:Ignore
    val attempt: Int get() = attemptCount

    @get:Ignore
    val bytesTotal: Long get() = totalSize

    @get:Ignore
    val bytesDone: Long get() = transferred

    @get:Ignore
    val uploadSessionUrl: String? get() = sessionUrl
}

/**
 * 传输方向
 */
enum class TransferDirection {
    /**
     * 上传本地文件。
     */
    UPLOAD,

    /**
     * 下载云端文件。
     */
    DOWNLOAD,

    /**
     * 删除目标文件。
     */
    DELETE,

    /**
     * 下载并覆盖已有文件。
     */
    DOWNLOAD_UPDATE,
}

/**
 * 列补丁三态（对标原项目 ColumnPatch）。
 * 用于 CAS 更新 sync_items：Keep 不动 / Set 设值 / Clear 置空。
 * 序列化保持只发送变更字段，减少乐观锁冲突。
 */
sealed class ColumnPatch<out T> {
    /**
     * 不变更此列（SQL UPDATE 不包含此字段）
     */
    object Keep : ColumnPatch<Nothing>()

    /**
     * 设置此列的值
     */
    data class Set<T>(val value: T) : ColumnPatch<T>()

    /**
     * 清空此列（设为 NULL）
     */
    object Clear : ColumnPatch<Nothing>()
}

/**
 * 一次传输生命周期迁移附带的原子字段补丁。
 */
data class TransferPatch(
    val errorKind: ColumnPatch<Int> = ColumnPatch.Keep,
    val errorMessage: ColumnPatch<String> = ColumnPatch.Keep,
    val nextRetryAt: ColumnPatch<Long> = ColumnPatch.Keep,
    val finishedAt: ColumnPatch<Long> = ColumnPatch.Keep,
    val remoteResultFileId: ColumnPatch<String> = ColumnPatch.Keep,
    val serverId: ColumnPatch<String> = ColumnPatch.Keep,
    val uploadId: ColumnPatch<String> = ColumnPatch.Keep,
    val sessionUrl: ColumnPatch<String> = ColumnPatch.Keep,
    val transferred: Long? = null,
    val resumeOffset: Long? = null,
    val attemptCount: Int? = null,
)

/**
 * 仅在同一 Running revision 内更新的进度与 resume 会话。
 */
data class RunningTransferPatch(
    val transferred: Long? = null,
    val resumeOffset: Long? = null,
    val serverId: ColumnPatch<String> = ColumnPatch.Keep,
    val uploadId: ColumnPatch<String> = ColumnPatch.Keep,
    val sessionUrl: ColumnPatch<String> = ColumnPatch.Keep,
)
