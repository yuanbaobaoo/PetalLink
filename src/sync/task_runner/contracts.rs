//! TaskRunner 对外合同与结果类型。

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use specta::Type;

use super::progress::TaskProgressReporter;
use crate::data::repository::TransferTask;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

/// 查询当前网络可用性的可共享回调。
pub type OnlineCheck = Arc<dyn Fn() -> bool + Send + Sync>;
/// 返回当前 epoch 毫秒的可共享时钟。
pub type NowMs = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 进入远端核验后的首次复核延迟（毫秒）。
pub(super) const VERIFY_FIRST_RECHECK_MS: i64 = 3_000;
/// 远端结果已确认但无法安全结算时的重试退避（毫秒）。
pub(super) const VERIFY_RETRY_BACKOFF_MS: i64 = 60_000;
/// 远端核验暂不可用（网络/限流）时的重试退避（毫秒）。
pub(super) const VERIFY_UNAVAILABLE_BACKOFF_MS: i64 = 15_000;

/// 上传任务结算为终态失败时的用户通知回调（任务快照 + 用户可读文案）。
///
/// 只有永久 Failed 才触发；VerifyingRemote/退避/等待网络等可恢复结算不触发，
/// 避免状态机随后的自动恢复被误报成“上传失败”。
pub type UploadFailedNotifier = Arc<dyn Fn(&TransferTask, &str) + Send + Sync>;

/// 传输任务成功结算后的通知回调（相对路径）。
///
/// 文件列表只在云端轮询周期里刷新时，上传完成后最长要等一个轮询间隔（默认 60 秒）
/// 才看得到新文件；结算成功即通知前端重载当前目录。
pub type TransferSettledNotifier = Arc<dyn Fn(&str) + Send + Sync>;

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
    /// 已到期、应交给后台执行通道的任务 ID。
    ///
    /// 恢复入口只负责「判定可运行」并返回 ID，由调度方投递到并发执行通道；
    /// 恢复循环内联执行会让一条大文件堵住整轮恢复（2026-11-23 压测：1 个/5.8s）。
    pub runnable_task_ids: Vec<i64>,
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
    /// 任务及本地记录均已完成。
    #[default]
    Completed,
    /// 任务已入队，等待执行。
    Pending,
    /// 任务正在执行。
    Running,
    /// 同一路径已有其他任务执行。
    BlockedByActiveIntent,
    /// 网络恢复后自动继续。
    WaitingForNetwork,
    /// 服务端退避时间到达后自动重试。
    BackingOff,
    /// 正在确认远端写入是否成功。
    VerifyingRemote,
    /// 文件状态变化，需重新检查后生成新任务。
    RestartRequired,
}

impl TaskDisposition {
    /// 活动持久状态对应的去向；终态（含 Completed）返回 None。
    /// 这是 TransferState → TaskDisposition 的唯一映射表，新增状态时编译器会要求在此补齐。
    pub fn from_active_state(state: TransferState) -> Option<Self> {
        match state {
            TransferState::Pending => Some(Self::Pending),
            TransferState::Running => Some(Self::Running),
            TransferState::WaitingForNetwork => Some(Self::WaitingForNetwork),
            TransferState::BackingOff => Some(Self::BackingOff),
            TransferState::VerifyingRemote => Some(Self::VerifyingRemote),
            TransferState::RestartRequired => Some(Self::RestartRequired),
            TransferState::Completed | TransferState::Failed | TransferState::Canceled => None,
        }
    }

    /// 可观察去向：Completed 也是有效的观察结果；仅 Failed/Canceled 返回 None。
    pub fn from_observable_state(state: TransferState) -> Option<Self> {
        match state {
            TransferState::Completed => Some(Self::Completed),
            other => Self::from_active_state(other),
        }
    }

