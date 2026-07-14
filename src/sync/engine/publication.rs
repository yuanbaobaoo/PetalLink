//! 同步状态发布。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rusqlite::params;
use tokio::sync::broadcast;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::RuntimeStatus;
use crate::sync::task_runner::TaskRunner;
use crate::sync::transfer_state::TransferState;

use super::SyncEngine;

impl SyncEngine {
    /// 绑定任务运行器的状态发布回调。
    pub(crate) fn bind_task_runner_state_sink(self: &Arc<Self>, task_runner: &Arc<TaskRunner>) {
        let weak_engine = Arc::downgrade(self);
        task_runner.set_state_sink(Arc::new(move || {
            let engine = weak_engine
                .upgrade()
                .ok_or_else(|| AppError::generic("同步引擎已停止"))?;
            match engine.recompute_and_broadcast_state() {
                Ok(snapshot) => {
                    engine.note_transfer_state_changed(snapshot.revision, snapshot.waiting_network);
                    Ok(())
                }
                Err(error) => {
                    engine.notify_backoff_schedule_changed();
                    Err(error)
                }
            }
        }));
        self.initialize_known_waiting_count();
    }

    /// 通知退避调度状态发生变化。
    pub(crate) fn notify_backoff_schedule_changed(&self) {
        self.schedule_revision.fetch_add(1, Ordering::AcqRel);
        self.backoff_changed.notify_one();
    }

    /// 初始化等待网络任务的计数基线。
    fn initialize_known_waiting_count(&self) {
        let baseline = match self.recompute_and_broadcast_state() {
            Ok(snapshot) => (snapshot.revision, snapshot.waiting_network),
            Err(error) => {
                tracing::warn!(%error, "初始化等待网络任务边沿失败，使用保守基线");
                let has_waiting = repository::has_transfer_in_state(
                    &self.db.lock(),
                    TransferState::WaitingForNetwork,
                )
                .unwrap_or(true);
                (
                    self.current_state().revision,
                    if has_waiting { u64::MAX } else { 0 },
                )
            }
        };
        let mut known = self.known_waiting_count.lock();
        let is_newer = match *known {
            Some((revision, _)) => baseline.0 > revision,
            None => true,
        };
        if is_newer {
            *known = Some(baseline);
        }
    }

    /// 记录传输状态版本并处理等待网络边沿。
    pub(crate) fn note_transfer_state_changed(&self, revision: u64, waiting_count: u64) {
        self.notify_backoff_schedule_changed();
        let waiting_increased = {
            let mut known = self.known_waiting_count.lock();
            match *known {
                Some((known_revision, known_count)) if revision > known_revision => {
                    let increased = waiting_count > known_count;
                    *known = Some((revision, waiting_count));
                    increased
                }
                None => {
                    *known = Some((revision, waiting_count));
                    false
                }
                Some(_) => false,
            }
        };
        if waiting_increased && self.is_online() {
            (self.request_network_failure_reporter)();
        }
    }
    /// 订阅同步状态广播。
    pub fn state_receiver(&self) -> broadcast::Receiver<SyncGlobalState> {
        self.state_tx.subscribe()
    }
    /// 获取当前同步状态快照。
    pub fn current_state(&self) -> SyncGlobalState {
        self.state.lock().clone()
    }

    /// 重算并发布完整权威状态快照。
    pub fn recompute_and_broadcast_state(&self) -> AppResult<SyncGlobalState> {
        self.update_runtime_and_broadcast(|_| {})
    }

    /// 应用运行态变更后，重算并发布全部持久化字段。
    pub(crate) fn update_runtime_and_broadcast(
        &self,
        update: impl FnOnce(&mut RuntimeStatus),
    ) -> AppResult<SyncGlobalState> {
        let _publish_guard = self.status_aggregator.lock_publication();
        if *self.shutdown.lock() {
            return Err(AppError::generic("同步引擎已停止，拒绝发布状态"));
        }
        let mut runtime = RuntimeStatus::from(&*self.state.lock());
        update(&mut runtime);
        let snapshot_result = {
            let conn = self.db.lock();
            self.status_aggregator.snapshot(&conn, runtime.clone())
        };
        let snapshot = match snapshot_result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                runtime.apply_to(&mut self.state.lock());
                return Err(error);
            }
        };
        *self.state.lock() = snapshot.clone();
        let _ = self.state_tx.send(snapshot.clone());
        Ok(snapshot)
    }

    /// 在错误后恢复空闲运行态。
    pub(super) fn restore_idle_runtime_after_error(&self) {
        let _ = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        });
    }

    /// 清除选定终态传输历史，不修改同步成功基线。
    pub(crate) fn clear_transfer_history_and_broadcast(
        &self,
        include_completed: bool,
        include_failed: bool,
    ) -> AppResult<SyncGlobalState> {
        {
            let conn = self.db.lock();
            conn.execute(
                "DELETE FROM transfer_queue
                 WHERE (?1=1 AND state=?2) OR (?3=1 AND state=?4)",
                params![
                    include_completed as i32,
                    i32::from(TransferState::Completed),
                    include_failed as i32,
                    i32::from(TransferState::Failed),
                ],
            )
            .map_err(|error| AppError::generic(format!("清除传输历史失败：{error}")))?;
        }
        self.recompute_and_broadcast_state()
    }

    /// 结束同步周期并发布最新聚合状态。
    pub(super) fn update_and_push_state(&self, content_changed: bool) -> AppResult<()> {
        self.update_runtime_and_broadcast(|runtime| {
            runtime.content_changed = content_changed;
            runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        })?;
        Ok(())
    }

    /// 保留当前运行态并重算完整聚合状态。
    pub fn push_live_transfer_state(&self) {
        if let Err(error) = self.recompute_and_broadcast_state() {
            tracing::warn!(%error, "传输变化后重算全局状态失败");
        }
    }
}
