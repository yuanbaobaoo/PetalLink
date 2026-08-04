//! 按需落地云端文件，并复用同步引擎的持久传输队列。
//!
//! UI、未来的 FUSE 读取层等调用方只需要提供稳定 `file_id` 与 backing 根下的
//! 规范相对路径；本服务负责身份校验、云端版本解析、任务构造与队列准入。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::data::repository::{self, SyncItem, TransferTask};
use crate::drive::files_api::FilesApi;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;
use crate::sync::engine::SyncEngine;
use crate::sync::task_runner::TaskDisposition;
use crate::sync::transfer_state::{TransferOperation, TransferState};

/// 并发传输状态的观察间隔。SQLite 查询很轻量，同时避免按需打开产生忙轮询。
#[cfg(target_os = "linux")]
const HYDRATION_TASK_OBSERVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
/// 只限制“完全无状态/进度变化”的停滞，不限制持续有进度的大文件下载总时长。
#[cfg(target_os = "linux")]
const HYDRATION_TASK_STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

#[cfg(target_os = "linux")]
use crate::error::{DriveApiErrorCode, DriveTransportKind};
#[cfg(target_os = "linux")]
use crate::virtual_fs::{HydrationRequest, Hydrator};

/// 一次按需落地调用的最终路径与持久任务结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HydrationOutcome {
    local_path: PathBuf,
    task_id: i64,
    disposition: TaskDisposition,
}

impl HydrationOutcome {
    /// backing 根内最终应承载完整内容的本地路径。
    pub(crate) fn local_path(&self) -> &Path {
        &self.local_path
    }

    /// 本次新建或复用的持久传输任务 ID。
    pub(crate) fn task_id(&self) -> i64 {
        self.task_id
    }

    /// 当前任务所处的调度去向。
    pub(crate) fn disposition(&self) -> TaskDisposition {
        self.disposition
    }
}

/// 将按需读取转换为现有下载任务的领域服务。
#[derive(Clone)]
pub(crate) struct HydrationService {
    engine: Arc<SyncEngine>,
    mount: Arc<MountManager>,
    db: Arc<Mutex<rusqlite::Connection>>,
    files_api: Arc<FilesApi>,
}

impl HydrationService {
    /// 绑定与同步引擎相同的 backing、数据库和 Files API。
    pub(crate) fn new(
        engine: Arc<SyncEngine>,
        mount: Arc<MountManager>,
        db: Arc<Mutex<rusqlite::Connection>>,
        files_api: Arc<FilesApi>,
    ) -> Self {
        Self {
            engine,
            mount,
            db,
            files_api,
        }
    }

    /// 确保指定云端文件通过持久下载队列落地到 backing 相对路径。
    ///
    /// 相同下载意图的并发请求由 [`crate::sync::task_runner::TaskRunner`] 复用，
    /// 因此调用方无需另建下载器或直接操作传输数据库。
    pub(crate) async fn ensure_local(
        &self,
        file_id: &str,
        relative_path: &str,
    ) -> AppResult<HydrationOutcome> {
        crate::core::paths::validate_relative_path(relative_path, false)?;
        let _activity = self.engine.begin_external_activity()?;

        let record = {
            let conn = self.db.lock();
            repository::find_by_file_id(&conn, file_id)?
        };
        let (dest_rel, dest) = resolve_destination(
            self.mount.mount_dir(),
            file_id,
            relative_path,
            record.as_ref(),
        )?;
        let destination_snapshot = inspect_destination(&dest)?;

        // 云端元数据必须包含真实 editedTime 与 size，以支持变更判断和传输进度。
        // 实时查询失败时仅允许回退到具备完整版本信息的可信 DB 同步基线。
        let cached_metadata = record
            .as_ref()
            .filter(|record| record.size >= 0 && record.cloud_edited_time.is_some());
        let cloud_file = match self.files_api.get(file_id).await {
            Ok(file) => Some(file),
            Err(error) if cached_metadata.is_some() => {
                tracing::warn!(
                    file_id,
                    error = %error,
                    "按需下载获取实时元数据失败，使用可信同步基线"
                );
                None
            }
            Err(error) => return Err(error),
        };
        let metadata = resolve_hydration_metadata(cloud_file.as_ref(), cached_metadata)?;
        let task = build_download_task(
            file_id,
            &dest_rel,
            &dest,
            destination_snapshot,
            metadata,
            chrono::Utc::now().timestamp_millis(),
        );

        let runner = self.engine.task_runner()?;
        let requested_task = task.clone();
        let mut result = runner.enqueue_and_run(task).await?;
        if result.outcome.disposition == TaskDisposition::BlockedByActiveIntent {
            if let Some(disposition) = retry_or_observe_failed_hydration(
                &runner,
                &self.db,
                result.task_id,
                &requested_task,
            )
            .await?
            {
                result.outcome.disposition = disposition;
            }
        }
        Ok(HydrationOutcome {
            local_path: dest,
            task_id: result.task_id,
            disposition: result.outcome.disposition,
        })
    }

    /// 等待同一路径的持久任务离开活动状态。
    ///
    /// FUSE 的普通 `read(2)` 不能把 TaskRunner 内部的 Running/Pending 直接暴露成
    /// `EAGAIN`，多数桌面应用不会自动重试。这里观察同一任务行，直到内容可用、
    /// 明确离线、需要重规划或进入失败终态。
    #[cfg(target_os = "linux")]
    async fn wait_for_task_settlement(&self, task_id: i64) -> AppResult<TaskDisposition> {
        wait_for_task_settlement_in_db(
            &self.db,
            task_id,
            HYDRATION_TASK_OBSERVE_INTERVAL,
            HYDRATION_TASK_STALL_TIMEOUT,
        )
        .await
    }
}

