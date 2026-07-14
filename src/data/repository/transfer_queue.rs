//! 传输任务的持久化状态转换、进度更新与队列查询。

use rusqlite::{params, Connection, OptionalExtension};

use super::{transfer_state, ColumnPatch, RunningTransferPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{
    can_transition, TransferErrorKind, TransferOperation, TransferState, TransitionError,
};

// ===== TransferQueue 仓储 =====

impl TransferTask {
    /// 按列名将当前数据库行解码为完整传输任务。
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            direction: row.get("direction")?,
            file_id: row.get("file_id")?,
            local_path: row.get("local_path")?,
            name: row.get("name")?,
            total_size: row.get("total_size")?,
            transferred: row.get("transferred")?,
            state: row.get("state")?,
            error_message: row.get("error_message")?,
            created_at: row.get("created_at")?,
            finished_at: row.get("finished_at")?,
            server_id: row.get("server_id")?,
            upload_id: row.get("upload_id")?,
            resume_offset: row.get("resume_offset")?,
            session_url: row.get("session_url")?,
            relative_path: row.get("relative_path")?,
            parent_file_id: row.get("parent_file_id")?,
            operation: row.get("operation")?,
            source_mtime: row.get("source_mtime")?,
            source_size: row.get("source_size")?,
            expected_cloud_edited_time: row.get("expected_cloud_edited_time")?,
            attempt_count: row.get("attempt_count")?,
            next_retry_at: row.get("next_retry_at")?,
            error_kind: row.get("error_kind")?,
            remote_result_file_id: row.get("remote_result_file_id")?,
            state_revision: row.get("state_revision")?,
        })
    }

    /// 将持久化数值解析为生命周期状态，并拒绝未知值。
    pub fn state_kind(&self) -> Result<TransferState, TransitionError> {
        TransferState::try_from(self.state)
    }

    /// 将可选持久化数值解析为传输操作，并拒绝未知值。
    pub fn operation_kind(&self) -> Result<Option<TransferOperation>, TransitionError> {
        self.operation.map(TransferOperation::try_from).transpose()
    }

    /// 将可选持久化数值解析为结构化错误类型，并拒绝未知值。
    pub fn error_kind_typed(&self) -> Result<Option<TransferErrorKind>, TransitionError> {
        self.error_kind.map(TransferErrorKind::try_from).transpose()
    }
}

/// 插入传输任务，返回自增 id。
pub fn insert_transfer(conn: &Connection, task: &TransferTask) -> AppResult<i64> {
    db_err!(
        "插入传输任务",
        conn.execute(
            "INSERT INTO transfer_queue
                (direction, file_id, local_path, name, total_size, transferred, state,
                 error_message, created_at, finished_at, server_id, upload_id, resume_offset,
                 session_url, relative_path, parent_file_id, operation, source_mtime,
                 source_size, expected_cloud_edited_time, attempt_count, next_retry_at,
                 error_kind, remote_result_file_id, state_revision)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,
                     ?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            params![
                task.direction,
                task.file_id,
                task.local_path,
                task.name,
                task.total_size,
                task.transferred,
                task.state,
                task.error_message,
                task.created_at,
                task.finished_at,
                task.server_id,
                task.upload_id,
                task.resume_offset,
                task.session_url,
                task.relative_path,
                task.parent_file_id,
                task.operation,
                task.source_mtime,
                task.source_size,
                task.expected_cloud_edited_time,
                task.attempt_count,
                task.next_retry_at,
                task.error_kind,
                task.remote_result_file_id,
                task.state_revision,
            ],
        )
    );
    Ok(conn.last_insert_rowid())
}

/// 按 id 查询单个传输任务。
pub fn get_transfer_by_id(conn: &Connection, id: i64) -> AppResult<Option<TransferTask>> {
    conn.query_row(
        "SELECT * FROM transfer_queue WHERE id = ?1",
        params![id],
        TransferTask::from_row,
    )
    .optional()
    .map_err(|e| AppError::generic(format!("查询失败：{e}")))
}

/// 将可空列三态补丁编码为 SQL 更新模式与可选值。
#[allow(dead_code)]
fn nullable_patch<T>(patch: ColumnPatch<T>) -> (i32, Option<T>) {
    match patch {
        ColumnPatch::Keep => (0, None),
        ColumnPatch::Set(value) => (1, Some(value)),
        ColumnPatch::Clear => (2, None),
    }
}

/// 按任务 ID 与预期状态版本原子转换任务。
/// 状态不匹配或版本陈旧时拒绝写入。
#[allow(dead_code)]
pub fn transition_transfer(
    conn: &Connection,
    task_id: i64,
    expected_revision: i64,
    next_state: TransferState,
    patch: TransferPatch,
) -> Result<TransferTask, TransitionError> {
    let transaction = conn.unchecked_transaction()?;
    let updated = transition_transfer_in_transaction(
        &transaction,
        task_id,
        expected_revision,
        next_state,
        patch,
    )?;
    transaction.commit()?;
    Ok(updated)
}

