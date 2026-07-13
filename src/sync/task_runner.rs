//! Durable transfer task execution boundary shared by automatic sync, manual retry, startup
//! recovery and (from Task 5 onward) stable-online recovery.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};

use crate::data::repository::{
    self, ColumnPatch, RunningTransferPatch, TransferPatch, TransferTask,
};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::retry_policy::{classify_transfer_error, RecoveryContext, RecoveryDecision};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

const MAX_AUTOMATIC_ATTEMPTS: u32 = 5;
const PROGRESS_THROTTLE_MS: i64 = 500;

pub type OnlineCheck = Arc<dyn Fn() -> bool + Send + Sync>;
pub type NowMs = Arc<dyn Fn() -> i64 + Send + Sync>;

pub trait TaskActivityGate: Send + Sync {
    fn begin(&self) -> AppResult<Box<dyn Send>>;
}

/// Rebuild and publish the complete authoritative state after every accepted or rejected task
/// mutation. The runner owns only this interface and never depends on SyncEngine.
pub trait TaskStateSink: Send + Sync {
    fn recompute_and_broadcast(&self) -> AppResult<()>;
}

impl<F> TaskStateSink for F
where
    F: Fn() -> AppResult<()> + Send + Sync,
{
    fn recompute_and_broadcast(&self) -> AppResult<()> {
        self()
    }
}

/// Backend output retained for engine baseline/cloud-tree settlement.
#[derive(Debug, Clone, Default)]
pub struct TaskExecutionOutcome {
    pub cloud_file: Option<DriveFile>,
    pub disposition: TaskDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskDisposition {
    #[default]
    Completed,
    Pending,
    Running,
    BlockedByActiveIntent,
    WaitingForNetwork,
    BackingOff,
    VerifyingRemote,
    RestartRequired,
}

#[async_trait]
pub trait TransferOperations: Send + Sync {
    async fn preflight(&self, _task: &TransferTask) -> Result<(), BackendPreflightFailure> {
        Ok(())
    }

