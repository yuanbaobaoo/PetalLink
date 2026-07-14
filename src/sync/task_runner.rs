//! 提供持久化传输任务执行边界，供自动同步、手动重试、启动恢复以及（从 Task 5 起）
//! 稳定在线恢复共同使用。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::retry_policy::{classify_transfer_error, RecoveryContext, RecoveryDecision};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

mod admission;
mod contracts;
mod progress;

use admission::{
    active_task_disposition, has_ambiguous_remote_write_result, has_persisted_remote_result,
    is_path_blocking_state, same_transfer_intent, ExistingOrInsertedTask, RunningGateOutcome,
};
pub use contracts::{
    BackendPreflightFailure, EnqueuedTaskOutcome, NowMs, OnlineCheck, RemoteVerification,
    StartupRecoverySummary, TaskActivityGate, TaskDisposition, TaskExecutionError,
    TaskExecutionOutcome, TaskStateSink, TransferOperations,
};
pub use progress::TaskProgressReporter;

const MAX_AUTOMATIC_ATTEMPTS: u32 = 5;

pub struct TaskRunner {
    db: Arc<Mutex<rusqlite::Connection>>,
    mount_root: PathBuf,
    operations: Arc<dyn TransferOperations>,
    online_check: OnlineCheck,
    now_ms: NowMs,
    state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    activity_gate: Arc<RwLock<Option<Arc<dyn TaskActivityGate>>>>,
}

