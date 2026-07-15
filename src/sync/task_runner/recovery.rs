//! 提供任务恢复流程。

use super::admission::has_ambiguous_remote_write_result;
use super::contracts::{
    RecoveredCloudFile, RemoteVerification, StartupRecoverySummary, TaskDisposition,
    TaskExecutionOutcome, TaskRecoverySummary,
};
use super::persistence::transition_error;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::AppResult;
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

impl TaskRunner {
    /// 将保存了远端结果 ID 的 RestartRequired 恢复为待核验状态。
    pub fn promote_ambiguous_restarts(&self) -> AppResult<usize> {
        let tasks = self.list_states(&[TransferState::RestartRequired])?;
        let mut promoted = 0usize;
        for task in tasks.into_iter().filter(has_ambiguous_remote_write_result) {
            let _activity = self.begin_activity(&task)?;
            let conn = self.db.lock();
            self.promote_restart_to_verifying(&conn, &task)?;
            promoted += 1;
        }
        if promoted > 0 {
            self.notify_best_effort();
        }
        Ok(promoted)
    }

    /// 恢复并核验远端写入结果不确定的任务。
    pub async fn resume_verifying(&self) -> AppResult<TaskRecoverySummary> {
        if !(self.online_check)() {
            return Ok(TaskRecoverySummary::default());
        }
        let tasks = self.list_states(&[TransferState::VerifyingRemote])?;
        let mut summary = TaskRecoverySummary::default();
        for task in tasks {
            // 核验可能串行执行较久，逐任务取时钟，避免本轮处理中跨过 deadline 后仍被跳过。
            let now = (self.now_ms)();
            if task
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > now)
            {
                continue;
            }
            match self.resume_verifying_task(&task).await {
                Ok(Some(outcome)) => Self::record_recovered_task(&mut summary, &task, &outcome),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "单个远端核验任务恢复失败，继续处理其他任务");
                }
            }
        }
        Ok(summary)
    }

    /// 在单一路径许可内核验并结算一个远端结果不确定的任务。
    async fn resume_verifying_task(
        &self,
        task: &TransferTask,
    ) -> AppResult<Option<TaskExecutionOutcome>> {
        // Committed 的状态校验与本地结算必须和远端 GET 共用同一路径许可。
        let activity = self.begin_activity(task)?;
        match self.operations.verify_remote(task).await {
            Ok(RemoteVerification::Committed(file)) => {
                let mut outcome = TaskExecutionOutcome {
                    cloud_file: Some(file.clone()),
                    disposition: TaskDisposition::Completed,
                };
                if let Err(failure) = self.validate_success_outcome(task, &outcome) {
                    let patch = TransferPatch {
                        error_kind: ColumnPatch::Set(failure.kind),
                        error_message: ColumnPatch::Set(format!(
                            "远端写入已确认，但结果仍无法安全结算：{}",
                            failure.message
                        )),
                        next_retry_at: if failure.target == TransferState::VerifyingRemote {
                            ColumnPatch::Set((self.now_ms)().saturating_add(60_000))
                        } else {
                            ColumnPatch::Clear
                        },
                        remote_result_file_id: ColumnPatch::Set(file.id),
                        ..Default::default()
                    };
                    if failure.target == TransferState::VerifyingRemote {
                        let conn = self.db.lock();
                        repository::patch_transfer_in_state(
                            &conn,
                            task.id,
                            task.state_revision,
                            TransferState::VerifyingRemote,
                            patch,
                        )
                        .map_err(transition_error)?;
                        self.notify_best_effort();
                    } else {
                        self.transition(task.id, task.state_revision, failure.target, patch)?;
                    }
                    return Ok(None);
                }
                if let Err(error) = self.settle_success(task, &outcome) {
                    self.recover_success_settlement_failure(task, &mut outcome, error)?;
                }
                Ok(Some(outcome))
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
                // run_expected 会自行获取同一路径许可；确认未提交后才释放核验许可。
                drop(activity);
                self.run_expected(pending, true).await.map(Some)
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
                            // 保留会话过期标记，直至确定远端不存在结果，才能丢弃旧会话标识。
                            error_kind: ColumnPatch::Set(error_kind),
                            error_message: ColumnPatch::Set(message),
                            next_retry_at: ColumnPatch::Set((self.now_ms)().saturating_add(60_000)),
                            ..Default::default()
                        },
                    )
                    .map_err(transition_error)?;
                }
                self.notify_best_effort();
                Ok(None)
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
                            error_message: ColumnPatch::Set(format!("远端核验暂不可用：{error}")),
                            next_retry_at: ColumnPatch::Set((self.now_ms)().saturating_add(15_000)),
                            ..Default::default()
                        },
                    )
                    .map_err(transition_error)?;
                }
                self.notify_best_effort();
                Ok(None)
            }
        }
    }

    /// 恢复等待网络的任务。
    pub async fn resume_waiting(&self) -> AppResult<TaskRecoverySummary> {
        if !(self.online_check)() {
            self.notify_rejection();
            return Ok(TaskRecoverySummary::default());
        }
        let tasks = self.list_states(&[TransferState::WaitingForNetwork])?;
        let mut summary = TaskRecoverySummary::default();
        for task in tasks {
            let task_id = task.id;
            match self.run_expected(task.clone(), true).await {
                Ok(outcome) => Self::record_recovered_task(&mut summary, &task, &outcome),
                Err(error) => {
                    tracing::warn!(task_id, %error, "等待网络任务恢复失败");
                }
            }
        }
        Ok(summary)
    }

    /// 记录真正完成的恢复任务，并保留远端写入的权威路径元数据。
    fn record_recovered_task(
        summary: &mut TaskRecoverySummary,
        task: &TransferTask,
        outcome: &TaskExecutionOutcome,
    ) {
        if outcome.disposition != TaskDisposition::Completed {
            return;
        }
        summary.completed += 1;
        if let (Some(relative_path), Some(file)) =
            (task.relative_path.as_ref(), outcome.cloud_file.as_ref())
        {
            summary.recovered_cloud_files.push(RecoveredCloudFile {
                relative_path: relative_path.clone(),
                file: file.clone(),
            });
        }
    }

    /// 恢复已到期的退避任务，不执行休眠。
    pub async fn resume_due_backoff(&self) -> AppResult<TaskRecoverySummary> {
        let now = (self.now_ms)();
        let tasks = self.list_states(&[TransferState::BackingOff])?;
        let mut summary = TaskRecoverySummary::default();
        for task in tasks {
            if task
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > now)
            {
                continue;
            }
            match self.run_expected(task.clone(), true).await {
                Ok(outcome) => Self::record_recovered_task(&mut summary, &task, &outcome),
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "退避任务恢复失败");
                }
            }
        }
        Ok(summary)
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
                    match self.suppress_startup_duplicate(&task) {
                        Ok(true) => summary.verifying_remote += 1,
                        Ok(false) => summary.failed += 1,
                        Err(error) => {
                            tracing::warn!(task_id = task.id, %error, "启动期重复任务收敛失败");
                            summary.failed += 1;
                        }
                    }
                }
                continue;
            }
            let selected = same_path.remove(0);
            selected_tasks.push(selected);
            for task in same_path {
                match self.suppress_startup_duplicate(&task) {
                    Ok(true) => summary.verifying_remote += 1,
                    Ok(false) => summary.failed += 1,
                    Err(error) => {
                        tracing::warn!(task_id = task.id, %error, "启动期重复任务收敛失败");
                        summary.failed += 1;
                    }
                }
            }
        }
        for task in selected_tasks {
            let task_id = task.id;
            match self.recover_startup_task(task).await {
                Ok(task_summary) => Self::merge_startup_summary(&mut summary, task_summary),
                Err(error) => {
                    tracing::warn!(task_id, %error, "单个启动任务恢复失败，继续处理其他任务");
                    summary.failed += 1;
                }
            }
        }
        Ok(summary)
    }

    /// 独立恢复一个启动期任务，避免后续任务失败丢失已经完成的恢复结果。
    async fn recover_startup_task(&self, task: TransferTask) -> AppResult<StartupRecoverySummary> {
        let mut summary = StartupRecoverySummary::default();
        // 逐行获取许可，确保关闭时尚未准入的任务保持原样。
        let _activity = self.begin_activity(&task)?;
        let state = task.state_kind().map_err(transition_error)?;
        if state != TransferState::Running {
            let recovered_path = task.relative_path.clone();
            self.record_startup_outcome(
                self.run_expected(task, true).await,
                recovered_path.as_deref(),
                &mut summary,
            );
            return Ok(summary);
        }

        let operation = match task.operation_kind().map_err(transition_error)? {
            Some(operation) => operation,
            None => {
                self.transition_failure(
                    &task,
                    TransferState::Failed,
                    TransferErrorKind::Validation,
                    "中断任务缺少合法 operation",
                )?;
                summary.failed += 1;
                return Ok(summary);
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
            }
            TransferOperation::Download | TransferOperation::DownloadUpdate => {
                if let Err(failure) = self.validate_static(&task) {
                    self.persist_preflight_rejection(&task, failure)?;
                    summary.failed += 1;
                    return Ok(summary);
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
                let recovered_path = pending.relative_path.clone();
                self.record_startup_outcome(
                    self.run_expected(pending, true).await,
                    recovered_path.as_deref(),
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
        Ok(summary)
    }

    /// 将单任务启动恢复统计合并到本轮汇总。
    fn merge_startup_summary(
        summary: &mut StartupRecoverySummary,
        task_summary: StartupRecoverySummary,
    ) {
        summary.completed += task_summary.completed;
        summary.waiting_network += task_summary.waiting_network;
        summary.verifying_remote += task_summary.verifying_remote;
        summary.failed += task_summary.failed;
        summary
            .recovered_cloud_files
            .extend(task_summary.recovered_cloud_files);
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
        relative_path: Option<&str>,
        summary: &mut StartupRecoverySummary,
    ) {
        match result {
            Ok(outcome) => match outcome.disposition {
                TaskDisposition::Completed => {
                    summary.completed += 1;
                    if let (Some(relative_path), Some(file)) =
                        (relative_path, outcome.cloud_file.as_ref())
                    {
                        summary.recovered_cloud_files.push(RecoveredCloudFile {
                            relative_path: relative_path.to_string(),
                            file: file.clone(),
                        });
                    }
                }
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
