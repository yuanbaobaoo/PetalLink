//! 同步执行器 —— 并发池 + 传输队列 + 稳定性检查 + 配额校验。
//!
//! 对齐 `legacy/lib/sync/sync_executor.dart`。
//!
//! 并发数默认 6（可配置 1-20），使用 tokio Semaphore 限流。
//! 传输队列（TransferQueue 表）记录进度，修剪历史（保留最近 100 条已结束任务）。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use tokio::sync::Semaphore;

use crate::data::repository::{self, sync_status, transfer_direction, SyncItem, TransferTask};
use crate::drive::{
    download_api::DownloadApi,
    files_api::FilesApi,
    upload_api::{ResumeSession, UploadApi},
};
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;
use crate::sync::conflict::ConflictResolver;
use crate::sync::stability::StabilityChecker;
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};
use crate::sync::task_runner::{
    BackendPreflightFailure, OnlineCheck, TaskDisposition, TaskExecutionError,
    TaskExecutionOutcome, TaskProgressReporter, TaskRunner, TaskStateSink, TransferOperations,
};
use crate::sync::transfer_state::{TransferOperation, TransferState};

/// API/mount adapter used by TaskRunner. It has no dependency on SyncExecutor or SyncEngine.
struct ExecutorTransferOperations {
    files_api: Arc<FilesApi>,
    download_api: Arc<DownloadApi>,
    upload_api: Arc<UploadApi>,
    mount: Arc<MountManager>,
    stability: Option<Arc<tokio::sync::Mutex<StabilityChecker>>>,
    app_handle: Option<tauri::AppHandle>,
}

fn verify_source_snapshot(task: &TransferTask, path: &std::path::Path) -> AppResult<()> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| AppError::generic(format!("读取上传源失败：{error}")))?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    if !metadata.is_file()
        || task.source_mtime != mtime
        || task.source_size != Some(metadata.len() as i64)
        || task.total_size != metadata.len() as i64
    {
        return Err(AppError::generic("本地上传源在执行前发生变化"));
    }
    Ok(())
}

#[async_trait]
impl TransferOperations for ExecutorTransferOperations {
    async fn preflight(&self, task: &TransferTask) -> Result<(), BackendPreflightFailure> {
        let operation = task
            .operation_kind()
            .map_err(|error| BackendPreflightFailure::restart_required(error.to_string()))?;
        if !matches!(
            operation,
            Some(TransferOperation::Create | TransferOperation::Update)
        ) {
            return Ok(());
        }
        let path =
            PathBuf::from(task.local_path.as_deref().ok_or_else(|| {
                BackendPreflightFailure::restart_required("上传任务缺少本地路径")
            })?);
        verify_source_snapshot(task, &path)
            .map_err(|error| BackendPreflightFailure::restart_required(error.to_string()))?;
        let Some(stability) = &self.stability else {
            return Ok(());
        };
        for delay in [0_u64, 2, 3, 5] {
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            match stability.lock().await.check(&path).await {
                crate::sync::stability::StabilityResult::Stable => return Ok(()),
                crate::sync::stability::StabilityResult::Editing => {
                    return Err(BackendPreflightFailure::restart_required(
                        "用户正在编辑，等待重新规划",
                    ));
                }
                crate::sync::stability::StabilityResult::Unstable => {}
            }
        }
        Err(BackendPreflightFailure::restart_required(
            "文件尚不稳定，等待重新规划",
        ))
    }

