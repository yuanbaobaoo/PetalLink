//! TaskRunner 对外合同与结果类型。

use std::sync::Arc;

use async_trait::async_trait;

use super::progress::TaskProgressReporter;
use crate::data::repository::TransferTask;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

pub type OnlineCheck = Arc<dyn Fn() -> bool + Send + Sync>;
pub type NowMs = Arc<dyn Fn() -> i64 + Send + Sync>;

pub trait TaskActivityGate: Send + Sync {
    fn begin(&self, relative_path: Option<&str>) -> AppResult<Box<dyn Send>>;
}

/// 每次接受或拒绝任务变更后，重新构建并发布完整的权威状态。
/// TaskRunner 只持有此接口，绝不依赖 SyncEngine。
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

/// 为引擎的基线与云树结算保留后端输出。
#[derive(Debug, Clone, Default)]
pub struct TaskExecutionOutcome {
    pub cloud_file: Option<DriveFile>,
    pub disposition: TaskDisposition,
}

#[derive(Debug, Clone)]
pub enum RemoteVerification {
    Committed(DriveFile),
    NotCommitted,
    Ambiguous(String),
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

    async fn verify_remote(&self, _task: &TransferTask) -> AppResult<RemoteVerification> {
        Ok(RemoteVerification::Ambiguous(
            "当前后端不支持远端结果核验".to_string(),
        ))
    }
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
