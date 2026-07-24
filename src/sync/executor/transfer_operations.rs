//! TaskRunner 的持久传输后端与动作桥接。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::data::repository::{transfer_direction, TransferTask};
use crate::drive::{
    download_api::{DownloadApi, DownloadExpectation, LocalDestinationSnapshot},
    files_api::FilesApi,
    models::DriveFile,
    upload_api::{ResumeSession, UploadApi},
};
use crate::error::{AppError, AppResult};
use crate::mount::file_hasher::FileHasher;
use crate::mount::manager::MountManager;
use crate::sync::stability::StabilityChecker;
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};
use crate::sync::task_runner::{
    BackendPreflightFailure, OnlineCheck, RemoteVerification, TaskDisposition, TaskExecutionError,
    TaskExecutionOutcome, TaskProgressReporter, TaskRunner, TaskStateSink, TransferOperations,
};
use crate::sync::transfer_state::{TransferOperation, TransferState};

use super::SyncExecutor;

/// TaskRunner 的 API 与挂载适配器。
struct ExecutorTransferOperations {
    files_api: Arc<FilesApi>,
    download_api: Arc<DownloadApi>,
    upload_api: Arc<UploadApi>,
    mount: Arc<MountManager>,
    stability: Option<Arc<tokio::sync::Mutex<StabilityChecker>>>,
    app_handle: Option<tauri::AppHandle>,
}

/// 确认上传源的类型、修改时间与大小仍匹配持久任务。
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

/// 返回可与本地 SHA-256 直接比较的云端哈希。
fn comparable_sha256(file: &DriveFile) -> Option<&str> {
    file.content_hash
        .as_deref()
        .map(str::trim)
        .filter(|hash| hash.len() == 64 && hash.as_bytes().iter().all(u8::is_ascii_hexdigit))
}

/// 在双方都有可比哈希时校验内容一致性。
fn content_hash_matches(file: &DriveFile, local_sha256: Option<&str>) -> bool {
    match (comparable_sha256(file), local_sha256) {
        (Some(remote), Some(local)) => remote.eq_ignore_ascii_case(local),
        _ => true,
    }
}

impl ExecutorTransferOperations {
    /// 仅散列仍匹配持久任务快照的上传源。
    async fn source_sha256_if_current(&self, task: &TransferTask) -> AppResult<Option<String>> {
        let Some(local_path) = task.local_path.as_deref() else {
            return Ok(None);
        };
        let local_path = PathBuf::from(local_path);
        if verify_source_snapshot(task, &local_path).is_err() {
            return Ok(None);
        }
        let sha256 = FileHasher::new()
            .hash_file(&local_path)
            .await
            .map_err(|error| AppError::generic(format!("计算上传源 SHA256 失败：{error}")))?;
        if verify_source_snapshot(task, &local_path).is_err() {
            return Ok(None);
        }
        Ok(Some(sha256))
    }

    /// 仅为仍匹配上传源快照的文件写入远端 fileId。
    async fn set_upload_file_id_if_current(
        &self,
        task: &TransferTask,
        file_id: &str,
    ) -> AppResult<bool> {
        let Some(local_path) = task.local_path.as_deref() else {
            return Ok(false);
        };
        let local_path = PathBuf::from(local_path);
        if let Err(error) = verify_source_snapshot(task, &local_path) {
            tracing::info!(task_id = task.id, %error, "上传源已变化，跳过 fileId xattr 写入");
            return Ok(false);
        }
        self.mount.set_file_id_xattr(&local_path, file_id).await?;
        Ok(true)
    }

    /// 将已确认的上传结果收敛为提交状态并尽力回写 fileId。
    async fn committed_upload(
        &self,
        task: &TransferTask,
        file: DriveFile,
    ) -> AppResult<RemoteVerification> {
        if let Err(error) = self.set_upload_file_id_if_current(task, &file.id).await {
            tracing::warn!(
                task_id = task.id,
                remote_id = %file.id,
                %error,
                "远端上传已确认，fileId xattr 补写失败但不阻塞基线结算"
            );
        }
        Ok(RemoteVerification::Committed(file))
    }
}

