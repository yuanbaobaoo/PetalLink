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
///
/// 新文件首次上传失败时 `sync_items` 还没有对应基线行，只做 UPDATE 会让失败
/// 在文件视图里彻底消失；因此 UPDATE 未命中时补写一条 FAILED 行。
/// 已存在行（含冲突/已删除等状态）一律不覆盖，避免把更精确的状态降级成失败。
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
    let updated = conn
        .execute(
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
    if updated == 0 {
        // 没有可更新的基线行：补写失败记录，让文件视图能看见这次失败。
        conn.execute(
            "INSERT INTO sync_items (
                file_id, local_path, parent_folder_id, name, is_folder, size, status, error_message
             ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)
             ON CONFLICT(file_id, local_path) DO NOTHING",
            rusqlite::params![
                file_id,
                relative_path,
                task.parent_file_id,
                task.name,
                task.total_size,
                repository::sync_status::FAILED,
                error_message,
            ],
        )
        .map_err(|error| AppError::generic(format!("补写同步失败记录失败：{error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::transfer_state::TransferOperation;

    /// 建立只含同步基线与任务表的最小数据库。
    fn open_database() -> rusqlite::Connection {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sync_items (
                    file_id TEXT NOT NULL,
                    local_path TEXT NOT NULL,
                    parent_folder_id TEXT,
                    name TEXT NOT NULL,
                    is_folder INTEGER NOT NULL DEFAULT 0,
                    size INTEGER NOT NULL DEFAULT 0,
                    local_size INTEGER,
                    sha256 TEXT,
                    local_mtime INTEGER,
                    cloud_edited_time INTEGER,
                    last_sync_time INTEGER,
                    status INTEGER NOT NULL DEFAULT 0,
                    error_message TEXT,
                    PRIMARY KEY (file_id, local_path)
                );",
            )
            .unwrap();
        connection
    }

    /// 构造一个失败的上传任务；file_id 为空表示新文件。
    fn task(relative_path: &str, file_id: Option<&str>) -> TransferTask {
        TransferTask {
            id: 1,
            direction: 0,
            file_id: file_id.map(str::to_string),
            local_path: Some(format!("/mount/{relative_path}")),
            name: relative_path.rsplit('/').next().unwrap().to_string(),
            total_size: 8,
            transferred: 0,
            state: i32::from(TransferState::Failed),
            error_message: None,
            created_at: 1,
            finished_at: Some(2),
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(relative_path.to_string()),
            parent_file_id: Some("parent-folder-id".to_string()),
            operation: Some(i32::from(TransferOperation::Create)),
            source_mtime: Some(1),
            source_size: Some(8),
            expected_cloud_edited_time: None,
            attempt_count: 1,
            verify_attempt_count: 0,
            next_retry_at: None,
            error_kind: Some(i32::from(TransferErrorKind::Network)),
            remote_result_file_id: None,
            state_revision: 1,
        }
    }

    /// 新文件首次上传失败时没有基线行，必须补写一条 FAILED，否则文件视图里彻底消失。
    #[test]
    fn failed_first_upload_inserts_missing_baseline_row() {
        let connection = open_database();
        let upload = task("videos/new.mp4", None);

        mark_compatibility_sync_failed(&connection, &upload, "网络连接失败").unwrap();

        let row: (String, i32, Option<String>) = connection
            .query_row(
                "SELECT file_id, status, error_message FROM sync_items WHERE local_path=?1",
                ["videos/new.mp4"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            row.0,
            format!("{}{}", repository::PENDING_FILE_ID_PREFIX, "videos/new.mp4")
        );
        assert_eq!(row.1, repository::sync_status::FAILED);
        assert_eq!(row.2.as_deref(), Some("网络连接失败"));
    }

    /// 已有基线行只更新状态，不新增第二行。
    #[test]
    fn failed_upload_updates_existing_baseline_row() {
        let connection = open_database();
        connection
            .execute(
                "INSERT INTO sync_items (file_id, local_path, name, status)
                 VALUES ('remote-id', 'videos/old.mp4', 'old.mp4', ?1)",
                [repository::sync_status::SYNCING],
            )
            .unwrap();
        let upload = task("videos/old.mp4", Some("remote-id"));

        mark_compatibility_sync_failed(&connection, &upload, "服务器异常").unwrap();

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sync_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "已有基线行不得再插入重复记录");
        let row: (i32, Option<String>) = connection
            .query_row(
                "SELECT status, error_message FROM sync_items WHERE file_id='remote-id'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, repository::sync_status::FAILED);
        assert_eq!(row.1.as_deref(), Some("服务器异常"));
    }
}
