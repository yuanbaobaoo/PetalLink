//! 提供任务入队、重试准入与 Running 仲裁。

use std::sync::Arc;

use super::contracts::{
    EnqueuedTaskOutcome, TaskActivityGate, TaskDisposition, TaskExecutionOutcome,
};
use super::persistence::{transition_error, update_compatibility_sync_status};
use super::preflight::PreflightFailure;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

/// 入队时对既有意图、重新规划与新增任务的仲裁结果。
pub(super) enum ExistingOrInsertedTask {
    Existing(Box<TransferTask>),
    Replanned(Box<TransferTask>),
    Blocked(i64),
    Inserted(i64),
}

/// 任务尝试进入 Running 时的门禁结果。
pub(super) enum RunningGateOutcome {
    Running(Box<TransferTask>),
    Blocked,
}

/// 判断持久状态是否会阻止同路径新意图。
pub(super) fn is_path_blocking_state(state: TransferState) -> bool {
    matches!(
        state,
        TransferState::Pending
            | TransferState::Running
            | TransferState::WaitingForNetwork
            | TransferState::BackingOff
            | TransferState::VerifyingRemote
    )
}

/// 按操作、路径、远程身份与源快照判断两任务是否同一意图。
pub(super) fn same_transfer_intent(left: &TransferTask, right: &TransferTask) -> bool {
    if left.relative_path != right.relative_path
        || left.local_path != right.local_path
        || left.name != right.name
        || left.direction != right.direction
        || left.operation != right.operation
        || left.file_id != right.file_id
        || left.total_size != right.total_size
    {
        return false;
    }
    match left.operation_kind().ok().flatten() {
        Some(TransferOperation::Create | TransferOperation::Update) => {
            left.parent_file_id == right.parent_file_id
                && left.source_mtime == right.source_mtime
                && left.source_size == right.source_size
                && (left.operation_kind().ok().flatten() != Some(TransferOperation::Update)
                    || left.expected_cloud_edited_time == right.expected_cloud_edited_time)
        }
        Some(TransferOperation::Download | TransferOperation::DownloadUpdate) => {
            left.parent_file_id == right.parent_file_id
                && left.expected_cloud_edited_time == right.expected_cloud_edited_time
        }
        _ => false,
    }
}

/// 判断写任务是否已持久可能提交的远程结果。
pub(super) fn has_ambiguous_remote_write_result(task: &TransferTask) -> bool {
    matches!(
        task.operation_kind().ok().flatten(),
        Some(TransferOperation::Create | TransferOperation::Update)
    ) && has_persisted_remote_result(task)
}

/// 判断任务是否保存了非空远程结果 ID。
pub(super) fn has_persisted_remote_result(task: &TransferTask) -> bool {
    task.remote_result_file_id
        .as_deref()
        .is_some_and(|file_id| !file_id.trim().is_empty())
}

/// 将活动持久状态映射为调度去向。
pub(super) fn active_task_disposition(state: TransferState) -> Option<TaskDisposition> {
    match state {
        TransferState::Pending => Some(TaskDisposition::Pending),
        TransferState::Running => Some(TaskDisposition::Running),
        TransferState::WaitingForNetwork => Some(TaskDisposition::WaitingForNetwork),
        TransferState::BackingOff => Some(TaskDisposition::BackingOff),
        TransferState::VerifyingRemote => Some(TaskDisposition::VerifyingRemote),
        TransferState::RestartRequired => Some(TaskDisposition::RestartRequired),
        TransferState::Completed | TransferState::Failed | TransferState::Canceled => None,
    }
}

impl TaskRunner {
    /// 设置任务活动准入门。
    pub fn set_activity_gate(&self, activity_gate: Arc<dyn TaskActivityGate>) {
        *self.activity_gate.write() = Some(activity_gate);
    }

    /// 获取任务活动许可。
    pub(super) fn begin_activity(&self, task: &TransferTask) -> AppResult<Option<Box<dyn Send>>> {
        self.activity_gate
            .read()
            .clone()
            .map(|gate| gate.begin(task.relative_path.as_deref()))
            .transpose()
    }

