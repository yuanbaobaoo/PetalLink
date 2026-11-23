//! 目录同步作业（`folder_sync_jobs`）的 SQLite 查询与写入。
//!
//! 作业只负责「遍历目录并投递传输任务」这一段可观测的准备工作，
//! 真正的文件传输一律落到 `transfer_queue`，两者通过 `enqueued` 计数关联。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};

/// 目录同步作业状态（持久化值，与前端同源）。
pub mod job_state {
    /// 已入队，等待 worker 取走。
    pub const QUEUED: i32 = 0;
    /// 正在遍历目录并投递传输任务。
    pub const PLANNING: i32 = 1;
    /// 已完成：差异已全部投递（传输本身仍在传输队列里推进）。
    pub const COMPLETED: i32 = 2;
    /// 执行失败。
    pub const FAILED: i32 = 3;
    /// 用户取消。
    pub const CANCELED: i32 = 4;
}

/// 一次「双端对齐」作业（对应 `folder_sync_jobs` 表一行）。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct FolderSyncJob {
    /// 自增主键。
    pub id: i64,
    /// 云端目录 ID。
    pub folder_id: String,
    /// 挂载根内的相对路径。
    pub rel_path: String,
    /// 作业状态（见 `job_state`）。
    pub state: i32,
    /// 本轮需要处理的文件总数；规划完成前为 0。
    pub planned_total: i64,
    /// 已投递到传输队列的文件数。
    pub enqueued: i64,
    /// 失败原因。
    pub error_message: Option<String>,
    /// 入队时间（毫秒）。
    pub created_at: i64,
    /// 开始规划时间（毫秒）。
    pub started_at: Option<i64>,
    /// 结束时间（毫秒）。
    pub finished_at: Option<i64>,
    /// 乐观并发版本。
    pub state_revision: i64,
}

impl FolderSyncJob {
    /// 从查询行构造作业实体。
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            folder_id: row.get("folder_id")?,
            rel_path: row.get("rel_path")?,
            state: row.get("state")?,
            planned_total: row.get("planned_total")?,
            enqueued: row.get("enqueued")?,
            error_message: row.get("error_message")?,
            created_at: row.get("created_at")?,
            started_at: row.get("started_at")?,
            finished_at: row.get("finished_at")?,
            state_revision: row.get("state_revision")?,
        })
    }
}

/// 入队一次目录同步作业；同目录同路径已有未完成作业时直接复用。
pub fn enqueue(
    conn: &Connection,
    folder_id: &str,
    rel_path: &str,
    now: i64,
) -> AppResult<FolderSyncJob> {
    // 先查活跃作业：重复点击同一次「双端对齐」不应该排队第二次。
    let existing = conn
        .query_row(
            "SELECT * FROM folder_sync_jobs
             WHERE folder_id=?1 AND rel_path=?2 AND state IN (?3, ?4)
             LIMIT 1",
            params![folder_id, rel_path, job_state::QUEUED, job_state::PLANNING],
            FolderSyncJob::from_row,
        )
        .ok();
    if let Some(job) = existing {
        return Ok(job);
    }
    conn.execute(
        "INSERT INTO folder_sync_jobs (folder_id, rel_path, state, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![folder_id, rel_path, job_state::QUEUED, now],
    )
    .map_err(|error| AppError::generic(format!("目录同步作业入队失败：{error}")))?;
    let id = conn.last_insert_rowid();
    get(conn, id)?.ok_or_else(|| AppError::generic("目录同步作业入队后读取失败"))
}

/// 按 ID 查询作业。
pub fn get(conn: &Connection, id: i64) -> AppResult<Option<FolderSyncJob>> {
    let mut statement = conn
        .prepare("SELECT * FROM folder_sync_jobs WHERE id=?1")
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    let mut rows = statement
        .query_map([id], FolderSyncJob::from_row)
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    match rows.next() {
        Some(Ok(job)) => Ok(Some(job)),
        Some(Err(error)) => Err(AppError::generic(format!("读取目录同步作业失败：{error}"))),
        None => Ok(None),
    }
}

/// 列出最近的作业（含终态），按入队时间倒序。
pub fn list_recent(conn: &Connection, limit: i64) -> AppResult<Vec<FolderSyncJob>> {
    let mut statement = conn
        .prepare("SELECT * FROM folder_sync_jobs ORDER BY created_at DESC, id DESC LIMIT ?1")
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    let rows = statement
        .query_map([limit], FolderSyncJob::from_row)
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    collect(rows)
}

