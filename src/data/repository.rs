//! 类型化实体 + 仓储操作（对齐 dart `SyncItemEntity` / `TransferTaskEntity` + DAO）。
//!
//! 状态/方向常量以 i32 形式持久化，提供枚举转换。

use serde::{Deserialize, Serialize};

use crate::sync::transfer_state::TransferErrorKind;

/// 统一 DB 操作错误映射：`db_err!("查询", expr)` 等价于
/// `expr.map_err(|e| AppError::generic(format!("查询失败：{e}")))?`。
///
/// 替代散布在仓储层的重复 `.map_err(|e| AppError::generic(format!("XX失败：{e}")))?` 模式。
macro_rules! db_err {
    ($op:literal, $expr:expr) => {
        $expr.map_err(|e| AppError::generic(format!("{}失败：{e}", $op)))?
    };
}

/// 同步基线记录的查询与写入实现。
// 同步基线仓储实现。
/// 同步基线记录的 SQLite 实现。
mod sync_items;
#[allow(unused_imports)]
pub use sync_items::{
    delete_all, delete_by_local_path, find_by_file_id, load_all, reset_stale_statuses, upsert,
};

// ===== 同步状态常量（对齐 dart SyncStatusType） =====
/// 0=已同步 1=仅云端 2=仅本地 3=同步中 4=失败 5=冲突
pub mod sync_status {
    /// 已完成双向同步。
    pub const SYNCED: i32 = 0;
    /// 仅云端存在。
    pub const CLOUD_ONLY: i32 = 1;
    /// 仅本地存在。
    #[allow(dead_code)]
    pub const LOCAL_ONLY: i32 = 2;
    /// 正在同步。
    pub const SYNCING: i32 = 3;
    /// 最近同步失败。
    pub const FAILED: i32 = 4;
    /// 本地与云端发生冲突。
    pub const CONFLICT: i32 = 5;
    /// 用户已主动删除（tombstone：防云端重建）
    pub const DELETED: i32 = 7;
}

// ===== 传输方向常量（对齐 dart TransferDirectionType） =====
/// 传输方向的持久化数值协议。
pub mod transfer_direction {
    /// 上传到云端。
    pub const UPLOAD: i32 = 0;
    /// 首次从云端下载。
    pub const DOWNLOAD: i32 = 1;
    /// 删除目标资源。
    pub const DELETE: i32 = 2;
    /// 云端新版本覆盖本地已有文件（语义为「更新」，区别于首次拉取的 DOWNLOAD）。
    /// 仅同步引擎的 Download 动作在本地已有真实内容时使用；与 DOWNLOAD 共享下载执行路径。
    pub const DOWNLOAD_UPDATE: i32 = 3;
}

/// 新增上传失败的占位 fileId 前缀。
/// 新增文件上传时云端无真实 fileId，失败时用此前缀 + 相对路径生成占位 fileId 写入 sync_items，
/// 让 retry_failed 能找到失败项。成功上传后由真实 fileId 覆盖（先清占位行）。
/// planner 据此前缀判断「待上传占位项」→ 重新 Upload，绝不删本地。
pub const PENDING_FILE_ID_PREFIX: &str = "pending:";

// ===== 传输状态常量（保持 Tauri/TypeScript 数字协议） =====
/// 传输生命周期的持久化数值协议。
pub mod transfer_state {
    use crate::sync::transfer_state::TransferState;

    /// 等待调度。
    pub const PENDING: i32 = TransferState::Pending as i32;
    /// 正在传输。
    pub const RUNNING: i32 = TransferState::Running as i32;
    /// 等待网络恢复。
    #[allow(dead_code)]
    pub const WAITING_FOR_NETWORK: i32 = TransferState::WaitingForNetwork as i32;
    /// 等待退避到期。
    #[allow(dead_code)]
    pub const BACKING_OFF: i32 = TransferState::BackingOff as i32;
    /// 正在复核远端结果。
    #[allow(dead_code)]
    pub const VERIFYING_REMOTE: i32 = TransferState::VerifyingRemote as i32;
    /// 必须从头重启传输。
    #[allow(dead_code)]
    pub const RESTART_REQUIRED: i32 = TransferState::RestartRequired as i32;
    /// 传输完成。
    pub const COMPLETED: i32 = TransferState::Completed as i32;
    /// 传输失败。
    pub const FAILED: i32 = TransferState::Failed as i32;
    /// 传输已取消。
    pub const CANCELED: i32 = TransferState::Canceled as i32;
}

/// 同步状态项实体（对应 sync_items 表一行）。
/// 对齐 dart `SyncItemEntity`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    /// 云端文件 ID（主键之一）
    pub file_id: String,
    /// 相对挂载根的规范 UTF-8 路径（主键之二）
    pub local_path: String,
    /// 父目录 fileId
    pub parent_folder_id: Option<String>,
    /// 文件名
    pub name: String,
    /// 是否文件夹
    pub is_folder: bool,
    /// 云端大小（字节）
    pub size: i64,
    /// 本地大小（字节，v3，变更检测用）
    pub local_size: Option<i64>,
    /// 本地 SHA256
    pub sha256: Option<String>,
    /// 本地 mtime（毫秒）
    pub local_mtime: Option<i64>,
    /// 云端 editedTime（毫秒）
    pub cloud_edited_time: Option<i64>,
    /// 最后成功同步时间（毫秒）
    pub last_sync_time: Option<i64>,
    /// 同步状态（见 sync_status 常量）
    pub status: i32,
    /// 失败/冲突原因
    pub error_message: Option<String>,
}

