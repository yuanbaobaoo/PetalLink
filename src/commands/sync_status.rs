//! 同步状态命令。

use std::collections::HashMap;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::RuntimeStatus;

use super::{mount, try_sync_engine, DB, STATUS_AGGREGATOR};

/// 查询文件本地同步状态（供前端删除确认用）。
/// 返回 "folder" | "synced" | "placeholder" | "not_synced"
#[tauri::command]
pub fn sync_check_file_local_status(file_id: String) -> AppResult<String> {
    let conn = DB.lock();
    let record = repository::find_by_file_id(&conn, &file_id)?;
    let Some(record) = record else {
        return Ok("not_synced".to_string());
    };
    if record.is_folder {
        return Ok("folder".to_string());
    }
    // 占位状态只以 xattr 为准；真实的 0 字节文件不能按长度误判成占位符。
    if let Ok(m) = mount() {
        let abs_path = m.mount_dir().join(&record.local_path);
        match std::fs::symlink_metadata(&abs_path) {
            Ok(_) => {
                if crate::mount::manager::is_placeholder_file(&abs_path) {
                    return Ok("placeholder".to_string());
                }
                return Ok("synced".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::generic(format!("读取本地同步状态失败：{error}"))),
        }
    }
    Ok("not_synced".to_string())
}

/// 批量查询文件同步状态（供前端文件列表状态列展示用）。
/// 接受文件 ID 列表，返回 fileId → "folder" | "synced" | "placeholder" | "not_synced" 映射。
/// 未挂载同步目录时回退到仅 DB 状态判断。
#[tauri::command]
pub fn sync_batch_file_status(file_ids: Vec<String>) -> AppResult<HashMap<String, String>> {
    let conn = DB.lock();
    let mount_opt = mount().ok();
    let mut result: HashMap<String, String> = HashMap::with_capacity(file_ids.len());

    for file_id in &file_ids {
        let status = match repository::find_by_file_id(&conn, file_id)? {
            None => "not_synced",
            Some(record) => {
                if record.is_folder {
                    "folder"
                } else if let Some(ref m) = mount_opt {
                    let abs_path = m.mount_dir().join(&record.local_path);
                    match std::fs::symlink_metadata(&abs_path) {
                        Ok(_) if crate::mount::manager::is_placeholder_file(&abs_path) => {
                            "placeholder"
                        }
                        Ok(_) => "synced",
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "not_synced",
                        Err(error) => {
                            return Err(AppError::generic(format!("读取本地同步状态失败：{error}")))
                        }
                    }
                } else {
                    // 未配置挂载目录：仅从 DB 状态判定
                    if record.status == repository::sync_status::SYNCED {
                        "synced"
                    } else {
                        "not_synced"
                    }
                }
            }
        };
        result.insert(file_id.clone(), status.to_string());
    }

    Ok(result)
}

/// 获取完整同步状态快照。
#[tauri::command]
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