/// 用户再次打开只可重试严格相同的失败下载，不能把同路径失败上传误当成 hydration。
///
/// 若另一入口已在“检查 Failed”与 `retry` 之间接受了同一任务，则返回其当前活动状态，
/// 让 FUSE 进入结算观察；这类正常竞态不应变成面向应用的 `EIO`。
async fn retry_or_observe_failed_hydration(
    runner: &crate::sync::task_runner::TaskRunner,
    db: &Arc<Mutex<rusqlite::Connection>>,
    task_id: i64,
    requested: &TransferTask,
) -> AppResult<Option<TaskDisposition>> {
    let current = {
        let conn = db.lock();
        repository::get_transfer_by_id(&conn, task_id)?
    }
    .ok_or_else(|| AppError::generic("按需下载任务已不存在"))?;
    let current_state = current
        .state_kind()
        .map_err(|error| AppError::generic(format!("按需下载任务状态无效：{error}")))?;

    if current_state == TransferState::Failed {
        if !same_hydration_intent(&current, requested) {
            return Err(AppError::generic(
                "同一路径存在另一项失败的同步任务；拒绝因打开文件而重放非下载操作",
            ));
        }
        match runner.retry(task_id).await {
            Ok(outcome) => return Ok(Some(outcome.disposition)),
            Err(retry_error) => {
                // 手动重试或另一读取可能刚刚接受了同一 Failed 行；只观察仍与本次
                // hydration 完全相同的任务，绝不借竞态放宽身份约束。
                let observed = {
                    let conn = db.lock();
                    repository::get_transfer_by_id(&conn, task_id)?
                };
                if let Some(observed) =
                    observed.filter(|task| same_hydration_intent(task, requested))
                {
                    if let Some(disposition) = observable_hydration_disposition(&observed)? {
                        return Ok(Some(disposition));
                    }
                }
                return Err(retry_error);
            }
        }
    }

    if same_hydration_intent(&current, requested) {
        return observable_hydration_disposition(&current);
    }
    Ok(None)
}

/// 与 TaskRunner 的下载去重合同保持一致，同时显式排除上传与其他同路径操作。
fn same_hydration_intent(existing: &TransferTask, requested: &TransferTask) -> bool {
    let existing_operation = existing.operation_kind().ok().flatten();
    let requested_operation = requested.operation_kind().ok().flatten();
    matches!(
        existing_operation,
        Some(TransferOperation::Download | TransferOperation::DownloadUpdate)
    ) && existing_operation == requested_operation
        && existing.relative_path == requested.relative_path
        && existing.local_path == requested.local_path
        && existing.name == requested.name
        && existing.direction == requested.direction
        && existing.file_id == requested.file_id
        && existing.total_size == requested.total_size
        && existing.parent_file_id == requested.parent_file_id
        && existing.expected_cloud_edited_time == requested.expected_cloud_edited_time
}

/// 把并发接受后的持久状态转换为 hydration 可以继续观察的去向。
fn observable_hydration_disposition(task: &TransferTask) -> AppResult<Option<TaskDisposition>> {
    let state = task
        .state_kind()
        .map_err(|error| AppError::generic(format!("按需下载任务状态无效：{error}")))?;
    Ok(match state {
        TransferState::Completed => Some(TaskDisposition::Completed),
        TransferState::Pending => Some(TaskDisposition::Pending),
        TransferState::Running => Some(TaskDisposition::Running),
        TransferState::WaitingForNetwork => Some(TaskDisposition::WaitingForNetwork),
        TransferState::BackingOff => Some(TaskDisposition::BackingOff),
        TransferState::VerifyingRemote => Some(TaskDisposition::VerifyingRemote),
        TransferState::RestartRequired => Some(TaskDisposition::RestartRequired),
        TransferState::Failed | TransferState::Canceled => None,
    })
}

