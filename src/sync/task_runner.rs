//! 提供持久化传输任务的执行门面，统一承载自动同步、手动重试与恢复入口。

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

/// 判定任务准入、去重与路径屏障。
mod admission;
/// 定义任务执行依赖与公开结果合同。
mod contracts;
/// 执行已准入任务并收敛恢复决策。
mod execution;
/// 封装传输任务的持久化读写。
mod persistence;
/// 在执行前验证来源与远端状态。
mod preflight;
/// 节流并持久化传输进度。
mod progress;
/// 发布权威任务状态与兼容通知。
mod publication;
/// 恢复进程重启前未完成的持久任务。
mod recovery;
/// 结算成功、重试、失败与取消状态。
mod settlement;

/// 持久传输任务的公开合同类型。
pub use crate::data::repository::TransferTask;
pub use contracts::{
    BackendPreflightFailure, EnqueuedTaskOutcome, NowMs, OnlineCheck, RecoveredCloudFile,
    RemoteVerification, StartupRecoverySummary, TaskActivityGate, TaskDisposition,
    TaskExecutionError, TaskExecutionOutcome, TaskRecoverySummary, TaskStateSink,
    TransferOperations,
};
pub use progress::TaskProgressReporter;

/// 负责持久化传输任务的准入、执行、恢复与状态发布。
pub struct TaskRunner {
    // 任务持久化与路径校验上下文。
    db: Arc<Mutex<rusqlite::Connection>>,
    mount_root: PathBuf,
    // 后端执行与运行时判定依赖。
    operations: Arc<dyn TransferOperations>,
    online_check: OnlineCheck,
    now_ms: NowMs,
    // 权威状态与兼容通知发布通道。
    state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    // 任务生命周期准入门。
    activity_gate: Arc<RwLock<Option<Arc<dyn TaskActivityGate>>>>,
}

impl TaskRunner {
    /// 使用系统时钟创建任务执行器。
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

    /// 使用指定时钟创建任务执行器。
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
}
