package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.sync.TransferState

/**
 * 数据模型（对标原项目 src/data/ 与 src/sync/state.rs）
 *
 * 详见 docs/04-数据模型与持久化.md。
 */

/** 同步项：云端↔本地的基线记录 */
data class SyncItem(
    val fileId: String,
    val localPath: String,
    val parentFolderId: String?,
    val name: String,
    val isFolder: Boolean,
    val size: Long,
    val localSize: Long?,
    val sha256: String?,
    val localMtime: Long?,
    val cloudEditedTime: Long?,
    val lastSyncTime: Long?,
    val status: Int,
    val errorMessage: String?,
) {
    // 兼容现有 UI/状态聚合命名，存储语义以终态 schema 为准。
    val parentFileId: String? get() = parentFolderId
    val mtime: Long get() = localMtime ?: 0L
    val syncStatus: Int get() = status
    val lastError: String? get() = errorMessage
}

/** 传输任务（对应 transfer_queue 表） */
data class TransferTask(
    val id: Long?,
    val direction: TransferDirection,
    val fileId: String?,
    val localPath: String?,
    val name: String,
    val totalSize: Long = 0L,
    val transferred: Long = 0L,
    val state: TransferState,
    val errorMessage: String?,
    val createdAt: Long,
    val finishedAt: Long? = null,
    val serverId: String? = null,
    val uploadId: String? = null,
    val resumeOffset: Long = 0L,
    val sessionUrl: String? = null,
    val relativePath: String? = null,
    val parentFileId: String? = null,
    val operation: Int? = null,
    val sourceMtime: Long? = null,
    val sourceSize: Long? = null,
    val expectedCloudEditedTime: Long? = null,
    val attemptCount: Int = 0,
    val nextRetryAt: Long? = null,
    val errorKind: Int? = null,
    val remoteResultFileId: String? = null,
    val stateRevision: Long = 0L,
) {
    val attempt: Int get() = attemptCount
    val bytesTotal: Long get() = totalSize
    val bytesDone: Long get() = transferred
    val uploadSessionUrl: String? get() = sessionUrl
}

/** 传输方向 */
enum class TransferDirection { UPLOAD, DOWNLOAD, DELETE, DOWNLOAD_UPDATE }

/**
 * 列补丁三态（对标原项目 ColumnPatch）。
 * 用于 CAS 更新 sync_items：Keep 不动 / Set 设值 / Clear 置空。
 * 序列化保持只发送变更字段，减少乐观锁冲突。
 */
sealed class ColumnPatch<out T> {
    /** 不变更此列（SQL UPDATE 不包含此字段） */
    object Keep : ColumnPatch<Nothing>()
    /** 设置此列的值 */
    data class Set<T>(val value: T) : ColumnPatch<T>()
    /** 清空此列（设为 NULL） */
    object Clear : ColumnPatch<Nothing>()
}

/** 一次传输生命周期迁移附带的原子字段补丁。 */
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

/** 仅在同一 Running revision 内更新的进度与 resume 会话。 */
data class RunningTransferPatch(
    val transferred: Long? = null,
    val resumeOffset: Long? = null,
    val serverId: ColumnPatch<String> = ColumnPatch.Keep,
    val uploadId: ColumnPatch<String> = ColumnPatch.Keep,
    val sessionUrl: ColumnPatch<String> = ColumnPatch.Keep,
)