    /// 持久化 Pending 意图后执行对应任务行。
    pub async fn enqueue_and_run(&self, task: TransferTask) -> AppResult<EnqueuedTaskOutcome> {
        if task.id != 0
            || task.state_revision != 0
            || task.state_kind().map_err(transition_error)? != TransferState::Pending
        {
            self.notify_rejection();
            return Err(AppError::generic(
                "新传输意图必须是 id=0/revision=0 的 Pending 任务",
            ));
        }
        // 在持久化入队前完成准入，避免独占路径许可释放后立即执行的 Pending 行绕过限制。
        let _enqueue_activity = self.begin_activity(&task)?;
        let existing_or_task_id = {
            let conn = self.db.lock();
            let path_tasks = match task.relative_path.as_deref() {
                Some(relative_path) => repository::list_all_transfers(&conn)?
                    .into_iter()
                    .filter(|candidate| candidate.relative_path.as_deref() == Some(relative_path))
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            };
            let blocking = path_tasks
                .iter()
                .filter(|candidate| candidate.state_kind().is_ok_and(is_path_blocking_state))
                .collect::<Vec<_>>();
            if let Some(inflight) = blocking.iter().find(|candidate| {
                candidate.state_kind().is_ok_and(|state| {
                    matches!(
                        state,
                        TransferState::Running | TransferState::VerifyingRemote
                    )
                })
            }) {
                if same_transfer_intent(inflight, &task) {
                    Ok(ExistingOrInsertedTask::Existing(Box::new(
                        (*inflight).clone(),
                    )))
                } else {
                    Ok(ExistingOrInsertedTask::Blocked(inflight.id))
                }
            } else if let Some(ambiguous_restart) = path_tasks.iter().find(|candidate| {
                candidate.state_kind() == Ok(TransferState::RestartRequired)
                    && has_ambiguous_remote_write_result(candidate)
            }) {
                self.promote_restart_to_verifying(&conn, ambiguous_restart)
                    .map(|task| ExistingOrInsertedTask::Existing(Box::new(task)))
            } else if let Some(existing) = blocking
                .iter()
                .find(|candidate| same_transfer_intent(candidate, &task))
            {
                Ok(ExistingOrInsertedTask::Existing(Box::new(
                    (*existing).clone(),
                )))
            } else if let Some(replannable) = blocking.first() {
                self.replan_task(&conn, replannable, &task)
                    .map(|task| ExistingOrInsertedTask::Replanned(Box::new(task)))
            } else if let Some(restart) = path_tasks
                .iter()
                .find(|candidate| candidate.state_kind() == Ok(TransferState::RestartRequired))
            {
                self.replan_task(&conn, restart, &task)
                    .map(|task| ExistingOrInsertedTask::Replanned(Box::new(task)))
            } else if let Some(failed) = path_tasks
                .iter()
                .find(|candidate| candidate.state_kind() == Ok(TransferState::Failed))
            {
                // Failed 是自动任务的终态；保留任务行及可见错误作为路径屏障，显式重试复用该 ID。
                Ok(ExistingOrInsertedTask::Blocked(failed.id))
            } else {
                repository::insert_transfer(&conn, &task).map(ExistingOrInsertedTask::Inserted)
            }
        };
        let existing_or_task_id = match existing_or_task_id {
            Ok(value) => value,
            Err(error) => {
                self.notify_rejection();
                return Err(error);
            }
        };
        self.notify_best_effort();
        let (task_id, outcome) = match existing_or_task_id {
            ExistingOrInsertedTask::Inserted(task_id) => {
                let inserted = self.load(task_id)?;
                (task_id, self.run_existing_or_observe(inserted).await?)
            }
            ExistingOrInsertedTask::Existing(existing) => {
                let task_id = existing.id;
                (task_id, self.run_existing_or_observe(*existing).await?)
            }
            ExistingOrInsertedTask::Replanned(replanned) => {
                let task_id = replanned.id;
                (task_id, self.run_existing_or_observe(*replanned).await?)
            }
            ExistingOrInsertedTask::Blocked(task_id) => (
                task_id,
                TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::BlockedByActiveIntent,
                },
            ),
        };
        Ok(EnqueuedTaskOutcome { task_id, outcome })
    }

    /// 校验并接受失败任务的手动重试。
    pub async fn prepare_retry(&self, task_id: i64) -> AppResult<TransferTask> {
        let current = self.load(task_id)?;
        // 重试校验可能持久化拒绝结果，因此关闭准入必须先于校验并持续到 Pending 迁移完成。
        let _activity = self.begin_activity(&current)?;
        if current.state_kind().map_err(transition_error)? != TransferState::Failed {
            self.notify_rejection();
            return Err(AppError::generic("任务不存在或非失败状态"));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        if let Err(failure) = self.operations.preflight(&current).await {
            let failure = PreflightFailure::from(failure);
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        self.accept_retry_after_preflight(task_id, current.state_revision)
    }

    /// 接受前置校验通过的重试并迁移为 Pending。
    fn accept_retry_after_preflight(
        &self,
        task_id: i64,
        expected_revision: i64,
    ) -> AppResult<TransferTask> {
        let current = self.load(task_id)?;
        if current.state_revision != expected_revision
            || current.state_kind().map_err(transition_error)? != TransferState::Failed
        {
            self.notify_rejection();
            return Err(AppError::generic("传输任务状态已变化，请刷新后重试"));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        let pending = {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始重试接受事务失败：{error}")))?;
            let pending = repository::transition_transfer_in_transaction(
                &transaction,
                current.id,
                current.state_revision,
                TransferState::Pending,
                TransferPatch {
                    error_kind: ColumnPatch::Clear,
                    error_message: ColumnPatch::Clear,
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Clear,
                    attempt_count: Some(current.attempt_count.saturating_add(1)),
                    ..Default::default()
                },
            )
            .map_err(transition_error)?;
            update_compatibility_sync_status(
                &transaction,
                &pending,
                repository::sync_status::SYNCING,
                None,
                Some(repository::sync_status::FAILED),
            )?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交重试接受事务失败：{error}")))?;
            pending
        };
        self.notify_best_effort();
        Ok(pending)
    }

    /// 使用新意图重规划现有任务。
    fn replan_task(
        &self,
        conn: &rusqlite::Connection,
        current: &TransferTask,
        replacement: &TransferTask,
    ) -> AppResult<TransferTask> {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始任务重规划事务失败：{error}")))?;
        let current_state = current.state_kind().map_err(transition_error)?;
        let restart = if current_state == TransferState::RestartRequired {
            current.clone()
        } else {
            repository::transition_transfer_in_transaction(
                &transaction,
                current.id,
                current.state_revision,
                TransferState::RestartRequired,
                TransferPatch {
                    error_kind: ColumnPatch::Set(TransferErrorKind::LocalChanged),
                    error_message: ColumnPatch::Set(
                        "新的 planner intent 已取代尚未执行的旧任务".to_string(),
                    ),
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Clear,
                    ..Default::default()
                },
            )
            .map_err(transition_error)?
        };
        let pending = repository::transition_transfer_in_transaction(
            &transaction,
            restart.id,
            restart.state_revision,
            TransferState::Pending,
            TransferPatch {
                error_kind: ColumnPatch::Clear,
                error_message: ColumnPatch::Clear,
                next_retry_at: ColumnPatch::Clear,
                finished_at: ColumnPatch::Clear,
                remote_result_file_id: ColumnPatch::Clear,
                session_url: replacement
                    .session_url
                    .clone()
                    .map(ColumnPatch::Set)
                    .unwrap_or(ColumnPatch::Clear),
                transferred: Some(replacement.transferred),
                resume_offset: Some(replacement.resume_offset),
                attempt_count: Some(replacement.attempt_count),
            },
        )
        .map_err(transition_error)?;
        let changed = transaction
            .execute(
                "UPDATE transfer_queue SET
                    direction=?1,
                    file_id=?2,
                    local_path=?3,
                    name=?4,
                    total_size=?5,
                    transferred=?6,
                    created_at=?7,
                    server_id=?8,
                    upload_id=?9,
                    resume_offset=?10,
                    session_url=?11,
                    relative_path=?12,
                    parent_file_id=?13,
                    operation=?14,
                    source_mtime=?15,
                    source_size=?16,
                    expected_cloud_edited_time=?17,
                    attempt_count=?18
                 WHERE id=?19 AND state=?20 AND state_revision=?21",
                rusqlite::params![
                    replacement.direction,
                    replacement.file_id.as_deref(),
                    replacement.local_path.as_deref(),
                    replacement.name,
                    replacement.total_size,
                    replacement.transferred,
                    replacement.created_at,
                    replacement.server_id.as_deref(),
                    replacement.upload_id.as_deref(),
                    replacement.resume_offset,
                    replacement.session_url.as_deref(),
                    replacement.relative_path.as_deref(),
                    replacement.parent_file_id.as_deref(),
                    replacement.operation,
                    replacement.source_mtime,
                    replacement.source_size,
                    replacement.expected_cloud_edited_time,
                    replacement.attempt_count,
                    pending.id,
                    i32::from(TransferState::Pending),
                    pending.state_revision,
                ],
            )
            .map_err(|error| AppError::generic(format!("更新任务重规划意图失败：{error}")))?;
        if changed != 1 {
            return Err(AppError::generic(
                "任务重规划期间状态已变化，请等待下次同步",
            ));
        }
        let replanned = transaction
            .query_row(
                "SELECT * FROM transfer_queue WHERE id=?1",
                [pending.id],
                TransferTask::from_row,
            )
            .map_err(|error| AppError::generic(format!("读取重规划任务失败：{error}")))?;
        update_compatibility_sync_status(
            &transaction,
            &replanned,
            repository::sync_status::SYNCING,
            None,
            None,
        )?;
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交任务重规划事务失败：{error}")))?;
        Ok(replanned)
    }

    /// 将含歧义远端结果的重启任务提升为待核验。
    fn promote_restart_to_verifying(
        &self,
        conn: &rusqlite::Connection,
        restart: &TransferTask,
    ) -> AppResult<TransferTask> {
        repository::transition_transfer(
            conn,
            restart.id,
            restart.state_revision,
            TransferState::VerifyingRemote,
            TransferPatch {
                error_kind: ColumnPatch::Set(TransferErrorKind::RemoteAmbiguous),
                error_message: ColumnPatch::Set(
                    "远端写入已返回资源 ID，禁止重放并等待核验".to_string(),
                ),
                next_retry_at: ColumnPatch::Clear,
                finished_at: ColumnPatch::Clear,
                ..Default::default()
            },
        )
        .map_err(transition_error)
    }

    /// 仲裁任务进入 Running 或被活动意图阻塞。
    pub(super) fn transition_to_running_or_block(
        &self,
        current: &TransferTask,
    ) -> AppResult<RunningGateOutcome> {
        let outcome = {
            let conn = self.db.lock();
            let transaction = conn.unchecked_transaction().map_err(|error| {
                AppError::generic(format!("开始 Running 仲裁事务失败：{error}"))
            })?;
            let relative_path = current
                .relative_path
                .as_deref()
                .ok_or_else(|| AppError::generic("Running 仲裁缺少 relative_path"))?;
            let mut blocked = false;
            for candidate in repository::list_all_transfers(&transaction)?
                .into_iter()
                .filter(|candidate| {
                    candidate.id != current.id
                        && candidate.relative_path.as_deref() == Some(relative_path)
                })
            {
                match candidate.state_kind().map_err(transition_error)? {
                    TransferState::Running | TransferState::VerifyingRemote => {
                        blocked = true;
                    }
                    TransferState::RestartRequired if has_persisted_remote_result(&candidate) => {
                        repository::transition_transfer_in_transaction(
                            &transaction,
                            candidate.id,
                            candidate.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                error_kind: ColumnPatch::Set(TransferErrorKind::RemoteAmbiguous),
                                error_message: ColumnPatch::Set(
                                    "远端结果 ID 已存在；Running 仲裁禁止重放并等待核验"
                                        .to_string(),
                                ),
                                next_retry_at: ColumnPatch::Clear,
                                finished_at: ColumnPatch::Clear,
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                        blocked = true;
                    }
                    _ => {}
                }
            }
            let outcome = if blocked {
                RunningGateOutcome::Blocked
            } else {
                let running = repository::transition_transfer_in_transaction(
                    &transaction,
                    current.id,
                    current.state_revision,
                    TransferState::Running,
                    TransferPatch {
                        error_kind: ColumnPatch::Clear,
                        error_message: ColumnPatch::Clear,
                        next_retry_at: ColumnPatch::Clear,
                        finished_at: ColumnPatch::Clear,
                        ..Default::default()
                    },
                )
                .map_err(transition_error)?;
                RunningGateOutcome::Running(Box::new(running))
            };
            transaction.commit().map_err(|error| {
                AppError::generic(format!("提交 Running 仲裁事务失败：{error}"))
            })?;
            outcome
        };
        self.notify_best_effort();
        Ok(outcome)
    }
}