/// 列出全部未完成作业（排队 + 规划中），按入队时间正序。
pub fn list_active(conn: &Connection) -> AppResult<Vec<FolderSyncJob>> {
    let mut statement = conn
        .prepare(
            "SELECT * FROM folder_sync_jobs
             WHERE state IN (?1, ?2) ORDER BY created_at ASC, id ASC",
        )
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    let rows = statement
        .query_map(
            params![job_state::QUEUED, job_state::PLANNING],
            FolderSyncJob::from_row,
        )
        .map_err(|error| AppError::generic(format!("查询目录同步作业失败：{error}")))?;
    collect(rows)
}

/// 取最早的排队作业并原子置为规划中；没有排队作业时返回 None。
pub fn take_next_queued(conn: &Connection, now: i64) -> AppResult<Option<FolderSyncJob>> {
    let candidate = conn
        .query_row(
            "SELECT * FROM folder_sync_jobs WHERE state=?1 ORDER BY created_at ASC, id ASC LIMIT 1",
            [job_state::QUEUED],
            FolderSyncJob::from_row,
        )
        .ok();
    let Some(job) = candidate else {
        return Ok(None);
    };
    // CAS：只有仍是排队中的那一行才能被取走，避免与取消并发交错。
    let changed = conn
        .execute(
            "UPDATE folder_sync_jobs
             SET state=?1, started_at=?2, state_revision=state_revision+1
             WHERE id=?3 AND state=?4 AND state_revision=?5",
            params![
                job_state::PLANNING,
                now,
                job.id,
                job_state::QUEUED,
                job.state_revision
            ],
        )
        .map_err(|error| AppError::generic(format!("领取目录同步作业失败：{error}")))?;
    if changed != 1 {
        return Ok(None);
    }
    get(conn, job.id)
}

/// 更新规划总数（遍历完云端子树后写入，便于 UI 展示进度分母）。
pub fn set_planned_total(conn: &Connection, id: i64, total: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE folder_sync_jobs SET planned_total=?1 WHERE id=?2",
        params![total, id],
    )
    .map_err(|error| AppError::generic(format!("更新目录同步作业进度失败：{error}")))?;
    Ok(())
}

/// 写入已投递数量（绝对值）。
///
/// 规划期间按步长写进度；完成时再用权威计数覆盖一次。绝对值写法让重复写入幂等，
/// 不会像累加那样在争锁失败后永久少计。
pub fn set_enqueued(conn: &Connection, id: i64, enqueued: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE folder_sync_jobs SET enqueued=?1 WHERE id=?2",
        params![enqueued, id],
    )
    .map_err(|error| AppError::generic(format!("更新目录同步作业进度失败：{error}")))?;
    Ok(())
}

/// 把作业标记为完成，并写入权威的已投递数量。
pub fn mark_completed(conn: &Connection, id: i64, enqueued: i64, now: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE folder_sync_jobs
         SET state=?1, enqueued=?2, finished_at=?3, state_revision=state_revision+1
         WHERE id=?4 AND state=?5",
        params![job_state::COMPLETED, enqueued, now, id, job_state::PLANNING],
    )
    .map_err(|error| AppError::generic(format!("完成目录同步作业失败：{error}")))?;
    Ok(())
}

/// 把作业标记为失败并记录原因。
pub fn mark_failed(conn: &Connection, id: i64, message: &str, now: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE folder_sync_jobs
         SET state=?1, error_message=?2, finished_at=?3, state_revision=state_revision+1
         WHERE id=?4 AND state IN (?5, ?6)",
        params![
            job_state::FAILED,
            message,
            now,
            id,
            job_state::QUEUED,
            job_state::PLANNING
        ],
    )
    .map_err(|error| AppError::generic(format!("标记目录同步作业失败：{error}")))?;
    Ok(())
}

/// 用户取消排队的作业；已在规划的作业会先标记取消，由 worker 自行收敛。
pub fn cancel(conn: &Connection, id: i64, now: i64) -> AppResult<bool> {
    let changed = conn
        .execute(
            "UPDATE folder_sync_jobs
             SET state=?1, finished_at=?2, state_revision=state_revision+1
             WHERE id=?3 AND state IN (?4, ?5)",
            params![
                job_state::CANCELED,
                now,
                id,
                job_state::QUEUED,
                job_state::PLANNING
            ],
        )
        .map_err(|error| AppError::generic(format!("取消目录同步作业失败：{error}")))?;
    Ok(changed == 1)
}