    async fn execute(
        &self,
        task: &TransferTask,
        progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
        let operation = task
            .operation_kind()
            .map_err(|error| AppError::generic(error.to_string()))?
            .ok_or_else(|| AppError::generic("任务缺少 operation"))?;
        let local_path = PathBuf::from(
            task.local_path
                .as_deref()
                .ok_or_else(|| AppError::generic("任务缺少本地路径"))?,
        );
        match operation {
            TransferOperation::Create | TransferOperation::Update => {
                // Close the preflight-to-request window: re-read the source immediately before
                // issuing any remote write and refuse if the persisted snapshot changed.
                verify_source_snapshot(task, &local_path)
                    .map_err(|error| TaskExecutionError::RestartRequired(error.to_string()))?;
                let total = task.total_size;
                let progress_reporter = progress.clone();
                let on_progress: crate::drive::upload_api::ProgressFn = Box::new(move |ratio| {
                    let transferred = (ratio.clamp(0.0, 1.0) * total as f64) as i64;
                    if let Err(error) = progress_reporter.update_transferred(transferred) {
                        tracing::debug!(%error, "忽略过期上传进度回调");
                    }
                });
                let resume_reporter = progress.clone();
                let on_resume: crate::drive::upload_api::ResumeProgressFn =
                    Box::new(move |server_id, upload_id, offset, session_url| {
                        if let Err(error) = resume_reporter.update_resume(
                            server_id,
                            upload_id,
                            offset as i64,
                            session_url,
                        ) {
                            tracing::debug!(%error, "忽略过期上传断点回调");
                        }
                    });
                let parent_id = task.parent_file_id.as_deref();
                let upload_result = if operation == TransferOperation::Update {
                    self.upload_api
                        .upload_update(
                            task.file_id.as_deref().expect("preflight requires file id"),
                            &local_path,
                            parent_id,
                            Some(&on_progress),
                        )
                        .await
                } else if task.resume_offset > 0
                    && task
                        .session_url
                        .as_deref()
                        .is_some_and(|url| !url.is_empty())
                {
                    let session = ResumeSession {
                        server_id: task.server_id.clone().unwrap_or_default(),
                        upload_id: task.upload_id.clone().unwrap_or_default(),
                        session_url: task.session_url.clone().unwrap_or_default(),
                        chunk_size: 0,
                        start_offset: task.resume_offset as u64,
                    };
                    self.upload_api
                        .upload_resume(
                            &local_path,
                            parent_id,
                            Some(&session),
                            Some(&on_progress),
                            Some(&on_resume),
                        )
                        .await
                } else {
                    self.upload_api
                        .upload(&local_path, parent_id, Some(&on_progress), Some(&on_resume))
                        .await
                };
                let uploaded = match upload_result {
                    Ok(uploaded) => uploaded,
                    Err(error) => {
                        if let Some(app) = &self.app_handle {
                            use tauri::Emitter;
                            let relative_path = task.relative_path.as_deref().unwrap_or(&task.name);
                            let _ = app.emit(
                                "upload_failed",
                                serde_json::json!({
                                    "rel_path": relative_path,
                                    "name": task.name,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        return Err(TaskExecutionError::App(error));
                    }
                };
                let (cloud_file, disposition) = if uploaded.edited_time.is_none() {
                    match self.files_api.get(&uploaded.id).await {
                        Ok(full) if full.id == uploaded.id && full.edited_time.is_some() => {
                            (full, TaskDisposition::Completed)
                        }
                        Ok(partial) => (partial, TaskDisposition::VerifyingRemote),
                        Err(error) => {
                            tracing::warn!(
                                remote_id = %uploaded.id,
                                %error,
                                "上传已返回 ID 但完整元数据补取失败，等待远端核验"
                            );
                            (uploaded, TaskDisposition::VerifyingRemote)
                        }
                    }
                } else {
                    (uploaded, TaskDisposition::Completed)
                };
                Ok(TaskExecutionOutcome {
                    cloud_file: Some(cloud_file),
                    disposition,
                })
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => {
                if let Some(parent) = local_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|error| AppError::generic(format!("创建下载目录失败：{error}")))?;
                }
                self.mount
                    .backup_modified_placeholder_if_needed(&local_path)
                    .await?;
                let progress_reporter = progress.clone();
                let on_progress: crate::drive::download_api::ProgressFn =
                    Box::new(move |received, _total| {
                        if let Err(error) = progress_reporter.update_transferred(received as i64) {
                            tracing::debug!(%error, "忽略过期下载进度回调");
                        }
                    });
                let file_id = task.file_id.as_deref().expect("preflight requires file id");
                self.download_api
                    .download(file_id, &local_path, Some(&on_progress))
                    .await?;
                let _ = self.mount.mark_downloaded(&local_path).await;
                let _ = self.mount.set_file_id_xattr(&local_path, file_id).await;
                Ok(TaskExecutionOutcome::default())
            }
            _ => Err(TaskExecutionError::App(AppError::generic(
                "该 operation 不支持传输执行",
            ))),
        }
    }
}

/// 同步执行器 —— 持有全部外部依赖。
pub struct SyncExecutor {
    concurrency: u32,
    files_api: Arc<FilesApi>,
    download_api: Arc<DownloadApi>,
    upload_api: Arc<UploadApi>,
    mount: Option<Arc<MountManager>>,
    conflict: Option<Arc<std::sync::Mutex<ConflictResolver>>>,
    stability: Option<Arc<tokio::sync::Mutex<StabilityChecker>>>,
    db: Option<Arc<parking_lot::Mutex<rusqlite::Connection>>>,
    /// 传输更新通知发送端（每次传输结算时触发前端刷新）
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// AppHandle（用于上传失败时广播事件给前端弹 toast）
    app_handle: Option<tauri::AppHandle>,
    task_runner: Option<Arc<TaskRunner>>,
    /// Engine-bound admission checked after an action wins an executor concurrency slot.
    action_activity_gate: Option<Arc<dyn crate::sync::task_runner::TaskActivityGate>>,
}

impl SyncExecutor {
    #[allow(clippy::too_many_arguments)]
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
        }
    }

    /// 设置 mount manager（延迟注入，避免循环依赖）。
    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        self.mount = Some(mount);
    }