    /// 返回适合直接展示给用户的任务状态说明。
    pub fn user_message(self) -> &'static str {
        match self {
            Self::Completed => "同步已完成。",
            Self::Pending => "任务已加入传输队列，稍后会自动开始。",
            Self::Running => "文件正在同步。",
            Self::BlockedByActiveIntent => "该文件正在执行其他同步任务，请稍后再试。",
            Self::WaitingForNetwork => "网络不可用，恢复后会自动继续。",
            Self::BackingOff => "服务暂时不可用，稍后会自动重试。",
            Self::VerifyingRemote => "正在确认上次同步是否成功。",
            Self::RestartRequired => "文件状态已变化，请重新检查并重试。",
        }
    }
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
    /// 目标目录存在同名但内容不一致的远端文件，必须等用户选择处理方式。
    NameConflict(String),
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
    /// 保持 Pending、应交给后台执行通道的任务 ID。
    ///
    /// 启动恢复只是「准入 + 去重」：把待执行任务交给统一并发通道，
    /// 而不是在恢复循环里逐条上传（否则 1000 条待传任务按 5 秒/条串行）。
    pub runnable_task_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
/// 任务入队并运行后的持久 ID 与执行结果。
pub struct EnqueuedTaskOutcome {
    pub task_id: i64,
    pub outcome: TaskExecutionOutcome,
}

/// 单个重试请求的入队结果。
///
/// `Queued`/`AlreadyQueued` 都属于成功：前者表示本次请求把任务推进到 Pending，
/// 后者表示任务本来就在队列里（幂等重复点击），前端无需区分即可继续。
#[derive(Debug, Clone, Serialize, Type)]
pub enum RetryEnqueued {
    /// 本次请求已把任务推进到 Pending。
    Queued {
        /// 被接受的任务 ID。
        task_id: i64,
    },
    /// 任务本来就在队列中（幂等），不视为错误。
    AlreadyQueued {
        /// 已处于队列中的任务 ID。
        task_id: i64,
    },
}

impl RetryEnqueued {
    /// 返回被接受或已在队列中的任务 ID。
    pub fn task_id(&self) -> i64 {
        match self {
            RetryEnqueued::Queued { task_id } | RetryEnqueued::AlreadyQueued { task_id } => {
                *task_id
            }
        }
    }

    /// 本次请求是否真的把任务推进到了 Pending。
    pub fn is_newly_queued(&self) -> bool {
        matches!(self, RetryEnqueued::Queued { .. })
    }
}

/// 一键重试全部失败项的汇总。
#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct RetryAllSummary {
    /// 本次新入队的任务数。
    pub queued: u32,
    /// 已在队列中而跳过的任务数。
    pub already_queued: u32,
    /// 不可重试而跳过（同名冲突、未知操作类型）。
    pub skipped: u32,
    /// 校验失败的任务数（源文件已变化等）。
    pub rejected: u32,
}

/// 一键重试的内部执行计划：对外汇总 + 需要在后台执行的任务 ID。
#[derive(Debug, Clone, Default)]
pub struct RetryAllPlan {
    /// 可直接返回给前端的汇总计数。
    pub summary: RetryAllSummary,
    /// 本次新入队、需要在后台启动执行的任务 ID。
    pub queued_task_ids: Vec<i64>,
}

/// 把已入队的任务交给执行通道在后台运行的回调。
///
/// 核验通道（探针）判定「未提交」时只投递重放意图，绝不同步等待整次上传，
/// 否则一个 GB 级文件就会把唯一的同步周期占住数十分钟。
pub type BackgroundTaskDispatcher = Arc<dyn Fn(i64) + Send + Sync>;

/// 只入队不执行的结果。
///
/// `disposition` 为 `Pending` 等可执行去向时，调用方需要自行在后台启动执行；
/// 为 `BlockedByActiveIntent` 时说明同路径已有活动意图，本次不重复调度。
#[derive(Debug, Clone)]
pub struct EnqueuedTask {
    /// 已落库（或已存在）的任务 ID。
    pub task_id: i64,
    /// 准入后的去向。
    pub disposition: TaskDisposition,
}
