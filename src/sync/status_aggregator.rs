//! Authoritative, read-only aggregation of persisted and runtime sync status.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{params, Connection};

use crate::data::repository::{sync_status, transfer_direction};
use crate::error::{AppError, AppResult};
use crate::sync::state::{FailedItem, SyncGlobalState};
use crate::sync::transfer_state::TransferState;

/// Runtime-only state that has no persistent source in SQLite.
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
    /// Keep lifecycle gates accurate even when a DB snapshot cannot be produced.
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

/// Builds complete global snapshots without modifying persisted facts.
#[derive(Debug, Default)]
pub struct StatusAggregator {
    next_revision: AtomicU64,
    publication: Mutex<()>,
}

impl StatusAggregator {
    /// Serialize revision allocation through publication across replacement engines.
    pub(crate) fn lock_publication(&self) -> MutexGuard<'_, ()> {
        self.publication.lock()
    }

    /// Acquire publication while exposing only a confirmed contention point to concurrency tests.
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

#[cfg(test)]
mod tests {
    use super::{RuntimeStatus, StatusAggregator};
    use crate::data::repository::{self, sync_status, transfer_direction, SyncItem, TransferTask};
    use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&conn).unwrap();
        conn
    }

    fn sync_item(path: &str, status: i32, error_message: Option<&str>) -> SyncItem {
        SyncItem {
            file_id: format!("file:{path}"),
            local_path: path.to_string(),
            parent_folder_id: Some("parent".into()),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_folder: false,
            size: 128,
            local_size: Some(128),
            sha256: Some("sha256".into()),
            local_mtime: Some(1_000),
            cloud_edited_time: Some(2_000),
            last_sync_time: Some(3_000),
            status,
            error_message: error_message.map(str::to_string),
        }
    }

    fn transfer_task(
        name: &str,
        state: TransferState,
        direction: i32,
        relative_path: &str,
    ) -> TransferTask {
        TransferTask {
            id: 0,
            direction,
            file_id: Some(format!("file:{relative_path}")),
            local_path: Some(format!("/mount/{relative_path}")),
            name: name.into(),
            total_size: 128,
            transferred: 64,
            state: state.into(),
            error_message: (state == TransferState::Failed).then(|| "permanent".into()),
            created_at: 1_000,
            finished_at: matches!(state, TransferState::Completed | TransferState::Failed)
                .then_some(2_000),
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(relative_path.into()),
            parent_file_id: Some("parent".into()),
            operation: Some(TransferOperation::Create.into()),
            source_mtime: Some(900),
            source_size: Some(128),
            expected_cloud_edited_time: Some(800),
            attempt_count: 1,
            next_retry_at: None,
            error_kind: (state == TransferState::Failed)
                .then(|| i32::from(TransferErrorKind::Permission)),
            remote_result_file_id: None,
            state_revision: 0,
        }
    }

    #[test]
    fn snapshot_derives_complete_state_without_mutating_the_database() {
        let conn = fresh_db();
        repository::upsert(&conn, &sync_item("ok.txt", sync_status::SYNCED, None)).unwrap();
        repository::upsert(
            &conn,
            &sync_item("failed.txt", sync_status::FAILED, Some("sync failed")),
        )
        .unwrap();
        repository::insert_transfer(
            &conn,
            &transfer_task(
                "completed",
                TransferState::Completed,
                transfer_direction::UPLOAD,
                "completed.txt",
            ),
        )
        .unwrap();
        repository::insert_transfer(
            &conn,
            &transfer_task(
                "waiting",
                TransferState::WaitingForNetwork,
                transfer_direction::DOWNLOAD,
                "waiting.txt",
            ),
        )
        .unwrap();
        repository::insert_transfer(
            &conn,
            &transfer_task(
                "failed",
                TransferState::Failed,
                transfer_direction::UPLOAD,
                "transfer-failed.txt",
            ),
        )
        .unwrap();
        let changes_before_snapshot = conn.total_changes();
        let aggregator = StatusAggregator::default();
        let runtime = RuntimeStatus {
            editing: 2,
            is_running: true,
            last_sync_time: Some(9_000),
            is_indexing: true,
            indexing_scanned_folders: 7,
            indexing_discovered_items: 11,
            content_changed: true,
            sync_phase: Some("indexing-manual".into()),
        };

        let first = aggregator.snapshot(&conn, runtime.clone()).unwrap();
        let second = aggregator.snapshot(&conn, runtime).unwrap();

        assert_eq!(first.total, 2);
        assert_eq!(first.completed, 1);
        assert_eq!(first.uploading, 0);
        assert_eq!(first.downloading, 0);
        assert_eq!(first.waiting_network, 1);
        assert_eq!(first.failed, 1);
        assert_eq!(first.transfer_failed, 1);
        assert_eq!(first.conflict, 0);
        assert_eq!(first.failed_items.len(), 1);
        assert_eq!(first.failed_items[0].relative_path, "failed.txt");
        assert_eq!(
            first.failed_items[0].error_message.as_deref(),
            Some("sync failed")
        );
        assert_eq!(first.editing, 2);
        assert!(first.is_running);
        assert_eq!(first.last_sync_time, Some(9_000));
        assert!(first.is_indexing);
        assert_eq!(first.indexing_scanned_folders, 7);
        assert_eq!(first.indexing_discovered_items, 11);
        assert!(first.content_changed);
        assert_eq!(first.sync_phase.as_deref(), Some("indexing-manual"));
        assert!(second.revision > first.revision);
        assert_eq!(conn.total_changes(), changes_before_snapshot);
    }
}