    /// 设置冲突解决器。
    pub fn set_conflict(&mut self, conflict: Arc<std::sync::Mutex<ConflictResolver>>) {
        self.conflict = Some(conflict);
    }

    /// 设置稳定性检查器。
    pub fn set_stability(&mut self, s: Arc<tokio::sync::Mutex<StabilityChecker>>) {
        self.stability = Some(s);
    }

    /// 设置 DB 连接。
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

    /// Build the shared durable runner after DB, mount and notification dependencies are set.
    pub fn initialize_task_runner(&mut self) -> AppResult<Arc<TaskRunner>> {
        let db = self
            .db
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 缺少数据库"))?;
        let mount = self
            .mount
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 缺少挂载目录"))?;
        let operations = Arc::new(ExecutorTransferOperations {
            files_api: self.files_api.clone(),
            download_api: self.download_api.clone(),
            upload_api: self.upload_api.clone(),
            mount: mount.clone(),
            stability: self.stability.clone(),
            app_handle: self.app_handle.clone(),
        });
        let online_check: OnlineCheck = Arc::new(crate::core::net_guard::is_online);
        let initial_sink: Arc<dyn TaskStateSink> = Arc::new(|| Ok(()));
        let runner = Arc::new(TaskRunner::new(
            db,
            mount.mount_dir().to_path_buf(),
            operations,
            online_check,
            initial_sink,
            self.transfer_update_tx.clone(),
        ));
        self.task_runner = Some(runner.clone());
        Ok(runner)
    }

    pub fn task_runner(&self) -> AppResult<Arc<TaskRunner>> {
        self.task_runner
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 未初始化"))
    }

    pub(crate) fn set_action_activity_gate(
        &mut self,
        activity_gate: Arc<dyn crate::sync::task_runner::TaskActivityGate>,
    ) {
        self.action_activity_gate = Some(activity_gate);
    }