/// 应用生命周期转换时原子失效全部持久化断点上传身份。
/// 仅当远端复核确认目标写入不存在后方可调用；单事务可避免新尝试观察到半清理会话。
pub fn transition_transfer_clearing_upload_session(
    conn: &Connection,
    task_id: i64,
    expected_revision: i64,
    next_state: TransferState,
    mut patch: TransferPatch,
) -> Result<TransferTask, TransitionError> {
    patch.session_url = ColumnPatch::Clear;
    patch.transferred = Some(0);
    patch.resume_offset = Some(0);

    let transaction = conn.unchecked_transaction()?;
    let transitioned = transition_transfer_in_transaction(
        &transaction,
        task_id,
        expected_revision,
        next_state,
        patch,
    )?;
    let changed = transaction.execute(
        "UPDATE transfer_queue
         SET server_id=NULL, upload_id=NULL
         WHERE id=?1 AND state_revision=?2 AND state=?3",
        params![task_id, transitioned.state_revision, i32::from(next_state)],
    )?;
    if changed != 1 {
        return Err(TransitionError::Database {
            message: format!("清理上传会话失败：task {task_id} 未保持目标状态"),
        });
    }
    let updated = transaction
        .query_row(
            "SELECT * FROM transfer_queue WHERE id=?1",
            params![task_id],
            TransferTask::from_row,
        )
        .optional()?
        .ok_or(TransitionError::NotFound { task_id })?;
    transaction.commit()?;
    Ok(updated)
}

/// 在生命周期不变时更新错误与重试事实，并校验任务版本。
/// 不改变生命周期状态，仅更新可变错误与重试字段。
/// 只用于被拒绝的手动重试仍保持 Failed 等同状态事实。
/// 生命周期变化必须使用 [`transition_transfer`]。
pub fn patch_transfer_in_state(
    conn: &Connection,
    task_id: i64,
    expected_revision: i64,
    expected_state: TransferState,
    patch: TransferPatch,
) -> Result<TransferTask, TransitionError> {
    let current = conn
        .query_row(
            "SELECT * FROM transfer_queue WHERE id=?1",
            params![task_id],
            TransferTask::from_row,
        )
        .optional()?
        .ok_or(TransitionError::NotFound { task_id })?;
    if current.state_revision != expected_revision || current.state_kind()? != expected_state {
        return Err(TransitionError::StaleRevision {
            task_id,
            expected_revision,
        });
    }

    let TransferPatch {
        error_kind,
        error_message,
        next_retry_at,
        finished_at,
        remote_result_file_id,
        session_url,
        transferred,
        resume_offset,
        attempt_count,
    } = patch;
    let (error_kind_mode, error_kind) = nullable_patch(error_kind);
    let error_kind = error_kind.map(i32::from);
    let (error_message_mode, error_message) = nullable_patch(error_message);
    let (next_retry_at_mode, next_retry_at) = nullable_patch(next_retry_at);
    let (finished_at_mode, finished_at) = nullable_patch(finished_at);
    let (remote_result_file_id_mode, remote_result_file_id) = nullable_patch(remote_result_file_id);
    let (session_url_mode, session_url) = nullable_patch(session_url);
    let changed = conn.execute(
        "UPDATE transfer_queue SET
            error_kind=CASE ?1 WHEN 0 THEN error_kind WHEN 1 THEN ?2 ELSE NULL END,
            error_message=CASE ?3 WHEN 0 THEN error_message WHEN 1 THEN ?4 ELSE NULL END,
            next_retry_at=CASE ?5 WHEN 0 THEN next_retry_at WHEN 1 THEN ?6 ELSE NULL END,
            finished_at=CASE ?7 WHEN 0 THEN finished_at WHEN 1 THEN ?8 ELSE NULL END,
            remote_result_file_id=CASE ?9 WHEN 0 THEN remote_result_file_id WHEN 1 THEN ?10 ELSE NULL END,
            session_url=CASE ?11 WHEN 0 THEN session_url WHEN 1 THEN ?12 ELSE NULL END,
            transferred=CASE WHEN ?13 IS NULL THEN transferred ELSE ?13 END,
            resume_offset=CASE WHEN ?14 IS NULL THEN resume_offset ELSE ?14 END,
            attempt_count=CASE WHEN ?15 IS NULL THEN attempt_count ELSE ?15 END,
            state_revision=state_revision+1
         WHERE id=?16 AND state_revision=?17 AND state=?18",
        params![
            error_kind_mode,
            error_kind,
            error_message_mode,
            error_message,
            next_retry_at_mode,
            next_retry_at,
            finished_at_mode,
            finished_at,
            remote_result_file_id_mode,
            remote_result_file_id,
            session_url_mode,
            session_url,
            transferred,
            resume_offset,
            attempt_count,
            task_id,
            expected_revision,
            i32::from(expected_state),
        ],
    )?;
    if changed != 1 {
        return Err(TransitionError::StaleRevision {
            task_id,
            expected_revision,
        });
    }
    conn.query_row(
        "SELECT * FROM transfer_queue WHERE id=?1",
        params![task_id],
        TransferTask::from_row,
    )
    .optional()?
    .ok_or(TransitionError::NotFound { task_id })
}

