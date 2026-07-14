//! 提供任务恢复流程。

use super::contracts::{
    RemoteVerification, StartupRecoverySummary, TaskDisposition, TaskExecutionOutcome,
};
use super::persistence::transition_error;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::AppResult;
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

impl TaskRunner {
    /// 恢复并核验远端写入结果不确定的任务。
    pub async fn resume_verifying(&self) -> AppResult<usize> {
        if !(self.online_check)() {
            return Ok(0);
        }
        let tasks = self.list_states(&[TransferState::VerifyingRemote])?;
        let mut resolved = 0usize;
        for task in tasks {
            let verification = {
                let _activity = self.begin_activity(&task)?;
                self.operations.verify_remote(&task).await
            };
            match verification {
                Ok(RemoteVerification::Committed(file)) => {
                    let mut outcome = TaskExecutionOutcome {
                        cloud_file: Some(file.clone()),
                        disposition: TaskDisposition::Completed,
                    };
                    if let Err(failure) = self.validate_success_outcome(&task, &outcome) {
                        self.transition(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            TransferPatch {
                                error_kind: ColumnPatch::Set(failure.kind),
                                error_message: ColumnPatch::Set(format!(
                                    "远端写入已确认，但本地源无法安全结算：{}",
                                    failure.message
                                )),
                                remote_result_file_id: ColumnPatch::Set(file.id),
                                ..Default::default()
                            },
                        )?;
                        continue;
                    }
                    match self.settle_success(&task, &outcome) {
                        Ok(_) => resolved += 1,
                        Err(error) => {
                            self.recover_success_settlement_failure(&task, &mut outcome, error)?;
                        }
                    }
                }
                Ok(RemoteVerification::NotCommitted) => {
                    let session_expired = task.error_kind_typed().map_err(transition_error)?
                        == Some(TransferErrorKind::SessionExpired);
                    let restart_patch = TransferPatch {
                        error_kind: ColumnPatch::Set(if session_expired {
                            TransferErrorKind::SessionExpired
                        } else {
                            TransferErrorKind::RemoteAmbiguous
                        }),
                        error_message: ColumnPatch::Set(if session_expired {
                            "远端核验确认写入未提交，已清理失效会话，可以安全新建会话".to_string()
                        } else {
                            "远端核验确认写入未提交，可以安全重放".to_string()
                        }),
                        next_retry_at: ColumnPatch::Clear,
                        finished_at: ColumnPatch::Clear,
                        remote_result_file_id: ColumnPatch::Clear,
                        ..Default::default()
                    };
                    let restart = if session_expired {
                        self.transition_clearing_upload_session(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            restart_patch,
                        )?
                    } else {
                        self.transition(
                            task.id,
                            task.state_revision,
                            TransferState::RestartRequired,
                            restart_patch,
                        )?
                    };
                    let pending = self.transition(
                        restart.id,
                        restart.state_revision,
                        TransferState::Pending,
                        TransferPatch {
                            error_kind: ColumnPatch::Clear,
                            error_message: ColumnPatch::Clear,
                            next_retry_at: ColumnPatch::Clear,
                            finished_at: ColumnPatch::Clear,
                            ..Default::default()
                        },
                    )?;
                    match self.run_expected(pending, true).await {
                        Ok(outcome)
                            if !matches!(
                                outcome.disposition,
                                TaskDisposition::VerifyingRemote
                                    | TaskDisposition::WaitingForNetwork
                                    | TaskDisposition::BackingOff
                                    | TaskDisposition::BlockedByActiveIntent
                            ) =>
                        {
                            resolved += 1;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(task_id = task.id, %error, "核验后安全重放失败");
                        }
                    }
                }
                Ok(RemoteVerification::Ambiguous(message)) => {
                    let error_kind = if task.error_kind_typed().map_err(transition_error)?
                        == Some(TransferErrorKind::SessionExpired)
                    {
                        TransferErrorKind::SessionExpired
                    } else {
                        TransferErrorKind::RemoteAmbiguous
                    };
                    {
                        let conn = self.db.lock();
                        repository::patch_transfer_in_state(
                            &conn,
                            task.id,
                            task.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                // 保留会话过期标记，直至确定远端不存在结果，才能原子丢弃旧会话标识。
                                error_kind: ColumnPatch::Set(error_kind),
                                error_message: ColumnPatch::Set(message),
                                next_retry_at: ColumnPatch::Set(
                                    (self.now_ms)().saturating_add(60_000),
                                ),
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                    }
                    self.notify_best_effort();
                }
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "远端写入核验暂不可用，保留歧义状态");
                    {
                        let conn = self.db.lock();
                        repository::patch_transfer_in_state(
                            &conn,
                            task.id,
                            task.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                error_message: ColumnPatch::Set(format!(
                                    "远端核验暂不可用：{error}"
                                )),
                                next_retry_at: ColumnPatch::Set(
                                    (self.now_ms)().saturating_add(15_000),
                                ),
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                    }
                    self.notify_best_effort();
                }
            }
        }
        Ok(resolved)
    }

