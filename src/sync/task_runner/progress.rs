//! TaskRunner 传输进度上报器。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use super::contracts::TaskStateSink;
use super::persistence::transition_error;
use super::publication::publish_state_best_effort;
use crate::data::repository::{self, ColumnPatch, RunningTransferPatch};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::TransferState;

/// 可见传输进度的最小持久间隔。
const PROGRESS_THROTTLE_MS: i64 = 500;

#[derive(Clone)]
/// 以 Running 修订号为门禁的传输进度报告器。
pub struct TaskProgressReporter {
    db: Arc<Mutex<rusqlite::Connection>>,
    task_id: i64,
    running_revision: i64,
    total_size: i64,
    state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    last_progress_ms: Arc<AtomicI64>,
}

impl TaskProgressReporter {
    /// 为指定 Running 任务创建进度报告器。
    pub(super) fn new(
        db: Arc<Mutex<rusqlite::Connection>>,
        task_id: i64,
        running_revision: i64,
        total_size: i64,
        state_sink: Arc<RwLock<Arc<dyn TaskStateSink>>>,
        transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
    ) -> Self {
        Self {
            db,
            task_id,
            running_revision,
            total_size,
            state_sink,
            transfer_update_tx,
            last_progress_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    /// 节流持久已传输字节数，并拒绝越界进度。
    pub fn update_transferred(&self, transferred: i64) -> AppResult<()> {
        if transferred < 0 || transferred > self.total_size {
            return Err(AppError::generic("传输进度超出任务总大小"));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let previous = self.last_progress_ms.load(Ordering::Relaxed);
        if previous != 0 && now.saturating_sub(previous) < PROGRESS_THROTTLE_MS {
            return Ok(());
        }
        self.last_progress_ms.store(now, Ordering::Relaxed);
        self.update(RunningTransferPatch {
            transferred: Some(transferred),
            ..Default::default()
        })
    }

    /// 持久化下载断点及其可见进度。与上传会话不同，此断点不需要服务端 URL；
    /// Download API 会先校验 sidecar 元数据和 `.tmp` 实际长度，再发送下一个 Range 请求。
    pub fn update_download_progress(&self, transferred: i64) -> AppResult<()> {
        if transferred < 0 || transferred > self.total_size {
            return Err(AppError::generic("下载断点超出任务总大小"));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let previous = self.last_progress_ms.load(Ordering::Relaxed);
        if previous != 0 && now.saturating_sub(previous) < PROGRESS_THROTTLE_MS {
            return Ok(());
        }
        self.last_progress_ms.store(now, Ordering::Relaxed);
        self.update(RunningTransferPatch {
            transferred: Some(transferred),
            resume_offset: Some(transferred),
            ..Default::default()
        })
    }

    /// 持久上传会话身份、断点与可见进度。
    pub fn update_resume(
        &self,
        server_id: &str,
        upload_id: &str,
        offset: i64,
        session_url: &str,
    ) -> AppResult<()> {
        if offset < 0 || offset > self.total_size {
            return Err(AppError::generic("断点偏移超出任务总大小"));
        }
        if offset > 0 && session_url.trim().is_empty() {
            return Err(AppError::generic("非零断点缺少 session_url"));
        }
        self.update(RunningTransferPatch {
            transferred: Some(offset),
            resume_offset: Some(offset),
            server_id: ColumnPatch::Set(server_id.to_string()),
            upload_id: ColumnPatch::Set(upload_id.to_string()),
            session_url: ColumnPatch::Set(session_url.to_string()),
        })
    }

    /// 确认报告器仍指向同一 Running 修订。
    pub fn ensure_current(&self) -> AppResult<()> {
        let task = repository::get_transfer_by_id(&self.db.lock(), self.task_id)?
            .ok_or_else(|| AppError::generic("传输任务不存在"))?;
        if task.state_revision != self.running_revision
            || task.state_kind().map_err(transition_error)? != TransferState::Running
        {
            return Err(AppError::generic("传输任务状态已变化，忽略过期回调"));
        }
        Ok(())
    }

    /// 以预期修订更新 Running 任务，并发布新状态。
    pub(super) fn update(&self, patch: RunningTransferPatch) -> AppResult<()> {
        {
            let conn = self.db.lock();
            repository::update_running_transfer(&conn, self.task_id, self.running_revision, patch)
                .map_err(transition_error)?;
        }
        publish_state_best_effort(&self.state_sink, &self.transfer_update_tx);
        Ok(())
    }
}