/// 仅当精确任务版本仍为 Running 时更新进度与会话数据。
pub fn update_running_transfer(
    conn: &Connection,
    task_id: i64,
    expected_revision: i64,
    patch: RunningTransferPatch,
) -> Result<TransferTask, TransitionError> {
    let (server_mode, server_id) = nullable_patch(patch.server_id);
    let (upload_mode, upload_id) = nullable_patch(patch.upload_id);
    let (session_mode, session_url) = nullable_patch(patch.session_url);
    let changed = conn.execute(
        "UPDATE transfer_queue SET
            transferred=CASE WHEN ?1 IS NULL THEN transferred ELSE ?1 END,
            resume_offset=CASE WHEN ?2 IS NULL THEN resume_offset ELSE ?2 END,
            server_id=CASE ?3 WHEN 0 THEN server_id WHEN 1 THEN ?4 ELSE NULL END,
            upload_id=CASE ?5 WHEN 0 THEN upload_id WHEN 1 THEN ?6 ELSE NULL END,
            session_url=CASE ?7 WHEN 0 THEN session_url WHEN 1 THEN ?8 ELSE NULL END
         WHERE id=?9 AND state_revision=?10 AND state=?11",
        params![
            patch.transferred,
            patch.resume_offset,
            server_mode,
            server_id,
            upload_mode,
            upload_id,
            session_mode,
            session_url,
            task_id,
            expected_revision,
            i32::from(TransferState::Running),
        ],
    )?;
    if changed != 1 {
        return Err(TransitionError::StaleRevision {
            task_id,
            expected_revision,
        });
    }
    conn.query_row(
        "SELECT * FROM transfer_queue WHERE id=?1",
        params![task_id],
        TransferTask::from_row,
    )
    .optional()?
    .ok_or(TransitionError::NotFound { task_id })
}

/// 供必须在同一事务中收束关联行的调用方执行转换核心逻辑。
pub(crate) fn transition_transfer_in_transaction(
    conn: &Connection,
    task_id: i64,
    expected_revision: i64,
    next_state: TransferState,
    patch: TransferPatch,
) -> Result<TransferTask, TransitionError> {
    let current = conn
        .query_row(
            "SELECT * FROM transfer_queue WHERE id=?1",
            params![task_id],
            TransferTask::from_row,
        )
        .optional()?
        .ok_or(TransitionError::NotFound { task_id })?;

    if current.state_revision != expected_revision {
        return Err(TransitionError::StaleRevision {
            task_id,
            expected_revision,
        });
    }

    let from = current.state_kind()?;
    if !can_transition(from, next_state) {
        return Err(TransitionError::IllegalTransition {
            from,
            to: next_state,
        });
    }

    let TransferPatch {
        error_kind,
        error_message,
        next_retry_at,
        finished_at,
        remote_result_file_id,
        session_url,
        transferred,
        resume_offset,
        attempt_count,
    } = patch;
    let (error_kind_mode, error_kind) = nullable_patch(error_kind);
    let error_kind = error_kind.map(i32::from);
    let (error_message_mode, error_message) = nullable_patch(error_message);
    let (next_retry_at_mode, next_retry_at) = nullable_patch(next_retry_at);
    let (finished_at_mode, finished_at) = nullable_patch(finished_at);
    let (remote_result_file_id_mode, remote_result_file_id) = nullable_patch(remote_result_file_id);
    let (session_url_mode, session_url) = nullable_patch(session_url);

    let changed = conn.execute(
        "UPDATE transfer_queue SET
            state=?1,
            error_kind=CASE ?2 WHEN 0 THEN error_kind WHEN 1 THEN ?3 ELSE NULL END,
            error_message=CASE ?4 WHEN 0 THEN error_message WHEN 1 THEN ?5 ELSE NULL END,
            next_retry_at=CASE ?6 WHEN 0 THEN next_retry_at WHEN 1 THEN ?7 ELSE NULL END,
            finished_at=CASE ?8 WHEN 0 THEN finished_at WHEN 1 THEN ?9 ELSE NULL END,
            remote_result_file_id=CASE ?10 WHEN 0 THEN remote_result_file_id WHEN 1 THEN ?11 ELSE NULL END,
            session_url=CASE ?12 WHEN 0 THEN session_url WHEN 1 THEN ?13 ELSE NULL END,
            transferred=CASE WHEN ?14 IS NULL THEN transferred ELSE ?14 END,
            resume_offset=CASE WHEN ?15 IS NULL THEN resume_offset ELSE ?15 END,
            attempt_count=CASE WHEN ?16 IS NULL THEN attempt_count ELSE ?16 END,
            state_revision=state_revision+1
         WHERE id=?17 AND state_revision=?18",
        params![
            i32::from(next_state),
            error_kind_mode,
            error_kind,
            error_message_mode,
            error_message,
            next_retry_at_mode,
            next_retry_at,
            finished_at_mode,
            finished_at,
            remote_result_file_id_mode,
            remote_result_file_id,
            session_url_mode,
            session_url,
            transferred,
            resume_offset,
            attempt_count,
            task_id,
            expected_revision,
        ],
    )?;
    if changed != 1 {
        return Err(TransitionError::StaleRevision {
            task_id,
            expected_revision,
        });
    }

    let updated = conn
        .query_row(
            "SELECT * FROM transfer_queue WHERE id=?1",
            params![task_id],
            TransferTask::from_row,
        )
        .optional()?
        .ok_or(TransitionError::NotFound { task_id })?;
    Ok(updated)
}

