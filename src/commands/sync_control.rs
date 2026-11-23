//! 同步控制命令。

use tauri::AppHandle;

use crate::error::AppResult;
use crate::sync::task_runner::RetryAllSummary;

use super::sync_engine;

/// 触发云端树全量刷新与同步周期。
#[tauri::command]
#[specta::specta]
pub async fn sync_manual_refresh(_app: AppHandle) -> AppResult<()> {
    let e = sync_engine()?;
    e.trigger_manual_sync().await
}

/// 清理一个已失效的本地占位文件。
///
/// 只在「本地仍是 0 字节占位符、且云端确实没有该路径」时才会删除；
/// 云端副本若已恢复，命令会拒绝并提示，避免把用户刚恢复的内容二次删掉。
#[tauri::command]
#[specta::specta]
pub async fn sync_cleanup_placeholder(rel_path: String) -> AppResult<bool> {
    let e = sync_engine()?;
    e.cleanup_invalid_placeholder(&rel_path)
}

/// 确认执行本轮被安全闸门拦下的大批量云端删除。
///
/// 只有在状态里看到 `blocked_cloud_deletes > 0` 时才应该调用；确认后引擎会立刻
/// 重新规划一次，这批删除才会真正下发。
#[tauri::command]
#[specta::specta]
pub async fn sync_approve_mass_delete() -> AppResult<()> {
    let e = sync_engine()?;
    e.approve_mass_delete();
    e.request_background_cycle("approve-mass-delete");
    Ok(())
}

/// 重试全部失败任务：全部入队后立即返回汇总，不等待执行。
#[tauri::command]
#[specta::specta]
pub async fn sync_retry_failed() -> AppResult<RetryAllSummary> {
    let e = sync_engine()?;
    e.retry_all_failed().await
}
