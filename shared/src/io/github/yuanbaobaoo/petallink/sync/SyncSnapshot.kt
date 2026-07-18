package io.github.yuanbaobaoo.petallink.sync

import io.github.yuanbaobaoo.petallink.drive.DriveFile

/**
 * DB 基线条目（对标 DbSnapshotEntry）。
 *
 * @param fileId 云端文件 ID（pending: 前缀表示上传待确认）
 * @param localMtime 本地 mtime（null 表示首次记录/文件夹）
 * @param localSize 本地大小（null 表示未知）
 * @param cloudEditedTimeMs 云端 editedTime 毫秒（null 表示首次记录）
 * @param status 同步状态（0=SYNCED, 7=DELETED 墓碑, 等）
 * @param isFolder 是否文件夹
 */
data class DbBaselineEntry(
    val fileId: String,
    val localMtime: Long?,
    val localSize: Long?,
    val cloudEditedTimeMs: Long?,
    val status: Int,
    val isFolder: Boolean,
)

/**
 * sync_status 枚举值（对标 docs/04 §sync_status）
 */
object SyncStatus {
    const val SYNCED = 0
    const val CLOUD_ONLY = 1
    const val LOCAL_ONLY = 2
    const val SYNCING = 3
    const val PENDING = SYNCING
    const val UPLOADING = SYNCING
    const val FAILED = 4
    const val CONFLICT = 5
    const val DELETED = 7   // 墓碑
}

/**
 * 同步快照（三方状态，对标 SyncSnapshot）。
 *
 * @param local 本地文件系统扫描结果（path → 条目）
 * @param cloud 云端文件树（path → DriveFile）
 * @param db DB 基线（path → DbBaselineEntry）
 * @param cloudTreeTrusted 云端树是否可信（不可信时抑制删除）
 * @param isStartupResume 是否启动恢复期
 */
data class SyncSnapshot(
    val local: Map<String, LocalEntry>,
    val cloud: Map<String, DriveFile>,
    val db: Map<String, DbBaselineEntry>,
    val cloudTreeTrusted: Boolean,
    val isStartupResume: Boolean,
)

/**
 * 本地条目（精简版，对标 LocalFileEntry 但面向 planner）。
 */
data class LocalEntry(
    val relativePath: String,
    val mtime: Long,
    val size: Long,
    val isPlaceholder: Boolean,
    val isFolder: Boolean,
) {
    /**
     * 是否有真实内容（非占位符）
     */
    val hasContent: Boolean get() = !isPlaceholder
}