    async fn execute(
        &self,
        task: &TransferTask,
        progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError>;
}

#[derive(Debug)]
pub enum TaskExecutionError {
    App(AppError),
    RestartRequired(String),
}

impl From<AppError> for TaskExecutionError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

#[derive(Debug, Clone)]
pub struct BackendPreflightFailure {
    pub kind: TransferErrorKind,
    pub target: TransferState,
    pub message: String,
}

impl BackendPreflightFailure {
    pub fn restart_required(message: impl Into<String>) -> Self {
        Self {
            kind: TransferErrorKind::LocalChanged,
            target: TransferState::RestartRequired,
            message: message.into(),
        }
    }
}

#[derive(Clone)]
pub struct TaskProgressReporter {
    db: Arc<Mutex<rusqlite::Connection>>,
    task_id: i64,
    running_revision: i64,
    total_size: i64,
    state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    last_progress_ms: Arc<AtomicI64>,
}

impl TaskProgressReporter {
    fn new(
        db: Arc<Mutex<rusqlite::Connection>>,
        task_id: i64,
        running_revision: i64,
        total_size: i64,
        state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
        transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    ) -> Self {
        Self {
            db,
            task_id,
            running_revision,
            total_size,
            state_sink,
            transfer_update_tx,
            last_progress_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    pub fn update_transferred(&self, transferred: i64) -> AppResult<()> {
        if transferred < 0 || transferred > self.total_size {
            return Err(AppError::generic("传输进度超出任务总大小"));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let previous = self.last_progress_ms.load(Ordering::Relaxed);
        if previous != 0 && now.saturating_sub(previous) < PROGRESS_THROTTLE_MS {
            return Ok(());
        }
        self.last_progress_ms.store(now, Ordering::Relaxed);
        self.update(RunningTransferPatch {
            transferred: Some(transferred),
            ..Default::default()
        })
    }

    pub fn update_resume(
        &self,
        server_id: &str,
        upload_id: &str,
        offset: i64,
        session_url: &str,
    ) -> AppResult<()> {
        if offset < 0 || offset > self.total_size {
            return Err(AppError::generic("断点偏移超出任务总大小"));
        }
        if offset > 0 && session_url.trim().is_empty() {
            return Err(AppError::generic("非零断点缺少 session_url"));
        }
        self.update(RunningTransferPatch {
            transferred: Some(offset),
            resume_offset: Some(offset),
            server_id: ColumnPatch::Set(server_id.to_string()),
            upload_id: ColumnPatch::Set(upload_id.to_string()),
            session_url: ColumnPatch::Set(session_url.to_string()),
        })
    }

    pub fn ensure_current(&self) -> AppResult<()> {
        let task = repository::get_transfer_by_id(&self.db.lock(), self.task_id)?
            .ok_or_else(|| AppError::generic("传输任务不存在"))?;
        if task.state_revision != self.running_revision
            || task.state_kind().map_err(transition_error)? != TransferState::Running
        {
            return Err(AppError::generic("传输任务状态已变化，忽略过期回调"));
        }
        Ok(())
    }

    fn update(&self, patch: RunningTransferPatch) -> AppResult<()> {
        {
            let conn = self.db.lock();
            repository::update_running_transfer(&conn, self.task_id, self.running_revision, patch)
                .map_err(transition_error)?;
        }
        publish_state_best_effort(&self.state_sink, &self.transfer_update_tx);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupRecoverySummary {
    pub completed: usize,
    pub waiting_network: usize,
    pub verifying_remote: usize,
    pub failed: usize,
}

#[derive(Debug, Clone)]
pub struct EnqueuedTaskOutcome {
    pub task_id: i64,
    pub outcome: TaskExecutionOutcome,
}

enum ExistingOrInsertedTask {
    Existing(Box<TransferTask>),
    Replanned(Box<TransferTask>),
    Blocked(i64),
    Inserted(i64),
}

enum RunningGateOutcome {
    Running(Box<TransferTask>),
    Blocked,
}

fn is_path_blocking_state(state: TransferState) -> bool {
    matches!(
        state,
        TransferState::Pending
            | TransferState::Running
            | TransferState::WaitingForNetwork
            | TransferState::BackingOff
            | TransferState::VerifyingRemote
    )
}

fn same_transfer_intent(left: &TransferTask, right: &TransferTask) -> bool {
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

fn has_ambiguous_remote_write_result(task: &TransferTask) -> bool {
    matches!(
        task.operation_kind().ok().flatten(),
        Some(TransferOperation::Create | TransferOperation::Update)
    ) && has_persisted_remote_result(task)
}

fn has_persisted_remote_result(task: &TransferTask) -> bool {
    task.remote_result_file_id
        .as_deref()
        .is_some_and(|file_id| !file_id.trim().is_empty())
}

fn active_task_disposition(state: TransferState) -> Option<TaskDisposition> {
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

    fn begin_activity(&self) -> AppResult<Option<Box<dyn Send>>> {
        self.activity_gate
            .read()
            .clone()
            .map(|gate| gate.begin())
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
        let _activity = self.begin_activity()?;
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
        Ok(self
            .list_states(&[TransferState::BackingOff])?
            .into_iter()
            .filter_map(|task| task.next_retry_at)
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
            let _activity = self.begin_activity()?;
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
                        let _ = std::fs::remove_file(tmp_path);
                        let restart = self.transition_failure(
                            &task,
                            TransferState::RestartRequired,
                            TransferErrorKind::SessionExpired,
                            "进程中断，下载将从头重启",
                        )?;
                        let pending = self.transition(
                            restart.id,
                            restart.state_revision,
                            TransferState::Pending,
                            TransferPatch {
                                error_kind: ColumnPatch::Clear,
                                error_message: ColumnPatch::Clear,
                                finished_at: ColumnPatch::Clear,
                                transferred: Some(0),
                                resume_offset: Some(0),
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
        let _activity = self.begin_activity()?;
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
        let _activity = self.begin_activity()?;
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
                None,
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
        let metadata = std::fs::metadata(local_path)
            .map_err(|_| PreflightFailure::local_changed("成功核验时本地文件不存在"))?;
        if !metadata.is_file() {
            return Err(PreflightFailure::local_changed(
                "成功核验时本地目标不是普通文件",
            ));
        }
        match operation {
            TransferOperation::Create | TransferOperation::Update => {
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64);
                if running.source_mtime != mtime
                    || running.source_size != Some(metadata.len() as i64)
                {
                    return Err(PreflightFailure::local_changed("上传过程中本地源发生变化"));
                }
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
        let metadata = std::fs::metadata(local_path)
            .map_err(|error| AppError::generic(format!("成功结算读取本地文件失败：{error}")))?;
        if !metadata.is_file() {
            return Err(AppError::generic("成功结算目标不是普通文件"));
        }
        let local_mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .ok_or_else(|| AppError::generic("成功结算无法读取本地修改时间"))?;
        let local_size = metadata.len() as i64;
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
                if !local_path.is_file() {
                    return Err(PreflightFailure::local_changed(
                        "更新下载目标已不存在，需要重新规划",
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use tempfile::TempDir;

    use super::*;
    use crate::data::repository::{self, TransferTask};
    use crate::error::{AppError, AppResult};
    use crate::error::{DriveTransportKind, RequestSemantics, RetryAfter};
    use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<TransferTask>>,
        reporters: Mutex<Vec<TaskProgressReporter>>,
        results: Mutex<VecDeque<AppResult<TaskExecutionOutcome>>>,
        progress_bytes: Mutex<Option<i64>>,
        mutate_upload_source: AtomicBool,
    }

    impl FakeBackend {
        fn succeeding() -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from([Ok(TaskExecutionOutcome::default())])),
                ..Self::default()
            })
        }

        fn calls(&self) -> Vec<TransferTask> {
            self.calls.lock().clone()
        }

        fn fail_with(&self, error: AppError) {
            *self.results.lock() = VecDeque::from([Err(error)]);
        }
    }

    #[async_trait]
    impl TransferOperations for FakeBackend {
        async fn execute(
            &self,
            task: &TransferTask,
            progress: &TaskProgressReporter,
        ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
            self.calls.lock().push(task.clone());
            self.reporters.lock().push(progress.clone());
            if let Some(bytes) = *self.progress_bytes.lock() {
                progress.update_transferred(bytes)?;
            }
            let mut outcome = self
                .results
                .lock()
                .pop_front()
                .unwrap_or_else(|| Ok(TaskExecutionOutcome::default()))
                .map_err(TaskExecutionError::App)?;
            match task.operation_kind().unwrap() {
                Some(TransferOperation::Create | TransferOperation::Update) => {
                    if outcome.cloud_file.is_none() {
                        outcome.cloud_file = Some(crate::drive::models::DriveFile {
                            id: task.file_id.clone().unwrap_or_else(|| "created-id".into()),
                            name: task.name.clone(),
                            category: crate::drive::models::FileCategory::Document,
                            size: task.total_size,
                            parent_folder: task.parent_file_id.clone().map(|id| vec![id]),
                            description: None,
                            created_time: None,
                            edited_time: chrono::DateTime::from_timestamp_millis(123),
                            mime_type: None,
                            content_hash: None,
                            thumbnail_link: None,
                        });
                    }
                    if self.mutate_upload_source.load(Ordering::SeqCst) {
                        std::fs::write(task.local_path.as_deref().unwrap(), b"changed-source")
                            .unwrap();
                    }
                }
                Some(TransferOperation::Download | TransferOperation::DownloadUpdate) => {
                    if let Some(path) = task.local_path.as_deref() {
                        if let Some(parent) = Path::new(path).parent() {
                            std::fs::create_dir_all(parent).unwrap();
                        }
                        std::fs::write(path, vec![0; task.total_size as usize]).unwrap();
                    }
                }
                _ => {}
            }
            Ok(outcome)
        }
    }

    struct BlockingPreflightBackend {
        preflight_started: tokio::sync::Notify,
        preflight_release: tokio::sync::Semaphore,
        execute_calls: AtomicUsize,
        fail_execute: AtomicBool,
    }

    impl Default for BlockingPreflightBackend {
        fn default() -> Self {
            Self {
                preflight_started: tokio::sync::Notify::new(),
                preflight_release: tokio::sync::Semaphore::new(0),
                execute_calls: AtomicUsize::new(0),
                fail_execute: AtomicBool::new(false),
            }
        }
    }

    #[async_trait]
    impl TransferOperations for BlockingPreflightBackend {
        async fn preflight(&self, _task: &TransferTask) -> Result<(), BackendPreflightFailure> {
            self.preflight_started.notify_one();
            self.preflight_release
                .acquire()
                .await
                .expect("preflight gate closed")
                .forget();
            Ok(())
        }

        async fn execute(
            &self,
            task: &TransferTask,
            _progress: &TaskProgressReporter,
        ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
            self.execute_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_execute.load(Ordering::SeqCst) {
                return Err(AppError::generic("coordinated winner failed").into());
            }
            if matches!(
                task.operation_kind().unwrap(),
                Some(TransferOperation::Download | TransferOperation::DownloadUpdate)
            ) {
                std::fs::write(
                    task.local_path.as_deref().unwrap(),
                    vec![0; task.total_size as usize],
                )
                .map_err(|error| AppError::generic(error.to_string()))?;
            }
            Ok(TaskExecutionOutcome::default())
        }
    }

    struct BlockingExecuteBackend {
        execute_started: tokio::sync::Notify,
        execute_release: tokio::sync::Semaphore,
        calls: Mutex<Vec<i64>>,
    }

    struct ClosableActivityGate {
        accepting: AtomicBool,
    }

    impl ClosableActivityGate {
        fn new(accepting: bool) -> Self {
            Self {
                accepting: AtomicBool::new(accepting),
            }
        }

        fn close(&self) {
            self.accepting.store(false, Ordering::SeqCst);
        }
    }

    impl TaskActivityGate for ClosableActivityGate {
        fn begin(&self) -> AppResult<Box<dyn Send>> {
            if self.accepting.load(Ordering::SeqCst) {
                Ok(Box::new(()))
            } else {
                Err(AppError::generic("engine shutdown"))
            }
        }
    }

    impl Default for BlockingExecuteBackend {
        fn default() -> Self {
            Self {
                execute_started: tokio::sync::Notify::new(),
                execute_release: tokio::sync::Semaphore::new(0),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TransferOperations for BlockingExecuteBackend {
        async fn execute(
            &self,
            task: &TransferTask,
            _progress: &TaskProgressReporter,
        ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
            self.calls.lock().push(task.id);
            self.execute_started.notify_one();
            self.execute_release
                .acquire()
                .await
                .expect("execute gate closed")
                .forget();
            if let Some(path) = task.local_path.as_deref() {
                std::fs::write(path, vec![0; task.total_size as usize])
                    .map_err(|error| AppError::generic(error.to_string()))?;
            }
            Ok(TaskExecutionOutcome::default())
        }
    }

    struct Fixture {
        root: TempDir,
        db: Arc<Mutex<rusqlite::Connection>>,
        backend: Arc<FakeBackend>,
        online: Arc<AtomicBool>,
        notifications: Arc<AtomicUsize>,
        runner: Arc<TaskRunner>,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
            crate::data::migrations::run(&db.lock()).unwrap();
            let backend = FakeBackend::succeeding();
            let online = Arc::new(AtomicBool::new(true));
            let notifications = Arc::new(AtomicUsize::new(0));
            let online_check = {
                let online = online.clone();
                Arc::new(move || online.load(Ordering::SeqCst)) as OnlineCheck
            };
            let state_sink = {
                let notifications = notifications.clone();
                Arc::new(move || {
                    notifications.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }) as Arc<dyn TaskStateSink>
            };
            let runner = Arc::new(TaskRunner::new(
                db.clone(),
                root.path().to_path_buf(),
                backend.clone(),
                online_check,
                state_sink,
                None,
            ));
            Self {
                root,
                db,
                backend,
                online,
                notifications,
                runner,
            }
        }

        fn insert(&self, task: TransferTask) -> i64 {
            repository::insert_transfer(&self.db.lock(), &task).unwrap()
        }

        fn get(&self, id: i64) -> TransferTask {
            repository::get_transfer_by_id(&self.db.lock(), id)
                .unwrap()
                .unwrap()
        }

        fn path(&self, rel: &str) -> String {
            self.root.path().join(rel).to_string_lossy().into_owned()
        }
    }

    fn runner_with_backend(
        db: Arc<Mutex<rusqlite::Connection>>,
        mount_root: &Path,
        backend: Arc<dyn TransferOperations>,
    ) -> Arc<TaskRunner> {
        Arc::new(TaskRunner::new(
            db,
            mount_root.to_path_buf(),
            backend,
            Arc::new(|| true),
            Arc::new(|| Ok(())),
            None,
        ))
    }

    fn source_snapshot(path: &Path) -> (i64, i64) {
        let metadata = std::fs::metadata(path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        (mtime, metadata.len() as i64)
    }

    fn task(
        fixture: &Fixture,
        state: TransferState,
        operation: TransferOperation,
        rel: &str,
    ) -> TransferTask {
        let local_path = fixture.path(rel);
        let (direction, file_id, parent_file_id, source_mtime, source_size) = match operation {
            TransferOperation::Create | TransferOperation::Update => {
                if let Some(parent) = Path::new(&local_path).parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&local_path, b"payload").unwrap();
                let (mtime, size) = source_snapshot(Path::new(&local_path));
                (
                    repository::transfer_direction::UPLOAD,
                    (operation == TransferOperation::Update).then(|| "remote-id".to_string()),
                    Some("persisted-parent".to_string()),
                    Some(mtime),
                    Some(size),
                )
            }
            TransferOperation::Download => (
                repository::transfer_direction::DOWNLOAD,
                Some("remote-id".to_string()),
                rel.contains('/').then(|| "persisted-parent".to_string()),
                None,
                None,
            ),
            TransferOperation::DownloadUpdate => {
                if let Some(parent) = Path::new(&local_path).parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&local_path, b"olddata").unwrap();
                (
                    repository::transfer_direction::DOWNLOAD_UPDATE,
                    Some("remote-id".to_string()),
                    rel.contains('/').then(|| "persisted-parent".to_string()),
                    None,
                    None,
                )
            }
            _ => (
                repository::transfer_direction::DELETE,
                Some("remote-id".to_string()),
                None,
                None,
                None,
            ),
        };
        TransferTask {
            id: 0,
            direction,
            file_id,
            local_path: Some(local_path),
            name: rel.rsplit('/').next().unwrap().to_string(),
            total_size: source_size.unwrap_or(7),
            transferred: 0,
            state: i32::from(state),
            error_message: (state == TransferState::Failed).then(|| "old failure".into()),
            created_at: 1,
            finished_at: (state == TransferState::Failed).then_some(2),
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(rel.into()),
            parent_file_id,
            operation: Some(i32::from(operation)),
            source_mtime,
            source_size,
            expected_cloud_edited_time: Some(3),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: Some(i32::from(TransferErrorKind::Unknown)),
            remote_result_file_id: None,
            state_revision: 0,
        }
    }

    #[tokio::test]
    async fn failed_download_retry_validates_destination_then_executes_download() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Download,
            "nested/download.bin",
        ));

        fixture.runner.retry(id).await.unwrap();

        let calls = fixture.backend.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, id);
        assert_eq!(
            calls[0].operation_kind().unwrap(),
            Some(TransferOperation::Download)
        );
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Completed
        );
    }

    #[tokio::test]
    async fn missing_upload_source_stays_failed_with_same_task_id() {
        let fixture = Fixture::new();
        let task = task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Create,
            "missing.bin",
        );
        std::fs::remove_file(task.local_path.as_ref().unwrap()).unwrap();
        let id = fixture.insert(task);

        assert!(fixture.runner.retry(id).await.is_err());

        let rejected = fixture.get(id);
        assert_eq!(rejected.id, id);
        assert_eq!(rejected.state_kind().unwrap(), TransferState::Failed);
        assert_eq!(
            rejected.error_kind_typed().unwrap(),
            Some(TransferErrorKind::Validation)
        );
        assert!(fixture.backend.calls().is_empty());
        assert!(fixture.notifications.load(Ordering::SeqCst) > 0);
    }

    #[tokio::test]
    async fn subfolder_upload_retry_preserves_persisted_parent_id() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Create,
            "folder/upload.bin",
        ));

        fixture.runner.retry(id).await.unwrap();

        let calls = fixture.backend.calls();
        assert_eq!(calls[0].parent_file_id.as_deref(), Some("persisted-parent"));
    }

    #[tokio::test]
    async fn root_upload_allows_no_parent_id() {
        let fixture = Fixture::new();
        let mut root_task = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "root-upload.bin",
        );
        root_task.parent_file_id = None;
        let id = fixture.insert(root_task);

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls()[0].parent_file_id, None);
    }

