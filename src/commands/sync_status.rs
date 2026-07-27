//! 同步状态命令。

use std::collections::HashMap;

use serde::Serialize;
use specta::Type;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::RuntimeStatus;

use super::{mount, try_sync_engine, DB, STATUS_AGGREGATOR};

/// 文件在本地挂载目录中的可观察同步状态。
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
/// 接受文件 ID 列表，返回 fileId → "folder" | "synced" | "placeholder" | "not_synced" 映射。
/// 未挂载同步目录时回退到仅 DB 状态判断。
#[tauri::command]
#[specta::specta]
pub fn sync_batch_file_status(
    file_ids: Vec<String>,
) -> AppResult<HashMap<String, FileLocalStatus>> {
    let conn = DB.lock();
    let mount_opt = mount().ok();
    let mut result: HashMap<String, FileLocalStatus> = HashMap::with_capacity(file_ids.len());

    for file_id in &file_ids {
        let status = match repository::find_by_file_id(&conn, file_id)? {
            None => FileLocalStatus::NotSynced,
            Some(record) => {
                if record.is_folder {
                    FileLocalStatus::Folder
                } else if let Some(ref m) = mount_opt {
                    let abs_path = m.mount_dir().join(&record.local_path);
                    match std::fs::symlink_metadata(&abs_path) {
                        Ok(_) if crate::mount::manager::is_placeholder_file(&abs_path) => {
                            FileLocalStatus::Placeholder
                        }
                        Ok(_) => FileLocalStatus::Synced,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            FileLocalStatus::NotSynced
                        }
                        Err(error) => {
                            return Err(AppError::generic(format!("读取本地同步状态失败：{error}")))
                        }
                    }
                } else {
                    // 未配置挂载目录：仅从 DB 状态判定
                    if record.status == repository::sync_status::SYNCED {
                        FileLocalStatus::Synced
                    } else {
                        FileLocalStatus::NotSynced
                    }
                }
            }
        };
        result.insert(file_id.clone(), status);
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
