//! 同步状态命令。

use std::collections::HashMap;

use serde::Serialize;
use specta::Type;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::RuntimeStatus;
use crate::sync::transfer_state::TransferState;

use super::{mount, try_sync_engine, DB, STATUS_AGGREGATOR};

/// 文件在本地挂载目录中的可观察同步状态。
///
/// 前四个取值描述「本地事实」，后三个来自 `transfer_queue` 的在途事实：
/// 文件列表必须能直接回答「谁在传、谁在确认、谁失败了」，而不是让用户去传输队列里猜。
#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileLocalStatus {
    /// 本地项是目录。
    Folder,
    /// 本地真实文件与成功同步基线一致。
    Synced,
    /// 本地项是按需下载占位文件。
    Placeholder,
    /// 本地不存在可信同步文件。
    NotSynced,
    /// 该路径有排队/执行/退避中的传输任务。
    Syncing,
    /// 上传已完成，正在确认云端写入结果。
    Verifying,
    /// 该路径最近一次同步失败，等待重试。
    Failed,
}

/// 同一路径存在多条传输任务时的展示优先级：数字越大越需要用户注意。
fn transfer_state_priority(state: TransferState) -> u8 {
    match state {
        TransferState::Failed => 5,
        TransferState::RestartRequired => 4,
        TransferState::VerifyingRemote => 3,
        TransferState::Running => 2,
        TransferState::BackingOff | TransferState::WaitingForNetwork | TransferState::Pending => 1,
        TransferState::Completed | TransferState::Canceled => 0,
    }
}

/// 把传输状态映射成文件列表的展示状态；终态不覆盖本地事实。
fn status_for_transfer_state(state: TransferState) -> Option<FileLocalStatus> {
    match state {
        TransferState::Failed | TransferState::RestartRequired => Some(FileLocalStatus::Failed),
        TransferState::VerifyingRemote => Some(FileLocalStatus::Verifying),
        TransferState::Running
        | TransferState::Pending
        | TransferState::WaitingForNetwork
        | TransferState::BackingOff => Some(FileLocalStatus::Syncing),
        TransferState::Completed | TransferState::Canceled => None,
    }
}

/// 读取「相对路径 → 最需要展示的传输状态」快照。
///
/// 新文件上传时任务的 `file_id` 为空，只能按 `relative_path` 关联，因此这里
/// 一次性把传输表读成映射，避免逐条查库。
fn transfer_states_by_path(
    conn: &rusqlite::Connection,
) -> AppResult<HashMap<String, TransferState>> {
    let mut by_path: HashMap<String, TransferState> = HashMap::new();
    for task in repository::list_all_transfers(conn)? {
        let (Some(relative_path), Ok(state)) = (task.relative_path.clone(), task.state_kind())
        else {
            continue;
        };
        // 同一路径可能有多条历史任务，保留最需要用户注意的那一条。
        match by_path.get(&relative_path) {
            Some(existing)
                if transfer_state_priority(*existing) >= transfer_state_priority(state) => {}
            _ => {
                by_path.insert(relative_path, state);
            }
        }
    }
    Ok(by_path)
}

/// 查询文件本地同步状态（供前端删除确认用）。
/// 返回 "folder" | "synced" | "placeholder" | "not_synced"
#[tauri::command]
#[specta::specta]
pub fn sync_check_file_local_status(file_id: String) -> AppResult<FileLocalStatus> {
    let conn = DB.lock();
    let record = repository::find_by_file_id(&conn, &file_id)?;
    let Some(record) = record else {
        return Ok(FileLocalStatus::NotSynced);
    };
    if record.is_folder {
        return Ok(FileLocalStatus::Folder);
    }
    // 占位状态只以 xattr 为准；真实的 0 字节文件不能按长度误判成占位符。
    if let Ok(m) = mount() {
        let abs_path = m.mount_dir().join(&record.local_path);
        match std::fs::symlink_metadata(&abs_path) {
            Ok(_) => {
                if crate::mount::manager::is_placeholder_file(&abs_path) {
                    return Ok(FileLocalStatus::Placeholder);
                }
                return Ok(FileLocalStatus::Synced);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::generic(format!("读取本地同步状态失败：{error}"))),
        }
    }
    Ok(FileLocalStatus::NotSynced)
}

