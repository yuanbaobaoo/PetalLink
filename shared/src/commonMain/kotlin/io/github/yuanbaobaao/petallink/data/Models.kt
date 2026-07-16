package io.github.yuanbaobaao.petallink.data

import io.github.yuanbaobaao.petallink.sync.TransferState

/**
 * 数据模型（对标原项目 src/data/ 与 src/sync/state.rs）
 *
 * 详见 docs/04-数据模型与持久化.md。
 */

/** 同步项：云端↔本地的基线记录 */
data class SyncItem(
    val id: Long?,
    val fileId: String,
    val localPath: String,
    val parentFileId: String?,
    val isFolder: Boolean,
    val size: Long,
    val mtime: Long,
    val etag: String?,
    val syncStatus: Int,
    val stateRevision: Long,
    val lastError: String?,
)

/** 传输任务（对应 transfer_queue 表） */
data class TransferTask(
    val id: Long?,
    val fileId: String,
    val localPath: String,
    val direction: TransferDirection,
    val state: TransferState,
    val stateRevision: Long,
    val attempt: Int,
    val bytesTotal: Long,
    val bytesDone: Long,
    val errorMessage: String?,
    val uploadSessionUrl: String?,
)

/** 传输方向 */
enum class TransferDirection { UPLOAD, DOWNLOAD }

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
