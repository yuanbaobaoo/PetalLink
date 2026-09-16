//! 用户对同名冲突与滞留任务的显式操作入口。

use super::persistence::transition_error;
use super::TaskRunner;
use crate::data::repository::{ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

impl TaskRunner {
    /// 读取任务并校验它处于「同名冲突待用户决策」状态。
    pub fn load_name_conflict_task(&self, task_id: i64) -> AppResult<TransferTask> {
        let task = self.load(task_id)?;
        if task.state_kind().map_err(transition_error)? != TransferState::RestartRequired {
            return Err(AppError::generic("任务状态已变化，请刷新后重试"));
        }
        let kind = task
            .error_kind
            .map(|value| TransferErrorKind::try_from(value).map_err(transition_error))
            .transpose()?;
        if kind != Some(TransferErrorKind::NameConflict) {
            return Err(AppError::generic("该任务不是同名冲突状态，无法执行此操作"));
        }
        Ok(task)
    }

    /// 冲突已由用户操作解除（远端已改名让位），清空错误并迁移回 Pending 等待重放。
    /// 计数器归零：用户显式操作开启新一轮预算周期，与人工重试同语义。
    pub fn requeue_name_conflict_task(&self, task: &TransferTask) -> AppResult<TransferTask> {
        // CAS 迁移：状态被并发改动时放弃，保留对方结果。
        let pending = self.transition(
            task.id,
            task.state_revision,
            TransferState::Pending,
            TransferPatch {
                error_kind: ColumnPatch::Clear,
                error_message: ColumnPatch::Clear,
                next_retry_at: ColumnPatch::Clear,
                finished_at: ColumnPatch::Clear,
                attempt_count: Some(0),
                verify_attempt_count: Some(0),
                ..Default::default()
            },
        )?;
        self.notify_best_effort();
        Ok(pending)
    }

    /// 用户显式取消任务。仅允许不再自动推进的状态，活动任务必须先等其收敛。
    pub fn cancel_user_task(&self, task_id: i64, message: &str) -> AppResult<TransferTask> {
        let task = self.load(task_id)?;
        let state = task.state_kind().map_err(transition_error)?;
        if !matches!(
            state,
            TransferState::RestartRequired | TransferState::Failed
        ) {
            return Err(AppError::generic("仅「需要重新检查」或失败的任务可以取消"));
        }
        let canceled = self.transition(
            task.id,
            task.state_revision,
            TransferState::Canceled,
            TransferPatch {
                error_kind: ColumnPatch::Set(TransferErrorKind::LocalChanged),
                error_message: ColumnPatch::Set(message.to_string()),
                finished_at: ColumnPatch::Set((self.now_ms)()),
                ..Default::default()
            },
        )?;
        self.notify_best_effort();
        Ok(canceled)
    }
}
