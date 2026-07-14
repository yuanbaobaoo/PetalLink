//! 传输命令。

use tauri::AppHandle;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::{RuntimeStatus, StatusAggregator};

use super::{emit_sync_state, sync_engine, try_sync_engine, DB, STATUS_AGGREGATOR};

/// 列出传输任务。
#[tauri::command]
pub fn transfer_list_all() -> AppResult<Vec<repository::TransferTask>> {
    let conn = DB.lock();
    repository::list_all_transfers(&conn)
}

/// 检查活动传输。
#[tauri::command]
pub fn transfer_has_active() -> AppResult<bool> {
    let conn = DB.lock();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transfer_queue WHERE state IN (?1, ?2)",
            rusqlite::params![
                repository::transfer_state::PENDING,
                repository::transfer_state::RUNNING
            ],
            |row| row.get(0),
        )
        .map_err(|e| AppError::generic(format!("查询传输状态失败：{e}")))?;
    Ok(count > 0)
}

/// 删除指定终态的传输历史，并在同一数据库视图上生成状态快照。
fn clear_transfer_history_and_snapshot(
    conn: &rusqlite::Connection,
    aggregator: &StatusAggregator,
    include_completed: bool,
    include_failed: bool,
) -> AppResult<SyncGlobalState> {
    conn.execute(
        "DELETE FROM transfer_queue
         WHERE (?1=1 AND state=?2) OR (?3=1 AND state=?4)",
        rusqlite::params![
            include_completed as i32,
            i32::from(crate::sync::transfer_state::TransferState::Completed),
            include_failed as i32,
            i32::from(crate::sync::transfer_state::TransferState::Failed),
        ],
    )
    .map_err(|error| AppError::generic(format!("清除传输历史失败：{error}")))?;
    aggregator.snapshot(conn, RuntimeStatus::default())
}

/// 清除已完成传输。
#[tauri::command]
pub fn transfer_clear_completed(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(true, false)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, true, false)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

/// 清除失败传输。
#[tauri::command]
pub fn transfer_clear_failed(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(false, true)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, false, true)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

/// 清除已结束传输。
#[tauri::command]
pub fn transfer_clear_finished(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(true, true)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, true, true)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

/// 重试传输任务。
#[tauri::command]
pub async fn transfer_retry(task_id: i64) -> AppResult<()> {
    let engine = sync_engine()?;
    engine.retry_transfer(task_id).await
}