/// 传输任务实体（对应 transfer_queue 表一行）。
/// 对齐 dart `TransferTaskEntity`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTask {
    /// 自增主键
    pub id: i64,
    /// 上传/下载（见 transfer_direction 常量）
    pub direction: i32,
    /// 关联的 SyncItem fileId（可空，手动传输无对应项）
    pub file_id: Option<String>,
    /// 本地路径（可空）
    pub local_path: Option<String>,
    /// 文件名
    pub name: String,
    /// 总大小（字节）
    pub total_size: i64,
    /// 已传输（字节）
    pub transferred: i64,
    /// 传输状态（见 transfer_state 常量）
    pub state: i32,
    /// 失败原因
    pub error_message: Option<String>,
    /// 入队时间（毫秒）
    pub created_at: i64,
    /// 完成时间（毫秒）
    pub finished_at: Option<i64>,
    /// 华为 resume 上传会话标识（v2）
    pub server_id: Option<String>,
    /// 华为 uploadId（v2）
    pub upload_id: Option<String>,
    /// 已上传字节偏移（断点续传恢复点，v2）
    pub resume_offset: i64,
    /// 华为 resume 上传 Location 头返回的会话 URL（v4，断点续传必需的唯一 token）。
    /// 新 API 不再在 body 返回 serverId/uploadId，分片 PUT 必须直接用此 URL。
    pub session_url: Option<String>,
    /// 相对挂载根的规范 UTF-8 路径（绝不替代 absolute local_path）。
    pub relative_path: Option<String>,
    /// 规划时的云端父目录 fileId。
    pub parent_file_id: Option<String>,
    /// 持久化操作类型（见 `TransferOperation`）。
    pub operation: Option<i32>,
    /// 入队时本地源 mtime 快照。
    pub source_mtime: Option<i64>,
    /// 入队时本地源大小快照。
    pub source_size: Option<i64>,
    /// 规划时观察到的云端 editedTime。
    pub expected_cloud_edited_time: Option<i64>,
    /// 已消耗的持久化尝试次数。
    pub attempt_count: i64,
    /// 下一次允许重试的时间戳。
    pub next_retry_at: Option<i64>,
    /// 结构化错误类型（见 `TransferErrorKind`）。
    pub error_kind: Option<i32>,
    /// 远端结果复核确认的资源 fileId。
    pub remote_result_file_id: Option<String>,
    /// 乐观并发状态版本。
    pub state_revision: i64,
}

/// 为可空传输列表达不改、设值或清空。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ColumnPatch<T> {
    /// 保留当前数据库值。
    #[default]
    Keep,
    /// 替换当前值。
    Set(T),
    /// 写入 SQL NULL。
    Clear,
}

/// 汇总一次状态转换附带的可变字段更新。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransferPatch {
    pub error_kind: ColumnPatch<TransferErrorKind>,
    pub error_message: ColumnPatch<String>,
    pub next_retry_at: ColumnPatch<i64>,
    pub finished_at: ColumnPatch<i64>,
    pub remote_result_file_id: ColumnPatch<String>,
    pub session_url: ColumnPatch<String>,
    /// `Some` 替换非空计数器，`None` 保留原值。
    pub transferred: Option<i64>,
    /// `Some` 替换非空偏移量，`None` 保留原值。
    pub resume_offset: Option<i64>,
    /// `Some` 替换非空尝试次数，`None` 保留原值。
    pub attempt_count: Option<i64>,
}

/// 仅在任务仍为同一运行版本时写入的进度与会话补丁。
/// 由任务 ID 与生命周期版本保护、仅限 Running 状态的进度和会话补丁。
/// 更新刻意不递增 `state_revision`；生命周期收束会递增，使迟到回调无法通过
/// `(id, revision, Running)` 条件。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunningTransferPatch {
    pub transferred: Option<i64>,
    pub resume_offset: Option<i64>,
    pub server_id: ColumnPatch<String>,
    pub upload_id: ColumnPatch<String>,
    pub session_url: ColumnPatch<String>,
}

// ===== SyncItems 仓储 =====

impl SyncItem {
    /// 按列名解码完整同步记录；缺列或类型不匹配时返回数据库错误。
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            file_id: row.get("file_id")?,
            local_path: row.get("local_path")?,
            parent_folder_id: row.get("parent_folder_id")?,
            name: row.get("name")?,
            is_folder: row.get::<_, i64>("is_folder")? != 0,
            size: row.get("size")?,
            local_size: row.get("local_size")?,
            sha256: row.get("sha256")?,
            local_mtime: row.get("local_mtime")?,
            cloud_edited_time: row.get("cloud_edited_time")?,
            last_sync_time: row.get("last_sync_time")?,
            status: row.get("status")?,
            error_message: row.get("error_message")?,
        })
    }
}

/// 传输队列的状态转换与查询实现。
// 传输队列仓储实现。
/// 传输任务的 SQLite 仓储实现。
mod transfer_queue;
pub(crate) use transfer_queue::transition_transfer_in_transaction;
#[allow(unused_imports)]
pub use transfer_queue::{
    delete_all_transfers, get_transfer_by_id, has_transfer_in_state, insert_transfer,
    list_all_transfers, list_transfers, patch_transfer_in_state, prune_transfer_history,
    transition_transfer, transition_transfer_clearing_upload_session, update_running_transfer,
};
