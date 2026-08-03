//! 提供传输任务状态快照发布。

use std::sync::Arc;

use parking_lot::RwLock;

use super::contracts::TaskStateSink;
use super::{TaskRunner, TransferTask};
use crate::error::AppResult;

impl TaskRunner {
    /// 替换任务状态发布接收器。
    pub fn set_state_sink(&self, state_sink: Arc<dyn TaskStateSink>) {
        *self.state_sink.write() = state_sink;
    }

    /// 注入上传终态失败的用户通知回调。
    pub fn set_upload_failed_notifier(&self, notifier: super::contracts::UploadFailedNotifier) {
        *self.upload_failed_notifier.write() = Some(notifier);
    }

    /// 上传任务结算为终态失败时通知用户；未注入回调时静默跳过。
    pub(super) fn notify_upload_failed(&self, task: &TransferTask, user_message: &str) {
        if let Some(notifier) = self.upload_failed_notifier.read().as_ref() {
            notifier(task, user_message);
        }
    }

    /// 发布任务状态并返回发布错误。
    fn notify(&self) -> AppResult<()> {
        publish_state(&self.state_sink, &self.transfer_update_tx)
    }

    /// 尽力发布任务状态。
    pub(super) fn notify_best_effort(&self) {
        publish_state_best_effort(&self.state_sink, &self.transfer_update_tx);
    }

    /// 发布任务拒绝后的状态。
    pub(super) fn notify_rejection(&self) {
        if let Err(error) = self.notify() {
            tracing::warn!(%error, "任务拒绝后重算状态失败");
        }
    }
}

/// 发布任务状态与传输更新通知。
fn publish_state(
    state_sink: &Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: &Option<tokio::sync::broadcast::Sender<()>>,
) -> AppResult<()> {
    let snapshot_result = state_sink.read().recompute_and_broadcast();
    if let Some(sender) = transfer_update_tx {
        let _ = sender.send(());
    }
    snapshot_result
}

/// 尽力发布任务状态与传输更新通知。
pub(super) fn publish_state_best_effort(
    state_sink: &Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: &Option<tokio::sync::broadcast::Sender<()>>,
) {
    if let Err(error) = publish_state(state_sink, transfer_update_tx) {
        tracing::warn!(%error, "任务状态变化后重算权威快照失败");
    }
}
