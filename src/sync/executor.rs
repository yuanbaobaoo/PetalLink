//! 同步执行器 —— 并发池 + 传输队列 + 稳定性检查 + 配额校验。
//!
//! 对齐 `legacy/lib/sync/sync_executor.dart`。
//!
//! 并发数默认 6（可配置 1-20），使用 tokio Semaphore 限流。
//! 传输队列（TransferQueue 表）记录进度，修剪历史（保留最近 100 条已结束任务）。

use std::sync::Arc;

use crate::drive::{download_api::DownloadApi, files_api::FilesApi, upload_api::UploadApi};
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;
use crate::sync::conflict::ConflictResolver;
use crate::sync::stability::StabilityChecker;
use crate::sync::task_runner::TaskRunner;

/// 实现各类同步动作及其顺序约束。
mod actions;
/// 在删除本地项前复核远端事实与本地版本。
mod local_delete;
/// 将同步动作桥接到持久传输后端。
mod transfer_operations;

pub(crate) use local_delete::verify_local_delete_snapshot;
pub(crate) use transfer_operations::verify_source_snapshot;

/// 后台执行投递器：把已入队的传输任务交给执行通道。
///
/// 注入后周期不再等待传输完成（16b）；未注入时退回同步执行，便于独立测试。
pub type BackgroundTransferDispatcher = Arc<dyn Fn(i64) + Send + Sync>;

/// 同步执行器 —— 持有全部外部依赖。
pub struct SyncExecutor {
    concurrency: u32,
    files_api: Arc<FilesApi>,
    download_api: Arc<DownloadApi>,
    upload_api: Arc<UploadApi>,
    mount: Option<Arc<MountManager>>,
    conflict: Option<Arc<std::sync::Mutex<ConflictResolver>>>,
    stability: Option<Arc<StabilityChecker>>,
    db: Option<Arc<parking_lot::Mutex<rusqlite::Connection>>>,
    /// 传输更新通知发送端（每次传输结算时触发前端刷新）
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// AppHandle（用于上传失败时广播事件给前端弹 toast）
    app_handle: Option<tauri::AppHandle>,
    task_runner: Option<Arc<TaskRunner>>,
    /// 动作取得并发槽后检查的引擎活动门。
    action_activity_gate: Option<Arc<dyn crate::sync::task_runner::TaskActivityGate>>,
    /// 传输动作的后台投递入口；未注入时同步执行。
    background_dispatcher: Option<BackgroundTransferDispatcher>,
}

impl SyncExecutor {
    #[allow(clippy::too_many_arguments)]
    /// 创建仅包含 Drive 依赖的执行器，其余依赖延迟注入。
    pub fn new(
        concurrency: u32,
        files_api: Arc<FilesApi>,
        download_api: Arc<DownloadApi>,
        upload_api: Arc<UploadApi>,
    ) -> Self {
        Self {
            concurrency,
            files_api,
            download_api,
            upload_api,
            mount: None,
            conflict: None,
            stability: None,
            db: None,
            transfer_update_tx: None,
            app_handle: None,
            task_runner: None,
            action_activity_gate: None,
            background_dispatcher: None,
        }
    }

    /// 设置 mount manager（延迟注入，避免循环依赖）。
    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        self.mount = Some(mount);
    }

    pub fn set_conflict(&mut self, conflict: Arc<std::sync::Mutex<ConflictResolver>>) {
        self.conflict = Some(conflict);
    }

    pub fn set_stability(&mut self, s: Arc<StabilityChecker>) {
        self.stability = Some(s);
    }

    /// 注入传输动作的后台投递入口。
    pub fn set_background_dispatcher(&mut self, dispatcher: BackgroundTransferDispatcher) {
        self.background_dispatcher = Some(dispatcher);
    }

    /// 将单个已落库为可执行状态的任务投递到后台执行通道。
    ///
    /// 恢复入口只返回可运行 ID，由这里投递；未注入投递器（独立测试）时返回 false，
    /// 任务保持 Pending，不会在周期内被同步执行。
    pub fn dispatch_background(&self, task_id: i64) -> bool {
        match &self.background_dispatcher {
            Some(dispatcher) => {
                dispatcher(task_id);
                true
            }
            None => false,
        }
    }

    pub fn set_db(&mut self, db: Arc<parking_lot::Mutex<rusqlite::Connection>>) {
        self.db = Some(db);
    }

    /// 设置传输更新通知通道（每次结算时触发前端刷新）。
    pub fn set_transfer_update_tx(&mut self, tx: tokio::sync::broadcast::Sender<()>) {
        self.transfer_update_tx = Some(tx);
    }

    /// 注入 AppHandle（用于上传失败时广播事件给前端）。
    pub fn set_app_handle(&mut self, handle: tauri::AppHandle) {
        self.app_handle = Some(handle);
    }

    /// 返回已初始化的持久传输调度器。
    pub fn task_runner(&self) -> AppResult<Arc<TaskRunner>> {
        self.task_runner
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 未初始化"))
    }

    /// 设置每个动作取得并发槽后必须通过的活动门。
    pub(crate) fn set_action_activity_gate(
        &mut self,
        activity_gate: Arc<dyn crate::sync::task_runner::TaskActivityGate>,
    ) {
        self.action_activity_gate = Some(activity_gate);
    }

    /// 克隆执行器（只保留 Arc 字段的引用，轻量）。
    pub fn clone_executor(&self) -> Self {
        Self {
            concurrency: self.concurrency,
            files_api: self.files_api.clone(),
            download_api: self.download_api.clone(),
            upload_api: self.upload_api.clone(),
            mount: self.mount.clone(),
            conflict: self.conflict.clone(),
            stability: self.stability.clone(),
            db: self.db.clone(),
            transfer_update_tx: self.transfer_update_tx.clone(),
            app_handle: self.app_handle.clone(),
            task_runner: self.task_runner.clone(),
            action_activity_gate: self.action_activity_gate.clone(),
            background_dispatcher: self.background_dispatcher.clone(),
        }
    }
}
