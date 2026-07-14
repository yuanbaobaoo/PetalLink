//! 只读汇总持久化事实与运行时同步状态。

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{params, Connection};

use crate::data::repository::{sync_status, transfer_direction};
use crate::error::{AppError, AppResult};
use crate::sync::state::{FailedItem, SyncGlobalState};
use crate::sync::transfer_state::TransferState;

/// SQLite 中没有持久化来源的运行时状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub editing: u64,
    pub is_running: bool,
    pub last_sync_time: Option<i64>,
    pub is_indexing: bool,
    pub indexing_scanned_folders: u64,
    pub indexing_discovered_items: u64,
    pub content_changed: bool,
    pub sync_phase: Option<String>,
}

impl From<&SyncGlobalState> for RuntimeStatus {
    /// 从完整全局状态中提取运行时字段。
    fn from(state: &SyncGlobalState) -> Self {
        Self {
            editing: state.editing,
            is_running: state.is_running,
            last_sync_time: state.last_sync_time,
            is_indexing: state.is_indexing,
            indexing_scanned_folders: state.indexing_scanned_folders,
            indexing_discovered_items: state.indexing_discovered_items,
            content_changed: state.content_changed,
            sync_phase: state.sync_phase.clone(),
        }
    }
}

impl RuntimeStatus {
    /// 即使数据库快照失败，也要保持生命周期门状态准确。
    pub(crate) fn apply_to(&self, state: &mut SyncGlobalState) {
        state.editing = self.editing;
        state.is_running = self.is_running;
        state.last_sync_time = self.last_sync_time;
        state.is_indexing = self.is_indexing;
        state.indexing_scanned_folders = self.indexing_scanned_folders;
        state.indexing_discovered_items = self.indexing_discovered_items;
        state.content_changed = self.content_changed;
        state.sync_phase.clone_from(&self.sync_phase);
    }
}

/// 在不改写持久化事实的前提下构建完整全局快照。
#[derive(Debug, Default)]
pub struct StatusAggregator {
    next_revision: AtomicU64,
    publication: Mutex<()>,
}

impl StatusAggregator {
    /// 串行化跨 Engine 替换的版本分配与状态发布。
    pub(crate) fn lock_publication(&self) -> MutexGuard<'_, ()> {
        self.publication.lock()
    }

    /// 获取发布锁，并仅在确认竞争时执行回调。
    pub(crate) fn lock_publication_with_contention_hook(
        &self,
        on_contention: impl FnOnce(),
    ) -> MutexGuard<'_, ()> {
        match self.publication.try_lock() {
            Some(guard) => guard,
            None => {
                on_contention();
                self.publication.lock()
            }
        }
    }

    /// 汇总数据库事实与运行时字段，生成新版本快照。
    pub fn snapshot(
        &self,
        conn: &Connection,
        runtime: RuntimeStatus,
    ) -> AppResult<SyncGlobalState> {
        let (total, failed, conflict, uploading, downloading, waiting_network, transfer_failed): (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM sync_items),
                    (SELECT COUNT(*) FROM sync_items WHERE status=?1),
                    (SELECT COUNT(*) FROM sync_items WHERE status=?2),
                    (SELECT COUNT(*) FROM transfer_queue WHERE state=?3 AND direction=?4),
                    (SELECT COUNT(*) FROM transfer_queue
                        WHERE state=?3 AND direction IN (?5, ?6)),
                    (SELECT COUNT(*) FROM transfer_queue WHERE state=?7),
                    (SELECT COUNT(*) FROM transfer_queue WHERE state=?8)",
                params![
                    sync_status::FAILED,
                    sync_status::CONFLICT,
                    i32::from(TransferState::Running),
                    transfer_direction::UPLOAD,
                    transfer_direction::DOWNLOAD,
                    transfer_direction::DOWNLOAD_UPDATE,
                    i32::from(TransferState::WaitingForNetwork),
                    i32::from(TransferState::Failed),
                ],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(|error| AppError::generic(format!("聚合同步状态失败：{error}")))?;

        let failed_items = {
            let mut statement = conn
                .prepare(
                    "SELECT local_path, error_message
                     FROM sync_items
                     WHERE status=?1
                     ORDER BY local_path
                     LIMIT 20",
                )
                .map_err(|error| AppError::generic(format!("查询失败项失败：{error}")))?;
            let rows = statement
                .query_map(params![sync_status::FAILED], |row| {
                    Ok(FailedItem {
                        relative_path: row.get(0)?,
                        error_message: row.get(1)?,
                    })
                })
                .map_err(|error| AppError::generic(format!("查询失败项失败：{error}")))?;
            let mut items = Vec::new();
            for row in rows {
                items.push(
                    row.map_err(|error| AppError::generic(format!("读取失败项失败：{error}")))?,
                );
            }
            items
        };

        let total = total as u64;
        let failed = failed as u64;
        let conflict = conflict as u64;
        let revision = self.next_revision.fetch_add(1, Ordering::Relaxed) + 1;

        Ok(SyncGlobalState {
            revision,
            total,
            completed: total.saturating_sub(failed + conflict),
            uploading: uploading as u64,
            downloading: downloading as u64,
            waiting_network: waiting_network as u64,
            failed,
            transfer_failed: transfer_failed as u64,
            failed_items,
            conflict,
            editing: runtime.editing,
            is_running: runtime.is_running,
            last_sync_time: runtime.last_sync_time,
            is_indexing: runtime.is_indexing,
            indexing_scanned_folders: runtime.indexing_scanned_folders,
            indexing_discovered_items: runtime.indexing_discovered_items,
            content_changed: runtime.content_changed,
            sync_phase: runtime.sync_phase,
        })
    }
}