    #[tokio::test]
    async fn malformed_or_unsupported_pending_task_fails_without_running() {
        let fixture = Fixture::new();
        let mut malformed = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "bad.bin",
        );
        malformed.operation = Some(99);
        let malformed_id = fixture.insert(malformed);
        let unsupported_id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Delete,
            "delete.bin",
        ));

        assert!(fixture.runner.run(malformed_id).await.is_err());
        assert!(fixture.runner.run(unsupported_id).await.is_err());

        assert_eq!(
            fixture.get(malformed_id).state_kind().unwrap(),
            TransferState::Failed
        );
        assert_eq!(
            fixture.get(unsupported_id).state_kind().unwrap(),
            TransferState::Failed
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn missing_required_ids_path_mismatch_and_stale_source_reject_preflight() {
        let fixture = Fixture::new();
        let mut no_file = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "no-file.bin",
        );
        no_file.file_id = None;
        let no_file_id = fixture.insert(no_file);

        let mut no_parent = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "folder/no-parent.bin",
        );
        no_parent.parent_file_id = None;
        let no_parent_id = fixture.insert(no_parent);

        let mut mismatch = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "expected.bin",
        );
        mismatch.local_path = Some(fixture.path("elsewhere.bin"));
        let mismatch_id = fixture.insert(mismatch);

        let mut stale = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "stale.bin",
        );
        stale.source_size = Some(stale.source_size.unwrap() + 1);
        let stale_id = fixture.insert(stale);

        for id in [no_file_id, no_parent_id, mismatch_id] {
            assert!(fixture.runner.run(id).await.is_err());
        }
        assert_eq!(
            fixture.runner.run(stale_id).await.unwrap().disposition,
            TaskDisposition::RestartRequired
        );

        assert_eq!(
            fixture.get(stale_id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert_eq!(
            fixture.get(stale_id).error_kind_typed().unwrap(),
            Some(TransferErrorKind::LocalChanged)
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn download_preflight_rejects_missing_mount_root_and_parent_file() {
        let missing_mount = Fixture::new();
        let missing_mount_id = missing_mount.insert(task(
            &missing_mount,
            TransferState::Pending,
            TransferOperation::Download,
            "missing-mount.bin",
        ));
        std::fs::remove_dir_all(missing_mount.root.path()).unwrap();

        assert!(missing_mount.runner.run(missing_mount_id).await.is_err());
        assert_eq!(
            missing_mount.get(missing_mount_id).state_kind().unwrap(),
            TransferState::Failed
        );
        assert!(missing_mount.backend.calls().is_empty());

        let parent_file = Fixture::new();
        std::fs::write(parent_file.root.path().join("blocker"), b"not a dir").unwrap();
        let parent_file_id = parent_file.insert(task(
            &parent_file,
            TransferState::Pending,
            TransferOperation::Download,
            "blocker/download.bin",
        ));

        assert!(parent_file.runner.run(parent_file_id).await.is_err());
        assert_eq!(
            parent_file.get(parent_file_id).state_kind().unwrap(),
            TransferState::Failed
        );
        assert!(parent_file.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn download_target_changes_before_running_require_replanning() {
        let new_download = Fixture::new();
        let new_download_id = new_download.insert(task(
            &new_download,
            TransferState::Pending,
            TransferOperation::Download,
            "became-nonempty.bin",
        ));
        std::fs::write(
            new_download.root.path().join("became-nonempty.bin"),
            b"local",
        )
        .unwrap();

        let outcome = new_download.runner.run(new_download_id).await.unwrap();
        assert_eq!(outcome.disposition, TaskDisposition::RestartRequired);
        assert_eq!(
            new_download.get(new_download_id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert!(new_download.backend.calls().is_empty());

        let update = Fixture::new();
        let update_task = task(
            &update,
            TransferState::Pending,
            TransferOperation::DownloadUpdate,
            "disappeared.bin",
        );
        std::fs::remove_file(update_task.local_path.as_ref().unwrap()).unwrap();
        let update_id = update.insert(update_task);

        let outcome = update.runner.run(update_id).await.unwrap();
        assert_eq!(outcome.disposition, TaskDisposition::RestartRequired);
        assert_eq!(
            update.get(update_id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert!(update.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn nonzero_upload_resume_without_session_is_rejected_before_api() {
        let fixture = Fixture::new();
        let mut malformed = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "bad-resume.bin",
        );
        malformed.resume_offset = 1;
        malformed.transferred = 1;
        malformed.session_url = None;
        let id = fixture.insert(malformed);

        assert!(fixture.runner.run(id).await.is_err());
        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn offline_pending_task_waits_without_calling_backend() {
        let fixture = Fixture::new();
        fixture.online.store(false, Ordering::SeqCst);
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "offline.bin",
        ));

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::WaitingForNetwork);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::WaitingForNetwork
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn startup_running_write_stops_for_verification_but_download_restarts_through_runner() {
        let fixture = Fixture::new();
        let write_id = fixture.insert(task(
            &fixture,
            TransferState::Running,
            TransferOperation::Create,
            "write.bin",
        ));
        let download_id = fixture.insert(task(
            &fixture,
            TransferState::Running,
            TransferOperation::Download,
            "download.bin",
        ));

        let summary = fixture.runner.recover_startup().await.unwrap();

        assert_eq!(summary.verifying_remote, 1);
        assert_eq!(summary.completed, 1);
        assert_eq!(
            fixture.get(write_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            fixture.get(download_id).state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(fixture.backend.calls().len(), 1);
        assert_eq!(fixture.backend.calls()[0].id, download_id);
    }

    #[tokio::test]
    async fn closed_activity_gate_prevents_startup_recovery_row_mutation() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Running,
            TransferOperation::Create,
            "startup-closed.bin",
        ));
        let before = fixture.get(id);
        let gate = Arc::new(ClosableActivityGate::new(false));
        fixture.runner.set_activity_gate(gate);

        let error = fixture.runner.recover_startup().await.unwrap_err();

        assert!(error.to_string().contains("shutdown"));
        let after = fixture.get(id);
        assert_eq!(after.state_kind().unwrap(), TransferState::Running);
        assert_eq!(after.state_revision, before.state_revision);
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn shutdown_between_waiting_rows_keeps_rejected_next_row_unchanged() {
        let fixture = Fixture::new();
        let backend = Arc::new(BlockingExecuteBackend::default());
        let runner = runner_with_backend(fixture.db.clone(), fixture.root.path(), backend.clone());
        let gate = Arc::new(ClosableActivityGate::new(true));
        runner.set_activity_gate(gate.clone());

        let mut first = task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "waiting-first.bin",
        );
        first.created_at = 2;
        let first_id = fixture.insert(first);
        let mut second = task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "waiting-invalid.bin",
        );
        second.created_at = 1;
        second.local_path = Some(
            tempfile::tempdir()
                .unwrap()
                .path()
                .join("outside.bin")
                .to_string_lossy()
                .into_owned(),
        );
        let second_id = fixture.insert(second);
        let second_before = fixture.get(second_id);

        let recovery = tokio::spawn(async move { runner.resume_waiting().await });
        backend.execute_started.notified().await;
        gate.close();
        backend.execute_release.add_permits(1);
        recovery.await.unwrap().unwrap();

        assert_eq!(
            fixture.get(first_id).state_kind().unwrap(),
            TransferState::Completed
        );
        let second_after = fixture.get(second_id);
        assert_eq!(
            second_after.state_kind().unwrap(),
            TransferState::WaitingForNetwork
        );
        assert_eq!(second_after.state_revision, second_before.state_revision);
        assert_eq!(backend.calls.lock().as_slice(), &[first_id]);
    }

    #[tokio::test]
    async fn startup_recovers_only_latest_pending_row_per_path() {
        let fixture = Fixture::new();
        let mut older = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "startup-duplicate.bin",
        );
        older.created_at = 1;
        let older_id = fixture.insert(older);
        let mut latest = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "startup-duplicate.bin",
        );
        latest.created_at = 2;
        let latest_id = fixture.insert(latest);

        let summary = fixture.runner.recover_startup().await.unwrap();

        assert_eq!(fixture.backend.calls().len(), 1);
        assert_eq!(fixture.backend.calls()[0].id, latest_id);
        assert_eq!(
            fixture.get(latest_id).state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(
            fixture.get(older_id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.failed, 1);
    }

    #[tokio::test]
    async fn startup_running_remote_write_suppresses_all_same_path_api_replay() {
        let fixture = Fixture::new();
        let mut older_running = task(
            &fixture,
            TransferState::Running,
            TransferOperation::Create,
            "startup-ambiguous-write.bin",
        );
        older_running.created_at = 1;
        let running_id = fixture.insert(older_running);
        let mut newer_pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "startup-ambiguous-write.bin",
        );
        newer_pending.created_at = 2;
        let pending_id = fixture.insert(newer_pending);

        let summary = fixture.runner.recover_startup().await.unwrap();

        assert!(fixture.backend.calls().is_empty());
        assert_eq!(
            fixture.get(running_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            fixture.get(pending_id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert_eq!(summary.verifying_remote, 1);
        assert_eq!(summary.failed, 1);
    }

    #[tokio::test]
    async fn startup_recovery_validates_download_path_before_touching_tmp_file() {
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.bin");
        let victim_tmp = outside.path().join("victim.bin.tmp");
        std::fs::write(&victim_tmp, b"sentinel").unwrap();
        let mut malformed = task(
            &fixture,
            TransferState::Running,
            TransferOperation::Download,
            "safe.bin",
        );
        malformed.local_path = Some(victim.to_string_lossy().into_owned());
        let id = fixture.insert(malformed);

        let summary = fixture.runner.recover_startup().await.unwrap();

        assert_eq!(std::fs::read(&victim_tmp).unwrap(), b"sentinel");
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(summary.failed, 1);
        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
        assert_eq!(
            fixture.get(id).error_kind_typed().unwrap(),
            Some(TransferErrorKind::Validation)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_recovery_rejects_symlink_parent_before_touching_outside_tmp() {
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        let outside_tmp = outside.path().join("victim.bin.tmp");
        std::fs::write(&outside_tmp, b"outside-sentinel").unwrap();
        std::os::unix::fs::symlink(outside.path(), fixture.root.path().join("link")).unwrap();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Running,
            TransferOperation::Download,
            "link/victim.bin",
        ));

        let summary = fixture.runner.recover_startup().await.unwrap();

        assert_eq!(std::fs::read(&outside_tmp).unwrap(), b"outside-sentinel");
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(summary.failed, 1);
        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
        assert_eq!(
            fixture.get(id).error_kind_typed().unwrap(),
            Some(TransferErrorKind::Validation)
        );
    }

    #[tokio::test]
    async fn progress_and_settlement_are_isolated_by_task_id_and_revision() {
        let fixture = Fixture::new();
        *fixture.backend.progress_bytes.lock() = Some(4);
        let first = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "same.bin",
        ));
        let second = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "same.bin",
        ));

        fixture.runner.run(first).await.unwrap();

        assert_eq!(
            fixture.get(first).state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(
            fixture.get(first).transferred,
            fixture.get(first).total_size
        );
        assert_eq!(
            fixture.get(second).state_kind().unwrap(),
            TransferState::Pending
        );
        assert_eq!(fixture.get(second).transferred, 0);
        assert_eq!(fixture.get(second).state_revision, 0);
    }

    #[tokio::test]
    async fn accepted_retry_increments_attempt_and_second_prepare_cannot_mutate() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Download,
            "retry.bin",
        ));

        let accepted = fixture.runner.prepare_retry(id).await.unwrap();
        assert_eq!(accepted.state_kind().unwrap(), TransferState::Pending);
        assert_eq!(accepted.attempt_count, 1);
        assert!(fixture.runner.prepare_retry(id).await.is_err());
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Pending
        );
    }

    #[tokio::test]
    async fn resume_waiting_rechecks_preflight_and_network() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "resume.bin",
        ));
        fixture.online.store(false, Ordering::SeqCst);
        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 0);
        assert!(fixture.backend.calls().is_empty());

        fixture.online.store(true, Ordering::SeqCst);
        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 1);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Completed
        );
    }

    #[tokio::test]
    async fn resume_waiting_does_not_count_failed_execution_as_resumed() {
        let fixture = Fixture::new();
        fixture.backend.fail_with(AppError::drive_from_response(
            403,
            "{}",
            None,
            RequestSemantics::Read,
            true,
        ));
        let id = fixture.insert(task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "resume-fails.bin",
        ));

        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 0);
        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
    }

    #[tokio::test]
    async fn running_gate_blocks_resume_waiting_then_allows_progress_after_resolution() {
        let fixture = Fixture::new();
        let waiting = task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Create,
            "gate-waiting.bin",
        );
        let mut ambiguous = waiting.clone();
        ambiguous.state = i32::from(TransferState::RestartRequired);
        ambiguous.remote_result_file_id = Some("known-remote-id".into());
        let waiting_id = fixture.insert(waiting);
        let ambiguous_id = fixture.insert(ambiguous);

        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 0);
        assert_eq!(
            fixture.get(waiting_id).state_kind().unwrap(),
            TransferState::WaitingForNetwork
        );
        assert_eq!(
            fixture.get(ambiguous_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert!(fixture.backend.calls().is_empty());

        let blocker = fixture.get(ambiguous_id);
        repository::transition_transfer(
            &fixture.db.lock(),
            ambiguous_id,
            blocker.state_revision,
            TransferState::Failed,
            TransferPatch {
                error_kind: ColumnPatch::Set(TransferErrorKind::RemoteAmbiguous),
                error_message: ColumnPatch::Set("verification resolved".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 1);
        assert_eq!(
            fixture.get(waiting_id).state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(fixture.backend.calls().len(), 1);
    }

    #[tokio::test]
    async fn running_gate_blocks_due_backoff_behind_ambiguous_restart() {
        let fixture = Fixture::new();
        let mut backing_off = task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Create,
            "gate-backoff.bin",
        );
        backing_off.next_retry_at = Some(chrono::Utc::now().timestamp_millis() - 1);
        let mut ambiguous = backing_off.clone();
        ambiguous.state = i32::from(TransferState::RestartRequired);
        ambiguous.next_retry_at = None;
        ambiguous.remote_result_file_id = Some("known-remote-id".into());
        let backing_off_id = fixture.insert(backing_off);
        let ambiguous_id = fixture.insert(ambiguous);

        assert_eq!(fixture.runner.resume_due_backoff().await.unwrap(), 0);
        assert_eq!(
            fixture.get(backing_off_id).state_kind().unwrap(),
            TransferState::BackingOff
        );
        assert_eq!(
            fixture.get(ambiguous_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn running_gate_blocks_prepared_manual_retry_behind_ambiguous_restart() {
        let fixture = Fixture::new();
        let failed = task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Create,
            "gate-manual.bin",
        );
        let mut ambiguous = failed.clone();
        ambiguous.state = i32::from(TransferState::RestartRequired);
        ambiguous.error_message = None;
        ambiguous.finished_at = None;
        ambiguous.remote_result_file_id = Some("known-remote-id".into());
        let failed_id = fixture.insert(failed);
        let ambiguous_id = fixture.insert(ambiguous);

        fixture.runner.prepare_retry(failed_id).await.unwrap();
        let outcome = fixture.runner.run_prepared(failed_id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::BlockedByActiveIntent);
        assert_eq!(
            fixture.get(failed_id).state_kind().unwrap(),
            TransferState::Pending
        );
        assert_eq!(
            fixture.get(ambiguous_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn backoff_before_deadline_does_not_call_backend() {
        let fixture = Fixture::new();
        let mut backing_off = task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Download,
            "backoff.bin",
        );
        backing_off.next_retry_at = Some(chrono::Utc::now().timestamp_millis() + 60_000);
        let id = fixture.insert(backing_off);

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::BackingOff);
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::BackingOff
        );
    }

    #[tokio::test]
    async fn backoff_without_deadline_fails_validation_without_backend_call() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Download,
            "missing-deadline.bin",
        ));

        assert_eq!(fixture.runner.resume_due_backoff().await.unwrap(), 0);
        let persisted = fixture.get(id);
        assert_eq!(persisted.state_kind().unwrap(), TransferState::Failed);
        assert_eq!(
            persisted.error_kind_typed().unwrap(),
            Some(TransferErrorKind::Validation)
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn backend_cannot_emit_backoff_without_a_persisted_deadline() {
        let fixture = Fixture::new();
        *fixture.backend.results.lock() = VecDeque::from([Ok(TaskExecutionOutcome {
            cloud_file: None,
            disposition: TaskDisposition::BackingOff,
        })]);
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "backend-backoff.bin",
        ));

        assert!(fixture.runner.run(id).await.is_err());
        let persisted = fixture.get(id);
        assert_eq!(persisted.state_kind().unwrap(), TransferState::Failed);
        assert!(persisted.next_retry_at.is_none());
        assert_eq!(fixture.backend.calls().len(), 1);
    }

    #[tokio::test]
    async fn prepare_retry_accepts_without_starting_backend() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Download,
            "scheduled.bin",
        ));

        let pending = fixture.runner.prepare_retry(id).await.unwrap();

        assert_eq!(pending.state_kind().unwrap(), TransferState::Pending);
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(pending.attempt_count, 1);
    }

    #[tokio::test]
    async fn manual_retry_stays_failed_until_backend_preflight_finishes() {
        let fixture = Fixture::new();
        let backend = Arc::new(BlockingPreflightBackend::default());
        let runner = runner_with_backend(fixture.db.clone(), fixture.root.path(), backend.clone());
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Download,
            "preflight-barrier.bin",
        ));

        let runner_for_retry = runner.clone();
        let retry = tokio::spawn(async move { runner_for_retry.prepare_retry(id).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.preflight_started.notified(),
        )
        .await
        .expect("manual retry must run backend preflight before acceptance");

        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
        assert_eq!(backend.execute_calls.load(Ordering::SeqCst), 0);

        backend.preflight_release.add_permits(1);
        let pending = retry.await.unwrap().unwrap();
        assert_eq!(pending.state_kind().unwrap(), TransferState::Pending);
        assert_eq!(backend.execute_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn manual_retry_keeps_home_and_transfer_failure_status_in_sync() {
        let fixture = Fixture::new();
        repository::upsert(
            &fixture.db.lock(),
            &repository::SyncItem {
                file_id: "remote-id".into(),
                local_path: "consistent.bin".into(),
                parent_folder_id: None,
                name: "consistent.bin".into(),
                is_folder: false,
                size: 7,
                local_size: Some(7),
                sha256: None,
                local_mtime: Some(1),
                cloud_edited_time: Some(3),
                last_sync_time: Some(4),
                status: repository::sync_status::FAILED,
                error_message: Some("old failure".into()),
            },
        )
        .unwrap();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Failed,
            TransferOperation::Download,
            "consistent.bin",
        ));

        fixture.runner.prepare_retry(id).await.unwrap();
        let accepted = repository::find_by_file_id(&fixture.db.lock(), "remote-id")
            .unwrap()
            .unwrap();
        assert_eq!(accepted.status, repository::sync_status::SYNCING);
        assert_eq!(accepted.error_message, None);

        fixture.backend.fail_with(AppError::drive_from_response(
            403,
            "{}",
            None,
            RequestSemantics::Read,
            true,
        ));
        assert!(fixture.runner.run_prepared(id).await.is_err());
        let failed = repository::find_by_file_id(&fixture.db.lock(), "remote-id")
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, repository::sync_status::FAILED);
        assert!(failed.error_message.is_some());
        assert_eq!(fixture.get(id).state_kind().unwrap(), TransferState::Failed);
    }

    #[tokio::test]
    async fn recoverable_backend_errors_persist_structured_scheduling_decisions() {
        struct Case {
            name: &'static str,
            operation: TransferOperation,
            error: AppError,
            expected_state: TransferState,
            expected_kind: TransferErrorKind,
            expected_attempts: i64,
        }

        let cases = [
            Case {
                name: "connect read",
                operation: TransferOperation::Download,
                error: AppError::drive_transport(
                    DriveTransportKind::Connect,
                    RequestSemantics::Read,
                    false,
                    Some("offline"),
                ),
                expected_state: TransferState::WaitingForNetwork,
                expected_kind: TransferErrorKind::Network,
                expected_attempts: 0,
            },
            Case {
                name: "timeout read",
                operation: TransferOperation::Download,
                error: AppError::drive_transport(
                    DriveTransportKind::Timeout,
                    RequestSemantics::Read,
                    false,
                    Some("timeout"),
                ),
                expected_state: TransferState::WaitingForNetwork,
                expected_kind: TransferErrorKind::Timeout,
                expected_attempts: 0,
            },
            Case {
                name: "rate limited",
                operation: TransferOperation::Download,
                error: AppError::drive_from_response(
                    429,
                    "{}",
                    Some(RetryAfter::DelaySeconds(3)),
                    RequestSemantics::Read,
                    false,
                ),
                expected_state: TransferState::BackingOff,
                expected_kind: TransferErrorKind::RateLimit,
                expected_attempts: 1,
            },
            Case {
                name: "server busy",
                operation: TransferOperation::Download,
                error: AppError::drive_from_response(
                    503,
                    "{}",
                    None,
                    RequestSemantics::Read,
                    false,
                ),
                expected_state: TransferState::BackingOff,
                expected_kind: TransferErrorKind::Server,
                expected_attempts: 1,
            },
            Case {
                name: "submitted write timeout",
                operation: TransferOperation::Create,
                error: AppError::drive_transport_with_submission(
                    DriveTransportKind::Timeout,
                    true,
                    false,
                    Some("lost response"),
                ),
                expected_state: TransferState::VerifyingRemote,
                expected_kind: TransferErrorKind::RemoteAmbiguous,
                expected_attempts: 0,
            },
            Case {
                name: "submitted write decode",
                operation: TransferOperation::Create,
                error: AppError::drive_transport_with_submission(
                    DriveTransportKind::Decode,
                    true,
                    false,
                    Some("invalid response"),
                ),
                expected_state: TransferState::VerifyingRemote,
                expected_kind: TransferErrorKind::RemoteAmbiguous,
                expected_attempts: 0,
            },
        ];

        for case in cases {
            let fixture = Fixture::new();
            fixture.backend.fail_with(case.error);
            let id = fixture.insert(task(
                &fixture,
                TransferState::Pending,
                case.operation,
                &format!("{}.bin", case.name.replace(' ', "-")),
            ));

            let outcome = fixture.runner.run(id).await.unwrap();
            let persisted = fixture.get(id);

            assert_eq!(
                persisted.state_kind().unwrap(),
                case.expected_state,
                "{}",
                case.name
            );
            assert_eq!(
                persisted.error_kind_typed().unwrap(),
                Some(case.expected_kind),
                "{}",
                case.name
            );
            assert!(persisted.error_message.is_some(), "{}", case.name);
            assert_eq!(
                persisted.attempt_count, case.expected_attempts,
                "{}",
                case.name
            );
            assert!(persisted.state_revision >= 2, "{}", case.name);
            assert!(
                fixture.notifications.load(Ordering::SeqCst) >= 2,
                "{}",
                case.name
            );
            if case.expected_state == TransferState::BackingOff {
                assert!(persisted.next_retry_at.is_some(), "{}", case.name);
                assert_eq!(outcome.disposition, TaskDisposition::BackingOff);
            }
        }
    }

    #[tokio::test]
    async fn first_401_and_permanent_error_fail_without_advancing_success_baseline() {
        for error in [
            AppError::drive_from_response(401, "{}", None, RequestSemantics::Read, false),
            AppError::drive_from_response(403, "{}", None, RequestSemantics::Read, true),
        ] {
            let fixture = Fixture::new();
            fixture.backend.fail_with(error);
            repository::upsert(
                &fixture.db.lock(),
                &repository::SyncItem {
                    file_id: "remote-id".into(),
                    local_path: "baseline.bin".into(),
                    parent_folder_id: None,
                    name: "baseline.bin".into(),
                    is_folder: false,
                    size: 41,
                    local_size: Some(41),
                    sha256: Some("baseline-hash".into()),
                    local_mtime: Some(111),
                    cloud_edited_time: Some(222),
                    last_sync_time: Some(333),
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                },
            )
            .unwrap();
            let id = fixture.insert(task(
                &fixture,
                TransferState::Pending,
                TransferOperation::Download,
                "baseline.bin",
            ));

            assert!(fixture.runner.run(id).await.is_err());

            let failed = fixture.get(id);
            assert_eq!(failed.state_kind().unwrap(), TransferState::Failed);
            assert!(matches!(
                failed.error_kind_typed().unwrap(),
                Some(TransferErrorKind::Auth | TransferErrorKind::Permission)
            ));
            let baseline = repository::find_by_file_id(&fixture.db.lock(), "remote-id")
                .unwrap()
                .unwrap();
            assert_eq!(baseline.local_mtime, Some(111));
            assert_eq!(baseline.local_size, Some(41));
            assert_eq!(baseline.cloud_edited_time, Some(222));
            assert_eq!(baseline.last_sync_time, Some(333));
            assert_eq!(baseline.sha256.as_deref(), Some("baseline-hash"));
            assert_eq!(baseline.status, repository::sync_status::FAILED);
        }
    }

    #[tokio::test]
    async fn illegal_terminal_state_is_rejected_without_backend_call() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Completed,
            TransferOperation::Download,
            "done.bin",
        ));
        let before = fixture.get(id);

        assert!(fixture.runner.run(id).await.is_err());

        let after = fixture.get(id);
        assert_eq!(after.state_revision, before.state_revision);
        assert_eq!(after.state_kind().unwrap(), TransferState::Completed);
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn enqueue_failure_never_calls_backend_and_success_reuses_inserted_task_id() {
        let fixture = Fixture::new();
        let pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "enqueue.bin",
        );
        let result = fixture.runner.enqueue_and_run(pending).await.unwrap();
        assert_eq!(fixture.backend.calls().len(), 1);
        assert_eq!(fixture.backend.calls()[0].id, result.task_id);
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let broken = Fixture::new();
        broken
            .db
            .lock()
            .execute_batch("DROP TABLE transfer_queue")
            .unwrap();
        let pending = task(
            &broken,
            TransferState::Pending,
            TransferOperation::Download,
            "never-called.bin",
        );
        assert!(broken.runner.enqueue_and_run(pending).await.is_err());
        assert!(broken.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn automatic_cycle_reuses_future_and_due_backoff_rows() {
        let future = Fixture::new();
        let mut existing = task(
            &future,
            TransferState::BackingOff,
            TransferOperation::Download,
            "future-backoff.bin",
        );
        existing.next_retry_at = Some(chrono::Utc::now().timestamp_millis() + 60_000);
        let existing_id = future.insert(existing);

        let outcome = future
            .runner
            .enqueue_and_run(task(
                &future,
                TransferState::Pending,
                TransferOperation::Download,
                "future-backoff.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, existing_id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::BackingOff);
        assert!(future.backend.calls().is_empty());
        let count: i64 = future
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let due = Fixture::new();
        let mut existing = task(
            &due,
            TransferState::BackingOff,
            TransferOperation::Download,
            "due-backoff.bin",
        );
        existing.next_retry_at = Some(chrono::Utc::now().timestamp_millis() - 1);
        let existing_id = due.insert(existing);

        let outcome = due
            .runner
            .enqueue_and_run(task(
                &due,
                TransferState::Pending,
                TransferOperation::Download,
                "due-backoff.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, existing_id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(due.backend.calls()[0].id, existing_id);
        let count: i64 = due
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn update_backoff_with_old_cloud_version_is_replanned_not_exactly_reused() {
        let fixture = Fixture::new();
        let mut old = task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Update,
            "update-version.bin",
        );
        old.expected_cloud_edited_time = Some(100);
        old.next_retry_at = Some(chrono::Utc::now().timestamp_millis() + 60_000);
        let mut latest = old.clone();
        latest.state = i32::from(TransferState::Pending);
        latest.next_retry_at = None;
        latest.expected_cloud_edited_time = Some(200);
        let id = fixture.insert(old);

        let outcome = fixture.runner.enqueue_and_run(latest).await.unwrap();

        assert_eq!(outcome.task_id, id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls().len(), 1);
        assert_eq!(
            fixture.backend.calls()[0].expected_cloud_edited_time,
            Some(200)
        );
    }

    #[tokio::test]
    async fn unresolved_active_rows_block_duplicate_automatic_attempts() {
        for (state, expected) in [
            (TransferState::Running, TaskDisposition::Running),
            (
                TransferState::VerifyingRemote,
                TaskDisposition::VerifyingRemote,
            ),
        ] {
            let fixture = Fixture::new();
            let existing_id = fixture.insert(task(
                &fixture,
                state,
                TransferOperation::Download,
                "active.bin",
            ));

            let outcome = fixture
                .runner
                .enqueue_and_run(task(
                    &fixture,
                    TransferState::Pending,
                    TransferOperation::Download,
                    "active.bin",
                ))
                .await
                .unwrap();

            assert_eq!(outcome.task_id, existing_id, "{state:?}");
            assert_eq!(outcome.outcome.disposition, expected, "{state:?}");
            assert!(fixture.backend.calls().is_empty(), "{state:?}");
            let count: i64 = fixture
                .db
                .lock()
                .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1, "{state:?}");
        }
    }

    #[tokio::test]
    async fn same_path_different_intent_never_executes_or_reports_the_old_task() {
        let fixture = Fixture::new();
        let mut existing = task(
            &fixture,
            TransferState::Running,
            TransferOperation::Download,
            "identity.bin",
        );
        existing.file_id = Some("old-remote".into());
        let existing_id = fixture.insert(existing);
        let mut replacement = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "identity.bin",
        );
        replacement.file_id = Some("new-remote".into());

        let outcome = fixture.runner.enqueue_and_run(replacement).await.unwrap();

        assert_eq!(outcome.task_id, existing_id);
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::BlockedByActiveIntent
        );
        assert_eq!(
            fixture.get(existing_id).state_kind().unwrap(),
            TransferState::Running
        );
        assert!(fixture.backend.calls().is_empty());
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn different_running_intent_blocks_even_when_an_exact_pending_row_also_exists() {
        let fixture = Fixture::new();
        let pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "legacy-multiple.bin",
        );
        let pending_id = fixture.insert(pending.clone());
        let mut running = pending.clone();
        running.state = i32::from(TransferState::Running);
        running.file_id = Some("different-running-remote".into());
        running.created_at = 2;
        let running_id = fixture.insert(running);

        let outcome = fixture.runner.enqueue_and_run(pending).await.unwrap();

        assert_eq!(outcome.task_id, running_id);
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::BlockedByActiveIntent
        );
        assert_eq!(
            fixture.get(pending_id).state_kind().unwrap(),
            TransferState::Pending
        );
        assert!(fixture.backend.calls().is_empty());
    }

    #[tokio::test]
    async fn different_pending_intent_is_atomically_replanned_before_any_backend_call() {
        let fixture = Fixture::new();
        let mut old = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "pending-replan.bin",
        );
        old.file_id = Some("old-remote".into());
        let id = fixture.insert(old);
        let mut latest = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "pending-replan.bin",
        );
        latest.file_id = Some("new-remote".into());

        let outcome = fixture.runner.enqueue_and_run(latest).await.unwrap();

        assert_eq!(outcome.task_id, id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls().len(), 1);
        assert_eq!(
            fixture.backend.calls()[0].file_id.as_deref(),
            Some("new-remote")
        );
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn restart_required_is_replanned_on_same_task_id_without_row_growth() {
        let fixture = Fixture::new();
        let existing_id = fixture.insert(task(
            &fixture,
            TransferState::RestartRequired,
            TransferOperation::Download,
            "restart-replan.bin",
        ));

        let outcome = fixture
            .runner
            .enqueue_and_run(task(
                &fixture,
                TransferState::Pending,
                TransferOperation::Download,
                "restart-replan.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, existing_id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls()[0].id, existing_id);
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn ambiguous_remote_write_restart_promotes_to_verifying_without_second_create() {
        let fixture = Fixture::new();
        let mut ambiguous = task(
            &fixture,
            TransferState::RestartRequired,
            TransferOperation::Create,
            "ambiguous-restart.bin",
        );
        ambiguous.remote_result_file_id = Some("known-remote-id".into());
        let id = fixture.insert(ambiguous);

        let outcome = fixture
            .runner
            .enqueue_and_run(task(
                &fixture,
                TransferState::Pending,
                TransferOperation::Create,
                "ambiguous-restart.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, id);
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::VerifyingRemote
        );
        assert!(fixture.backend.calls().is_empty());
        let persisted = fixture.get(id);
        assert_eq!(
            persisted.state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            persisted.remote_result_file_id.as_deref(),
            Some("known-remote-id")
        );
    }

    #[tokio::test]
    async fn ambiguous_restart_blocks_existing_exact_pending_row_before_any_replay() {
        let fixture = Fixture::new();
        let mut ambiguous = task(
            &fixture,
            TransferState::RestartRequired,
            TransferOperation::Create,
            "ambiguous-plus-pending.bin",
        );
        ambiguous.remote_result_file_id = Some("known-remote-id".into());
        ambiguous.created_at = 1;
        let ambiguous_id = fixture.insert(ambiguous);
        let mut pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "ambiguous-plus-pending.bin",
        );
        pending.created_at = 2;
        let pending_id = fixture.insert(pending.clone());

        let outcome = fixture.runner.enqueue_and_run(pending).await.unwrap();

        assert_eq!(outcome.task_id, ambiguous_id);
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::VerifyingRemote
        );
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(
            fixture.get(ambiguous_id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            fixture.get(ambiguous_id).remote_result_file_id.as_deref(),
            Some("known-remote-id")
        );
        assert_eq!(
            fixture.get(pending_id).state_kind().unwrap(),
            TransferState::Pending
        );

        let summary = fixture.runner.recover_startup().await.unwrap();
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(summary.completed, 0);
        assert_eq!(
            fixture.get(pending_id).state_kind().unwrap(),
            TransferState::Pending
        );
    }

    #[tokio::test]
    async fn waiting_task_that_restarts_can_be_consumed_by_next_planner_intent() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "waiting-replan.bin",
        ));
        std::fs::write(
            fixture.root.path().join("waiting-replan.bin"),
            b"local-change",
        )
        .unwrap();

        assert_eq!(fixture.runner.resume_waiting().await.unwrap(), 1);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::RestartRequired
        );
        assert!(fixture.backend.calls().is_empty());

        let outcome = fixture
            .runner
            .enqueue_and_run(task(
                &fixture,
                TransferState::Pending,
                TransferOperation::Create,
                "waiting-replan.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls()[0].id, id);
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn automatic_cycle_resumes_waiting_row_instead_of_inserting_duplicate() {
        let fixture = Fixture::new();
        let existing_id = fixture.insert(task(
            &fixture,
            TransferState::WaitingForNetwork,
            TransferOperation::Download,
            "waiting-active.bin",
        ));

        let outcome = fixture
            .runner
            .enqueue_and_run(task(
                &fixture,
                TransferState::Pending,
                TransferOperation::Download,
                "waiting-active.bin",
            ))
            .await
            .unwrap();

        assert_eq!(outcome.task_id, existing_id);
        assert_eq!(outcome.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(fixture.backend.calls()[0].id, existing_id);
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn concurrent_automatic_cycle_reuses_running_row() {
        let fixture = Fixture::new();
        let backend = Arc::new(BlockingExecuteBackend::default());
        let runner = runner_with_backend(fixture.db.clone(), fixture.root.path(), backend.clone());
        let first_task = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "concurrent.bin",
        );
        let duplicate = first_task.clone();
        let first_runner = runner.clone();
        let first = tokio::spawn(async move { first_runner.enqueue_and_run(first_task).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.execute_started.notified(),
        )
        .await
        .expect("first automatic cycle must reach backend execution");

        let second = runner.enqueue_and_run(duplicate).await.unwrap();
        assert_eq!(second.outcome.disposition, TaskDisposition::Running);
        assert_eq!(&*backend.calls.lock(), &[second.task_id]);
        let count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        backend.execute_release.add_permits(1);
        let first = first.await.unwrap().unwrap();
        assert_eq!(first.task_id, second.task_id);
        assert_eq!(first.outcome.disposition, TaskDisposition::Completed);
    }

    #[tokio::test]
    async fn cas_loser_observes_winner_completed_instead_of_returning_stale_error() {
        let fixture = Fixture::new();
        let backend = Arc::new(BlockingPreflightBackend::default());
        let runner = runner_with_backend(fixture.db.clone(), fixture.root.path(), backend.clone());
        let pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "cas-completed.bin",
        );
        let duplicate = pending.clone();
        let first_runner = runner.clone();
        let first = tokio::spawn(async move { first_runner.enqueue_and_run(pending).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.preflight_started.notified(),
        )
        .await
        .unwrap();
        let second_runner = runner.clone();
        let second = tokio::spawn(async move { second_runner.enqueue_and_run(duplicate).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.preflight_started.notified(),
        )
        .await
        .unwrap();

        backend.preflight_release.add_permits(2);
        let (first, second) = tokio::join!(first, second);
        let first = first.unwrap().unwrap();
        let second = second.unwrap().unwrap();

        assert_eq!(first.task_id, second.task_id);
        assert_eq!(first.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(second.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(backend.execute_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cas_loser_reports_winner_failed_as_converged_not_stale_revision() {
        let fixture = Fixture::new();
        let backend = Arc::new(BlockingPreflightBackend::default());
        backend.fail_execute.store(true, Ordering::SeqCst);
        let runner = runner_with_backend(fixture.db.clone(), fixture.root.path(), backend.clone());
        let pending = task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "cas-failed.bin",
        );
        let duplicate = pending.clone();
        let first_runner = runner.clone();
        let first = tokio::spawn(async move { first_runner.enqueue_and_run(pending).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.preflight_started.notified(),
        )
        .await
        .unwrap();
        let second_runner = runner.clone();
        let second = tokio::spawn(async move { second_runner.enqueue_and_run(duplicate).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            backend.preflight_started.notified(),
        )
        .await
        .unwrap();

        backend.preflight_release.add_permits(2);
        let (first, second) = tokio::join!(first, second);
        let messages = [
            first.unwrap().unwrap_err().to_string(),
            second.unwrap().unwrap_err().to_string(),
        ];

        assert!(messages
            .iter()
            .any(|message| message.contains("已由并发执行收敛为 Failed")));
        assert!(messages
            .iter()
            .all(|message| !message.contains("stale transfer revision")));
        assert_eq!(backend.execute_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn due_backoff_scheduler_seam_resumes_same_row_without_sleeping() {
        let fixture = Fixture::new();
        let mut due = task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Download,
            "scheduler-due.bin",
        );
        due.next_retry_at = Some(chrono::Utc::now().timestamp_millis() - 1);
        let id = fixture.insert(due);

        assert_eq!(fixture.runner.resume_due_backoff().await.unwrap(), 1);
        assert_eq!(fixture.backend.calls()[0].id, id);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Completed
        );
    }

    #[tokio::test]
    async fn closed_engine_activity_gate_prevents_running_and_backend_submission() {
        struct ClosedGate;
        impl super::TaskActivityGate for ClosedGate {
            fn begin(&self) -> AppResult<Box<dyn Send>> {
                Err(AppError::generic("engine shutdown"))
            }
        }

        let fixture = Fixture::new();
        fixture.runner.set_activity_gate(Arc::new(ClosedGate));
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "shutdown-gate.bin",
        ));

        let error = fixture.runner.run(id).await.unwrap_err();

        assert!(error.to_string().contains("shutdown"));
        assert!(fixture.backend.calls().is_empty());
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Pending
        );
    }

    #[tokio::test]
    async fn injected_clock_resumes_due_backoff_at_exact_deadline_with_same_id() {
        let fixture = Fixture::new();
        let now = Arc::new(AtomicI64::new(10_000));
        let clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst)) as super::NowMs
        };
        let runner = TaskRunner::new_with_clock(
            fixture.db.clone(),
            fixture.root.path().to_path_buf(),
            fixture.backend.clone(),
            Arc::new(|| true),
            Arc::new(|| Ok(())),
            None,
            clock,
        );
        let mut due = task(
            &fixture,
            TransferState::BackingOff,
            TransferOperation::Download,
            "clock-due.bin",
        );
        due.next_retry_at = Some(12_000);
        let id = fixture.insert(due);

        assert_eq!(runner.next_backoff_deadline_ms().unwrap(), Some(12_000));
        assert_eq!(runner.resume_due_backoff().await.unwrap(), 0);
        now.store(11_999, Ordering::SeqCst);
        assert_eq!(runner.resume_due_backoff().await.unwrap(), 0);
        now.store(12_000, Ordering::SeqCst);
        assert_eq!(runner.resume_due_backoff().await.unwrap(), 1);
        assert_eq!(fixture.backend.calls()[0].id, id);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(runner.next_backoff_deadline_ms().unwrap(), None);
    }

    #[tokio::test]
    async fn settled_task_rejects_late_progress_callback() {
        let fixture = Fixture::new();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "late.bin",
        ));
        fixture.runner.run(id).await.unwrap();
        let reporter = fixture.backend.reporters.lock()[0].clone();
        let before = fixture.get(id);

        assert!(reporter.update_transferred(1).is_err());

        let after = fixture.get(id);
        assert_eq!(after.state_revision, before.state_revision);
        assert_eq!(after.transferred, before.transferred);
        assert_eq!(after.state_kind().unwrap(), TransferState::Completed);
    }

    #[tokio::test]
    async fn retry_publishes_failed_pending_running_completed_for_one_row() {
        let root = tempfile::tempdir().unwrap();
        let db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        crate::data::migrations::run(&db.lock()).unwrap();
        let backend = FakeBackend::succeeding();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let db = db.clone();
            let observed = observed.clone();
            Arc::new(move || {
                let tasks = repository::list_all_transfers(&db.lock())?;
                if let Some(task) = tasks.first() {
                    observed.lock().push(
                        task.state_kind()
                            .map_err(|error| AppError::generic(error.to_string()))?,
                    );
                }
                Ok(())
            }) as Arc<dyn TaskStateSink>
        };
        let runner = TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend,
            Arc::new(|| true),
            sink,
            None,
        );
        let local_path = root.path().join("sequence.bin");
        std::fs::write(&local_path, b"payload").unwrap();
        let (source_mtime, source_size) = source_snapshot(&local_path);
        let id = repository::insert_transfer(
            &db.lock(),
            &TransferTask {
                id: 0,
                direction: repository::transfer_direction::UPLOAD,
                file_id: None,
                local_path: Some(local_path.to_string_lossy().into_owned()),
                name: "sequence.bin".into(),
                total_size: source_size,
                transferred: 0,
                state: i32::from(TransferState::Failed),
                error_message: Some("old".into()),
                created_at: 1,
                finished_at: Some(2),
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some("sequence.bin".into()),
                parent_file_id: Some("persisted-parent".into()),
                operation: Some(i32::from(TransferOperation::Create)),
                source_mtime: Some(source_mtime),
                source_size: Some(source_size),
                expected_cloud_edited_time: None,
                attempt_count: 0,
                next_retry_at: None,
                error_kind: Some(i32::from(TransferErrorKind::Unknown)),
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();

        runner.retry(id).await.unwrap();

        assert_eq!(
            &*observed.lock(),
            &[
                TransferState::Pending,
                TransferState::Running,
                TransferState::Completed,
            ]
        );
        let rows: i64 = db
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM transfer_queue WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(
            repository::get_transfer_by_id(&db.lock(), id)
                .unwrap()
                .unwrap()
                .state_revision,
            3
        );
    }

    #[tokio::test]
    async fn baseline_write_failure_rolls_back_completed_transition() {
        let fixture = Fixture::new();
        fixture
            .db
            .lock()
            .execute_batch(
                "CREATE TRIGGER reject_sync_baseline
                 BEFORE INSERT ON sync_items
                 BEGIN SELECT RAISE(FAIL, 'forced baseline failure'); END;",
            )
            .unwrap();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Download,
            "rollback.bin",
        ));

        let outcome = fixture.runner.run(id).await.unwrap();

        let task = fixture.get(id);
        assert_eq!(outcome.disposition, TaskDisposition::RestartRequired);
        assert_eq!(task.state_kind().unwrap(), TransferState::RestartRequired);
        let baseline_count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM sync_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(baseline_count, 0);
    }

    #[tokio::test]
    async fn upload_baseline_failure_preserves_remote_identity_for_verification() {
        let fixture = Fixture::new();
        fixture
            .db
            .lock()
            .execute_batch(
                "CREATE TRIGGER reject_upload_baseline
                 BEFORE INSERT ON sync_items
                 BEGIN SELECT RAISE(FAIL, 'forced upload baseline failure'); END;",
            )
            .unwrap();
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "upload-rollback.bin",
        ));

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::VerifyingRemote);
        let persisted = fixture.get(id);
        assert_eq!(
            persisted.state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            persisted.remote_result_file_id.as_deref(),
            Some("created-id")
        );
        let baseline_count: i64 = fixture
            .db
            .lock()
            .query_row("SELECT COUNT(*) FROM sync_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(baseline_count, 0);
    }

    #[tokio::test]
    async fn incomplete_upload_metadata_stops_in_verifying_remote_without_baseline() {
        let fixture = Fixture::new();
        let partial = crate::drive::models::DriveFile {
            id: "known-remote-id".into(),
            name: "partial.bin".into(),
            category: crate::drive::models::FileCategory::Document,
            size: 7,
            parent_folder: Some(vec!["persisted-parent".into()]),
            description: None,
            created_time: None,
            edited_time: None,
            mime_type: None,
            content_hash: None,
            thumbnail_link: None,
        };
        *fixture.backend.results.lock() = VecDeque::from([Ok(TaskExecutionOutcome {
            cloud_file: Some(partial),
            disposition: TaskDisposition::VerifyingRemote,
        })]);
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "partial.bin",
        ));

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::VerifyingRemote);
        let persisted = fixture.get(id);
        assert_eq!(
            persisted.state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            persisted.remote_result_file_id.as_deref(),
            Some("known-remote-id")
        );
        assert!(repository::load_all(&fixture.db.lock()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn source_changed_during_upload_never_completes_or_advances_baseline() {
        let fixture = Fixture::new();
        fixture
            .backend
            .mutate_upload_source
            .store(true, Ordering::SeqCst);
        let id = fixture.insert(task(
            &fixture,
            TransferState::Pending,
            TransferOperation::Create,
            "changing.bin",
        ));

        let outcome = fixture.runner.run(id).await.unwrap();

        assert_eq!(outcome.disposition, TaskDisposition::VerifyingRemote);
        assert_eq!(
            fixture.get(id).state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            fixture.get(id).remote_result_file_id.as_deref(),
            Some("created-id")
        );
        assert!(repository::load_all(&fixture.db.lock()).unwrap().is_empty());
    }

    #[test]
    fn fake_backend_error_is_constructible_for_recovery_tests() {
        let error = AppError::generic("boom");
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn transfer_list_notification_survives_snapshot_sink_failure() {
        let sink: Arc<RwLock<Arc<dyn TaskStateSink>>> = Arc::new(RwLock::new(Arc::new(|| {
            Err(AppError::generic("snapshot failed"))
        })));
        let (sender, mut receiver) = tokio::sync::broadcast::channel(1);

        assert!(publish_state(&sink, &Some(sender)).is_err());
        assert!(receiver.try_recv().is_ok());
    }
}