    /// Deterministic engine-chain tests inject a fake durable backend while retaining the real
    /// planner/executor/TaskRunner path. Production initializes this field through
    /// `initialize_task_runner`.
    #[cfg(test)]
    pub(crate) fn set_task_runner_for_test(&mut self, task_runner: Arc<TaskRunner>) {
        self.task_runner = Some(task_runner);
    }

    /// 并发执行全部动作。
    /// 对齐 dart `executor.executeAll`。
    pub async fn execute_all(&self, actions: &[SyncAction]) -> Vec<ActionResult> {
        // History pruning is itself an observable DB mutation, so it needs its own operation
        // permit. This permit does not admit any action; every action rechecks after it wins a
        // concurrency slot below.
        if self.db.is_some() {
            let prune_activity = match self.begin_action_activity() {
                Ok(activity) => activity,
                Err(error) => {
                    return actions
                        .iter()
                        .map(|_| Self::engine_stopped_result(&error))
                        .collect()
                }
            };
            self.prune_transfer_history();
            drop(prune_activity);
        }

        let concurrency = self.concurrency.max(1) as usize;
        let semaphore = Arc::new(Semaphore::new(concurrency));
        stream::iter(actions.iter().cloned())
            .map(|action| {
                let sem = semaphore.clone();
                let executor = self.clone_executor();
                async move {
                    let _slot = match sem.acquire_owned().await {
                        Ok(slot) => slot,
                        Err(error) => {
                            return ActionResult {
                                success: false,
                                error_message: Some(format!("执行器已关闭：{error}")),
                                deferred: true,
                                cloud_file: None,
                            }
                        }
                    };
                    // This is the action linearization point: shutdown closes the gate before it
                    // waits for old work, so queued actions cannot start callbacks afterwards.
                    let _activity = match executor.begin_action_activity() {
                        Ok(activity) => activity,
                        Err(error) => return Self::engine_stopped_result(&error),
                    };
                    executor.execute_one(&action).await
                }
            })
            .buffered(concurrency)
            .collect()
            .await
    }

    fn begin_action_activity(&self) -> AppResult<Option<Box<dyn Send>>> {
        self.action_activity_gate
            .as_ref()
            .map(|gate| gate.begin())
            .transpose()
    }

