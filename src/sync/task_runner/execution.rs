//! 提供传输任务执行主链。

use super::admission::{active_task_disposition, RunningGateOutcome};
use super::contracts::{TaskDisposition, TaskExecutionError, TaskExecutionOutcome};
use super::persistence::transition_error;
use super::preflight::PreflightFailure;
use super::progress::TaskProgressReporter;
use super::TaskRunner;
use crate::data::repository::{ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

impl TaskRunner {
    /// 重试指定失败任务并执行。
    pub async fn retry(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let pending = self.prepare_retry(task_id).await?;
        self.run_expected(pending, false).await
    }

    /// 执行指定任务。
    pub async fn run(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let current = self.load(task_id)?;
        self.run_expected(current, true).await
    }

    /// 执行现有活动任务，或观察并发收敛结果。
    pub(super) async fn run_existing_or_observe(
        &self,
        existing: TransferTask,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = existing.state_kind().map_err(transition_error)?;
        if matches!(
            state,
            TransferState::Pending | TransferState::WaitingForNetwork | TransferState::BackingOff
        ) {
            match self.run_expected(existing.clone(), true).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) => {
                    let observed = self.load(existing.id)?;
                    if observed.state_revision != existing.state_revision {
                        return self.observed_concurrent_outcome(&observed);
                    }
                    return Err(error);
                }
            }
        }
        let disposition = active_task_disposition(state)
            .ok_or_else(|| AppError::generic("自动周期发现的任务已不再活动"))?;
        Ok(TaskExecutionOutcome {
            cloud_file: None,
            disposition,
        })
    }

    /// 将并发收敛后的任务状态映射为执行结果。
    fn observed_concurrent_outcome(
        &self,
        observed: &TransferTask,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = observed.state_kind().map_err(transition_error)?;
        if state == TransferState::Completed {
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: TaskDisposition::Completed,
            });
        }
        if let Some(disposition) = active_task_disposition(state) {
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition,
            });
        }
        Err(AppError::generic(format!(
            "任务已由并发执行收敛为 {state:?}{}",
            observed
                .error_message
                .as_deref()
                .map(|message| format!("：{message}"))
                .unwrap_or_default()
        )))
    }

    /// 执行已通过静态与后端前置校验的手动重试。
    pub async fn run_prepared(&self, task_id: i64) -> AppResult<TaskExecutionOutcome> {
        let current = self.load(task_id)?;
        self.run_expected(current, false).await
    }

    /// 按预期状态执行任务主链。
    pub(super) async fn run_expected(
        &self,
        current: TransferTask,
        run_backend_preflight: bool,
    ) -> AppResult<TaskExecutionOutcome> {
        let state = current.state_kind().map_err(transition_error)?;
        // 这里是单行任务的线性化点，且有意先于静态校验：校验失败需要持久化，
        // 下载校验也可能创建父目录。准入许可持续到后端结算完成，包括远端写入歧义。
        let _activity = self.begin_activity(&current)?;
        if !matches!(
            state,
            TransferState::Pending | TransferState::WaitingForNetwork | TransferState::BackingOff
        ) {
            self.notify_rejection();
            return Err(AppError::generic(format!("任务状态 {state:?} 不可执行")));
        }
        if state == TransferState::BackingOff && current.next_retry_at.is_none() {
            let failure = PreflightFailure::validation("退避任务缺少 next_retry_at，拒绝立即重放");
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            if failure.target == TransferState::RestartRequired {
                return Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::RestartRequired,
                });
            }
            return Err(AppError::generic(failure.message));
        }
        if !(self.online_check)() {
            if state == TransferState::Pending {
                self.transition_failure(
                    &current,
                    TransferState::WaitingForNetwork,
                    TransferErrorKind::Network,
                    "网络不可用，等待恢复",
                )?;
            } else {
                self.notify_rejection();
            }
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: if state == TransferState::BackingOff {
                    TaskDisposition::BackingOff
                } else {
                    TaskDisposition::WaitingForNetwork
                },
            });
        }
        if state == TransferState::BackingOff
            && current
                .next_retry_at
                .is_some_and(|next_retry_at| next_retry_at > (self.now_ms)())
        {
            self.notify_rejection();
            return Ok(TaskExecutionOutcome {
                cloud_file: None,
                disposition: TaskDisposition::BackingOff,
            });
        }
        if run_backend_preflight {
            if let Err(failure) = self.operations.preflight(&current).await {
                let failure = PreflightFailure::from(failure);
                self.persist_preflight_rejection(&current, failure.clone())?;
                if failure.target == TransferState::RestartRequired {
                    return Ok(TaskExecutionOutcome {
                        cloud_file: None,
                        disposition: TaskDisposition::RestartRequired,
                    });
                }
                return Err(AppError::generic(failure.message));
            }
        }
        let running = match self.transition_to_running_or_block(&current)? {
            RunningGateOutcome::Running(running) => *running,
            RunningGateOutcome::Blocked => {
                return Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::BlockedByActiveIntent,
                });
            }
        };
        let progress = TaskProgressReporter::new(
            self.db.clone(),
            running.id,
            running.state_revision,
            running.total_size,
            self.state_sink.clone(),
            self.transfer_update_tx.clone(),
        );
        match self.operations.execute(&running, &progress).await {
            Ok(mut output) => {
                progress.ensure_current()?;
                if output.disposition != TaskDisposition::Completed {
                    if matches!(
                        output.disposition,
                        TaskDisposition::Pending
                            | TaskDisposition::Running
                            | TaskDisposition::BlockedByActiveIntent
                            | TaskDisposition::BackingOff
                    ) {
                        return self.settle_error(
                            &running,
                            AppError::generic(format!(
                                "后端返回缺少可持久化恢复条件的状态 {:?}",
                                output.disposition
                            )),
                        );
                    }
                    self.persist_backend_disposition(&running, &output)?;
                    return Ok(output);
                }
                if let Err(failure) = self.validate_success_outcome(&running, &output) {
                    let remote_id = output.cloud_file.as_ref().map(|file| file.id.clone());
                    let remote_write_is_ambiguous = remote_id
                        .as_deref()
                        .is_some_and(|file_id| !file_id.trim().is_empty())
                        && matches!(
                            running.operation_kind().map_err(transition_error)?,
                            Some(TransferOperation::Create | TransferOperation::Update)
                        );
                    let (target, kind, message) = if remote_write_is_ambiguous {
                        (
                            TransferState::VerifyingRemote,
                            TransferErrorKind::RemoteAmbiguous,
                            format!("{}；远端已返回资源 ID，禁止直接重放", failure.message),
                        )
                    } else {
                        (failure.target, failure.kind, failure.message)
                    };
                    self.transition(
                        running.id,
                        running.state_revision,
                        target,
                        TransferPatch {
                            error_kind: ColumnPatch::Set(kind),
                            error_message: ColumnPatch::Set(message),
                            remote_result_file_id: remote_id
                                .map(ColumnPatch::Set)
                                .unwrap_or(ColumnPatch::Keep),
                            ..Default::default()
                        },
                    )?;
                    output.disposition = match target {
                        TransferState::VerifyingRemote => TaskDisposition::VerifyingRemote,
                        TransferState::RestartRequired => TaskDisposition::RestartRequired,
                        _ => return Err(AppError::generic("非法成功核验目标状态")),
                    };
                    return Ok(output);
                }
                match self.settle_success(&running, &output) {
                    Ok(completed) => {
                        debug_assert_eq!(completed.id, running.id);
                        output.disposition = TaskDisposition::Completed;
                        Ok(output)
                    }
                    Err(error) => {
                        self.recover_success_settlement_failure(&running, &mut output, error)
                    }
                }
            }
            Err(TaskExecutionError::RestartRequired(message)) => {
                self.transition_failure(
                    &running,
                    TransferState::RestartRequired,
                    TransferErrorKind::LocalChanged,
                    &message,
                )?;
                Ok(TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::RestartRequired,
                })
            }
            Err(TaskExecutionError::App(error)) => self.settle_error(&running, error),
        }
    }
}
