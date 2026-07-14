//! 同步控制命令。

use tauri::AppHandle;

use crate::error::AppResult;

use super::sync_engine;

/// 触发云端树全量刷新与同步周期。
#[tauri::command]
pub async fn sync_manual_refresh(_app: AppHandle) -> AppResult<()> {
    let e = sync_engine()?;
    e.trigger_manual_sync().await
}

/// 重试失败的同步任务。
#[tauri::command]
pub async fn sync_retry_failed() -> AppResult<()> {
    let e = sync_engine()?;
    e.retry_failed().await
}