/// 观察持久任务直到结算；只有状态、进度和 revision 均不变化才计入停滞时间。
#[cfg(target_os = "linux")]
async fn wait_for_task_settlement_in_db(
    db: &Arc<Mutex<rusqlite::Connection>>,
    task_id: i64,
    observe_interval: std::time::Duration,
    stall_timeout: std::time::Duration,
) -> AppResult<TaskDisposition> {
    let mut last_observation: Option<(i32, i64, i64)> = None;
    let mut last_progress_at = tokio::time::Instant::now();
    loop {
        let task = {
            let conn = db.lock();
            repository::get_transfer_by_id(&conn, task_id)?
        }
        .ok_or_else(|| AppError::generic("按需下载任务已不存在"))?;
        let observation = (task.state, task.transferred, task.state_revision);
        if last_observation.as_ref() != Some(&observation) {
            last_observation = Some(observation);
            last_progress_at = tokio::time::Instant::now();
        } else if last_progress_at.elapsed() >= stall_timeout {
            return Err(AppError::generic(
                "按需下载任务长时间没有进展，请检查网络后重新打开文件",
            ));
        }
        match task
            .state_kind()
            .map_err(|error| AppError::generic(format!("按需下载任务状态无效：{error}")))?
        {
            TransferState::Completed => return Ok(TaskDisposition::Completed),
            TransferState::WaitingForNetwork => {
                return Ok(TaskDisposition::WaitingForNetwork);
            }
            TransferState::RestartRequired => {
                return Ok(TaskDisposition::RestartRequired);
            }
            TransferState::Failed | TransferState::Canceled => {
                return Err(AppError::generic(
                    task.error_message
                        .unwrap_or_else(|| "按需下载任务未能完成".to_string()),
                ));
            }
            TransferState::Pending
            | TransferState::Running
            | TransferState::BackingOff
            | TransferState::VerifyingRemote => {
                tokio::time::sleep(observe_interval).await;
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl Hydrator for HydrationService {
    /// 把 FUSE 占位读取交给统一 hydration 服务，并保留适合内核重试的 errno。
    async fn hydrate(&self, request: HydrationRequest) -> std::io::Result<()> {
        let relative_path = validate_fuse_backing_path(
            self.mount.mount_dir(),
            &request.relative_path,
            &request.backing_path,
        )?;
        // 另一条同步意图可能暂时占用同一路径；其结算后重新规划 hydration，
        // 而不是让普通应用看见 TaskRunner 的内部 EAGAIN。
        for _ in 0..8 {
            let outcome = match self.ensure_local(&request.file_id, &relative_path).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    let errno = hydration_error_errno(&error);
                    tracing::warn!(
                        file_id = %request.file_id,
                        relative_path,
                        backing_path = %request.backing_path.display(),
                        %error,
                        errno,
                        "FUSE 按需下载失败"
                    );
                    return Err(std::io::Error::from_raw_os_error(errno));
                }
            };
            let disposition = match outcome.disposition() {
                TaskDisposition::Completed => TaskDisposition::Completed,
                TaskDisposition::WaitingForNetwork => TaskDisposition::WaitingForNetwork,
                TaskDisposition::Pending
                | TaskDisposition::Running
                | TaskDisposition::BlockedByActiveIntent
                | TaskDisposition::BackingOff
                | TaskDisposition::VerifyingRemote
                | TaskDisposition::RestartRequired => {
                    match self.wait_for_task_settlement(outcome.task_id()).await {
                        Ok(disposition) => disposition,
                        Err(error) => {
                            let errno = hydration_error_errno(&error);
                            tracing::warn!(
                                file_id = %request.file_id,
                                relative_path,
                                task_id = outcome.task_id(),
                                %error,
                                errno,
                                "FUSE 等待并发下载结算失败"
                            );
                            return Err(std::io::Error::from_raw_os_error(errno));
                        }
                    }
                }
            };
            match disposition {
                TaskDisposition::Completed => {
                    if validate_fuse_backing_path(
                        self.mount.mount_dir(),
                        &request.relative_path,
                        outcome.local_path(),
                    )
                    .is_ok()
                        && !crate::mount::manager::is_placeholder_file(outcome.local_path())
                    {
                        return Ok(());
                    }
                    // 被等待的可能是同路径上传/移动；内容仍是占位时重新提交下载意图。
                }
                TaskDisposition::WaitingForNetwork => {
                    return Err(std::io::Error::from_raw_os_error(libc::EHOSTUNREACH));
                }
                TaskDisposition::RestartRequired => continue,
                TaskDisposition::Pending
                | TaskDisposition::Running
                | TaskDisposition::BlockedByActiveIntent
                | TaskDisposition::BackingOff
                | TaskDisposition::VerifyingRemote => {
                    // wait_for_task_settlement 不应返回活动态；防御性地重新规划。
                    continue;
                }
            }
        }
        tracing::warn!(
            file_id = %request.file_id,
            relative_path,
            "FUSE 按需下载在多次并发重规划后仍未落地"
        );
        Err(std::io::Error::from_raw_os_error(libc::EIO))
    }
}

/// 确认 FUSE 请求指向服务绑定的同一 backing 文件，并转换为领域层 UTF-8 路径。
#[cfg(target_os = "linux")]
fn validate_fuse_backing_path(
    mount_root: &Path,
    relative_path: &Path,
    backing_path: &Path,
) -> std::io::Result<String> {
    let relative_path = relative_path.to_str().ok_or_else(|| {
        tracing::warn!(
            relative_path = %relative_path.display(),
            "FUSE 路径不是 UTF-8，无法映射到云端身份"
        );
        std::io::Error::from_raw_os_error(libc::EIO)
    })?;
    let expected =
        crate::core::paths::safe_join_under(mount_root, relative_path, false).map_err(|error| {
            tracing::warn!(
                relative_path,
                %error,
                "FUSE 请求包含不安全的 backing 相对路径"
            );
            std::io::Error::from_raw_os_error(libc::EIO)
        })?;
    let expected = std::fs::canonicalize(&expected).map_err(|error| {
        tracing::warn!(
            expected_path = %expected.display(),
            %error,
            "无法解析 hydration 预期 backing 路径"
        );
        std::io::Error::from_raw_os_error(libc::EIO)
    })?;
    let requested = std::fs::canonicalize(backing_path).map_err(|error| {
        tracing::warn!(
            backing_path = %backing_path.display(),
            %error,
            "无法解析 FUSE hydration backing 路径"
        );
        std::io::Error::from_raw_os_error(libc::EIO)
    })?;
    if requested != expected {
        tracing::warn!(
            relative_path,
            expected_path = %expected.display(),
            backing_path = %requested.display(),
            "FUSE hydration 请求与服务 backing 不一致"
        );
        return Err(std::io::Error::from_raw_os_error(libc::EIO));
    }
    Ok(relative_path.to_string())
}

/// 将领域错误映射为 FUSE 可观察的稳定 errno。
#[cfg(target_os = "linux")]
fn hydration_error_errno(error: &AppError) -> i32 {
    match error {
        AppError::DriveApi {
            code: DriveApiErrorCode::Network,
            transport_kind: Some(DriveTransportKind::Decode),
            ..
        } => libc::EIO,
        AppError::DriveApi {
            code: DriveApiErrorCode::Network,
            ..
        } => libc::EHOSTUNREACH,
        AppError::DriveApi {
            status_code: Some(status),
            ..
        } if matches!(*status, 408 | 425 | 429 | 500..=599) => libc::EAGAIN,
        _ => libc::EIO,
    }
}

/// 将尚未完成但可恢复的持久任务状态转换为内核重试提示。
#[cfg(all(target_os = "linux", test))]
fn hydration_disposition_errno(disposition: TaskDisposition) -> i32 {
    match disposition {
        TaskDisposition::Completed => 0,
        TaskDisposition::WaitingForNetwork => libc::EHOSTUNREACH,
        TaskDisposition::Pending
        | TaskDisposition::Running
        | TaskDisposition::BlockedByActiveIntent
        | TaskDisposition::BackingOff
        | TaskDisposition::VerifyingRemote
        | TaskDisposition::RestartRequired => libc::EAGAIN,
    }
}

/// 入队时保存的本地目标版本快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DestinationSnapshot {
    mtime: i64,
    size: i64,
}

/// 构造下载任务所需的可信云端元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HydrationMetadata {
    edited_time: i64,
    size: i64,
    parent_file_id: Option<String>,
}

