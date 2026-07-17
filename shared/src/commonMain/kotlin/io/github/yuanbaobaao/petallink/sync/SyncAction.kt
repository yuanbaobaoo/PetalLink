package io.github.yuanbaobaao.petallink.sync

import io.github.yuanbaobaao.petallink.drive.DriveFile

/**
 * 同步动作类型（对标 src/sync/state.rs SyncActionType，10 种）。
 *
 * 详见 docs/06 §planner。
 * 注意：MoveInCloud 不由 decide() 产生（由重命名检测逻辑产生）。
 */
enum class SyncActionType {
    UPLOAD,                  // 上传本地内容到云端
    CREATE_PLACEHOLDER,      // 为云端文件创建本地占位符
    DOWNLOAD,                // 下载云端内容到本地
    DELETE_FROM_CLOUD,       // 删除云端文件（本地发起）
    DELETE_FROM_LOCAL,       // 删除本地文件（云端发起）
    CREATE_CONFLICT_COPY,    // 三方冲突 → 重命名本地副本后下载云端
    SKIP,                    // 无操作（pending 收敛时携带 cloud_file）
    CREATE_FOLDER,           // 创建文件夹
    MOVE_IN_CLOUD,           // 云端重命名（Files:update，非 decide 产生）
    BACKUP_BEFORE_CLOUD_DELETE, // 云端已删但本地有未上传修改 → 备份副本
}

/**
 * 同步动作（对标 SyncAction）。
 *
 * @param type 动作类型
 * @param relativePath 相对挂载目录的路径
 * @param fileId 云端文件 ID（可空：新增/孤儿场景）
 * @param cloudFile 云端文件元数据（占位/下载/收敛时携带）
 * @param reason 决策原因（日志/诊断用）
 */
data class SyncAction(
    val type: SyncActionType,
    val relativePath: String,
    val fileId: String? = null,
    val cloudFile: DriveFile? = null,
    val reason: String,
    val parentFileId: String? = null,
)