/// 批量查询文件同步状态（供前端文件列表状态列展示用）。
///
/// 未挂载同步目录时回退到仅 DB 状态判断。
///
/// 数据库与文件系统严格分段：先在锁内读完基线与在途任务，再释放锁，
/// 把 stat 与 xattr（`is_placeholder_file`）放到阻塞线程池批量执行。
/// 旧实现整段持有全局 DB 锁做逐条文件 I/O，大目录会让引擎写库一起排队（D14）。
#[tauri::command]
#[specta::specta]
pub async fn sync_batch_file_status(
    file_ids: Vec<String>,
) -> AppResult<HashMap<String, FileLocalStatus>> {
    // 第一段：锁内只做 DB 读取，不碰文件系统。
    let mount_dir = mount().ok().map(|m| m.mount_dir().to_path_buf());
    let (records, transfer_states) = {
        let conn = DB.lock();
        // 一次性读取在途/失败任务，按相对路径关联到同步基线。
        let transfer_states = transfer_states_by_path(&conn)?;
        let mut records: HashMap<String, Option<repository::SyncItem>> =
            HashMap::with_capacity(file_ids.len());
        for file_id in &file_ids {
            records.insert(
                file_id.clone(),
                repository::find_by_file_id(&conn, file_id)?,
            );
        }
        (records, transfer_states)
    };

    // 第二段：锁外批量判定本地事实（stat + xattr）。
    let mut pending_local: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut result: HashMap<String, FileLocalStatus> = HashMap::with_capacity(file_ids.len());
    for file_id in &file_ids {
        // 该文件的同步基线；没有基线视为未同步。
        let Some(Some(record)) = records.get(file_id) else {
            result.insert(file_id.clone(), FileLocalStatus::NotSynced);
            continue;
        };
        // 传输中的事实优先于本地文件事实：用户最想知道的是「它在动吗」。
        if let Some(status) = transfer_states
            .get(&record.local_path)
            .and_then(|state| status_for_transfer_state(*state))
        {
            result.insert(file_id.clone(), status);
            continue;
        }
        if record.is_folder {
            result.insert(file_id.clone(), FileLocalStatus::Folder);
            continue;
        }
        match &mount_dir {
            Some(mount_dir) => {
                pending_local.push((file_id.clone(), mount_dir.join(&record.local_path)));
            }
            None => {
                // 未配置挂载目录：仅从 DB 状态判定
                let status = if record.status == repository::sync_status::SYNCED {
                    FileLocalStatus::Synced
                } else {
                    FileLocalStatus::NotSynced
                };
                result.insert(file_id.clone(), status);
            }
        }
    }

    // 第三段：一次阻塞任务处理全部本地路径，避免逐条跨线程调度。
    if !pending_local.is_empty() {
        let local_statuses = tokio::task::spawn_blocking(move || {
            pending_local
                .into_iter()
                .map(|(file_id, abs_path)| {
                    let status = match std::fs::symlink_metadata(&abs_path) {
                        Ok(_) if crate::mount::manager::is_placeholder_file(&abs_path) => {
                            FileLocalStatus::Placeholder
                        }
                        Ok(_) => FileLocalStatus::Synced,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            FileLocalStatus::NotSynced
                        }
                        // 单条路径的 I/O 异常不应让整批状态查询失败，按未同步呈现。
                        Err(error) => {
                            tracing::warn!(%error, path = %abs_path.display(), "读取本地同步状态失败，按未同步处理");
                            FileLocalStatus::NotSynced
                        }
                    };
                    (file_id, status)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| AppError::generic(format!("批量读取本地同步状态失败：{error}")))?;
        result.extend(local_statuses);
    }

    Ok(result)
}

/// 获取完整同步状态快照。
#[tauri::command]
#[specta::specta]
pub async fn sync_state() -> AppResult<SyncGlobalState> {
    // 引擎已启动时以当前运行时状态重新聚合并广播完整快照。
    if let Some(e) = try_sync_engine() {
        return e.recompute_and_broadcast_state();
    }
    // 引擎未启动时复用进程级版本源，从数据库生成完整兜底快照。
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let conn = DB.lock();
    STATUS_AGGREGATOR.snapshot(&conn, RuntimeStatus::default())
}

/// 查询目录下的同步项。
#[tauri::command]
#[specta::specta]
pub fn sync_items_by_folder(folder_local_path: String) -> AppResult<Vec<repository::SyncItem>> {
    let conn = DB.lock();
    let mut stmt = conn
        .prepare("SELECT * FROM sync_items WHERE local_path LIKE ?1")
        .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
    let pattern = format!("{}%", folder_local_path);
    let rows = stmt
        .query_map(rusqlite::params![pattern], repository::SyncItem::from_row)
        .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
    let mut items = Vec::new();
    for item in rows {
        items.push(item.map_err(|error| AppError::generic(format!("读取同步项失败：{error}")))?);
    }
    Ok(items)
}
