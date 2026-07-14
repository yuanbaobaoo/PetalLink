//! TaskRunner 对外合同与结果类型。

use std::sync::Arc;

use async_trait::async_trait;

use super::progress::TaskProgressReporter;
use crate::data::repository::TransferTask;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

/// 查询当前网络可用性的可共享回调。
pub type OnlineCheck = Arc<dyn Fn() -> bool + Send + Sync>;
/// 返回当前 epoch 毫秒的可共享时钟。
pub type NowMs = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 为传输任务提供引擎关闭与路径冲突准入门。
pub trait TaskActivityGate: Send + Sync {
    /// 尝试登记任务活动，并返回自动释放的 guard。
    fn begin(&self, relative_path: Option<&str>) -> AppResult<Box<dyn Send>>;
}

/// 每次接受或拒绝任务变更后，重新构建并发布完整的权威状态。
/// TaskRunner 只持有此接口，绝不依赖 SyncEngine。
pub trait TaskStateSink: Send + Sync {
    /// 重算持久事实并广播完整同步状态。
    fn recompute_and_broadcast(&self) -> AppResult<()>;
}

impl<F> TaskStateSink for F
where
    F: Fn() -> AppResult<()> + Send + Sync,
{
    /// 调用闭包实现状态重算与发布。
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

/// 单个恢复任务已确认并结算的云端文件。
#[derive(Debug, Clone)]
pub struct RecoveredCloudFile {
    /// 任务持久化的规范相对路径。
    pub relative_path: String,
    /// 经过后端校验的完整云端元数据。
    pub file: DriveFile,
}

/// 在线恢复入口实际完成的任务及其权威云端结果。
#[derive(Debug, Clone, Default)]
pub struct TaskRecoverySummary {
    /// 本轮已进入 Completed 的任务数量。
    pub completed: usize,
    /// 需同步发布到云树检查点的远端写入结果。
    pub recovered_cloud_files: Vec<RecoveredCloudFile>,
}

#[derive(Debug, Clone)]
/// 对远程写入是否已提交的核验结果。
pub enum RemoteVerification {
    Committed(DriveFile),
    NotCommitted,
    Ambiguous(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// 任务执行或准入后的调度去向。
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
/// TaskRunner 执行传输所需的后端能力。
pub trait TransferOperations: Send + Sync {
    /// 在任务进入 Running 前执行本地与远程静态校验。
    async fn preflight(&self, _task: &TransferTask) -> Result<(), BackendPreflightFailure> {
        Ok(())
    }

    /// 执行传输并通过报告器持久进度。
    async fn execute(
        &self,
        task: &TransferTask,
        progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError>;

    /// 核实响应不确定的远程写入是否真实提交。
    async fn verify_remote(&self, _task: &TransferTask) -> AppResult<RemoteVerification> {
        Ok(RemoteVerification::Ambiguous(
            "当前后端不支持远端结果核验".to_string(),
        ))
    }
}

#[derive(Debug)]
/// 传输后端执行失败或需重新规划的原因。
pub enum TaskExecutionError {
    App(AppError),
    RestartRequired(String),
}

impl From<AppError> for TaskExecutionError {
    /// 将通用应用错误包装为任务执行错误。
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

#[derive(Debug, Clone)]
/// 后端前置校验对目标状态和错误分类的建议。
pub struct BackendPreflightFailure {
    pub kind: TransferErrorKind,
    pub target: TransferState,
    pub message: String,
}

impl BackendPreflightFailure {
    /// 构造需要重新规划的本地变化错误。
    pub fn restart_required(message: impl Into<String>) -> Self {
        Self {
            kind: TransferErrorKind::LocalChanged,
            target: TransferState::RestartRequired,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
/// 启动恢复期各类任务去向的统计。
pub struct StartupRecoverySummary {
    pub completed: usize,
    pub waiting_network: usize,
    pub verifying_remote: usize,
    pub failed: usize,
    /// 启动期已确认的远端写入结果。
    pub recovered_cloud_files: Vec<RecoveredCloudFile>,
}

#[derive(Debug, Clone)]
/// 任务入队并运行后的持久 ID 与执行结果。
pub struct EnqueuedTaskOutcome {
    pub task_id: i64,
    pub outcome: TaskExecutionOutcome,
}
