//! 提供传输任务的静态前置校验与拒绝持久化。

use std::path::Path;

use super::contracts::BackendPreflightFailure;
use super::persistence::transition_error;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::AppResult;
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

impl TaskRunner {
    /// 校验任务可安全执行所需的静态条件。
    pub(super) fn validate_static(
        &self,
        task: &TransferTask,
    ) -> Result<TransferOperation, PreflightFailure> {
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
                let metadata = std::fs::symlink_metadata(local_path).map_err(|_| {
                    PreflightFailure::local_changed("更新下载目标已不存在，需要重新规划")
                })?;
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64);
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || task.source_mtime.is_none()
                    || task.source_size.is_none()
                    || task.source_mtime != mtime
                    || task.source_size != Some(metadata.len() as i64)
                {
                    return Err(PreflightFailure::local_changed(
                        "更新下载目标已变化或缺少版本快照，需要重新规划",
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

    /// 校验并按需创建下载目标父目录。
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

    /// 持久化前置校验拒绝结果。
    pub(super) fn persist_preflight_rejection(
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
}

#[derive(Debug, Clone)]
/// 统一的前置校验失败状态、分类与消息。
pub(super) struct PreflightFailure {
    pub(super) target: TransferState,
    pub(super) kind: TransferErrorKind,
    pub(super) message: String,
}

impl PreflightFailure {
    /// 构造静态校验失败。
    pub(super) fn validation(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::Failed,
            kind: TransferErrorKind::Validation,
            message: message.into(),
        }
    }

    /// 构造本地内容变化失败。
    pub(super) fn local_changed(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::RestartRequired,
            kind: TransferErrorKind::LocalChanged,
            message: message.into(),
        }
    }

    /// 构造远端结果不确定失败。
    pub(super) fn remote_ambiguous(message: impl Into<String>) -> Self {
        Self {
            target: TransferState::VerifyingRemote,
            kind: TransferErrorKind::RemoteAmbiguous,
            message: message.into(),
        }
    }

    /// 生成前置校验失败补丁。
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
    /// 将后端校验失败转为 TaskRunner 统一表示。
    fn from(failure: BackendPreflightFailure) -> Self {
        Self {
            target: failure.target,
            kind: failure.kind,
            message: failure.message,
        }
    }
}