#[async_trait]
impl TransferOperations for ExecutorTransferOperations {
    /// 在远程写入前校验上传源快照、安全阈值与稳定性。
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
        if operation == Some(TransferOperation::Update)
            && task.total_size as u64 > crate::drive::upload_api::SAFE_EXISTING_UPDATE_MAX_BYTES
        {
            return Err(BackendPreflightFailure::restart_required(
                "现有云端文件超过 20 MiB，Huawei 当前接口不支持安全替换；已保留远端原文件",
            ));
        }
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

    /// 执行持久传输任务，并把进度与断点信息回写 TaskRunner。
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
                // 远端写入前再次核验上传源快照。
                verify_source_snapshot(task, &local_path)
                    .map_err(|error| TaskExecutionError::RestartRequired(error.to_string()))?;
                if operation == TransferOperation::Update {
                    let file_id = task.file_id.as_deref().expect("preflight requires file id");
                    let current = self.files_api.get(file_id).await?;
                    let current_edited = current.edited_time.map(|time| time.timestamp_millis());
                    if current.id != file_id || current_edited != task.expected_cloud_edited_time {
                        tracing::warn!(
                            task_id = task.id,
                            expected_file_id = file_id,
                            current_file_id = %current.id,
                            expected_edited_time = ?task.expected_cloud_edited_time,
                            current_edited_time = ?current_edited,
                            user_message = "云端文件已更新。为避免覆盖，请同步索引后重试。",
                            "更新上传前远端版本核验失败"
                        );
                        return Err(TaskExecutionError::RestartRequired(
                            "远端文件已在规划后变化，拒绝用旧任务覆盖".to_string(),
                        ));
                    }
                } else {
                    let collision = self
                        .files_api
                        .list_all(task.parent_file_id.as_deref())
                        .await?
                        .into_iter()
                        .any(|file| file.name == task.name);
                    if collision {
                        return Err(TaskExecutionError::RestartRequired(
                            "目标目录已存在同名远端文件，拒绝重复创建".to_string(),
                        ));
                    }
                }
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
                // 持久化会话即使偏移为零也必须走续传核验。
                let upload_result = if operation == TransferOperation::Update {
                    self.upload_api
                        .upload_update(
                            task.file_id.as_deref().expect("preflight requires file id"),
                            &local_path,
                            parent_id,
                            Some(&on_progress),
                        )
                        .await
                } else if task
                    .session_url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
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
                if disposition == TaskDisposition::Completed {
                    match self
                        .set_upload_file_id_if_current(task, &cloud_file.id)
                        .await
                    {
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(
                                task_id = task.id,
                                remote_id = %cloud_file.id,
                                %error,
                                "上传已提交但 fileId xattr 写入失败；继续按已上传源快照结算"
                            );
                        }
                    }
                }
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
                        if let Err(error) =
                            progress_reporter.update_download_progress(received as i64)
                        {
                            tracing::debug!(%error, "忽略过期下载进度回调");
                        }
                    });
                let file_id = task.file_id.as_deref().expect("preflight requires file id");
                let expectation = DownloadExpectation {
                    edited_time_ms: task.expected_cloud_edited_time,
                    size: u64::try_from(task.total_size).ok(),
                    content_hash: None,
                    destination_snapshot: if operation == TransferOperation::DownloadUpdate {
                        Some(LocalDestinationSnapshot {
                            mtime_ms: task.source_mtime.ok_or_else(|| {
                                AppError::generic("更新下载缺少本地目标修改时间快照")
                            })?,
                            size: u64::try_from(task.source_size.ok_or_else(|| {
                                AppError::generic("更新下载缺少本地目标大小快照")
                            })?)
                            .map_err(|_| AppError::generic("更新下载本地目标大小非法"))?,
                        })
                    } else {
                        None
                    },
                    placeholder_file_id: (operation == TransferOperation::Download)
                        .then(|| file_id.to_string()),
                };
                self.download_api
                    .download_with_expectation(
                        file_id,
                        &local_path,
                        Some(&expectation),
                        Some(&on_progress),
                    )
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

    /// 根据远程 ID、父目录、大小、时间与哈希核实写入结果。
    async fn verify_remote(&self, task: &TransferTask) -> AppResult<RemoteVerification> {
        let operation = task
            .operation_kind()
            .map_err(|error| AppError::generic(error.to_string()))?
            .ok_or_else(|| AppError::generic("远端核验缺少 operation"))?;
        match operation {
            TransferOperation::Create => {
                if let Some(remote_id) = task
                    .remote_result_file_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                {
                    let file = match self.files_api.get(remote_id).await {
                        Ok(file) => file,
                        Err(error) if error.drive_status() == Some(404) => {
                            return Ok(RemoteVerification::Ambiguous(
                                "上传曾返回远端 ID，但该资源当前不可见；禁止重复创建".to_string(),
                            ))
                        }
                        Err(error) => return Err(error),
                    };
                    if file.id != remote_id
                        || file.name != task.name
                        || file.size != task.source_size.unwrap_or(task.total_size)
                    {
                        return Ok(RemoteVerification::Ambiguous(
                            "远端结果 ID 存在，但名称或大小与创建任务不一致".to_string(),
                        ));
                    }
                    let local_sha256 = if comparable_sha256(&file).is_some() {
                        self.source_sha256_if_current(task).await?
                    } else {
                        None
                    };
                    if !content_hash_matches(&file, local_sha256.as_deref()) {
                        return Ok(RemoteVerification::Ambiguous(
                            "远端结果 ID 的 content_hash 与上传源不一致".to_string(),
                        ));
                    }
                    return self.committed_upload(task, file).await;
                }

                let expected_size = task.source_size.unwrap_or(task.total_size);
                let mut candidates = Vec::new();
                let mut missing_time_candidates = Vec::new();
                let lower_bound = task.created_at.saturating_sub(120_000);
                // 核验窗口以持久任务为锚，并覆盖慢速或中断续传。
                let upper_bound = task
                    .created_at
                    .saturating_add(30_i64 * 24 * 60 * 60 * 1_000);
                for file in self
                    .files_api
                    .list_all(task.parent_file_id.as_deref())
                    .await?
                {
                    let parent_matches = task.parent_file_id.as_deref().map_or(true, |parent| {
                        file.parent_folder
                            .as_deref()
                            .is_some_and(|parents| parents.len() == 1 && parents[0] == parent)
                    });
                    if file.name != task.name || file.size != expected_size || !parent_matches {
                        continue;
                    }
                    match file.created_time.map(|time| time.timestamp_millis()) {
                        Some(created_at) if (lower_bound..=upper_bound).contains(&created_at) => {
                            candidates.push(file)
                        }
                        None => missing_time_candidates.push(file),
                        Some(_) => {}
                    }
                }
                let needs_local_sha256 = candidates
                    .iter()
                    .chain(missing_time_candidates.iter())
                    .any(|file| comparable_sha256(file).is_some());
                let local_sha256 = if needs_local_sha256 {
                    self.source_sha256_if_current(task).await?
                } else {
                    None
                };
                candidates.retain(|file| content_hash_matches(file, local_sha256.as_deref()));
                missing_time_candidates
                    .retain(|file| content_hash_matches(file, local_sha256.as_deref()));
                if candidates.len() == 1 {
                    return self.committed_upload(task, candidates.remove(0)).await;
                }
                match candidates.len() {
                    0 if !missing_time_candidates.is_empty() => Ok(RemoteVerification::Ambiguous(
                        "发现同名同大小资源但缺少创建时间，无法排除重复文件".to_string(),
                    )),
                    0 => Ok(RemoteVerification::NotCommitted),
                    _ => Ok(RemoteVerification::Ambiguous(
                        "父目录内存在多个符合创建任务的远端资源".to_string(),
                    )),
                }
            }
            TransferOperation::Update => {
                let file_id = task
                    .file_id
                    .as_deref()
                    .ok_or_else(|| AppError::generic("Update 核验缺少 fileId"))?;
                let file = match self.files_api.get(file_id).await {
                    Ok(file) => file,
                    Err(error) if error.drive_status() == Some(404) => {
                        return Ok(RemoteVerification::Ambiguous(
                            "待更新的既有远端文件已不可见，禁止降级创建".to_string(),
                        ))
                    }
                    Err(error) => return Err(error),
                };
                if let Some(remote_result_id) = task
                    .remote_result_file_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                {
                    if remote_result_id == file_id
                        && file.id == file_id
                        && file.name == task.name
                        && file.size == task.source_size.unwrap_or(task.total_size)
                    {
                        // 已持久化的结果 ID 可在 editedTime 滞后时证明提交完成。
                        return self.committed_upload(task, file).await;
                    }
                    return Ok(RemoteVerification::Ambiguous(
                        "更新已返回远端结果 ID，但当前资源身份不一致".to_string(),
                    ));
                }
                let edited_time = file.edited_time.map(|time| time.timestamp_millis());
                if edited_time == task.expected_cloud_edited_time {
                    return Ok(RemoteVerification::NotCommitted);
                }
                if file.id == file_id
                    && file.name == task.name
                    && file.size == task.source_size.unwrap_or(task.total_size)
                    && edited_time.is_some()
                {
                    self.committed_upload(task, file).await
                } else {
                    Ok(RemoteVerification::Ambiguous(
                        "远端版本已变化，但内容身份与本次更新不一致".to_string(),
                    ))
                }
            }
            _ => Ok(RemoteVerification::Ambiguous(
                "该任务不是可核验的上传写入".to_string(),
            )),
        }
    }
}

