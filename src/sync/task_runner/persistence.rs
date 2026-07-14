//! 提供传输任务状态迁移与兼容状态持久化。

use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

impl TaskRunner {
    /// 持久化带错误信息的任务状态迁移。
    pub(super) fn transition_failure(
        &self,
        task: &TransferTask,
        state: TransferState,
        kind: TransferErrorKind,
        message: &str,
    ) -> AppResult<TransferTask> {
        self.transition(
            task.id,
            task.state_revision,
            state,
            TransferPatch {
                error_kind: ColumnPatch::Set(kind),
                error_message: ColumnPatch::Set(message.to_string()),
                finished_at: if state == TransferState::Failed {
                    ColumnPatch::Set(chrono::Utc::now().timestamp_millis())
                } else {
                    ColumnPatch::Clear
                },
                ..Default::default()
            },
        )
    }

    /// 持久化常规任务状态迁移。
    pub(super) fn transition(
        &self,
        task_id: i64,
        expected_revision: i64,
        state: TransferState,
        patch: TransferPatch,
    ) -> AppResult<TransferTask> {
        let task = {
            let conn = self.db.lock();
            repository::transition_transfer(&conn, task_id, expected_revision, state, patch)
                .map_err(transition_error)?
        };
        self.notify_best_effort();
        Ok(task)
    }

    /// 清理上传会话并持久化任务状态迁移。
    pub(super) fn transition_clearing_upload_session(
        &self,
        task_id: i64,
        expected_revision: i64,
        state: TransferState,
        patch: TransferPatch,
    ) -> AppResult<TransferTask> {
        let task = {
            let conn = self.db.lock();
            repository::transition_transfer_clearing_upload_session(
                &conn,
                task_id,
                expected_revision,
                state,
                patch,
            )
            .map_err(transition_error)?
        };
        self.notify_best_effort();
        Ok(task)
    }

    /// 按标识加载传输任务。
    pub(super) fn load(&self, task_id: i64) -> AppResult<TransferTask> {
        repository::get_transfer_by_id(&self.db.lock(), task_id)?
            .ok_or_else(|| AppError::generic("传输任务不存在"))
    }

    /// 列出指定状态的传输任务。
    pub(super) fn list_states(&self, states: &[TransferState]) -> AppResult<Vec<TransferTask>> {
        let all = repository::list_all_transfers(&self.db.lock())?;
        Ok(all
            .into_iter()
            .filter(|task| {
                task.state_kind()
                    .ok()
                    .is_some_and(|state| states.contains(&state))
            })
            .collect())
    }
}

/// 将状态迁移错误转换为应用错误。
pub(super) fn transition_error(error: impl std::fmt::Display) -> AppError {
    AppError::generic(error.to_string())
}

/// 更新旧同步条目的兼容状态。
pub(super) fn update_compatibility_sync_status(
    conn: &rusqlite::Connection,
    task: &TransferTask,
    next_status: i32,
    error_message: Option<&str>,
    expected_status: Option<i32>,
) -> AppResult<()> {
    let relative_path = task
        .relative_path
        .as_deref()
        .ok_or_else(|| AppError::generic("任务缺少相对路径，无法同步兼容状态"))?;
    let file_id = task
        .file_id
        .clone()
        .unwrap_or_else(|| format!("{}{}", repository::PENDING_FILE_ID_PREFIX, relative_path));
    conn.execute(
        "UPDATE sync_items SET status=?1, error_message=?2
         WHERE file_id=?3 AND local_path=?4
           AND (?5 IS NULL OR status=?5)",
        rusqlite::params![
            next_status,
            error_message,
            file_id,
            relative_path,
            expected_status,
        ],
    )
    .map_err(|error| AppError::generic(format!("更新同步兼容状态失败：{error}")))?;
    Ok(())
}

/// 标记旧同步条目的兼容失败状态。
pub(super) fn mark_compatibility_sync_failed(
    conn: &rusqlite::Connection,
    task: &TransferTask,
    error_message: &str,
) -> AppResult<()> {
    let relative_path = task
        .relative_path
        .as_deref()
        .ok_or_else(|| AppError::generic("任务缺少相对路径，无法记录兼容失败"))?;
    let file_id = task
        .file_id
        .clone()
        .unwrap_or_else(|| format!("{}{}", repository::PENDING_FILE_ID_PREFIX, relative_path));
    conn.execute(
        "UPDATE sync_items SET status=?1, error_message=?2
         WHERE file_id=?3 AND local_path=?4
           AND status IN (?5, ?6, ?7, ?8)",
        rusqlite::params![
            repository::sync_status::FAILED,
            error_message,
            file_id,
            relative_path,
            repository::sync_status::SYNCED,
            repository::sync_status::SYNCING,
            repository::sync_status::CLOUD_ONLY,
            repository::sync_status::FAILED,
        ],
    )
    .map_err(|error| AppError::generic(format!("记录同步兼容失败状态失败：{error}")))?;
    Ok(())
}
