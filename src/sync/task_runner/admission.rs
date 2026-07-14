//! TaskRunner 入队与运行仲裁使用的纯判定逻辑。

use super::contracts::TaskDisposition;
use crate::data::repository::TransferTask;
use crate::sync::transfer_state::{TransferOperation, TransferState};

pub(super) enum ExistingOrInsertedTask {
    Existing(Box<TransferTask>),
    Replanned(Box<TransferTask>),
    Blocked(i64),
    Inserted(i64),
}

pub(super) enum RunningGateOutcome {
    Running(Box<TransferTask>),
    Blocked,
}

pub(super) fn is_path_blocking_state(state: TransferState) -> bool {
    matches!(
        state,
        TransferState::Pending
            | TransferState::Running
            | TransferState::WaitingForNetwork
            | TransferState::BackingOff
            | TransferState::VerifyingRemote
    )
}

pub(super) fn same_transfer_intent(left: &TransferTask, right: &TransferTask) -> bool {
    if left.relative_path != right.relative_path
        || left.local_path != right.local_path
        || left.name != right.name
        || left.direction != right.direction
        || left.operation != right.operation
        || left.file_id != right.file_id
        || left.total_size != right.total_size
    {
        return false;
    }
    match left.operation_kind().ok().flatten() {
        Some(TransferOperation::Create | TransferOperation::Update) => {
            left.parent_file_id == right.parent_file_id
                && left.source_mtime == right.source_mtime
                && left.source_size == right.source_size
                && (left.operation_kind().ok().flatten() != Some(TransferOperation::Update)
                    || left.expected_cloud_edited_time == right.expected_cloud_edited_time)
        }
        Some(TransferOperation::Download | TransferOperation::DownloadUpdate) => {
            left.parent_file_id == right.parent_file_id
                && left.expected_cloud_edited_time == right.expected_cloud_edited_time
        }
        _ => false,
    }
}

pub(super) fn has_ambiguous_remote_write_result(task: &TransferTask) -> bool {
    matches!(
        task.operation_kind().ok().flatten(),
        Some(TransferOperation::Create | TransferOperation::Update)
    ) && has_persisted_remote_result(task)
}

pub(super) fn has_persisted_remote_result(task: &TransferTask) -> bool {
    task.remote_result_file_id
        .as_deref()
        .is_some_and(|file_id| !file_id.trim().is_empty())
}

pub(super) fn active_task_disposition(state: TransferState) -> Option<TaskDisposition> {
    match state {
        TransferState::Pending => Some(TaskDisposition::Pending),
        TransferState::Running => Some(TaskDisposition::Running),
        TransferState::WaitingForNetwork => Some(TaskDisposition::WaitingForNetwork),
        TransferState::BackingOff => Some(TaskDisposition::BackingOff),
        TransferState::VerifyingRemote => Some(TaskDisposition::VerifyingRemote),
        TransferState::RestartRequired => Some(TaskDisposition::RestartRequired),
        TransferState::Completed | TransferState::Failed | TransferState::Canceled => None,
    }
}
