//! 传输命令。

use tauri::AppHandle;

use crate::data::repository;
use crate::error::AppResult;
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::{RuntimeStatus, StatusAggregator};
use crate::sync::task_runner::RetryEnqueued;

use super::{emit_sync_state, sync_engine, try_sync_engine, DB, STATUS_AGGREGATOR};

/// 列出传输任务。
#[tauri::command]
#[specta::specta]
pub fn transfer_list_all() -> AppResult<Vec<repository::TransferTask>> {
    let conn = DB.lock();
    repository::list_all_transfers(&conn)
}

/// 检查活动传输。
#[tauri::command]
#[specta::specta]
pub fn transfer_has_active() -> AppResult<bool> {
    Ok(super::active_transfer_count()? > 0)
}

/// 删除指定终态的传输历史，并在同一数据库视图上生成状态快照。
fn clear_transfer_history_and_snapshot(
    conn: &rusqlite::Connection,
    aggregator: &StatusAggregator,
    include_completed: bool,
    include_failed: bool,
) -> AppResult<SyncGlobalState> {
    repository::clear_terminal_transfers(conn, include_completed, include_failed)?;
    aggregator.snapshot(conn, RuntimeStatus::default())
}

/// 清除指定终态传输并广播最新状态；引擎在线时走引擎路径保证任务视图一致。
fn clear_transfers(
    app: &AppHandle,
    include_completed: bool,
    include_failed: bool,
) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(include_completed, include_failed)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(
            &conn,
            &STATUS_AGGREGATOR,
            include_completed,
            include_failed,
        )?
    };
    emit_sync_state(app, &snapshot);
    Ok(())
}

/// 清除已完成传输。
#[tauri::command]
#[specta::specta]
pub fn transfer_clear_completed(app: AppHandle) -> AppResult<()> {
    clear_transfers(&app, true, false)
}

/// 清除失败传输。
#[tauri::command]
#[specta::specta]
pub fn transfer_clear_failed(app: AppHandle) -> AppResult<()> {
    clear_transfers(&app, false, true)
}

/// 清除已结束传输。
#[tauri::command]
#[specta::specta]
pub fn transfer_clear_finished(app: AppHandle) -> AppResult<()> {
    clear_transfers(&app, true, true)
}

/// 重试传输任务。
///
/// 只做幂等入队并立即返回，执行由后台调度；重复点击返回 `AlreadyQueued`。
#[tauri::command]
#[specta::specta]
pub async fn transfer_retry(task_id: i64) -> AppResult<RetryEnqueued> {
    let engine = sync_engine()?;
    engine.retry_transfer(task_id).await
}

/// 同名冲突决策：覆盖远端（远端旧版自动改名为云端副本保留，不丢数据）。
#[tauri::command]
#[specta::specta]
pub async fn transfer_overwrite_remote(task_id: i64) -> AppResult<()> {
    let engine = sync_engine()?;
    engine.overwrite_remote_conflict(task_id).await
}

/// 同名冲突决策：保留两者（本地文件改名为本地副本后作为新文件上传，云端原文件不动）。
#[tauri::command]
#[specta::specta]
pub async fn transfer_keep_both(task_id: i64) -> AppResult<()> {
    let engine = sync_engine()?;
    engine.keep_both_conflict(task_id).await
}

/// 取消「需要重新检查」或失败的传输任务。
#[tauri::command]
#[specta::specta]
pub async fn transfer_cancel(task_id: i64) -> AppResult<()> {
    let engine = sync_engine()?;
    engine.cancel_transfer_task(task_id).await
}

/// 批量取消全部失败任务，返回实际取消数量。
///
/// 队列里几百条失败任务只有「重试全部」时，用户无法清走已知脏任务；
/// 取消保留历史行但释放路径占用，让被隔离的同步动作继续执行。
#[tauri::command]
#[specta::specta]
pub async fn transfer_cancel_all_failed() -> AppResult<u32> {
    let engine = sync_engine()?;
    engine.cancel_all_failed_tasks().await
}
