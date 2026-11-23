//! 目录同步命令。
//!
//! 用户点击「双端对齐」只产生一条**作业**：入队立即返回，永不因引擎忙而报错。
//! 遍历与投递由 `sync/engine/folder_queue.rs` 的 worker 在后台完成，
//! 传输本身进入统一的传输队列。

use tauri::AppHandle;
use tauri_specta::Event;

use crate::data::repository::folder_sync_jobs::FolderSyncJob;
use crate::error::AppResult;
use crate::ipc::FolderSyncJobsChangedEvent;

use super::sync_engine;

/// 入队一次目录双端对齐。
///
/// 返回作业快照（含排队状态）；同一目录重复点击会复用活跃作业。
#[tauri::command]
#[specta::specta]
pub async fn sync_folder_enqueue(
    app: AppHandle,
    folder_id: String,
    rel_path: String,
) -> AppResult<FolderSyncJob> {
    let engine = sync_engine()?;
    let job = engine.enqueue_folder_sync(&folder_id, &rel_path)?;
    // 入队后立即广播，弹层无需等待 worker 开始规划就能显示排队状态。
    let _ = FolderSyncJobsChangedEvent.emit(&app);
    Ok(job)
}

/// 列出目录同步作业（活跃在前，其后是最近的终态历史）。
#[tauri::command]
#[specta::specta]
pub fn sync_folder_jobs() -> AppResult<Vec<FolderSyncJob>> {
    let engine = sync_engine()?;
    engine.list_folder_sync_jobs()
}

/// 取消一个排队中或规划中的目录同步作业。
#[tauri::command]
#[specta::specta]
pub async fn sync_folder_cancel(app: AppHandle, job_id: i64) -> AppResult<bool> {
    let engine = sync_engine()?;
    let cancelled = engine.cancel_folder_sync_job(job_id)?;
    if cancelled {
        let _ = FolderSyncJobsChangedEvent.emit(&app);
    }
    Ok(cancelled)
}