/// 用 DB 身份约束调用方路径，并解析 backing 内的最终目标。
fn resolve_destination(
    mount_root: &Path,
    file_id: &str,
    requested_relative_path: &str,
    record: Option<&SyncItem>,
) -> AppResult<(String, PathBuf)> {
    crate::core::paths::validate_relative_path(requested_relative_path, false)?;
    let requested_destination =
        crate::core::paths::safe_join_under(mount_root, requested_relative_path, false)?;
    let relative_path = match record {
        Some(record) => {
            crate::core::paths::validate_relative_path(&record.local_path, false)?;
            if record.local_path != requested_relative_path {
                let recorded_destination =
                    crate::core::paths::safe_join_under(mount_root, &record.local_path, false)?;
                validate_pending_move_destination(
                    mount_root,
                    file_id,
                    &requested_destination,
                    &recorded_destination,
                )?;
                requested_relative_path.to_string()
            } else {
                record.local_path.clone()
            }
        }
        None => requested_relative_path.to_string(),
    };
    let destination = if relative_path == requested_relative_path {
        requested_destination
    } else {
        crate::core::paths::safe_join_under(mount_root, &relative_path, false)?
    };
    Ok((relative_path, destination))
}

/// 仅把可证明为同一实体本地移动后的请求路径用作 hydration 目标。
pub(super) fn validate_pending_move_destination(
    mount_root: &Path,
    file_id: &str,
    requested_destination: &Path,
    recorded_destination: &Path,
) -> AppResult<()> {
    validate_safe_parent_chain(mount_root, requested_destination, false).map_err(|error| {
        AppError::config(format!(
            "下载路径不一致，且请求目标父目录不安全：file_id={file_id}, error={error}"
        ))
    })?;
    let requested_metadata = std::fs::symlink_metadata(requested_destination).map_err(|error| {
        AppError::config(format!(
            "下载路径不一致，且无法读取请求目标：file_id={file_id}, error={error}"
        ))
    })?;
    if requested_metadata.file_type().is_symlink() || !requested_metadata.is_file() {
        return Err(AppError::config(format!(
            "下载路径不一致，且请求目标不是安全的普通文件：file_id={file_id}"
        )));
    }
    let requested_owner =
        crate::platform::xattr::get(requested_destination, crate::mount::manager::XATTR_FILE_ID)
            .map_err(|error| {
                AppError::config(format!(
                    "下载路径不一致，且无法核验请求目标身份：file_id={file_id}, error={error}"
                ))
            })?;
    if requested_owner.as_deref() != Some(file_id.as_bytes()) {
        return Err(AppError::config(format!(
            "下载路径不一致，且请求目标 fileId 不匹配：file_id={file_id}"
        )));
    }

    validate_safe_parent_chain(mount_root, recorded_destination, true).map_err(|error| {
        AppError::config(format!(
            "下载路径不一致，且数据库旧路径父目录不安全：file_id={file_id}, error={error}"
        ))
    })?;
    match std::fs::symlink_metadata(recorded_destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(AppError::config(format!(
                    "下载路径不一致，且数据库旧路径是符号链接：file_id={file_id}"
                )));
            }
            let recorded_owner = crate::platform::xattr::get(
                recorded_destination,
                crate::mount::manager::XATTR_FILE_ID,
            )
            .map_err(|error| {
                AppError::config(format!(
                    "下载路径不一致，且无法核验数据库旧路径身份：file_id={file_id}, error={error}"
                ))
            })?;
            if recorded_owner.as_deref() == Some(file_id.as_bytes()) {
                return Err(AppError::config(format!(
                    "下载路径不一致，且新旧路径同时携带同一 fileId，拒绝把复制误判为移动：file_id={file_id}"
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::config(format!(
                "下载路径不一致，且无法确认数据库旧路径已消失：file_id={file_id}, error={error}"
            )));
        }
    }
    Ok(())
}

/// 拒绝从挂载根到目标父目录之间的符号链接或非目录节点。
fn validate_safe_parent_chain(
    mount_root: &Path,
    destination: &Path,
    allow_missing: bool,
) -> AppResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::config("按需下载目标缺少父目录".to_string()))?;
    let relative_parent = parent
        .strip_prefix(mount_root)
        .map_err(|_| AppError::config("按需下载目标不在挂载根目录内".to_string()))?;
    let mut current = mount_root.to_path_buf();
    for component in relative_parent.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(AppError::config(
                "按需下载目标父目录包含不安全路径片段".to_string(),
            ));
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(AppError::config(format!(
                    "按需下载目标父目录不安全：{}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => {
                return Err(AppError::config(format!(
                    "无法核验按需下载目标父目录（{}）：{error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

/// 读取目标文件快照；占位符和不存在的文件都表示首次下载。
fn inspect_destination(destination: &Path) -> AppResult<Option<DestinationSnapshot>> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(AppError::generic("按需下载目标不是安全的普通文件"));
            }
            if metadata.len() == 0 && crate::mount::manager::is_placeholder_file(destination) {
                return Ok(None);
            }
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .ok_or_else(|| AppError::generic("无法读取按需下载目标修改时间"))?;
            Ok(Some(DestinationSnapshot {
                mtime,
                size: metadata.len() as i64,
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::generic(format!("读取按需下载目标失败：{error}"))),
    }
}

/// 优先使用实时云端元数据，字段缺失时仅回退到完整可信同步基线。
fn resolve_hydration_metadata(
    cloud_file: Option<&DriveFile>,
    cached_metadata: Option<&SyncItem>,
) -> AppResult<HydrationMetadata> {
    let edited_time = cloud_file
        .and_then(|file| {
            file.edited_time
                .map(|edited_time| edited_time.timestamp_millis())
        })
        .or_else(|| cached_metadata.and_then(|record| record.cloud_edited_time))
        .ok_or_else(|| AppError::generic("按需下载缺少可信云端 editedTime，拒绝创建任务"))?;
    let size = cloud_file
        .map(|file| file.size)
        .filter(|size| *size >= 0)
        .or_else(|| cached_metadata.map(|record| record.size))
        .ok_or_else(|| AppError::generic("按需下载缺少可信云端文件大小，拒绝创建任务"))?;
    let parent_file_id = cloud_file
        .and_then(|file| file.parent_folder.as_ref())
        .and_then(|parents| parents.first().cloned())
        .or_else(|| cached_metadata.and_then(|record| record.parent_folder_id.clone()));
    Ok(HydrationMetadata {
        edited_time,
        size,
        parent_file_id,
    })
}

/// 按现有传输合同构造 Download 或 DownloadUpdate 意图。
fn build_download_task(
    file_id: &str,
    relative_path: &str,
    destination: &Path,
    destination_snapshot: Option<DestinationSnapshot>,
    metadata: HydrationMetadata,
    created_at: i64,
) -> TransferTask {
    let is_update = destination_snapshot.is_some();
    let operation = if is_update {
        TransferOperation::DownloadUpdate
    } else {
        TransferOperation::Download
    };
    let direction = if is_update {
        repository::transfer_direction::DOWNLOAD_UPDATE
    } else {
        repository::transfer_direction::DOWNLOAD
    };
    TransferTask {
        id: 0,
        direction,
        file_id: Some(file_id.to_string()),
        local_path: Some(destination.to_string_lossy().into_owned()),
        name: destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        total_size: metadata.size,
        transferred: 0,
        state: i32::from(TransferState::Pending),
        error_message: None,
        created_at,
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(relative_path.to_string()),
        parent_file_id: metadata.parent_file_id,
        operation: Some(i32::from(operation)),
        source_mtime: destination_snapshot.map(|snapshot| snapshot.mtime),
        source_size: destination_snapshot.map(|snapshot| snapshot.size),
        expected_cloud_edited_time: Some(metadata.edited_time),
        attempt_count: 0,
        verify_attempt_count: 0,
        next_retry_at: None,
        error_kind: None,
        remote_result_file_id: None,
        state_revision: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use super::{
        build_download_task, inspect_destination, resolve_destination, resolve_hydration_metadata,
        retry_or_observe_failed_hydration, same_hydration_intent, DestinationSnapshot,
        HydrationMetadata,
    };
    use crate::data::repository::{self, SyncItem};
    use crate::error::AppError;
    use crate::sync::task_runner::{
        TaskExecutionError, TaskExecutionOutcome, TaskProgressReporter, TaskRunner,
        TransferOperations, TransferTask,
    };
    use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

    #[cfg(target_os = "linux")]
    use super::{
        hydration_disposition_errno, hydration_error_errno, validate_fuse_backing_path,
        wait_for_task_settlement_in_db,
    };

    struct DownloadTestOperations {
        execute_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TransferOperations for DownloadTestOperations {
        async fn execute(
            &self,
            task: &TransferTask,
            _progress: &TaskProgressReporter,
        ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
            self.execute_calls.fetch_add(1, Ordering::SeqCst);
            std::fs::write(
                task.local_path.as_deref().expect("测试下载缺少目标"),
                b"data",
            )
            .map_err(AppError::from)?;
            Ok(TaskExecutionOutcome::default())
        }
    }

    fn hydration_task(root: &std::path::Path) -> TransferTask {
        build_download_task(
            "cloud-file",
            "docs/report.pdf",
            &root.join("docs/report.pdf"),
            None,
            HydrationMetadata {
                edited_time: 55,
                size: 4,
                parent_file_id: Some("parent".to_string()),
            },
            77,
        )
    }

    fn test_database(root: &std::path::Path) -> Arc<Mutex<rusqlite::Connection>> {
        Arc::new(Mutex::new(
            crate::data::open_at(&root.join("state.db")).expect("创建测试数据库失败"),
        ))
    }

    fn test_runner(
        db: Arc<Mutex<rusqlite::Connection>>,
        root: &std::path::Path,
        execute_calls: Arc<AtomicUsize>,
    ) -> TaskRunner {
        TaskRunner::new(
            db,
            root.to_path_buf(),
            Arc::new(DownloadTestOperations { execute_calls }),
            Arc::new(|| true),
            Arc::new(|| Ok(())),
            None,
        )
    }

    fn sync_item(local_path: &str) -> SyncItem {
        SyncItem {
            file_id: "cloud-file".to_string(),
            local_path: local_path.to_string(),
            parent_folder_id: Some("parent".to_string()),
            name: "report.pdf".to_string(),
            is_folder: false,
            size: 42,
            local_size: Some(42),
            sha256: None,
            local_mtime: Some(1_000),
            cloud_edited_time: Some(2_000),
            last_sync_time: Some(3_000),
            status: repository::sync_status::SYNCED,
            error_message: None,
        }
    }

    #[test]
    fn destination_identity_must_match_database_path() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let record = sync_item("docs/report.pdf");
        let (relative_path, destination) =
            resolve_destination(root.path(), "cloud-file", "docs/report.pdf", Some(&record))
                .expect("同一路径应通过");
        assert_eq!(relative_path, "docs/report.pdf");
        assert_eq!(destination, root.path().join("docs/report.pdf"));

        let error =
            resolve_destination(root.path(), "cloud-file", "other/report.pdf", Some(&record))
                .expect_err("无稳定身份的不同路径必须拒绝");
        assert!(error.to_string().contains("下载路径不一致"));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn pending_move_destination_accepts_missing_or_reassigned_old_path() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let record = sync_item("docs/report.pdf");
        std::fs::create_dir_all(root.path().join("moved")).expect("创建移动目标目录失败");
        let moved = root.path().join("moved/report.pdf");
        std::fs::write(&moved, b"").expect("创建移动目标失败");
        crate::platform::xattr::set(&moved, crate::mount::manager::XATTR_FILE_ID, b"cloud-file")
            .expect("写入移动目标身份失败");

        let (relative_path, destination) =
            resolve_destination(root.path(), "cloud-file", "moved/report.pdf", Some(&record))
                .expect("旧路径消失时应识别为待同步移动");
        assert_eq!(relative_path, "moved/report.pdf");
        assert_eq!(destination, moved);

        std::fs::create_dir_all(root.path().join("docs")).expect("创建旧路径目录失败");
        let old = root.path().join("docs/report.pdf");
        std::fs::write(&old, b"replacement").expect("创建旧路径替代文件失败");
        crate::platform::xattr::set(
            &old,
            crate::mount::manager::XATTR_FILE_ID,
            b"different-file",
        )
        .expect("写入旧路径替代身份失败");
        resolve_destination(root.path(), "cloud-file", "moved/report.pdf", Some(&record))
            .expect("旧路径已属于其他文件时仍应识别为待同步移动");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn pending_move_destination_rejects_ambiguous_copy_and_malformed_owner() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let record = sync_item("docs/report.pdf");
        std::fs::create_dir_all(root.path().join("docs")).expect("创建旧路径目录失败");
        std::fs::create_dir_all(root.path().join("moved")).expect("创建移动目标目录失败");
        let old = root.path().join("docs/report.pdf");
        let moved = root.path().join("moved/report.pdf");
        for path in [&old, &moved] {
            std::fs::write(path, b"").expect("创建身份文件失败");
            crate::platform::xattr::set(path, crate::mount::manager::XATTR_FILE_ID, b"cloud-file")
                .expect("写入身份失败");
        }

        let error =
            resolve_destination(root.path(), "cloud-file", "moved/report.pdf", Some(&record))
                .expect_err("新旧路径同时携带身份时必须拒绝复制歧义");
        assert!(error.to_string().contains("复制误判为移动"));

        std::fs::remove_file(&old).expect("删除旧路径失败");
        crate::platform::xattr::set(&moved, crate::mount::manager::XATTR_FILE_ID, &[0xff, 0xfe])
            .expect("写入损坏身份失败");
        let error =
            resolve_destination(root.path(), "cloud-file", "moved/report.pdf", Some(&record))
                .expect_err("非 UTF-8 身份不能匹配 fileId");
        assert!(error.to_string().contains("fileId 不匹配"));
    }

    #[cfg(unix)]
    #[test]
    fn pending_move_destination_rejects_symlink_target_or_parent() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let outside = tempfile::tempdir().expect("创建外部目录失败");
        let record = sync_item("docs/report.pdf");
        let outside_file = outside.path().join("report.pdf");
        std::fs::write(&outside_file, b"").expect("创建外部文件失败");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        crate::platform::xattr::set(
            &outside_file,
            crate::mount::manager::XATTR_FILE_ID,
            b"cloud-file",
        )
        .expect("写入外部文件身份失败");

        let direct_link = root.path().join("report.pdf");
        std::os::unix::fs::symlink(&outside_file, &direct_link).expect("创建文件符号链接失败");
        let error = resolve_destination(root.path(), "cloud-file", "report.pdf", Some(&record))
            .expect_err("文件符号链接必须拒绝");
        assert!(error.to_string().contains("安全的普通文件"));

        std::fs::remove_file(&direct_link).expect("删除文件符号链接失败");
        let parent_link = root.path().join("moved");
        std::os::unix::fs::symlink(outside.path(), &parent_link).expect("创建目录符号链接失败");
        let error =
            resolve_destination(root.path(), "cloud-file", "moved/report.pdf", Some(&record))
                .expect_err("父目录符号链接必须拒绝");
        assert!(error.to_string().contains("父目录不安全"));
    }

    #[test]
    fn destination_rejects_escape_without_database_record() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        for unsafe_path in ["../outside", "/tmp/outside", "docs//report.pdf"] {
            let error = resolve_destination(root.path(), "cloud-file", unsafe_path, None)
                .expect_err("不安全相对路径必须拒绝");
            assert!(error.to_string().contains("路径"));
        }
    }

    #[test]
    fn destination_snapshot_distinguishes_missing_and_existing_file() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let missing = root.path().join("missing.bin");
        assert_eq!(inspect_destination(&missing).unwrap(), None);

        let existing = root.path().join("existing.bin");
        let mut file = std::fs::File::create(&existing).expect("创建测试文件失败");
        file.write_all(b"content").expect("写入测试文件失败");
        file.sync_all().expect("持久化测试文件失败");
        let snapshot = inspect_destination(&existing)
            .expect("读取文件快照失败")
            .expect("普通文件应产生快照");
        assert_eq!(snapshot.size, 7);
    }

    #[cfg(unix)]
    #[test]
    fn destination_snapshot_rejects_symbolic_link() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let target = root.path().join("target");
        std::fs::write(&target, b"content").expect("写入目标文件失败");
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("创建符号链接失败");
        let error = inspect_destination(&link).expect_err("符号链接必须拒绝");
        assert!(error.to_string().contains("安全的普通文件"));
    }

    #[test]
    fn cached_metadata_requires_complete_version_baseline() {
        let record = sync_item("docs/report.pdf");
        let metadata = resolve_hydration_metadata(None, Some(&record)).expect("完整基线应可回退");
        assert_eq!(
            metadata,
            HydrationMetadata {
                edited_time: 2_000,
                size: 42,
                parent_file_id: Some("parent".to_string()),
            }
        );

        let mut incomplete = record;
        incomplete.cloud_edited_time = None;
        let error =
            resolve_hydration_metadata(None, Some(&incomplete)).expect_err("缺少云端版本必须拒绝");
        assert!(error.to_string().contains("editedTime"));
    }

    #[test]
    fn task_builder_preserves_download_contract() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let destination = root.path().join("report.pdf");
        let metadata = HydrationMetadata {
            edited_time: 55,
            size: 100,
            parent_file_id: Some("parent".to_string()),
        };
        let task = build_download_task(
            "cloud-file",
            "docs/report.pdf",
            &destination,
            None,
            metadata.clone(),
            77,
        );
        assert_eq!(task.direction, repository::transfer_direction::DOWNLOAD);
        assert_eq!(task.operation, Some(i32::from(TransferOperation::Download)));
        assert_eq!(task.state, i32::from(TransferState::Pending));
        assert_eq!(task.source_mtime, None);
        assert_eq!(task.source_size, None);
        assert_eq!(task.expected_cloud_edited_time, Some(55));
        assert_eq!(task.total_size, 100);
        assert_eq!(task.parent_file_id.as_deref(), Some("parent"));

        let update = build_download_task(
            "cloud-file",
            "docs/report.pdf",
            &destination,
            Some(DestinationSnapshot {
                mtime: 88,
                size: 99,
            }),
            metadata,
            77,
        );
        assert_eq!(
            update.direction,
            repository::transfer_direction::DOWNLOAD_UPDATE
        );
        assert_eq!(
            update.operation,
            Some(i32::from(TransferOperation::DownloadUpdate))
        );
        assert_eq!(update.source_mtime, Some(88));
        assert_eq!(update.source_size, Some(99));
    }

    #[test]
    fn hydration_retry_requires_the_exact_same_download_intent() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let requested = hydration_task(root.path());
        let mut existing = requested.clone();
        assert!(same_hydration_intent(&existing, &requested));

        existing.operation = Some(i32::from(TransferOperation::Create));
        existing.direction = repository::transfer_direction::UPLOAD;
        assert!(
            !same_hydration_intent(&existing, &requested),
            "同路径失败上传绝不能由文件 open 自动重试"
        );

        let mut wrong_file = requested.clone();
        wrong_file.file_id = Some("different-cloud-file".to_string());
        assert!(!same_hydration_intent(&wrong_file, &requested));
    }

    #[tokio::test]
    async fn failed_hydration_open_retries_same_task_id_and_completes() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let db = test_database(root.path());
        let mut failed = hydration_task(root.path());
        failed.state = i32::from(TransferState::Failed);
        failed.error_kind = Some(i32::from(TransferErrorKind::Network));
        failed.error_message = Some("先前下载失败".to_string());
        let task_id = {
            let conn = db.lock();
            repository::insert_transfer(&conn, &failed).expect("插入失败下载任务")
        };
        let execute_calls = Arc::new(AtomicUsize::new(0));
        let runner = test_runner(db.clone(), root.path(), execute_calls.clone());

        let disposition =
            retry_or_observe_failed_hydration(&runner, &db, task_id, &hydration_task(root.path()))
                .await
                .expect("再次打开应重试相同 hydration");

        assert_eq!(
            disposition,
            Some(crate::sync::task_runner::TaskDisposition::Completed)
        );
        assert_eq!(execute_calls.load(Ordering::SeqCst), 1);
        let persisted = {
            let conn = db.lock();
            repository::get_transfer_by_id(&conn, task_id)
                .expect("读取任务")
                .expect("任务应保留")
        };
        assert_eq!(persisted.state_kind().unwrap(), TransferState::Completed);
    }

    #[tokio::test]
    async fn failed_upload_on_same_path_is_never_replayed_by_hydration() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        std::fs::create_dir_all(root.path().join("docs")).expect("创建上传父目录");
        let upload_path = root.path().join("docs/report.pdf");
        std::fs::write(&upload_path, b"data").expect("创建上传源");
        let metadata = std::fs::metadata(&upload_path).expect("读取上传源");
        let mut failed_upload = hydration_task(root.path());
        failed_upload.direction = repository::transfer_direction::UPLOAD;
        failed_upload.operation = Some(i32::from(TransferOperation::Update));
        failed_upload.source_mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        failed_upload.source_size = Some(metadata.len() as i64);
        failed_upload.state = i32::from(TransferState::Failed);
        failed_upload.error_kind = Some(i32::from(TransferErrorKind::Network));
        failed_upload.error_message = Some("上传失败".to_string());
        let db = test_database(root.path());
        let task_id = {
            let conn = db.lock();
            repository::insert_transfer(&conn, &failed_upload).expect("插入失败上传任务")
        };
        let execute_calls = Arc::new(AtomicUsize::new(0));
        let runner = test_runner(db.clone(), root.path(), execute_calls.clone());

        let error =
            retry_or_observe_failed_hydration(&runner, &db, task_id, &hydration_task(root.path()))
                .await
                .expect_err("打开文件不得重试同路径上传");

        assert!(error.to_string().contains("非下载操作"));
        assert_eq!(execute_calls.load(Ordering::SeqCst), 0);
        let persisted = {
            let conn = db.lock();
            repository::get_transfer_by_id(&conn, task_id)
                .expect("读取任务")
                .expect("任务应保留")
        };
        assert_eq!(persisted.state_kind().unwrap(), TransferState::Failed);
    }

    #[tokio::test]
    async fn concurrently_accepted_hydration_is_observed_instead_of_retried() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let db = test_database(root.path());
        let mut running = hydration_task(root.path());
        running.state = i32::from(TransferState::Running);
        let task_id = {
            let conn = db.lock();
            repository::insert_transfer(&conn, &running).expect("插入运行中下载任务")
        };
        let execute_calls = Arc::new(AtomicUsize::new(0));
        let runner = test_runner(db.clone(), root.path(), execute_calls.clone());

        let disposition =
            retry_or_observe_failed_hydration(&runner, &db, task_id, &hydration_task(root.path()))
                .await
                .expect("并发接受后的任务应转入观察");

        assert_eq!(
            disposition,
            Some(crate::sync::task_runner::TaskDisposition::Running)
        );
        assert_eq!(execute_calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(start_paused = true)]
    async fn settlement_observer_times_out_only_when_progress_stalls() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let db = test_database(root.path());
        let mut running = hydration_task(root.path());
        running.state = i32::from(TransferState::Running);
        let task_id = {
            let conn = db.lock();
            repository::insert_transfer(&conn, &running).expect("插入运行中下载任务")
        };

        let error = wait_for_task_settlement_in_db(
            &db,
            task_id,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(20),
        )
        .await
        .expect_err("无进度任务必须结束等待");

        assert!(error.to_string().contains("长时间没有进展"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(start_paused = true)]
    async fn settlement_observer_allows_downloads_longer_than_stall_window_when_progress_moves() {
        let root = tempfile::tempdir().expect("创建临时目录失败");
        let db = test_database(root.path());
        let mut running = hydration_task(root.path());
        running.state = i32::from(TransferState::Running);
        let task_id = {
            let conn = db.lock();
            repository::insert_transfer(&conn, &running).expect("插入运行中下载任务")
        };
        let updater_db = db.clone();
        let updater = tokio::spawn(async move {
            for transferred in 1_i64..=3 {
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                updater_db
                    .lock()
                    .execute(
                        "UPDATE transfer_queue SET transferred=?1 WHERE id=?2",
                        rusqlite::params![transferred, task_id],
                    )
                    .expect("推进下载进度");
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            updater_db
                .lock()
                .execute(
                    "UPDATE transfer_queue SET state=?1, state_revision=state_revision+1 WHERE id=?2",
                    rusqlite::params![i32::from(TransferState::Completed), task_id],
                )
                .expect("完成下载");
        });

        let disposition = wait_for_task_settlement_in_db(
            &db,
            task_id,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(20),
        )
        .await
        .expect("持续推进的长下载不应超时");
        updater.await.expect("等待进度任务");

        assert_eq!(
            disposition,
            crate::sync::task_runner::TaskDisposition::Completed
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fuse_request_must_resolve_to_same_backing_file() {
        let root = tempfile::tempdir().expect("创建 backing 目录失败");
        let other = tempfile::tempdir().expect("创建外部目录失败");
        std::fs::create_dir(root.path().join("docs")).expect("创建 backing 子目录失败");
        let expected = root.path().join("docs/report.pdf");
        std::fs::write(&expected, b"placeholder").expect("创建 backing 文件失败");
        let outside = other.path().join("report.pdf");
        std::fs::write(&outside, b"outside").expect("创建外部文件失败");

        assert_eq!(
            validate_fuse_backing_path(
                root.path(),
                std::path::Path::new("docs/report.pdf"),
                &expected,
            )
            .expect("同一 backing 文件应通过"),
            "docs/report.pdf"
        );
        let error = validate_fuse_backing_path(
            root.path(),
            std::path::Path::new("docs/report.pdf"),
            &outside,
        )
        .expect_err("外部 backing 路径必须拒绝");
        assert_eq!(error.raw_os_error(), Some(libc::EIO));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fuse_error_mapping_distinguishes_network_retry_and_io_failure() {
        let network = crate::error::AppError::drive_network(Some("offline"));
        assert_eq!(hydration_error_errno(&network), libc::EHOSTUNREACH);

        let throttled = crate::error::AppError::drive_from_status(429, "{}");
        assert_eq!(hydration_error_errno(&throttled), libc::EAGAIN);

        let invalid = crate::error::AppError::generic("invalid");
        assert_eq!(hydration_error_errno(&invalid), libc::EIO);

        assert_eq!(
            hydration_disposition_errno(
                crate::sync::task_runner::TaskDisposition::WaitingForNetwork
            ),
            libc::EHOSTUNREACH
        );
        assert_eq!(
            hydration_disposition_errno(crate::sync::task_runner::TaskDisposition::Pending),
            libc::EAGAIN
        );
    }
}