    fn engine_stopped_result(error: &AppError) -> ActionResult {
        ActionResult {
            success: false,
            error_message: Some(error.to_string()),
            // Cancellation is not a synchronization failure and must not create a FAILED
            // compatibility baseline in SyncEngine::apply_results.
            deferred: true,
            cloud_file: None,
        }
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
        }
    }

    /// 执行单个动作。
    async fn execute_one(&self, action: &SyncAction) -> ActionResult {
        tracing::debug!(rel = action.relative_path.as_deref(), action_type = ?action.action_type, "executor: 开始执行");

        if matches!(
            action.action_type,
            SyncActionType::Upload | SyncActionType::Download
        ) {
            return self.execute_transfer_action(action).await;
        }

        let result = match action.action_type {
            SyncActionType::Upload | SyncActionType::Download => unreachable!(),
            SyncActionType::CreatePlaceholder => self.do_create_placeholder(action).await,
            SyncActionType::CreateFolder => self.do_create_folder(action).await,
            SyncActionType::DeleteFromCloud => self.do_delete_from_cloud(action).await,
            SyncActionType::DeleteFromLocal => self.do_delete_from_local(action).await,
            SyncActionType::CreateConflictCopy => self.do_conflict(action).await,
            SyncActionType::BackupBeforeCloudDelete => {
                self.do_backup_before_cloud_delete(action).await
            }
            SyncActionType::Skip => ActionResult {
                success: true,
                error_message: Some(action.reason.clone().unwrap_or_default()),
                deferred: false,
                cloud_file: None,
            },
        };

        result
    }

    async fn execute_transfer_action(&self, action: &SyncAction) -> ActionResult {
        let runner = match self.task_runner() {
            Ok(runner) => runner,
            Err(error) => {
                return ActionResult {
                    success: false,
                    error_message: Some(error.to_string()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let task = self.pending_task_for_action(action);
        match runner.enqueue_and_run(task).await {
            Ok(enqueued) => match enqueued.outcome.disposition {
                TaskDisposition::Completed => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: enqueued.outcome.cloud_file,
                },
                disposition => ActionResult {
                    success: false,
                    error_message: Some(format!("传输已调度为 {disposition:?}")),
                    deferred: true,
                    cloud_file: None,
                },
            },
            Err(error) => ActionResult {
                success: false,
                error_message: Some(error.to_string()),
                deferred: false,
                cloud_file: None,
            },
        }
    }

    fn pending_task_for_action(&self, action: &SyncAction) -> TransferTask {
        let operation = match action.action_type {
            SyncActionType::Upload if action.file_id.is_some() => TransferOperation::Update,
            SyncActionType::Upload => TransferOperation::Create,
            SyncActionType::Download => {
                let has_existing_content = action
                    .local_path
                    .as_deref()
                    .and_then(|path| std::fs::metadata(path).ok())
                    .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
                if has_existing_content {
                    TransferOperation::DownloadUpdate
                } else {
                    TransferOperation::Download
                }
            }
            _ => unreachable!("only upload/download create durable transfer tasks"),
        };
        let direction = match operation {
            TransferOperation::Create | TransferOperation::Update => transfer_direction::UPLOAD,
            TransferOperation::Download => transfer_direction::DOWNLOAD,
            TransferOperation::DownloadUpdate => transfer_direction::DOWNLOAD_UPDATE,
            _ => unreachable!(),
        };
        let source_metadata = matches!(
            operation,
            TransferOperation::Create | TransferOperation::Update
        )
        .then(|| {
            action
                .local_path
                .as_deref()
                .and_then(|path| std::fs::metadata(path).ok())
        })
        .flatten();
        let source_mtime = source_metadata.as_ref().and_then(|metadata| {
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
        });
        let source_size = source_metadata
            .as_ref()
            .map(|metadata| metadata.len() as i64);
        let total_size = source_size.unwrap_or_else(|| {
            action
                .cloud_file
                .as_ref()
                .map(|file| file.size)
                .unwrap_or(0)
        });
        TransferTask {
            id: 0,
            direction,
            file_id: action.file_id.clone(),
            local_path: action.local_path.clone(),
            name: action
                .relative_path
                .as_deref()
                .and_then(|path| path.rsplit('/').next())
                .unwrap_or("unknown")
                .to_string(),
            total_size,
            transferred: 0,
            state: i32::from(TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: action.relative_path.clone(),
            parent_file_id: action.parent_file_id.clone(),
            operation: Some(i32::from(operation)),
            source_mtime,
            source_size,
            expected_cloud_edited_time: action
                .cloud_file
                .as_ref()
                .and_then(|file| file.edited_time.map(|time| time.timestamp_millis())),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        }
    }

    /// 修剪传输历史（保留最近 100 条已结束任务）。
    fn prune_transfer_history(&self) {
        if let Some(db) = &self.db {
            {
                let conn = db.lock();
                let _ = repository::prune_transfer_history(&conn, 100);
            }
        }
    }

    // ===== 各动作实现 =====

    /// 统一记录动作执行结果日志（成功 info / 失败 warn）。
    ///
    /// 替代各 do_* 方法中重复的 `if result.success { info } else { warn }` 模式。
    /// - `deferred` 为 true 时跳过失败日志（仅 do_upload/do_download 的延迟场景用）
    fn log_action_result(rel: &str, verb_success: &str, verb_fail: &str, result: &ActionResult) {
        if result.success {
            tracing::info!(rel, "{verb_success}");
        } else if !result.deferred {
            tracing::warn!(
                rel,
                error = result.error_message.as_deref().unwrap_or("?"),
                "{verb_fail}"
            );
        }
    }

    async fn do_create_placeholder(&self, action: &SyncAction) -> ActionResult {
        let cloud = match &action.cloud_file {
            Some(c) => c,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少云端文件元数据".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let rel_path = match &action.relative_path {
            Some(p) => p,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少相对路径".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        if let Some(m) = &self.mount {
            match m
                .create_placeholder_if_needed(rel_path, &cloud.id, cloud.size)
                .await
            {
                Ok(()) => {
                    // 对齐 dart：写 DB 记录（status=cloudOnly），防止孤儿占位符
                    if let (Some(db), Some(local_path)) = (&self.db, &action.local_path) {
                        {
                            let conn = db.lock();
                            let _ = repository::upsert(
                                &conn,
                                &SyncItem {
                                    file_id: cloud.id.clone(),
                                    local_path: local_path.clone(),
                                    parent_folder_id: cloud
                                        .parent_folder
                                        .as_ref()
                                        .and_then(|v| v.first().cloned()),
                                    name: cloud.name.clone(),
                                    is_folder: false,
                                    size: cloud.size,
                                    local_size: None,
                                    sha256: None,
                                    local_mtime: None,
                                    cloud_edited_time: cloud
                                        .edited_time
                                        .map(|t| t.timestamp_millis()),
                                    last_sync_time: None,
                                    status: sync_status::CLOUD_ONLY,
                                    error_message: None,
                                },
                            );
                        }
                    }
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    }
                }
                Err(e) => ActionResult {
                    success: false,
                    error_message: Some(e.to_string()),
                    deferred: false,
                    cloud_file: None,
                },
            }
        } else {
            ActionResult {
                success: false,
                error_message: Some("mount manager 未初始化".into()),
                deferred: false,
                cloud_file: None,
            }
        }
    }

    async fn do_create_folder(&self, action: &SyncAction) -> ActionResult {
        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 本地新文件夹（无云端文件）→ 调 createFolder API
        let result = if let Some(cloud_file) = &action.cloud_file {
            // 云端已有文件夹 → 本地 ensure
            let _cloud = cloud_file;
            if let Some(m) = &self.mount {
                match m.ensure_folder(rel) {
                    Ok(_) => ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    },
                    Err(e) => ActionResult {
                        success: false,
                        error_message: Some(e.to_string()),
                        deferred: false,
                        cloud_file: None,
                    },
                }
            } else {
                ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
        } else {
            // FIX:fileName 取相对路径的**最后一段**，而非整条 relative_path。
            // 之前 name = "学习/程序设计"（含 /）撞华为文件名校验 → 400 21004002
            // fileName can not contain '<>|:"*?/\'。这与 engine.rs 写 DB 时
            // 取 rel.rsplit('/').next() 保持一致。
            let full = action.relative_path.as_deref().unwrap_or("新建文件夹");
            let name = full.rsplit('/').next().unwrap_or(full);

            // ★ 创建前先检查云端是否已存在同名目录。
            // 场景：目录被删除（cloud_tree 已清）→ 用户从回收站恢复 → watcher 先于
            // 云端刷新触发 → planner 生成 CreateFolder → 若不检查，华为 API 会创建
            // "name(1)" 后缀副本，而非复用已有目录。
            // parent_file_id 为 None 表示根目录，同样需要检查。
            {
                let pid = action.parent_file_id.as_deref();
                if let Ok(list) = self.files_api.list_all(pid).await {
                    if let Some(existing) = list.iter().find(|f| f.is_folder() && f.name == name) {
                        tracing::info!(
                            rel,
                            existing_id = %existing.id,
                            parent = pid.unwrap_or("root"),
                            "CreateFolder 跳过：云端已存在同名文件夹，复用已有 ID"
                        );
                        return ActionResult {
                            success: true,
                            error_message: None,
                            deferred: false,
                            cloud_file: Some(existing.clone()),
                        };
                    }
                }
            }

            match self
                .files_api
                .create_folder(name, action.parent_file_id.as_deref())
                .await
            {
                Ok(f) => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: Some(f),
                },
                // 对齐 dart：400/409 时查同名已存在文件夹，存在则视为成功
                // （同样用末段 name 匹配，与云端真名一致才能命中）
                Err(ref e) if matches!(e.drive_status(), Some(400 | 409)) => {
                    if let Some(pid) = action.parent_file_id.as_deref() {
                        if let Ok(list) = self.files_api.list_all(Some(pid)).await {
                            if let Some(existing) =
                                list.iter().find(|f| f.is_folder() && f.name == name)
                            {
                                ActionResult {
                                    success: true,
                                    error_message: None,
                                    deferred: false,
                                    cloud_file: Some(existing.clone()),
                                }
                            } else {
                                ActionResult {
                                    success: false,
                                    error_message: Some(format!("{e}")),
                                    deferred: false,
                                    cloud_file: None,
                                }
                            }
                        } else {
                            ActionResult {
                                success: false,
                                error_message: Some(format!("{e}")),
                                deferred: false,
                                cloud_file: None,
                            }
                        }
                    } else {
                        ActionResult {
                            success: false,
                            error_message: Some(format!("{e}")),
                            deferred: false,
                            cloud_file: None,
                        }
                    }
                }
                Err(e) => ActionResult {
                    success: false,
                    error_message: Some(e.to_string()),
                    deferred: false,
                    cloud_file: None,
                },
            }
        };
        Self::log_action_result(rel, "创建目录成功", "创建目录失败", &result);
        result
    }

    async fn do_delete_from_cloud(&self, action: &SyncAction) -> ActionResult {
        let file_id = match &action.file_id {
            Some(id) => id.clone(),
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少 fileId".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        let result = match self.files_api.delete(&file_id).await {
            Ok(()) => ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            },
            Err(e) => {
                // 404 表示云端已不存在（可能已被前序操作删除），视为成功
                if e.drive_status() == Some(404) {
                    tracing::info!(rel, file_id, "云端文件已不存在（404），视为删除成功");
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    ActionResult {
                        success: false,
                        error_message: Some(e.to_string()),
                        deferred: false,
                        cloud_file: None,
                    }
                }
            }
        };
        Self::log_action_result(rel, "删除云端文件成功", "删除云端文件失败", &result);
        result
    }

    async fn do_delete_from_local(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            } // DB 清理场景
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        let result = if let Some(m) = &self.mount {
            match m.delete_local(&path).await {
                Ok(()) => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                },
                Err(e) => ActionResult {
                    success: false,
                    error_message: Some(e.to_string()),
                    deferred: false,
                    cloud_file: None,
                },
            }
        } else {
            ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            }
        };
        Self::log_action_result(rel, "删除本地文件成功", "删除本地文件失败", &result);
        result
    }

    async fn do_conflict(&self, action: &SyncAction) -> ActionResult {
        let local_path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突处理缺少本地路径".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let cloud_file = match &action.cloud_file {
            Some(c) => c,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突处理缺少云端文件元数据".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };

        // 获取本地 mtime
        let local_mtime = tokio::fs::metadata(&local_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .unwrap_or(chrono::Utc::now())
            })
            .unwrap_or(chrono::Utc::now());

        // 解析冲突
        let resolution = if let Some(conflict) = &self.conflict {
            if let Ok(mut resolver) = conflict.lock() {
                resolver.resolve(&local_path, cloud_file, &local_mtime)
            } else {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突解决器获取失败".into()),
                    deferred: false,
                    cloud_file: None,
                };
            }
        } else {
            return ActionResult {
                success: false,
                error_message: Some("冲突解决器未初始化".into()),
                deferred: false,
                cloud_file: None,
            };
        };

        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 对齐 dart：cloud-wins → 本地副本保存到 copyPath，云端下载到 localPath
        // local-wins → 云端副本下载到 copyPath，本地覆盖上传到云端
        let result = match resolution.winner {
            crate::sync::conflict::ConflictSide::Cloud => {
                // 云端获胜：移动本地 → copyPath，下载云端 → localPath
                // 改名失败绝不能继续下载——否则本地修改被覆盖且无副本，数据丢失。
                // 返回失败保住本地原文件，下轮重试。
                if let Err(e) = tokio::fs::rename(&local_path, &resolution.copy_path).await {
                    ActionResult {
                        success: false,
                        error_message: Some(format!("冲突备份改名失败，跳过下载以保本地修改：{e}")),
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    match self
                        .download_api
                        .download(&cloud_file.id, &local_path, None)
                        .await
                    {
                        Ok(()) => {
                            if let Some(m) = &self.mount {
                                let _ = m.mark_downloaded(&local_path).await;
                            }
                            if let Some(m) = &self.mount {
                                let _ = m.clear_placeholder_xattr(&resolution.copy_path).await;
                            }
                            ActionResult {
                                success: true,
                                error_message: None,
                                deferred: false,
                                cloud_file: None,
                            }
                        }
                        Err(e) => {
                            let _ = tokio::fs::rename(&resolution.copy_path, &local_path).await;
                            ActionResult {
                                success: false,
                                error_message: Some(e.to_string()),
                                deferred: false,
                                cloud_file: None,
                            }
                        }
                    }
                }
            }
            crate::sync::conflict::ConflictSide::Local => {
                // 本地获胜：下载云端旧版 → copyPath（败方副本），上传本地覆盖云端。
                if let Err(e) = self
                    .download_api
                    .download(&cloud_file.id, &resolution.copy_path, None)
                    .await
                {
                    ActionResult {
                        success: false,
                        error_message: Some(format!(
                            "冲突副本（云端旧版）下载失败，跳过覆盖以保云端旧版：{e}"
                        )),
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    if let Some(m) = &self.mount {
                        let _ = m.clear_placeholder_xattr(&resolution.copy_path).await;
                    }
                    let parent_id = cloud_file
                        .parent_folder
                        .as_ref()
                        .and_then(|v| v.first().map(|s| s.as_str()));
                    match self
                        .upload_api
                        .upload_update(&cloud_file.id, &local_path, parent_id, None)
                        .await
                    {
                        Ok(_) => ActionResult {
                            success: true,
                            error_message: None,
                            deferred: false,
                            cloud_file: None,
                        },
                        Err(e) => ActionResult {
                            success: false,
                            error_message: Some(e.to_string()),
                            deferred: false,
                            cloud_file: None,
                        },
                    }
                }
            }
        };
        Self::log_action_result(rel, "冲突处理完成", "冲突处理失败", &result);
        result
    }

    /// 云端已删除但本地有未上传修改：改名备份副本（保内容），原路径腾空即满足云端删除。
    /// 副本清掉占位 xattr，下轮作为全新本地文件上传（救援用户改动）。
    async fn do_backup_before_cloud_delete(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        if !path.exists() {
            return ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            };
        }
        // 本地 mtime 作为副本时间戳（败方=本地的修改时间）
        let local_mtime = tokio::fs::metadata(&path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .unwrap_or(chrono::Utc::now())
            })
            .unwrap_or_else(chrono::Utc::now);
        let copy_path = crate::sync::conflict::dedupe_copy_path(&path, "本地副本", &local_mtime);
        match tokio::fs::rename(&path, &copy_path).await {
            Ok(()) => {
                if let Some(m) = &self.mount {
                    let _ = m.clear_placeholder_xattr(&copy_path).await;
                }
                tracing::info!(
                    src = %path.display(),
                    backup = %copy_path.display(),
                    "云端删除但本地有未上传修改，已备份副本"
                );
                ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
            Err(e) => ActionResult {
                success: false,
                error_message: Some(format!("备份副本失败：{e}")),
                deferred: false,
                cloud_file: None,
            },
        }
    }
}
