//! 同步状态类型 —— SyncAction / SyncGlobalState。
//!
//! 对齐 `legacy/lib/sync/sync_state.dart`。

use serde::Serialize;

/// 同步动作类型（对齐 dart SyncActionType 枚举）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncActionType {
    Upload,
    CreatePlaceholder,
    Download,
    DeleteFromCloud,
    DeleteFromLocal,
    CreateConflictCopy,
    Skip,
    CreateFolder,
    /// 本地文件跨目录移动：远端保持同一 fileId，通过 Files:update 成对 parent 参数移动，
    /// 并可在同一个 PATCH 中完成改名。
    MoveInCloud,
    /// 云端已删除该文件，但本地有未上传的真实修改 → 改名备份副本（保内容），
    /// 原路径腾空即满足云端删除。副本清掉占位 xattr，下轮作为全新本地文件上传。
    BackupBeforeCloudDelete,
}

/// 同步动作（对齐 dart SyncAction）
#[derive(Debug, Clone)]
pub struct SyncAction {
    /// 动作类型
    pub action_type: SyncActionType,
    /// 相对路径
    pub relative_path: Option<String>,
    /// 云端文件 ID
    pub file_id: Option<String>,
    /// 父目录 fileId
    pub parent_file_id: Option<String>,
    /// 本地绝对路径
    pub local_path: Option<String>,
    /// 云端文件元数据（动态，仅在 createPlaceholder/createFolder/download 时使用）
    pub cloud_file: Option<crate::drive::models::DriveFile>,
    /// 原因（日志用）
    pub reason: Option<String>,
}

/// 同步全局状态（对齐 dart SyncGlobalState，供 UI 透传）
#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncGlobalState {
    /// 权威快照的进程内单调版本。
    pub revision: u64,
    pub total: u64,
    pub completed: u64,
    pub uploading: u64,
    pub downloading: u64,
    /// 因网络不可用而等待恢复的传输任务数（不属于永久失败）。
    pub waiting_network: u64,
    pub failed: u64,
    /// 传输队列中永久失败的历史任务数，与当前同步失败分开统计。
    pub transfer_failed: u64,
    /// 失败项详情（供 SyncStatusBar 失败项弹窗，最多 20 条）
    pub failed_items: Vec<FailedItem>,
    pub conflict: u64,
    /// 被暂停编辑的文件数（F-MOUNT-11）
    pub editing: u64,
    /// 引擎是否正在运行
    pub is_running: bool,
    /// 上次同步时间（毫秒 epoch）
    pub last_sync_time: Option<i64>,
    /// 是否正在索引云端目录
    pub is_indexing: bool,
    /// 已扫描的文件夹数（索引用）
    pub indexing_scanned_folders: u64,
    /// 已发现的文件总数（索引用）
    pub indexing_discovered_items: u64,
    /// 是否有目录结构变更（触发前端目录重拉）
    pub content_changed: bool,
    /// 当前同步阶段（供前端状态条精确显示）。None = 空闲。
    /// 值：indexing-startup / indexing-manual / indexing-auto-full /
    ///     querying-changes / syncing-auto-incremental /
    ///     syncing-local / syncing-manual / syncing-retry / syncing-startup
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sync_phase: Option<String>,
}

/// 失败项详情（前端失败项弹窗用）
#[derive(Debug, Clone, Serialize)]
pub struct FailedItem {
    /// 相对路径（取自 sync_items.local_path）
    pub relative_path: String,
    /// 错误信息
    pub error_message: Option<String>,
}

impl SyncGlobalState {
    /// 同步完成度 0.0~1.0
    pub fn progress(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }
}

/// 同步动作执行结果
#[derive(Debug, Clone)]
pub struct ActionResult {
    /// 是否成功
    pub success: bool,
    /// 错误信息
    pub error_message: Option<String>,
    /// 是否延迟（稳定性检查未通过或用户编辑中）
    pub deferred: bool,
    /// 生成的云端文件（上传/建文件夹成功时）
    pub cloud_file: Option<crate::drive::models::DriveFile>,
}

/// 释放空间安全校验结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreeUpCheckResult {
    /// 可以安全释放
    Safe,
    /// 云端不存在（释放后无法找回）
    NotInCloud,
    /// 本地尚未同步到云端（有未上传修改）
    NotSynced,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_all_completed() {
        let state = SyncGlobalState {
            total: 100,
            completed: 100,
            ..Default::default()
        };
        assert_eq!(state.progress(), 1.0);
    }

    #[test]
    fn test_progress_half() {
        let state = SyncGlobalState {
            total: 100,
            completed: 50,
            ..Default::default()
        };
        assert_eq!(state.progress(), 0.5);
    }

    #[test]
    fn test_progress_zero_total() {
        let state = SyncGlobalState::default();
        assert_eq!(state.progress(), 1.0);
    }
}
