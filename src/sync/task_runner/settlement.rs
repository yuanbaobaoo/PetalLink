//! 提供任务结果结算与失败恢复流程。

use super::contracts::{TaskDisposition, TaskExecutionOutcome};
use super::persistence::{mark_compatibility_sync_failed, transition_error};
use super::preflight::PreflightFailure;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::retry_policy::{classify_transfer_error, RecoveryContext, RecoveryDecision};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::OptionalExtension;

/// 单个持久任务允许的最大自动重试次数。
const MAX_AUTOMATIC_ATTEMPTS: u32 = 5;

impl TaskRunner {
    /// 根据错误分类持久化失败或恢复状态。
    pub(super) fn settle_error(
        &self,
        running: &TransferTask,
        error: AppError,
    ) -> AppResult<TaskExecutionOutcome> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("任务缺少 operation"))?;
        let classified = classify_transfer_error(
            &error,
            RecoveryContext {
                operation,
                attempt_count: running.attempt_count.max(0) as u32,
                now_ms: (self.now_ms)(),
                jitter_ms: 0,
                auth_already_replayed: false,
                max_attempts: MAX_AUTOMATIC_ATTEMPTS,
            },
        );
        let attempts = running
            .attempt_count
            .saturating_add(i64::from(classified.consumes_retry_budget));
        let (state, disposition, next_retry_at) = match classified.decision {
            RecoveryDecision::WaitForNetwork => (
                TransferState::WaitingForNetwork,
                Some(TaskDisposition::WaitingForNetwork),
                None,
            ),
            RecoveryDecision::Backoff { next_retry_at } => (
                TransferState::BackingOff,
                Some(TaskDisposition::BackingOff),
                Some(next_retry_at),
            ),
            RecoveryDecision::VerifyRemote => (
                TransferState::VerifyingRemote,
                Some(TaskDisposition::VerifyingRemote),
                Some((self.now_ms)().saturating_add(3_000)),
            ),
            // DriveClient 负责唯一一次带认证重放；到达此边界的首次 401 不由 runner 盲目重放。
            RecoveryDecision::RefreshAuth | RecoveryDecision::Fail => {
                (TransferState::Failed, None, None)
            }
        };
        let patch = TransferPatch {
            error_kind: ColumnPatch::Set(classified.kind),
            error_message: ColumnPatch::Set(error.to_string()),
            next_retry_at: next_retry_at
                .map(ColumnPatch::Set)
                .unwrap_or(ColumnPatch::Clear),
            finished_at: if state == TransferState::Failed {
                ColumnPatch::Set((self.now_ms)())
            } else {
                ColumnPatch::Clear
            },
            attempt_count: Some(attempts),
            ..Default::default()
        };
        if state == TransferState::Failed {
            let error_message = error.to_string();
            {
                let conn = self.db.lock();
                let transaction = conn.unchecked_transaction().map_err(|db_error| {
                    AppError::generic(format!("开始失败结算事务失败：{db_error}"))
                })?;
                let failed = repository::transition_transfer_in_transaction(
                    &transaction,
                    running.id,
                    running.state_revision,
                    state,
                    patch,
                )
                .map_err(transition_error)?;
                mark_compatibility_sync_failed(&transaction, &failed, &error_message)?;
                transaction.commit().map_err(|db_error| {
                    AppError::generic(format!("提交失败结算事务失败：{db_error}"))
                })?;
            }
            self.notify_best_effort();
        } else {
            self.transition(running.id, running.state_revision, state, patch)?;
        }
        match disposition {
            Some(disposition) => Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition,
            }),
            None => Err(error),
        }
    }

    /// 持久化后端返回的非完成状态。
    pub(super) fn persist_backend_disposition(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> AppResult<TransferTask> {
        let (state, kind, message) = match output.disposition {
            TaskDisposition::Completed => {
                return Err(AppError::generic("Completed 不应走延迟结算"));
            }
            TaskDisposition::Pending
            | TaskDisposition::Running
            | TaskDisposition::BlockedByActiveIntent => {
                return Err(AppError::generic("活动任务状态不应由后端返回"));
            }
            TaskDisposition::WaitingForNetwork => (
                TransferState::WaitingForNetwork,
                TransferErrorKind::Network,
                "后端请求等待网络恢复",
            ),
            TaskDisposition::BackingOff => {
                return Err(AppError::generic("后端 BackingOff 缺少 next_retry_at"));
            }
            TaskDisposition::VerifyingRemote => (
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                "远端写入已返回资源 ID，但完整元数据尚未确认",
            ),
            TaskDisposition::RestartRequired => (
                TransferState::RestartRequired,
                TransferErrorKind::LocalChanged,
                "本地源已变化，需要重新规划",
            ),
        };
        self.transition(
            running.id,
            running.state_revision,
            state,
            TransferPatch {
                error_kind: ColumnPatch::Set(kind),
                error_message: ColumnPatch::Set(message.to_string()),
                next_retry_at: if output.disposition == TaskDisposition::VerifyingRemote {
                    ColumnPatch::Set((self.now_ms)().saturating_add(3_000))
                } else {
                    ColumnPatch::Clear
                },
                remote_result_file_id: output
                    .cloud_file
                    .as_ref()
                    .map(|file| ColumnPatch::Set(file.id.clone()))
                    .unwrap_or(ColumnPatch::Keep),
                ..Default::default()
            },
        )
    }

    /// 校验完成结果是否可安全结算。
    pub(super) fn validate_success_outcome(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> Result<(), PreflightFailure> {
        let operation = running
            .operation_kind()
            .map_err(|error| PreflightFailure::validation(error.to_string()))?
            .ok_or_else(|| PreflightFailure::validation("成功核验缺少 operation"))?;
        let local_path = running
            .local_path
            .as_deref()
            .ok_or_else(|| PreflightFailure::validation("成功核验缺少本地路径"))?;
        match operation {
            TransferOperation::Create | TransferOperation::Update => {
                let cloud = output
                    .cloud_file
                    .as_ref()
                    .ok_or_else(|| PreflightFailure::remote_ambiguous("上传结果缺少远端资源"))?;
                if cloud.id.trim().is_empty()
                    || cloud.name.trim().is_empty()
                    || cloud.name != running.name
                    || cloud.edited_time.is_none()
                    || cloud.size != running.source_size.unwrap_or(-1)
                    || (operation == TransferOperation::Update
                        && running.file_id.as_deref() != Some(cloud.id.as_str()))
                {
                    return Err(PreflightFailure::remote_ambiguous(
                        "上传结果元数据不完整或大小不一致",
                    ));
                }
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => {
                let metadata = std::fs::metadata(local_path)
                    .map_err(|_| PreflightFailure::local_changed("成功核验时下载文件不存在"))?;
                if !metadata.is_file() {
                    return Err(PreflightFailure::local_changed(
                        "成功核验时下载目标不是普通文件",
                    ));
                }
                if running.expected_cloud_edited_time.is_none()
                    || metadata.len() as i64 != running.total_size
                {
                    return Err(PreflightFailure::local_changed(
                        "下载结果大小或云端版本不匹配",
                    ));
                }
            }
            _ => return Err(PreflightFailure::validation("不支持该成功结果")),
        }
        Ok(())
    }

    /// 原子完成任务与同步基线结算。
    pub(super) fn settle_success(
        &self,
        running: &TransferTask,
        output: &TaskExecutionOutcome,
    ) -> AppResult<TransferTask> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("任务缺少 operation"))?;
        let relative_path = running
            .relative_path
            .as_deref()
            .ok_or_else(|| AppError::generic("任务缺少相对路径"))?;
        let local_path = running
            .local_path
            .as_deref()
            .ok_or_else(|| AppError::generic("任务缺少本地路径"))?;
        // 上传结算记录实际送达华为云的源快照，而非路径当前内容。若上传期间文件被编辑
        // 或替换，下一轮 planner 会发现差异并发起带版本校验的 Update，避免已提交任务
        // 因远端核验而循环执行。
        let (local_mtime, local_size) = if matches!(
            operation,
            TransferOperation::Create | TransferOperation::Update
        ) {
            (
                running
                    .source_mtime
                    .ok_or_else(|| AppError::generic("上传成功结算缺少源修改时间快照"))?,
                running
                    .source_size
                    .ok_or_else(|| AppError::generic("上传成功结算缺少源大小快照"))?,
            )
        } else {
            let metadata = std::fs::metadata(local_path)
                .map_err(|error| AppError::generic(format!("成功结算读取下载文件失败：{error}")))?;
            if !metadata.is_file() {
                return Err(AppError::generic("成功结算下载目标不是普通文件"));
            }
            let local_mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .ok_or_else(|| AppError::generic("成功结算无法读取下载文件修改时间"))?;
            (local_mtime, metadata.len() as i64)
        };
        let (file_id, name, size, cloud_edited_time, parent_folder_id) = match operation {
            TransferOperation::Create | TransferOperation::Update => {
                let cloud = output
                    .cloud_file
                    .as_ref()
                    .ok_or_else(|| AppError::generic("上传成功但缺少远端文件结果，拒绝结算"))?;
                (
                    cloud.id.clone(),
                    cloud.name.clone(),
                    cloud.size,
                    cloud.edited_time.map(|time| time.timestamp_millis()),
                    cloud
                        .parent_folder
                        .as_ref()
                        .and_then(|parents| parents.first().cloned())
                        .or_else(|| running.parent_file_id.clone()),
                )
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => (
                running
                    .file_id
                    .clone()
                    .ok_or_else(|| AppError::generic("下载成功结算缺少 fileId"))?,
                running.name.clone(),
                running.total_size,
                running.expected_cloud_edited_time,
                running.parent_file_id.clone(),
            ),
            _ => return Err(AppError::generic("该 operation 不支持成功结算")),
        };
        let finished_at = chrono::Utc::now().timestamp_millis();
        let completed = {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始传输结算事务失败：{error}")))?;
            let preserved_cloud_edited_time =
                if operation == TransferOperation::Update && cloud_edited_time.is_none() {
                    transaction
                        .query_row(
                            "SELECT cloud_edited_time FROM sync_items WHERE file_id=?1 LIMIT 1",
                            [file_id.as_str()],
                            |row| row.get::<_, Option<i64>>(0),
                        )
                        .optional()
                        .map_err(|error| {
                            AppError::generic(format!("读取更新前云端版本基线失败：{error}"))
                        })?
                        .flatten()
                } else {
                    None
                };
            let completed = repository::transition_transfer_in_transaction(
                &transaction,
                running.id,
                running.state_revision,
                TransferState::Completed,
                TransferPatch {
                    error_kind: ColumnPatch::Clear,
                    error_message: ColumnPatch::Clear,
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Set(finished_at),
                    remote_result_file_id: ColumnPatch::Set(file_id.clone()),
                    transferred: Some(running.total_size),
                    ..Default::default()
                },
            )
            .map_err(transition_error)?;
            transaction
                .execute(
                    "DELETE FROM sync_items
                     WHERE local_path=?1 AND file_id=?2",
                    rusqlite::params![
                        relative_path,
                        format!("{}{}", repository::PENDING_FILE_ID_PREFIX, relative_path)
                    ],
                )
                .map_err(|error| AppError::generic(format!("清理待确认同步基线失败：{error}")))?;
            if operation == TransferOperation::Update {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                        rusqlite::params![file_id, relative_path],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理改名/移动旧基线路径失败：{error}"))
                    })?;
            }
            repository::upsert(
                &transaction,
                &repository::SyncItem {
                    file_id,
                    local_path: relative_path.to_string(),
                    parent_folder_id,
                    name,
                    is_folder: false,
                    size,
                    local_size: Some(local_size),
                    sha256: None,
                    local_mtime: Some(local_mtime),
                    cloud_edited_time: cloud_edited_time.or(preserved_cloud_edited_time),
                    last_sync_time: Some(finished_at),
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                },
            )?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交传输结算事务失败：{error}")))?;
            completed
        };
        self.notify_best_effort();
        Ok(completed)
    }

    /// 收敛后端完成但本地结算失败的任务。
    pub(super) fn recover_success_settlement_failure(
        &self,
        running: &TransferTask,
        output: &mut TaskExecutionOutcome,
        error: AppError,
    ) -> AppResult<TaskExecutionOutcome> {
        let operation = running
            .operation_kind()
            .map_err(transition_error)?
            .ok_or_else(|| AppError::generic("成功结算恢复缺少 operation"))?;
        let message = format!("后端已完成，但本地同步基线结算失败：{error}");
        let (target, kind, disposition) = match operation {
            TransferOperation::Create | TransferOperation::Update => (
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                TaskDisposition::VerifyingRemote,
            ),
            TransferOperation::Download | TransferOperation::DownloadUpdate => (
                TransferState::RestartRequired,
                TransferErrorKind::Unknown,
                TaskDisposition::RestartRequired,
            ),
            _ => return Err(error),
        };
        self.transition(
            running.id,
            running.state_revision,
            target,
            TransferPatch {
                error_kind: ColumnPatch::Set(kind),
                error_message: ColumnPatch::Set(message),
                remote_result_file_id: output
                    .cloud_file
                    .as_ref()
                    .map(|cloud| ColumnPatch::Set(cloud.id.clone()))
                    .unwrap_or(ColumnPatch::Keep),
                ..Default::default()
            },
        )?;
        output.disposition = disposition;
        Ok(output.clone())
    }
}
