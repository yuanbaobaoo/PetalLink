//! 同步执行器 —— 并发池 + 传输队列 + 稳定性检查 + 配额校验。
//!
//! 对齐 `legacy/lib/sync/sync_executor.dart`。
//!
//! 并发数默认 6（可配置 1-20），使用 tokio Semaphore 限流。
//! 传输队列（TransferQueue 表）记录进度，修剪历史（保留最近 100 条已结束任务）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{stream, FutureExt, StreamExt};
use tokio::sync::Semaphore;

use crate::data::repository::{self, sync_status, transfer_direction, SyncItem, TransferTask};
use crate::drive::{
    download_api::{DownloadApi, DownloadExpectation, LocalDestinationSnapshot},
    files_api::FilesApi,
    models::DriveFile,
    upload_api::{ResumeSession, UploadApi},
};
use crate::error::{AppError, AppResult};
use crate::mount::file_hasher::FileHasher;
use crate::mount::manager::MountManager;
use crate::sync::conflict::ConflictResolver;
use crate::sync::stability::StabilityChecker;
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};
use crate::sync::task_runner::{
    BackendPreflightFailure, OnlineCheck, RemoteVerification, TaskDisposition, TaskExecutionError,
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

fn metadata_mtime_ms(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
}

/// Prove that every entry about to be removed still matches the durable sync baseline.
/// Unknown descendants and unreadable paths fail closed so a cloud deletion cannot erase a
/// local edit that arrived after planning.
pub(crate) fn verify_local_delete_snapshot(
    path: &Path,
    relative_path: &str,
    baselines: &HashMap<String, SyncItem>,
    allow_orphan_placeholder: bool,
) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| AppError::generic(format!("读取待删除路径失败：{error}")))?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::generic(format!(
            "待删除路径已变为符号链接：{relative_path}"
        )));
    }

    if metadata.is_dir() {
        let baseline = baselines
            .get(relative_path)
            .ok_or_else(|| AppError::generic(format!("目录不在同步基线中：{relative_path}")))?;
        if !baseline.is_folder
            || baseline.local_mtime != metadata_mtime_ms(&metadata)
            || baseline.local_size != Some(metadata.len() as i64)
        {
            return Err(AppError::generic(format!(
                "目录在删除执行前发生变化：{relative_path}"
            )));
        }
        for entry in std::fs::read_dir(path)
            .map_err(|error| AppError::generic(format!("读取目录失败：{error}")))?
        {
            let entry =
                entry.map_err(|error| AppError::generic(format!("读取目录项失败：{error}")))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                AppError::generic(format!("目录包含非 UTF-8 名称：{relative_path}"))
            })?;
            let child_relative = if relative_path.is_empty() {
                name.to_string()
            } else {
                format!("{relative_path}/{name}")
            };
            verify_local_delete_snapshot(&entry.path(), &child_relative, baselines, false)?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        return Err(AppError::generic(format!(
            "拒绝删除非普通文件：{relative_path}"
        )));
    }
    if crate::mount::manager::is_placeholder_file(path) {
        if allow_orphan_placeholder {
            return Ok(());
        }
        let baseline = baselines
            .get(relative_path)
            .ok_or_else(|| AppError::generic(format!("占位符不在同步基线中：{relative_path}")))?;
        if baseline.is_folder {
            return Err(AppError::generic(format!(
                "占位符类型与同步基线不一致：{relative_path}"
            )));
        }
        return Ok(());
    }

    let baseline = baselines
        .get(relative_path)
        .ok_or_else(|| AppError::generic(format!("文件不在同步基线中：{relative_path}")))?;
    if baseline.is_folder
        || baseline.local_mtime != metadata_mtime_ms(&metadata)
        || baseline.local_size != Some(metadata.len() as i64)
    {
        return Err(AppError::generic(format!(
            "文件在删除执行前发生变化：{relative_path}"
        )));
    }
    Ok(())
}