    /// 恢复等待网络的任务。
    pub async fn resume_waiting(&self) -> AppResult<usize> {
        if !(self.online_check)() {
            self.notify_rejection();
            return Ok(0);
        }
        let tasks = self.list_states(&[TransferState::WaitingForNetwork])?;
        let mut resumed = 0;
        for task in tasks {
            let task_id = task.id;
            match self.run_expected(task, true).await {
                Ok(outcome)
                    if !matches!(
                        outcome.disposition,
                        TaskDisposition::WaitingForNetwork | TaskDisposition::BlockedByActiveIntent
                    ) =>
                {
                    resumed += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(task_id, %error, "等待网络任务恢复失败");
                }
            }
        }
        Ok(resumed)
    }

    /// 恢复已到期的退避任务，不执行休眠。
    pub async fn resume_due_backoff(&self) -> AppResult<usize> {
        let now = (self.now_ms)();
        let tasks = self.list_states(&[TransferState::BackingOff])?;
        let mut resumed = 0;
        for task in tasks {
            if task
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > now)
            {
                continue;
            }
            match self.run_expected(task.clone(), true).await {
                Ok(outcome)
                    if !matches!(
                        outcome.disposition,
                        TaskDisposition::BackingOff | TaskDisposition::BlockedByActiveIntent
                    ) =>
                {
                    resumed += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "退避任务恢复失败");
                }
            }
        }
        Ok(resumed)
    }

    /// 返回下一次恢复截止时间。
    pub fn next_backoff_deadline_ms(&self) -> AppResult<Option<i64>> {
        let now = (self.now_ms)();
        Ok(self
            .list_states(&[TransferState::BackingOff, TransferState::VerifyingRemote])?
            .into_iter()
            .map(|task| task.next_retry_at.unwrap_or(now))
            .min())
    }

    /// 返回当前时间戳（毫秒）。
    pub(crate) fn current_time_ms(&self) -> i64 {
        (self.now_ms)()
    }

    /// 恢复启动时遗留的任务。
    pub async fn recover_startup(&self) -> AppResult<StartupRecoverySummary> {
        let mut tasks = self.list_states(&[TransferState::Pending, TransferState::Running])?;
        tasks.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        let mut summary = StartupRecoverySummary::default();
        let mut selected_tasks = Vec::new();
        let mut grouped = std::collections::HashMap::<String, Vec<TransferTask>>::new();
        for task in tasks {
            match task.relative_path.clone() {
                Some(relative_path) => grouped.entry(relative_path).or_default().push(task),
                None => selected_tasks.push(task),
            }
        }
        for (_, mut same_path) in grouped {
            let has_running_remote_write = same_path.iter().any(|task| {
                task.state_kind() == Ok(TransferState::Running)
                    && matches!(
                        task.operation_kind().ok().flatten(),
                        Some(TransferOperation::Create | TransferOperation::Update)
                    )
            });
            if has_running_remote_write {
                for task in same_path {
                    if self.suppress_startup_duplicate(&task)? {
                        summary.verifying_remote += 1;
                    } else {
                        summary.failed += 1;
                    }
                }
                continue;
            }
            let selected = same_path.remove(0);
            selected_tasks.push(selected);
            for task in same_path {
                if self.suppress_startup_duplicate(&task)? {
                    summary.verifying_remote += 1;
                } else {
                    summary.failed += 1;
                }
            }
        }
        for task in selected_tasks {
            // 启动恢复按行独立处理；逐行获取许可，确保行间关闭时尚未准入的任务保持原样。
            let _activity = self.begin_activity(&task)?;
            let state = match task.state_kind() {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "启动恢复跳过非法任务状态");
                    summary.failed += 1;
                    continue;
                }
            };
            if state == TransferState::Running {
                let operation = match task.operation_kind() {
                    Ok(Some(operation)) => operation,
                    _ => {
                        self.transition_failure(
                            &task,
                            TransferState::Failed,
                            TransferErrorKind::Validation,
                            "中断任务缺少合法 operation",
                        )?;
                        summary.failed += 1;
                        continue;
                    }
                };
                match operation {
                    TransferOperation::Create | TransferOperation::Update => {
                        self.transition_failure(
                            &task,
                            TransferState::VerifyingRemote,
                            TransferErrorKind::RemoteAmbiguous,
                            "进程中断时远端写入结果不确定，等待核验",
                        )?;
                        summary.verifying_remote += 1;
                        continue;
                    }
                    TransferOperation::Download | TransferOperation::DownloadUpdate => {
                        if let Err(failure) = self.validate_static(&task) {
                            self.persist_preflight_rejection(&task, failure)?;
                            summary.failed += 1;
                            continue;
                        }
                        let relative_path = task
                            .relative_path
                            .as_deref()
                            .expect("validated download task has relative path");
                        let validated_destination = self.mount_root.join(relative_path);
                        let tmp_path = crate::drive::download_api::tmp_path(&validated_destination);
                        let durable_offset = std::fs::metadata(&tmp_path)
                            .ok()
                            .filter(|metadata| metadata.is_file())
                            .map(|metadata| metadata.len().min(task.total_size as u64) as i64)
                            .unwrap_or(0);
                        let restart = self.transition_failure(
                            &task,
                            TransferState::RestartRequired,
                            TransferErrorKind::SessionExpired,
                            "进程中断，保留已验证下载断点并重新建立 Range 请求",
                        )?;
                        let pending = self.transition(
                            restart.id,
                            restart.state_revision,
                            TransferState::Pending,
                            TransferPatch {
                                error_kind: ColumnPatch::Clear,
                                error_message: ColumnPatch::Clear,
                                finished_at: ColumnPatch::Clear,
                                transferred: Some(durable_offset),
                                resume_offset: Some(durable_offset),
                                ..Default::default()
                            },
                        )?;
                        self.record_startup_outcome(
                            self.run_expected(pending, true).await,
                            &mut summary,
                        );
                    }
                    _ => {
                        self.transition_failure(
                            &task,
                            TransferState::Failed,
                            TransferErrorKind::Validation,
                            "该中断操作暂不支持自动恢复",
                        )?;
                        summary.failed += 1;
                    }
                }
            } else {
                self.record_startup_outcome(self.run_expected(task, true).await, &mut summary);
            }
        }
        Ok(summary)
    }

    /// 抑制启动期同路径旧任务，并保留可能的远程写入结果。
    fn suppress_startup_duplicate(&self, task: &TransferTask) -> AppResult<bool> {
        let _activity = self.begin_activity(task)?;
        let state = task.state_kind().map_err(transition_error)?;
        let operation = task.operation_kind().map_err(transition_error)?;
        if state == TransferState::Running
            && matches!(
                operation,
                Some(TransferOperation::Create | TransferOperation::Update)
            )
        {
            self.transition_failure(
                task,
                TransferState::VerifyingRemote,
                TransferErrorKind::RemoteAmbiguous,
                "启动恢复发现同路径多个活动任务；旧远端写入等待核验",
            )?;
            return Ok(true);
        }
        self.transition_failure(
            task,
            TransferState::RestartRequired,
            if state == TransferState::Running {
                TransferErrorKind::SessionExpired
            } else {
                TransferErrorKind::LocalChanged
            },
            "启动恢复仅保留同路径最新任务，旧任务等待重新规划",
        )?;
        Ok(false)
    }

    /// 将单任务恢复去向累计到启动统计。
    fn record_startup_outcome(
        &self,
        result: AppResult<TaskExecutionOutcome>,
        summary: &mut StartupRecoverySummary,
    ) {
        match result {
            Ok(outcome) => match outcome.disposition {
                TaskDisposition::Completed => summary.completed += 1,
                TaskDisposition::Pending
                | TaskDisposition::Running
                | TaskDisposition::BlockedByActiveIntent => {}
                TaskDisposition::WaitingForNetwork => summary.waiting_network += 1,
                TaskDisposition::VerifyingRemote => summary.verifying_remote += 1,
                TaskDisposition::BackingOff => {}
                TaskDisposition::RestartRequired => summary.failed += 1,
            },
            Err(error) => {
                tracing::warn!(%error, "启动任务恢复失败");
                summary.failed += 1;
            }
        }
    }
}