impl TaskRunner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Mutex<rusqlite::Connection>>,
        mount_root: PathBuf,
        operations: Arc<dyn TransferOperations>,
        online_check: OnlineCheck,
        state_sink: Arc<dyn TaskStateSink>,
        transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    ) -> Self {
        Self::new_with_clock(
            db,
            mount_root,
            operations,
            online_check,
            state_sink,
            transfer_update_tx,
            Arc::new(|| chrono::Utc::now().timestamp_millis()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_clock(
        db: Arc<Mutex<rusqlite::Connection>>,
        mount_root: PathBuf,
        operations: Arc<dyn TransferOperations>,
        online_check: OnlineCheck,
        state_sink: Arc<dyn TaskStateSink>,
        transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
        now_ms: NowMs,
    ) -> Self {
        Self {
            db,
            mount_root,
            operations,
            online_check,
            now_ms,
            state_sink: Arc::new(RwLock::new(state_sink)),
            transfer_update_tx,
            activity_gate: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_state_sink(&self, state_sink: Arc<dyn TaskStateSink>) {
        *self.state_sink.write() = state_sink;
    }

    pub fn set_activity_gate(&self, activity_gate: Arc<dyn TaskActivityGate>) {
        *self.activity_gate.write() = Some(activity_gate);
    }

    fn begin_activity(&self, task: &TransferTask) -> AppResult<Option<Box<dyn Send>>> {
        self.activity_gate
            .read()
            .clone()
            .map(|gate| gate.begin(task.relative_path.as_deref()))
            .transpose()
    }

    /// Persist a Pending intent before any backend call, then execute that exact task row.
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
        // Admission precedes the durable insert so an execution-time exclusive path lease cannot
        // be bypassed by queuing a Pending row that would run immediately after the lease drops.
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
                // Failed is terminal for automatic work. Keep the row and its visible error as a
                // path barrier; an explicit retry goes through prepare_retry and reuses this ID.
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

    pub async fn retry(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let pending = self.prepare_retry(task_id).await?;
        self.run_expected(pending, false).await
    }

    pub async fn prepare_retry(&self, task_id: i64) -> AppResult<TransferTask> {
        let current = self.load(task_id)?;
        // Retry validation can persist a rejection, so shutdown admission must precede it and
        // stay held through the accepted Pending transition.
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

    fn transition_to_running_or_block(
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

    pub async fn run(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let current = self.load(task_id)?;
        self.run_expected(current, true).await
    }

    async fn run_existing_or_observe(
        &self,
        existing: TransferTask,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = existing.state_kind().map_err(transition_error)?;
        if matches!(
            state,
            TransferState::Pending | TransferState::WaitingForNetwork | TransferState::BackingOff
        ) {
            match self.run_expected(existing.clone(), true).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) => {
                    let observed = self.load(existing.id)?;
                    if observed.state_revision != existing.state_revision {
                        return self.observed_concurrent_outcome(&observed);
                    }
                    return Err(error);
                }
            }
        }
        let disposition = active_task_disposition(state)
            .ok_or_else(|| AppError::generic("自动周期发现的任务已不再活动"))?;
        Ok(TaskExecutionOutcome {
            cloud_file: None,
            disposition,
        })
    }

    fn observed_concurrent_outcome(
        &self,
        observed: &TransferTask,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = observed.state_kind().map_err(transition_error)?;
        if state == TransferState::Completed {
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: TaskDisposition::Completed,
            });
        }
        if let Some(disposition) = active_task_disposition(state) {
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition,
            });
        }
        Err(AppError::generic(format!(
            "任务已由并发执行收敛为 {state:?}{}",
            observed
                .error_message
                .as_deref()
                .map(|message| format!("：{message}"))
                .unwrap_or_default()
        )))
    }

    /// Execute a manual retry accepted only after static and backend preflight. Static identity,
    /// destination/source snapshot and network state are rechecked immediately before Running.
    pub async fn run_prepared(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let current = self.load(task_id)?;
        self.run_expected(current, false).await
    }

    /// Resolve post-submit upload ambiguity before any replay. A committed result is settled
    /// atomically; an explicitly absent result is the only case allowed to return to Pending.
    pub async fn resume_verifying(&self) -> AppResult<usize> {
        if !(self.online_check)() {
            return Ok(0);
        }
        let tasks = self.list_states(&[TransferState::VerifyingRemote])?;
        let mut resolved = 0usize;
        for task in tasks {
            let verification = {
                let _activity = self.begin_activity(&task)?;
                self.operations.verify_remote(&task).await
            };
            match verification {
                Ok(RemoteVerification::Committed(file)) => {
                    let mut outcome = TaskExecutionOutcome {
                        cloud_file: Some(file.clone()),
                        disposition: TaskDisposition::Completed,
                    };
                    if let Err(failure) = self.validate_success_outcome(&task, &outcome) {
                        self.transition(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            TransferPatch {
                                error_kind: ColumnPatch::Set(failure.kind),
                                error_message: ColumnPatch::Set(format!(
                                    "远端写入已确认，但本地源无法安全结算：{}",
                                    failure.message
                                )),
                                remote_result_file_id: ColumnPatch::Set(file.id),
                                ..Default::default()
                            },
                        )?;
                        continue;
                    }
                    match self.settle_success(&task, &outcome) {
                        Ok(_) => resolved += 1,
                        Err(error) => {
                            self.recover_success_settlement_failure(&task, &mut outcome, error)?;
                        }
                    }
                }
                Ok(RemoteVerification::NotCommitted) => {
                    let session_expired = task.error_kind_typed().map_err(transition_error)?
                        == Some(TransferErrorKind::SessionExpired);
                    let restart_patch = TransferPatch {
                        error_kind: ColumnPatch::Set(if session_expired {
                            TransferErrorKind::SessionExpired
                        } else {
                            TransferErrorKind::RemoteAmbiguous
                        }),
                        error_message: ColumnPatch::Set(if session_expired {
                            "远端核验确认写入未提交，已清理失效会话，可以安全新建会话".to_string()
                        } else {
                            "远端核验确认写入未提交，可以安全重放".to_string()
                        }),
                        next_retry_at: ColumnPatch::Clear,
                        finished_at: ColumnPatch::Clear,
                        remote_result_file_id: ColumnPatch::Clear,
                        ..Default::default()
                    };
                    let restart = if session_expired {
                        self.transition_clearing_upload_session(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            restart_patch,
                        )?
                    } else {
                        self.transition(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            restart_patch,
                        )?
                    };
                    let pending = self.transition(
                        restart.id,
                        restart.state_revision,
                        TransferState::Pending,
                        TransferPatch {
                            error_kind: ColumnPatch::Clear,
                            error_message: ColumnPatch::Clear,
                            next_retry_at: ColumnPatch::Clear,
                            finished_at: ColumnPatch::Clear,
                            ..Default::default()
                        },
                    )?;
                    match self.run_expected(pending, true).await {
                        Ok(outcome)
                            if !matches!(
                                outcome.disposition,
                                TaskDisposition::VerifyingRemote
                                    | TaskDisposition::WaitingForNetwork
                                    | TaskDisposition::BackingOff
                                    | TaskDisposition::BlockedByActiveIntent
                            ) =>
                        {
                            resolved += 1;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(task_id = task.id, %error, "核验后安全重放失败");
                        }
                    }
                }
                Ok(RemoteVerification::Ambiguous(message)) => {
                    let error_kind = if task.error_kind_typed().map_err(transition_error)?
                        == Some(TransferErrorKind::SessionExpired)
                    {
                        TransferErrorKind::SessionExpired
                    } else {
                        TransferErrorKind::RemoteAmbiguous
                    };
                    {
                        let conn = self.db.lock();
                        repository::patch_transfer_in_state(
                            &conn,
                            task.id,
                            task.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                // Preserve the expired-session marker until a conclusive absence
                                // lets us atomically discard the old session identity.
                                error_kind: ColumnPatch::Set(error_kind),
                                error_message: ColumnPatch::Set(message),
                                next_retry_at: ColumnPatch::Set(
                                    (self.now_ms)().saturating_add(60_000),
                                ),
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                    }
                    self.notify_best_effort();
                }
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "远端写入核验暂不可用，保留歧义状态");
                    {
                        let conn = self.db.lock();
                        repository::patch_transfer_in_state(
                            &conn,
                            task.id,
                            task.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                error_message: ColumnPatch::Set(format!(
                                    "远端核验暂不可用：{error}"
                                )),
                                next_retry_at: ColumnPatch::Set(
                                    (self.now_ms)().saturating_add(15_000),
                                ),
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                    }
                    self.notify_best_effort();
                }
            }
        }
        Ok(resolved)
    }

    pub async fn resume_waiting(&self) -> AppResult<usize> {
        if !(self.online_check)() {
            self.notify_rejection();
            return Ok(0);
        }
        let tasks = self.list_states(&[TransferState::WaitingForNetwork])?;
        let mut resumed = 0;
        for task in tasks {
            let task_id = task.id;
            match self.run_expected(task, true).await {
                Ok(outcome)
                    if !matches!(
                        outcome.disposition,
                        TaskDisposition::WaitingForNetwork | TaskDisposition::BlockedByActiveIntent
                    ) =>
                {
                    resumed += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(task_id, %error, "等待网络任务恢复失败");
                }
            }
        }
        Ok(resumed)
    }

    /// Task 5 consumes this due-task polling seam; it intentionally performs no sleeping.
    pub async fn resume_due_backoff(&self) -> AppResult<usize> {
        let now = (self.now_ms)();
        let tasks = self.list_states(&[TransferState::BackingOff])?;
        let mut resumed = 0;
        for task in tasks {
            if task
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > now)
            {
                continue;
            }
            match self.run_expected(task.clone(), true).await {
                Ok(outcome)
                    if !matches!(
                        outcome.disposition,
                        TaskDisposition::BackingOff | TaskDisposition::BlockedByActiveIntent
                    ) =>
                {
                    resumed += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "退避任务恢复失败");
                }
            }
        }
        Ok(resumed)
    }

    pub fn next_backoff_deadline_ms(&self) -> AppResult<Option<i64>> {
        let now = (self.now_ms)();
        Ok(self
            .list_states(&[TransferState::BackingOff, TransferState::VerifyingRemote])?
            .into_iter()
            .map(|task| task.next_retry_at.unwrap_or(now))
            .min())
    }

    pub(crate) fn current_time_ms(&self) -> i64 {
        (self.now_ms)()
    }

    pub async fn recover_startup(&self) -> AppResult<StartupRecoverySummary> {
        let mut tasks = self.list_states(&[TransferState::Pending, TransferState::Running])?;
        tasks.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        let mut summary = StartupRecoverySummary::default();
        let mut selected_tasks = Vec::new();
        let mut grouped = std::collections::HashMap::<String, Vec<TransferTask>>::new();
        for task in tasks {
            match task.relative_path.clone() {
                Some(relative_path) => grouped.entry(relative_path).or_default().push(task),
                None => selected_tasks.push(task),
            }
        }
        for (_, mut same_path) in grouped {
            let has_running_remote_write = same_path.iter().any(|task| {
                task.state_kind() == Ok(TransferState::Running)
                    && matches!(
                        task.operation_kind().ok().flatten(),
                        Some(TransferOperation::Create | TransferOperation::Update)
                    )
            });
            if has_running_remote_write {
                for task in same_path {
                    if self.suppress_startup_duplicate(&task)? {
                        summary.verifying_remote += 1;
                    } else {
                        summary.failed += 1;
                    }
                }
                continue;
            }
            let selected = same_path.remove(0);
            selected_tasks.push(selected);
            for task in same_path {
                if self.suppress_startup_duplicate(&task)? {
                    summary.verifying_remote += 1;
                } else {
                    summary.failed += 1;
                }
            }
        }
        for task in selected_tasks {
            // Startup recovery is a stream of independent row operations. Acquire per row so a
            // close between rows leaves every not-yet-admitted row byte-for-byte unchanged.
            let _activity = self.begin_activity(&task)?;
            let state = match task.state_kind() {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "启动恢复跳过非法任务状态");
                    summary.failed += 1;
                    continue;
                }
            };
            if state == TransferState::Running {
                let operation = match task.operation_kind() {
                    Ok(Some(operation)) => operation,
                    _ => {
                        self.transition_failure(
                            &task,
                            TransferState::Failed,
                            TransferErrorKind::Validation,
                            "中断任务缺少合法 operation",
                        )?;
                        summary.failed += 1;
                        continue;
                    }
                };
                match operation {
                    TransferOperation::Create | TransferOperation::Update => {
                        self.transition_failure(
                            &task,
                            TransferState::VerifyingRemote,
                            TransferErrorKind::RemoteAmbiguous,
                            "进程中断时远端写入结果不确定，等待核验",
                        )?;
                        summary.verifying_remote += 1;
                        continue;
                    }
                    TransferOperation::Download | TransferOperation::DownloadUpdate => {
                        if let Err(failure) = self.validate_static(&task) {
                            self.persist_preflight_rejection(&task, failure)?;
                            summary.failed += 1;
                            continue;
                        }
                        let relative_path = task
                            .relative_path
                            .as_deref()
                            .expect("validated download task has relative path");
                        let validated_destination = self.mount_root.join(relative_path);
                        let tmp_path = crate::drive::download_api::tmp_path(&validated_destination);
                        let durable_offset = std::fs::metadata(&tmp_path)
                            .ok()
                            .filter(|metadata| metadata.is_file())
                            .map(|metadata| metadata.len().min(task.total_size as u64) as i64)
                            .unwrap_or(0);
                        let restart = self.transition_failure(
                            &task,
                            TransferState::RestartRequired,
                            TransferErrorKind::SessionExpired,
                            "进程中断，保留已验证下载断点并重新建立 Range 请求",
                        )?;
                        let pending = self.transition(
                            restart.id,
                            restart.state_revision,
                            TransferState::Pending,
                            TransferPatch {
                                error_kind: ColumnPatch::Clear,
                                error_message: ColumnPatch::Clear,
                                finished_at: ColumnPatch::Clear,
                                transferred: Some(durable_offset),
                                resume_offset: Some(durable_offset),
                                ..Default::default()
                            },
                        )?;
                        self.record_startup_outcome(
                            self.run_expected(pending, true).await,
                            &mut summary,
                        );
                    }
                    _ => {
                        self.transition_failure(
                            &task,
                            TransferState::Failed,
                            TransferErrorKind::Validation,
                            "该中断操作暂不支持自动恢复",
                        )?;
                        summary.failed += 1;
                    }
                }
            } else {
                self.record_startup_outcome(self.run_expected(task, true).await, &mut summary);
            }
        }
        Ok(summary)
    }

    fn suppress_startup_duplicate(&self, task: &TransferTask) -> AppResult<bool> {
        let _activity = self.begin_activity(task)?;
        let state = task.state_kind().map_err(transition_error)?;
        let operation = task.operation_kind().map_err(transition_error)?;
        if state == TransferState::Running
            && matches!(
                operation,
                Some(TransferOperation::Create | TransferOperation::Update)
            )
        {
            self.transition_failure(
                task,
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                "启动恢复发现同路径多个活动任务；旧远端写入等待核验",
            )?;
            return Ok(true);
        }
        self.transition_failure(
            task,
            TransferState::RestartRequired,
            if state == TransferState::Running {
                TransferErrorKind::SessionExpired
            } else {
                TransferErrorKind::LocalChanged
            },
            "启动恢复仅保留同路径最新任务，旧任务等待重新规划",
        )?;
        Ok(false)
    }

    fn record_startup_outcome(
        &self,
        result: AppResult<TaskExecutionOutcome>,
        summary: &mut StartupRecoverySummary,
    ) {
        match result {
            Ok(outcome) => match outcome.disposition {
                TaskDisposition::Completed => summary.completed += 1,
                TaskDisposition::Pending
                | TaskDisposition::Running
                | TaskDisposition::BlockedByActiveIntent => {}
                TaskDisposition::WaitingForNetwork => summary.waiting_network += 1,
                TaskDisposition::VerifyingRemote => summary.verifying_remote += 1,
                TaskDisposition::BackingOff => {}
                TaskDisposition::RestartRequired => summary.failed += 1,
            },
            Err(error) => {
                tracing::warn!(%error, "启动任务恢复失败");
                summary.failed += 1;
            }
        }
    }

    async fn run_expected(
        &self,
        current: TransferTask,
        run_backend_preflight: bool,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = current.state_kind().map_err(transition_error)?;
        // This is the per-row linearization point. It intentionally precedes static validation:
        // validation failures are persisted, and download validation may create a parent folder.
        // An admitted permit remains alive through backend settlement, including ambiguous writes.
        let _activity = self.begin_activity(&current)?;
        if !matches!(
            state,
            TransferState::Pending | TransferState::WaitingForNetwork | TransferState::BackingOff
        ) {
            self.notify_rejection();
            return Err(AppError::generic(format!("任务状态 {state:?} 不可执行")));
        }
        if state == TransferState::BackingOff && current.next_retry_at.is_none() {
            let failure = PreflightFailure::validation("退避任务缺少 next_retry_at，拒绝立即重放");
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            if failure.target == TransferState::RestartRequired {
                return Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::RestartRequired,
                });
            }
            return Err(AppError::generic(failure.message));
        }
        if !(self.online_check)() {
            if state == TransferState::Pending {
                self.transition_failure(
                    &current,
                    TransferState::WaitingForNetwork,
                    TransferErrorKind::Network,
                    "网络不可用，等待恢复",
                )?;
            } else {
                self.notify_rejection();
            }
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: if state == TransferState::BackingOff {
                    TaskDisposition::BackingOff
                } else {
                    TaskDisposition::WaitingForNetwork
                },
            });
        }
        if state == TransferState::BackingOff
            && current
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > (self.now_ms)())
        {
            self.notify_rejection();
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: TaskDisposition::BackingOff,
            });
        }
        if run_backend_preflight {
            if let Err(failure) = self.operations.preflight(&current).await {
                let failure = PreflightFailure::from(failure);
                self.persist_preflight_rejection(&current, failure.clone())?;
                if failure.target == TransferState::RestartRequired {
                    return Ok(TaskExecutionOutcome {
                        cloud_file: None,
                        disposition: TaskDisposition::RestartRequired,
                    });
                }
                return Err(AppError::generic(failure.message));
            }
        }
        let running = match self.transition_to_running_or_block(&current)? {
            RunningGateOutcome::Running(running) => *running,
            RunningGateOutcome::Blocked => {
                return Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::BlockedByActiveIntent,
                });
            }
        };
        let progress = TaskProgressReporter::new(
            self.db.clone(),
            running.id,
            running.state_revision,
            running.total_size,
            self.state_sink.clone(),
            self.transfer_update_tx.clone(),
        );
        match self.operations.execute(&running, &progress).await {
            Ok(mut output) => {
                progress.ensure_current()?;
                if output.disposition != TaskDisposition::Completed {
                    if matches!(
                        output.disposition,
                        TaskDisposition::Pending
                            | TaskDisposition::Running
                            | TaskDisposition::BlockedByActiveIntent
                            | TaskDisposition::BackingOff
                    ) {
                        return self.settle_error(
                            &running,
                            AppError::generic(format!(
                                "后端返回缺少可持久化恢复条件的状态 {:?}",
                                output.disposition
                            )),
                        );
                    }
                    self.persist_backend_disposition(&running, &output)?;
                    return Ok(output);
                }
                if let Err(failure) = self.validate_success_outcome(&running, &output) {
                    let remote_id = output.cloud_file.as_ref().map(|file| file.id.clone());
                    let remote_write_is_ambiguous = remote_id
                        .as_deref()
                        .is_some_and(|file_id| !file_id.trim().is_empty())
                        && matches!(
                            running.operation_kind().map_err(transition_error)?,
                            Some(TransferOperation::Create | TransferOperation::Update)
                        );
                    let (target, kind, message) = if remote_write_is_ambiguous {
                        (
                            TransferState::VerifyingRemote,
                            TransferErrorKind::RemoteAmbiguous,
                            format!("{}；远端已返回资源 ID，禁止直接重放", failure.message),
                        )
                    } else {
                        (failure.target, failure.kind, failure.message)
                    };
                    self.transition(
                        running.id,
                        running.state_revision,
                        target,
                        TransferPatch {
                            error_kind: ColumnPatch::Set(kind),
                            error_message: ColumnPatch::Set(message),
                            remote_result_file_id: remote_id
                                .map(ColumnPatch::Set)
                                .unwrap_or(ColumnPatch::Keep),
                            ..Default::default()
                        },
                    )?;
                    output.disposition = match target {
                        TransferState::VerifyingRemote => TaskDisposition::VerifyingRemote,
                        TransferState::RestartRequired => TaskDisposition::RestartRequired,
                        _ => return Err(AppError::generic("非法成功核验目标状态")),
                    };
                    return Ok(output);
                }
                match self.settle_success(&running, &output) {
                    Ok(completed) => {
                        debug_assert_eq!(completed.id, running.id);
                        output.disposition = TaskDisposition::Completed;
                        Ok(output)
                    }
                    Err(error) => {
                        self.recover_success_settlement_failure(&running, &mut output, error)
                    }
                }
            }
            Err(TaskExecutionError::RestartRequired(message)) => {
                self.transition_failure(
                    &running,
                    TransferState::RestartRequired,
                    TransferErrorKind::LocalChanged,
                    &message,
                )?;
                Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::RestartRequired,
                })
            }
            Err(TaskExecutionError::App(error)) => self.settle_error(&running, error),
        }
    }

    fn settle_error(
        &self,
        running: &TransferTask,
        error: AppError,
    ) -> AppResult<TaskExecutionOutcome> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("任务缺少 operation"))?;
        let classified = classify_transfer_error(
            &error,
            RecoveryContext {
                operation,
                attempt_count: running.attempt_count.max(0) as u32,
                now_ms: (self.now_ms)(),
                jitter_ms: 0,
                auth_already_replayed: false,
                max_attempts: MAX_AUTOMATIC_ATTEMPTS,
            },
        );
        let attempts = running
            .attempt_count
            .saturating_add(i64::from(classified.consumes_retry_budget));
        let (state, disposition, next_retry_at) = match classified.decision {
            RecoveryDecision::WaitForNetwork => (
                TransferState::WaitingForNetwork,
                Some(TaskDisposition::WaitingForNetwork),
                None,
            ),
            RecoveryDecision::Backoff { next_retry_at } => (
                TransferState::BackingOff,
                Some(TaskDisposition::BackingOff),
                Some(next_retry_at),
            ),
            RecoveryDecision::VerifyRemote => (
                TransferState::VerifyingRemote,
                Some(TaskDisposition::VerifyingRemote),
                Some((self.now_ms)().saturating_add(3_000)),
            ),
            // DriveClient owns the one authenticated replay. A first 401 reaching this boundary
            // is not replayed blindly by the runner.
            RecoveryDecision::RefreshAuth | RecoveryDecision::Fail => {
                (TransferState::Failed, None, None)
            }
        };
        let patch = TransferPatch {
            error_kind: ColumnPatch::Set(classified.kind),
            error_message: ColumnPatch::Set(error.to_string()),
            next_retry_at: next_retry_at
                .map(ColumnPatch::Set)
                .unwrap_or(ColumnPatch::Clear),
            finished_at: if state == TransferState::Failed {
                ColumnPatch::Set((self.now_ms)())
            } else {
                ColumnPatch::Clear
            },
            attempt_count: Some(attempts),
            ..Default::default()
        };
        if state == TransferState::Failed {
            let error_message = error.to_string();
            {
                let conn = self.db.lock();
                let transaction = conn.unchecked_transaction().map_err(|db_error| {
                    AppError::generic(format!("开始失败结算事务失败：{db_error}"))
                })?;
                let failed = repository::transition_transfer_in_transaction(
                    &transaction,
                    running.id,
                    running.state_revision,
                    state,
                    patch,
                )
                .map_err(transition_error)?;
                mark_compatibility_sync_failed(&transaction, &failed, &error_message)?;
                transaction.commit().map_err(|db_error| {
                    AppError::generic(format!("提交失败结算事务失败：{db_error}"))
                })?;
            }
            self.notify_best_effort();
        } else {
            self.transition(running.id, running.state_revision, state, patch)?;
        }
        match disposition {
            Some(disposition) => Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition,
            }),
            None => Err(error),
        }
    }

    fn persist_backend_disposition(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> AppResult<TransferTask> {
        let (state, kind, message) = match output.disposition {
            TaskDisposition::Completed => {
                return Err(AppError::generic("Completed 不应走延迟结算"));
            }
            TaskDisposition::Pending
            | TaskDisposition::Running
            | TaskDisposition::BlockedByActiveIntent => {
                return Err(AppError::generic("活动任务状态不应由后端返回"));
            }
            TaskDisposition::WaitingForNetwork => (
                TransferState::WaitingForNetwork,
                TransferErrorKind::Network,
                "后端请求等待网络恢复",
            ),
            TaskDisposition::BackingOff => {
                return Err(AppError::generic("后端 BackingOff 缺少 next_retry_at"));
            }
            TaskDisposition::VerifyingRemote => (
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                "远端写入已返回资源 ID，但完整元数据尚未确认",
            ),
            TaskDisposition::RestartRequired => (
                TransferState::RestartRequired,
                TransferErrorKind::LocalChanged,
                "本地源已变化，需要重新规划",
            ),
        };
        self.transition(
            running.id,
            running.state_revision,
            state,
            TransferPatch {
                error_kind: ColumnPatch::Set(kind),
                error_message: ColumnPatch::Set(message.to_string()),
                next_retry_at: if output.disposition == TaskDisposition::VerifyingRemote {
                    ColumnPatch::Set((self.now_ms)().saturating_add(3_000))
                } else {
                    ColumnPatch::Clear
                },
                remote_result_file_id: output
                    .cloud_file
                    .as_ref()
                    .map(|file| ColumnPatch::Set(file.id.clone()))
                    .unwrap_or(ColumnPatch::Keep),
                ..Default::default()
            },
        )
    }

    fn validate_success_outcome(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> Result<(), PreflightFailure> {
        let operation = running
            .operation_kind()
            .map_err(|error| PreflightFailure::validation(error.to_string()))?
            .ok_or_else(|| PreflightFailure::validation("成功核验缺少 operation"))?;
        let local_path = running
            .local_path
            .as_deref()
            .ok_or_else(|| PreflightFailure::validation("成功核验缺少本地路径"))?;
        match operation {
            TransferOperation::Create | TransferOperation::Update => {
                let cloud = output
                    .cloud_file
                    .as_ref()
                    .ok_or_else(|| PreflightFailure::remote_ambiguous("上传结果缺少远端资源"))?;
                if cloud.id.trim().is_empty()
                    || cloud.name.trim().is_empty()
                    || cloud.name != running.name
                    || cloud.edited_time.is_none()
                    || cloud.size != running.source_size.unwrap_or(-1)
                    || (operation == TransferOperation::Update
                        && running.file_id.as_deref() != Some(cloud.id.as_str()))
                {
                    return Err(PreflightFailure::remote_ambiguous(
                        "上传结果元数据不完整或大小不一致",
                    ));
                }
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => {
                let metadata = std::fs::metadata(local_path)
                    .map_err(|_| PreflightFailure::local_changed("成功核验时下载文件不存在"))?;
                if !metadata.is_file() {
                    return Err(PreflightFailure::local_changed(
                        "成功核验时下载目标不是普通文件",
                    ));
                }
                if running.expected_cloud_edited_time.is_none()
                    || metadata.len() as i64 != running.total_size
                {
                    return Err(PreflightFailure::local_changed(
                        "下载结果大小或云端版本不匹配",
                    ));
                }
            }
            _ => return Err(PreflightFailure::validation("不支持该成功结果")),
        }
        Ok(())
    }

    fn settle_success(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> AppResult<TransferTask> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("任务缺少 operation"))?;
        let relative_path = running
            .relative_path
            .as_deref()
            .ok_or_else(|| AppError::generic("任务缺少相对路径"))?;
        let local_path = running
            .local_path
            .as_deref()
            .ok_or_else(|| AppError::generic("任务缺少本地路径"))?;
        // Upload settlement records the exact source snapshot that reached Huawei, not whatever
        // currently occupies the path. If the user edited/replaced it during upload, the next
        // planner pass sees that delta and issues a version-checked Update instead of looping the
        // already-committed task through remote verification.
        let (local_mtime, local_size) = if matches!(
            operation,
            TransferOperation::Create | TransferOperation::Update
        ) {
            (
                running
                    .source_mtime
                    .ok_or_else(|| AppError::generic("上传成功结算缺少源修改时间快照"))?,
                running
                    .source_size
                    .ok_or_else(|| AppError::generic("上传成功结算缺少源大小快照"))?,
            )
        } else {
            let metadata = std::fs::metadata(local_path)
                .map_err(|error| AppError::generic(format!("成功结算读取下载文件失败：{error}")))?;
            if !metadata.is_file() {
                return Err(AppError::generic("成功结算下载目标不是普通文件"));
            }
            let local_mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .ok_or_else(|| AppError::generic("成功结算无法读取下载文件修改时间"))?;
            (local_mtime, metadata.len() as i64)
        };
        let (file_id, name, size, cloud_edited_time, parent_folder_id) = match operation {
            TransferOperation::Create | TransferOperation::Update => {
                let cloud = output
                    .cloud_file
                    .as_ref()
                    .ok_or_else(|| AppError::generic("上传成功但缺少远端文件结果，拒绝结算"))?;
                (
                    cloud.id.clone(),
                    cloud.name.clone(),
                    cloud.size,
                    cloud.edited_time.map(|time| time.timestamp_millis()),
                    cloud
                        .parent_folder
                        .as_ref()
                        .and_then(|parents| parents.first().cloned())
                        .or_else(|| running.parent_file_id.clone()),
                )
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => (
                running
                    .file_id
                    .clone()
                    .ok_or_else(|| AppError::generic("下载成功结算缺少 fileId"))?,
                running.name.clone(),
                running.total_size,
                running.expected_cloud_edited_time,
                running.parent_file_id.clone(),
            ),
            _ => return Err(AppError::generic("该 operation 不支持成功结算")),
        };
        let finished_at = chrono::Utc::now().timestamp_millis();
        let completed = {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始传输结算事务失败：{error}")))?;
            let completed = repository::transition_transfer_in_transaction(
                &transaction,
                running.id,
                running.state_revision,
                TransferState::Completed,
                TransferPatch {
                    error_kind: ColumnPatch::Clear,
                    error_message: ColumnPatch::Clear,
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Set(finished_at),
                    remote_result_file_id: ColumnPatch::Set(file_id.clone()),
                    transferred: Some(running.total_size),
                    ..Default::default()
                },
            )
            .map_err(transition_error)?;
            transaction
                .execute(
                    "DELETE FROM sync_items
                     WHERE local_path=?1 AND file_id=?2",
                    rusqlite::params![
                        relative_path,
                        format!("{}{}", repository::PENDING_FILE_ID_PREFIX, relative_path)
                    ],
                )
                .map_err(|error| AppError::generic(format!("清理待确认同步基线失败：{error}")))?;
            if operation == TransferOperation::Update {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                        rusqlite::params![file_id, relative_path],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理改名/移动旧基线路径失败：{error}"))
                    })?;
            }
            repository::upsert(
                &transaction,
                &repository::SyncItem {
                    file_id,
                    local_path: relative_path.to_string(),
                    parent_folder_id,
                    name,
                    is_folder: false,
                    size,
                    local_size: Some(local_size),
                    sha256: None,
                    local_mtime: Some(local_mtime),
                    cloud_edited_time,
                    last_sync_time: Some(finished_at),
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                },
            )?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交传输结算事务失败：{error}")))?;
            completed
        };
        self.notify_best_effort();
        Ok(completed)
    }

    fn recover_success_settlement_failure(
        &self,
        running: &TransferTask,
        output: &mut TaskExecutionOutcome,
        error: AppError,
    ) -> AppResult<TaskExecutionOutcome> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("成功结算恢复缺少 operation"))?;
        let message = format!("后端已完成，但本地同步基线结算失败：{error}");
        let (target, kind, disposition) = match operation {
            TransferOperation::Create | TransferOperation::Update => (
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                TaskDisposition::VerifyingRemote,
            ),
            TransferOperation::Download | TransferOperation::DownloadUpdate => (
                TransferState::RestartRequired,
                TransferErrorKind::Unknown,
                TaskDisposition::RestartRequired,
            ),
            _ => return Err(error),
        };
        self.transition(
            running.id,
            running.state_revision,
            target,
            TransferPatch {
                error_kind: ColumnPatch::Set(kind),
                error_message: ColumnPatch::Set(message),
                remote_result_file_id: output
                    .cloud_file
                    .as_ref()
                    .map(|cloud| ColumnPatch::Set(cloud.id.clone()))
                    .unwrap_or(ColumnPatch::Keep),
                ..Default::default()
            },
        )?;
        output.disposition = disposition;
        Ok(output.clone())
    }

    fn validate_static(&self, task: &TransferTask) -> Result<TransferOperation, PreflightFailure> {
        let operation = task
            .operation_kind()
            .map_err(|error| PreflightFailure::validation(error.to_string()))?
            .ok_or_else(|| PreflightFailure::validation("任务缺少 operation"))?;
        let rel = task
            .relative_path
            .as_deref()
            .ok_or_else(|| PreflightFailure::validation("任务缺少相对路径"))?;
        crate::core::paths::validate_relative_path(rel, false)
            .map_err(|error| PreflightFailure::validation(error.to_string()))?;
        let mount_metadata = std::fs::metadata(&self.mount_root)
            .map_err(|_| PreflightFailure::validation("挂载根目录不存在或不可访问"))?;
        if !mount_metadata.is_dir() {
            return Err(PreflightFailure::validation("挂载根路径不是目录"));
        }
        let local_path = task
            .local_path
            .as_deref()
            .ok_or_else(|| PreflightFailure::validation("任务缺少本地路径"))?;
        let local_path = Path::new(local_path);
        if !local_path.is_absolute() || self.mount_root.join(rel) != local_path {
            return Err(PreflightFailure::validation(
                "任务绝对路径与挂载相对路径不一致",
            ));
        }
        if task.total_size < 0 || task.resume_offset < 0 || task.resume_offset > task.total_size {
            return Err(PreflightFailure::validation("任务大小或断点偏移非法"));
        }
        let has_nonempty = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        };
        match operation {
            TransferOperation::Create | TransferOperation::Update => {
                if task.direction != repository::transfer_direction::UPLOAD {
                    return Err(PreflightFailure::validation(
                        "上传 operation 与 direction 不一致",
                    ));
                }
                if operation == TransferOperation::Create && has_nonempty(&task.file_id) {
                    return Err(PreflightFailure::validation("Create 任务不能携带 fileId"));
                }
                if operation == TransferOperation::Update
                    && !task.file_id.as_deref().map(str::trim).is_some_and(|id| {
                        !id.is_empty() && !id.starts_with(repository::PENDING_FILE_ID_PREFIX)
                    })
                {
                    return Err(PreflightFailure::validation("Update 任务缺少真实 fileId"));
                }
                if task.resume_offset > 0 && !has_nonempty(&task.session_url) {
                    return Err(PreflightFailure::validation(
                        "非零上传断点缺少 session_url，拒绝作为全新请求重放",
                    ));
                }
                if Path::new(rel)
                    .parent()
                    .is_some_and(|parent| !parent.as_os_str().is_empty())
                    && !has_nonempty(&task.parent_file_id)
                {
                    return Err(PreflightFailure::validation("子目录上传缺少 parentId"));
                }
                let metadata = std::fs::metadata(local_path)
                    .map_err(|_| PreflightFailure::validation("本地上传源不存在"))?;
                if !metadata.is_file() {
                    return Err(PreflightFailure::validation("本地上传源不是普通文件"));
                }
                let actual_mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64)
                    .ok_or_else(|| PreflightFailure::validation("无法读取本地源修改时间"))?;
                let actual_size = metadata.len() as i64;
                if task.source_mtime != Some(actual_mtime)
                    || task.source_size != Some(actual_size)
                    || task.total_size != actual_size
                {
                    return Err(PreflightFailure::local_changed(
                        "本地上传源已变化，需要重新规划",
                    ));
                }
            }
            TransferOperation::Download => {
                if task.direction != repository::transfer_direction::DOWNLOAD {
                    return Err(PreflightFailure::validation(
                        "Download operation 与 direction 不一致",
                    ));
                }
                if !has_nonempty(&task.file_id) {
                    return Err(PreflightFailure::validation("下载任务缺少 fileId"));
                }
                if task.expected_cloud_edited_time.is_none() {
                    return Err(PreflightFailure::validation("下载任务缺少云端版本"));
                }
                self.ensure_download_parent(local_path)?;
                match std::fs::metadata(local_path) {
                    Ok(metadata) if metadata.is_dir() => {
                        return Err(PreflightFailure::validation("下载目标不能是目录"));
                    }
                    Ok(metadata)
                        if !metadata.is_file()
                            || metadata.len() != 0
                            || !crate::mount::manager::is_placeholder_file(local_path) =>
                    {
                        return Err(PreflightFailure::local_changed(
                            "下载目标已出现本地内容，需要重新规划",
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(PreflightFailure::validation("下载目标不可访问"));
                    }
                }
            }
            TransferOperation::DownloadUpdate => {
                if task.direction != repository::transfer_direction::DOWNLOAD_UPDATE {
                    return Err(PreflightFailure::validation(
                        "DownloadUpdate operation 与 direction 不一致",
                    ));
                }
                if !has_nonempty(&task.file_id) {
                    return Err(PreflightFailure::validation("更新下载任务缺少 fileId"));
                }
                if task.expected_cloud_edited_time.is_none() {
                    return Err(PreflightFailure::validation("更新下载缺少云端版本"));
                }
                self.ensure_download_parent(local_path)?;
                let metadata = std::fs::symlink_metadata(local_path).map_err(|_| {
                    PreflightFailure::local_changed("更新下载目标已不存在，需要重新规划")
                })?;
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64);
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || task.source_mtime.is_none()
                    || task.source_size.is_none()
                    || task.source_mtime != mtime
                    || task.source_size != Some(metadata.len() as i64)
                {
                    return Err(PreflightFailure::local_changed(
                        "更新下载目标已变化或缺少版本快照，需要重新规划",
                    ));
                }
            }
            _ => {
                return Err(PreflightFailure::validation(
                    "该 operation 暂不支持安全重放",
                ))
            }
        }
        Ok(operation)
    }

    fn ensure_download_parent(&self, local_path: &Path) -> Result<(), PreflightFailure> {
        let parent = local_path
            .parent()
            .ok_or_else(|| PreflightFailure::validation("下载目标缺少父目录"))?;
        let relative_parent = parent
            .strip_prefix(&self.mount_root)
            .map_err(|_| PreflightFailure::validation("下载父目录不在配置的挂载根目录之下"))?;
        let canonical_root = self.mount_root.canonicalize().map_err(|error| {
            PreflightFailure::validation(format!("挂载根目录无法解析：{error}"))
        })?;
        let mut current = self.mount_root.clone();
        for component in relative_parent.components() {
            let std::path::Component::Normal(segment) = component else {
                return Err(PreflightFailure::validation("下载父目录包含非法路径分量"));
            };
            current.push(segment);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PreflightFailure::validation(
                        "下载父目录包含符号链接，拒绝越界文件操作",
                    ));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(PreflightFailure::validation("下载父路径不是目录"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&current).map_err(|error| {
                        PreflightFailure::validation(format!("创建下载父目录失败：{error}"))
                    })?;
                    let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
                        PreflightFailure::validation(format!("校验下载父目录失败：{error}"))
                    })?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(PreflightFailure::validation(
                            "下载父目录创建后被替换，拒绝继续",
                        ));
                    }
                }
                Err(error) => {
                    return Err(PreflightFailure::validation(format!(
                        "下载父目录不可访问：{error}"
                    )));
                }
            }
        }
        let canonical_parent = parent.canonicalize().map_err(|error| {
            PreflightFailure::validation(format!("下载父目录无法解析：{error}"))
        })?;
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(PreflightFailure::validation(
                "下载父目录解析到挂载根目录之外",
            ));
        }
        match std::fs::symlink_metadata(local_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PreflightFailure::validation(
                    "下载目标是符号链接，拒绝文件操作",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(PreflightFailure::validation(format!(
                    "下载目标不可访问：{error}"
                )));
            }
        }
        Ok(())
    }

    fn persist_preflight_rejection(
        &self,
        task: &TransferTask,
        failure: PreflightFailure,
    ) -> AppResult<TransferTask> {
        let current_state = task.state_kind().map_err(transition_error)?;
        if current_state == TransferState::Failed && failure.target == TransferState::Failed {
            let updated = {
                let conn = self.db.lock();
                repository::patch_transfer_in_state(
                    &conn,
                    task.id,
                    task.state_revision,
                    TransferState::Failed,
                    failure.patch(failure.target == TransferState::Failed),
                )
                .map_err(transition_error)?
            };
            self.notify_best_effort();
            return Ok(updated);
        }
        self.transition(
            task.id,
            task.state_revision,
            failure.target,
            failure.patch(failure.target == TransferState::Failed),
        )
    }

    fn transition_failure(
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

    fn transition(
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

    fn transition_clearing_upload_session(
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

    fn load(&self, task_id: i64) -> AppResult<TransferTask> {
        repository::get_transfer_by_id(&self.db.lock(), task_id)?
            .ok_or_else(|| AppError::generic("传输任务不存在"))
    }

    fn list_states(&self, states: &[TransferState]) -> AppResult<Vec<TransferTask>> {
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

    fn notify(&self) -> AppResult<()> {
        publish_state(&self.state_sink, &self.transfer_update_tx)
    }

    fn notify_best_effort(&self) {
        publish_state_best_effort(&self.state_sink, &self.transfer_update_tx);
    }

    fn notify_rejection(&self) {
        if let Err(error) = self.notify() {
            tracing::warn!(%error, "任务拒绝后重算状态失败");
        }
    }
}

#[derive(Debug, Clone)]
struct PreflightFailure {
    target: TransferState,
    kind: TransferErrorKind,
    message: String,
}

impl PreflightFailure {
    fn validation(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::Failed,
            kind: TransferErrorKind::Validation,
            message: message.into(),
        }
    }

    fn local_changed(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::RestartRequired,
            kind: TransferErrorKind::LocalChanged,
            message: message.into(),
        }
    }

    fn remote_ambiguous(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::VerifyingRemote,
            kind: TransferErrorKind::RemoteAmbiguous,
            message: message.into(),
        }
    }

    fn patch(&self, finished: bool) -> TransferPatch {
        TransferPatch {
            error_kind: ColumnPatch::Set(self.kind),
            error_message: ColumnPatch::Set(self.message.clone()),
            next_retry_at: ColumnPatch::Clear,
            finished_at: if finished {
                ColumnPatch::Set(chrono::Utc::now().timestamp_millis())
            } else {
                ColumnPatch::Clear
            },
            ..Default::default()
        }
    }
}

impl From<BackendPreflightFailure> for PreflightFailure {
    fn from(failure: BackendPreflightFailure) -> Self {
        Self {
            target: failure.target,
            kind: failure.kind,
            message: failure.message,
        }
    }
}

fn publish_state(
    state_sink: &Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: &Option<tokio::sync::broadcast::Sender<()>>,
) -> AppResult<()> {
    let snapshot_result = state_sink.read().recompute_and_broadcast();
    if let Some(sender) = transfer_update_tx {
        let _ = sender.send(());
    }
    snapshot_result
}

fn publish_state_best_effort(
    state_sink: &Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: &Option<tokio::sync::broadcast::Sender<()>>,
) {
    if let Err(error) = publish_state(state_sink, transfer_update_tx) {
        tracing::warn!(%error, "任务状态变化后重算权威快照失败");
    }
}

fn transition_error(error: impl std::fmt::Display) -> AppError {
    AppError::generic(error.to_string())
}

fn update_compatibility_sync_status(
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

fn mark_compatibility_sync_failed(
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