/// 按状态+方向查询传输任务（按 created_at 倒序）。对齐 dart 传输队列列表。
#[allow(dead_code)]
pub fn list_transfers(
    conn: &Connection,
    direction: Option<i32>,
    state_filter: Option<i32>,
) -> AppResult<Vec<TransferTask>> {
    match (direction, state_filter) {
        (Some(d), Some(s)) => {
            let mut stmt = db_err!(
                "查询",
                conn.prepare(
                    "SELECT * FROM transfer_queue WHERE direction = ?1 AND state = ?2 ORDER BY created_at DESC",
                )
            );
            collect_tasks(stmt.query_map(params![d, s], TransferTask::from_row))
        }
        (Some(d), None) => list_transfers_with_dir(conn, d),
        (None, Some(s)) => list_transfers_with_state(conn, s),
        (None, None) => list_all_transfers(conn),
    }
}

/// 收集迭代结果为 Vec<TransferTask>；任一行损坏时整体失败，禁止把活动任务漏读为空闲。
/// 接收 query_map 返回的 MappedRows（迭代产出 rusqlite::Result<TransferTask>）。
fn collect_tasks<I>(rows_result: rusqlite::Result<I>) -> AppResult<Vec<TransferTask>>
where
    I: Iterator<Item = rusqlite::Result<TransferTask>>,
{
    let rows = db_err!("查询", rows_result);
    let mut tasks = Vec::new();
    for task in rows {
        tasks.push(task.map_err(|error| AppError::generic(format!("读取传输任务失败：{error}")))?);
    }
    Ok(tasks)
}

/// 按方向查询传输任务。
#[allow(dead_code)]
fn list_transfers_with_dir(conn: &Connection, d: i32) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue WHERE direction = ?1 ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map(params![d], TransferTask::from_row))
}

/// 按持久化状态查询传输任务。
#[allow(dead_code)]
fn list_transfers_with_state(conn: &Connection, s: i32) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue WHERE state = ?1 ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map(params![s], TransferTask::from_row))
}

/// 查询指定持久化状态是否至少存在一个传输任务。
pub fn has_transfer_in_state(conn: &Connection, state: TransferState) -> AppResult<bool> {
    let exists: i64 = db_err!(
        "查询",
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM transfer_queue WHERE state=?1)",
            params![i32::from(state)],
            |row| row.get(0),
        )
    );
    Ok(exists != 0)
}

/// 查询所有传输任务（created_at 倒序）。
pub fn list_all_transfers(conn: &Connection) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map([], TransferTask::from_row))
}

/// 清空传输队列表。
pub fn delete_all_transfers(conn: &Connection) -> AppResult<()> {
    db_err!("清空", conn.execute("DELETE FROM transfer_queue", []));
    Ok(())
}

/// 修剪传输历史：保留最近 N 条已结束任务（completed/failed/canceled）。
/// 对齐 dart `_pruneTransferHistory`（保留最近 100 条）。
pub fn prune_transfer_history(conn: &Connection, keep: usize) -> AppResult<()> {
    db_err!(
        "修剪历史",
        conn.execute(
            "DELETE FROM transfer_queue
             WHERE id IN (
                SELECT id FROM transfer_queue
                WHERE state IN (?1, ?2, ?3)
                ORDER BY id DESC
                LIMIT -1 OFFSET ?4
             )",
            params![
                transfer_state::COMPLETED,
                transfer_state::FAILED,
                transfer_state::CANCELED,
                keep as i64,
            ],
        )
    );
    Ok(())
}