/// 修剪超过保留上限的终态作业，避免历史无限增长。
pub fn prune_terminal(conn: &Connection, keep: i64) -> AppResult<usize> {
    let changed = conn
        .execute(
            "DELETE FROM folder_sync_jobs
             WHERE state NOT IN (?1, ?2)
               AND id NOT IN (
                   SELECT id FROM folder_sync_jobs
                   WHERE state NOT IN (?1, ?2)
                   ORDER BY created_at DESC, id DESC LIMIT ?3
               )",
            params![job_state::QUEUED, job_state::PLANNING, keep],
        )
        .map_err(|error| AppError::generic(format!("修剪目录同步作业失败：{error}")))?;
    Ok(changed)
}

/// 收集查询结果并统一错误映射。
fn collect<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> AppResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| AppError::generic(format!("读取目录同步作业失败：{error}")))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个跑完全部迁移的内存数据库。
    fn open_database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        crate::data::migrations::run_with_mount(&connection, None).unwrap();
        connection
    }

    /// 同一目录重复入队必须复用活跃作业，不排队第二次。
    #[test]
    fn enqueue_is_idempotent_for_active_job() {
        let connection = open_database();

        let first = enqueue(&connection, "cloud-folder", "videos", 100).unwrap();
        let second = enqueue(&connection, "cloud-folder", "videos", 200).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.state, job_state::QUEUED);
        // 终态之后允许再次入队同样的目录。
        mark_failed(&connection, first.id, "读取云端失败", 300).unwrap();
        let third = enqueue(&connection, "cloud-folder", "videos", 400).unwrap();
        assert_ne!(third.id, first.id, "终态作业不应阻止新的同步请求");
    }

    /// 领取作业必须按入队顺序先进先出，并原子切到规划中。
    #[test]
    fn take_next_queued_is_fifo_and_cas() {
        let connection = open_database();
        // 先入队两条作业，模拟用户连续点击两个目录。
        let first = enqueue(&connection, "folder-a", "a", 100).unwrap();
        let second = enqueue(&connection, "folder-b", "b", 200).unwrap();

        let taken = take_next_queued(&connection, 300)
            .unwrap()
            .expect("应有排队作业");
        assert_eq!(taken.id, first.id);
        assert_eq!(taken.state, job_state::PLANNING);
        assert_eq!(taken.started_at, Some(300));

        let next = take_next_queued(&connection, 400)
            .unwrap()
            .expect("第二条应仍可领取");
        assert_eq!(next.id, second.id);
        // 全部领完后返回 None，worker 据此进入等待。
        assert!(take_next_queued(&connection, 500).unwrap().is_none());
    }

    /// 取消排队作业后 worker 不能再领取，且作业落为已取消终态。
    #[test]
    fn cancel_prevents_further_claim() {
        let connection = open_database();
        let job = enqueue(&connection, "folder-a", "a", 100).unwrap();

        assert!(cancel(&connection, job.id, 200).unwrap());

        let after = get(&connection, job.id).unwrap().unwrap();
        assert_eq!(after.state, job_state::CANCELED);
        assert_eq!(after.finished_at, Some(200));
        assert!(take_next_queued(&connection, 300).unwrap().is_none());
        // 重复取消返回 false，前端据此不重复提示。
        assert!(!cancel(&connection, job.id, 400).unwrap());
    }

    /// 完成的作业不能被迟到的取消覆盖。
    #[test]
    fn completed_job_is_not_overwritten_by_late_cancel() {
        let connection = open_database();
        let job = enqueue(&connection, "folder-a", "a", 100).unwrap();
        take_next_queued(&connection, 200).unwrap();

        mark_completed(&connection, job.id, 0, 300).unwrap();

        let after = get(&connection, job.id).unwrap().unwrap();
        assert_eq!(after.state, job_state::COMPLETED);
        assert_eq!(after.finished_at, Some(300));
        assert!(!cancel(&connection, job.id, 400).unwrap());
    }

    /// 规划进度必须能累加到作业行上，供 UI 展示「已投递 N/M 项」。
    #[test]
    fn planning_progress_is_persisted() {
        let connection = open_database();
        let job = enqueue(&connection, "folder-a", "a", 100).unwrap();
        take_next_queued(&connection, 200).unwrap();

        set_planned_total(&connection, job.id, 7).unwrap();
        set_enqueued(&connection, job.id, 3).unwrap();

        let after = get(&connection, job.id).unwrap().unwrap();
        assert_eq!(after.planned_total, 7);
        assert_eq!(after.enqueued, 3);
    }
}