fn comparable_sha256(file: &DriveFile) -> Option<&str> {
    file.content_hash
        .as_deref()
        .map(str::trim)
        .filter(|hash| hash.len() == 64 && hash.as_bytes().iter().all(u8::is_ascii_hexdigit))
}

fn content_hash_matches(file: &DriveFile, local_sha256: Option<&str>) -> bool {
    match (comparable_sha256(file), local_sha256) {
        (Some(remote), Some(local)) => remote.eq_ignore_ascii_case(local),
        _ => true,
    }
}

impl ExecutorTransferOperations {
    /// Hash only a source that still represents the persisted task snapshot. A second snapshot
    /// check prevents a hash calculated across a concurrent edit from becoming remote identity.
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

    /// Attach remote identity only while the local file is still the exact source snapshot that
    /// produced this upload. A changed source is left for TaskRunner's existing restart check.
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
                if operation == TransferOperation::Update {
                    let file_id = task.file_id.as_deref().expect("preflight requires file id");
                    let current = self.files_api.get(file_id).await?;
                    let current_edited = current.edited_time.map(|time| time.timestamp_millis());
                    if current.id != file_id || current_edited != task.expected_cloud_edited_time {
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
                // A persisted session URL is authoritative even at offset zero. Re-entering the
                // resume path forces a server status query before any byte is sent.
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
                // Keep the window anchored to the durable task rather than verification-time
                // `now`, but allow slow/interrupted resumable uploads to finish well after the
                // task was enqueued. Uniqueness, parent, size and (when available) SHA-256 still
                // have to match before the result can be committed.
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
                        // This ID is persisted only after UploadApi returned a completed File. It
                        // is stronger evidence than editedTime, whose resolution/visibility may
                        // lag, and lets xattr-only recovery finish without replaying the update.
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
            let prune_activity = match self.begin_action_activity(None) {
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
        let mut indexed_results: Vec<(usize, ActionResult)> =
            stream::iter(actions.iter().cloned().enumerate())
                .map(|(action_id, action)| {
                    let sem = semaphore.clone();
                    let executor = self.clone_executor();
                    async move {
                        let execution = std::panic::AssertUnwindSafe(async {
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
                            let _activity = match executor
                                .begin_action_activity(action.relative_path.as_deref())
                            {
                                Ok(activity) => activity,
                                Err(error) => return Self::engine_stopped_result(&error),
                            };
                            executor.execute_one(&action).await
                        })
                        .catch_unwind()
                        .await;
                        let result = match execution {
                            Ok(result) => result,
                            Err(_) => ActionResult {
                                success: false,
                                error_message: Some(format!(
                                    "动作 {action_id} 执行 panic，已按原身份记录失败"
                                )),
                                deferred: false,
                                cloud_file: None,
                            },
                        };
                        (action_id, result)
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;
        indexed_results.sort_by_key(|(action_id, _)| *action_id);
        indexed_results
            .into_iter()
            .map(|(_, result)| result)
            .collect()
    }

    fn begin_action_activity(
        &self,
        relative_path: Option<&str>,
    ) -> AppResult<Option<Box<dyn Send>>> {
        self.action_activity_gate
            .as_ref()
            .map(|gate| gate.begin(relative_path))
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
            SyncActionType::MoveInCloud => self.do_move_in_cloud(action).await,
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

            // FilesApi 内部统一执行父目录范围查重、严格响应核验和响应丢失后的唯一收敛；
            // 列表失败时会 fail closed，绝不会继续发送非幂等 POST。
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

    async fn do_move_in_cloud(&self, action: &SyncAction) -> ActionResult {
        let deferred = |message: String| ActionResult {
            success: false,
            error_message: Some(message),
            deferred: true,
            cloud_file: None,
        };
        let Some(file_id) = action.file_id.as_deref() else {
            return deferred("跨目录移动缺少 fileId，等待重新规划".to_string());
        };
        let Some(target_parent) = action.parent_file_id.as_deref() else {
            return deferred("跨目录移动的目标父目录尚未取得 fileId，等待重新规划".to_string());
        };
        let Some(relative_path) = action.relative_path.as_deref() else {
            return deferred("跨目录移动缺少目标相对路径，等待重新规划".to_string());
        };
        let Some(local_path) = action.local_path.as_deref().map(PathBuf::from) else {
            return deferred("跨目录移动缺少本地路径，等待重新规划".to_string());
        };

        // Close the scan-to-write race: the destination must still be the local file carrying the
        // same remote identity. If the user moved it again, leave the remote untouched and replan.
        let local_identity = std::fs::metadata(&local_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|_| {
                xattr::get(&local_path, crate::mount::manager::XATTR_FILE_ID)
                    .ok()
                    .flatten()
            })
            .and_then(|bytes| String::from_utf8(bytes).ok());
        if local_identity.as_deref() != Some(file_id) {
            return deferred("跨目录移动执行前本地路径或 fileId 已变化，等待重新规划".to_string());
        }

        let target_name = relative_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(relative_path);
        let target_files = match self.files_api.list_all(Some(target_parent)).await {
            Ok(files) => files,
            Err(error) => {
                return deferred(format!("核验移动目标目录失败，未发送远端写入：{error}"))
            }
        };
        if target_files
            .iter()
            .any(|file| file.id.as_str() != file_id && file.name.as_str() == target_name)
        {
            return deferred(format!(
                "目标目录已存在同名云端文件 {target_name}，拒绝覆盖并等待重新规划"
            ));
        }

        // FilesApi::update first GETs the current parent. If a previous response was lost but the
        // move committed, retry converges through an idempotent rename-only PATCH. Otherwise it
        // submits addParentFolder/removeParentFolder and fileName in one verified request.
        let result = match self
            .files_api
            .update(file_id, Some(target_name), Some(target_parent), None)
            .await
        {
            Ok(file) => ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: Some(file),
            },
            Err(error) => match self.files_api.get(file_id).await {
                Ok(file)
                    if file.id.as_str() == file_id
                        && file.name.as_str() == target_name
                        && file.parent_folder.as_deref().is_some_and(|parents| {
                            parents.len() == 1 && parents[0].as_str() == target_parent
                        }) =>
                {
                    tracing::info!(
                        file_id,
                        target_parent,
                        target_name,
                        %error,
                        "移动响应不确定，但 fileId GET 已确认目标名称与父目录"
                    );
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: Some(file),
                    }
                }
                Ok(_) => deferred(format!(
                    "远端跨目录移动尚未生效，保留原基线等待重新规划：{error}"
                )),
                Err(verification_error) => deferred(format!(
                    "远端跨目录移动结果不确定，保留原基线等待重新规划：{error}；核验失败：{verification_error}"
                )),
            },
        };
        Self::log_action_result(
            relative_path,
            "移动云端文件成功",
            "移动云端文件失败",
            &result,
        );
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
            Err(e) => match self.files_api.verify_deleted(&file_id).await {
                Ok(true) => {
                    tracing::info!(
                        rel,
                        file_id,
                        "删除响应不确定，但 fileId 核验已确认回收/不存在"
                    );
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    }
                }
                Ok(false) => ActionResult {
                    success: false,
                    error_message: Some(format!("{e}；远端核验显示文件仍未回收")),
                    deferred: false,
                    cloud_file: None,
                },
                Err(verification_error) => ActionResult {
                    success: false,
                    error_message: Some(format!("{e}；删除结果核验失败：{verification_error}")),
                    deferred: true,
                    cloud_file: None,
                },
            },
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
        let fail = |message: String, deferred: bool| ActionResult {
            success: false,
            error_message: Some(message),
            deferred,
            cloud_file: None,
        };
        let Some(mount) = &self.mount else {
            return fail("mount manager 未初始化，拒绝删除本地内容".into(), false);
        };

        let baselines = match &self.db {
            Some(db) => {
                let conn = db.lock();
                match repository::load_all(&conn) {
                    Ok(items) => {
                        let mut by_path = HashMap::with_capacity(items.len());
                        let mut duplicate = None;
                        for item in items {
                            let path = item.local_path.clone();
                            if by_path.insert(path.clone(), item).is_some() {
                                duplicate = Some(path);
                                break;
                            }
                        }
                        if let Some(path) = duplicate {
                            return fail(format!("同步基线存在重复路径，拒绝删除：{path}"), true);
                        }
                        by_path
                    }
                    Err(error) => {
                        return fail(format!("读取同步基线失败，保留本地内容：{error}"), true)
                    }
                }
            }
            None => return fail("同步数据库未初始化，拒绝删除本地内容".into(), false),
        };

        let mut path_exists = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return fail(format!("无法读取待删除路径，保留本地内容：{error}"), true),
            Ok(_) => {
                let allow_orphan_placeholder = action.file_id.is_none();
                if let Err(error) =
                    verify_local_delete_snapshot(&path, rel, &baselines, allow_orphan_placeholder)
                {
                    return fail(error.to_string(), true);
                }
                true
            }
        };

        // Keep the remote proof as close as possible to the irreversible unlink. Network or
        // protocol uncertainty is retryable, never permission to remove local content.
        if let Some(file_id) = action.file_id.as_deref() {
            if file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                return fail("待上传记录没有可核验的远端删除事实".into(), true);
            }
            match self.files_api.verify_deleted(file_id).await {
                Ok(true) => {}
                Ok(false) => {
                    return fail("云端文件仍存在，取消本地删除并等待重新规划".into(), true)
                }
                Err(error) => {
                    return fail(format!("无法确认云端已删除，保留本地内容：{error}"), true)
                }
            }
        }

        // The remote proof above can block on the network. Re-read the complete persisted local
        // snapshot after it returns so an edit made during that wait is never unlinked.
        if path_exists {
            path_exists = match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return fail(
                        format!("远端核验后无法读取待删除路径，保留本地内容：{error}"),
                        true,
                    )
                }
                Ok(_) => {
                    let allow_orphan_placeholder = action.file_id.is_none();
                    if let Err(error) = verify_local_delete_snapshot(
                        &path,
                        rel,
                        &baselines,
                        allow_orphan_placeholder,
                    ) {
                        return fail(
                            format!("远端核验期间本地内容发生变化，已取消删除：{error}"),
                            true,
                        );
                    }
                    true
                }
            };
        }

        let result = if !path_exists {
            ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            }
        } else {
            match mount.delete_local_confirmed(&path).await {
                Ok(()) => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                },
                Err(error) => fail(error.to_string(), true),
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
                    let expectation = DownloadExpectation {
                        edited_time_ms: cloud_file.edited_time.map(|time| time.timestamp_millis()),
                        size: u64::try_from(cloud_file.size).ok(),
                        content_hash: cloud_file.content_hash.clone(),
                        destination_snapshot: None,
                        placeholder_file_id: Some(cloud_file.id.clone()),
                    };
                    match self
                        .download_api
                        .download_with_expectation(
                            &cloud_file.id,
                            &local_path,
                            Some(&expectation),
                            None,
                        )
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
                let expectation = DownloadExpectation {
                    edited_time_ms: cloud_file.edited_time.map(|time| time.timestamp_millis()),
                    size: u64::try_from(cloud_file.size).ok(),
                    content_hash: cloud_file.content_hash.clone(),
                    destination_snapshot: None,
                    placeholder_file_id: Some(cloud_file.id.clone()),
                };
                if let Err(e) = self
                    .download_api
                    .download_with_expectation(
                        &cloud_file.id,
                        &resolution.copy_path,
                        Some(&expectation),
                        None,
                    )
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