impl SyncExecutor {
    /// 在依赖注入后构建共享持久任务运行器。
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

    /// 将上传/下载动作转为持久任务并交给 TaskRunner 执行。
    pub(super) async fn execute_transfer_action(&self, action: &SyncAction) -> ActionResult {
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
                disposition => {
                    tracing::info!(
                        disposition = ?disposition,
                        user_message = disposition.user_message(),
                        "传输动作进入等待处理状态"
                    );
                    ActionResult {
                        success: false,
                        error_message: Some(disposition.user_message().to_string()),
                        deferred: true,
                        cloud_file: None,
                    }
                }
            },
            Err(error) => ActionResult {
                success: false,
                error_message: Some(error.to_string()),
                deferred: false,
                cloud_file: None,
            },
        }
    }

    /// 根据动作类型与当前本地快照构造待入队传输任务。
    fn pending_task_for_action(&self, action: &SyncAction) -> TransferTask {
        let operation = match action.action_type {
            SyncActionType::Upload if action.file_id.is_some() => TransferOperation::Update,
            SyncActionType::Upload => TransferOperation::Create,
            SyncActionType::Download => {
                let has_existing_content = action
                    .local_path
                    .as_deref()
                    .and_then(|path| {
                        std::fs::symlink_metadata(path)
                            .ok()
                            .map(|metadata| (path, metadata))
                    })
                    .is_some_and(|(path, metadata)| {
                        metadata.is_file()
                            && !(metadata.len() == 0
                                && crate::mount::manager::is_placeholder_file(Path::new(path)))
                    });
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
            TransferOperation::Create
                | TransferOperation::Update
                | TransferOperation::DownloadUpdate
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
        let total_size = if matches!(
            operation,
            TransferOperation::Create | TransferOperation::Update
        ) {
            source_size.unwrap_or(0)
        } else {
            action
                .cloud_file
                .as_ref()
                .map(|file| file.size)
                .unwrap_or(0)
        };
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
}
