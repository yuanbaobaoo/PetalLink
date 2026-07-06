//! 类型化实体 + 仓储操作（对齐 dart `SyncItemEntity` / `TransferTaskEntity` + DAO）。
//!
//! 状态/方向常量以 i32 形式持久化，提供枚举转换。

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 统一 DB 操作错误映射：`db_err!("查询", expr)` 等价于
/// `expr.map_err(|e| AppError::generic(format!("查询失败：{e}")))?`。
///
/// 替代散布在仓储层的重复 `.map_err(|e| AppError::generic(format!("XX失败：{e}")))?` 模式。
macro_rules! db_err {
    ($op:literal, $expr:expr) => {
        $expr.map_err(|e| AppError::generic(format!("{}失败：{e}", $op)))?
    };
}

// ===== 同步状态常量（对齐 dart SyncStatusType） =====
/// 0=已同步 1=仅云端 2=仅本地 3=同步中 4=失败 5=冲突
pub mod sync_status {
    pub const SYNCED: i32 = 0;
    pub const CLOUD_ONLY: i32 = 1;
    #[allow(dead_code)]
    pub const LOCAL_ONLY: i32 = 2;
    pub const SYNCING: i32 = 3;
    pub const FAILED: i32 = 4;
    pub const CONFLICT: i32 = 5;
    /// 用户已主动删除（tombstone：防云端重建）
    pub const DELETED: i32 = 7;
}

// ===== 传输方向常量（对齐 dart TransferDirectionType） =====
pub mod transfer_direction {
    pub const UPLOAD: i32 = 0;
    pub const DOWNLOAD: i32 = 1;
    pub const DELETE: i32 = 2;
    /// 云端新版本覆盖本地已有文件（语义为「更新」，区别于首次拉取的 DOWNLOAD）。
    /// 仅同步引擎的 Download 动作在本地已有真实内容时使用；与 DOWNLOAD 共享下载执行路径。
    pub const DOWNLOAD_UPDATE: i32 = 3;
}

/// 新增上传失败的占位 fileId 前缀。
/// 新增文件上传时云端无真实 fileId，失败时用此前缀 + 相对路径生成占位 fileId 写入 sync_items，
/// 让 retry_failed 能找到失败项。成功上传后由真实 fileId 覆盖（先清占位行）。
/// planner 据此前缀判断「待上传占位项」→ 重新 Upload，绝不删本地。
pub const PENDING_FILE_ID_PREFIX: &str = "pending:";

// ===== 传输状态常量（对齐 dart TransferStateType） =====
pub mod transfer_state {
    pub const PENDING: i32 = 0;
    pub const RUNNING: i32 = 1;
    #[allow(dead_code)]
    pub const PAUSED: i32 = 2;
    pub const COMPLETED: i32 = 3;
    pub const FAILED: i32 = 4;
    pub const CANCELED: i32 = 5;
}

/// 同步状态项实体（对应 sync_items 表一行）。
/// 对齐 dart `SyncItemEntity`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    /// 云端文件 ID（主键之一）
    pub file_id: String,
    /// 本地绝对路径（主键之二）
    pub local_path: String,
    /// 父目录 fileId
    pub parent_folder_id: Option<String>,
    /// 文件名
    pub name: String,
    /// 是否文件夹
    pub is_folder: bool,
    /// 云端大小（字节）
    pub size: i64,
    /// 本地大小（字节，v3，变更检测用）
    pub local_size: Option<i64>,
    /// 本地 SHA256
    pub sha256: Option<String>,
    /// 本地 mtime（毫秒）
    pub local_mtime: Option<i64>,
    /// 云端 editedTime（毫秒）
    pub cloud_edited_time: Option<i64>,
    /// 最后成功同步时间（毫秒）
    pub last_sync_time: Option<i64>,
    /// 同步状态（见 sync_status 常量）
    pub status: i32,
    /// 失败/冲突原因
    pub error_message: Option<String>,
}

/// 传输任务实体（对应 transfer_queue 表一行）。
/// 对齐 dart `TransferTaskEntity`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTask {
    /// 自增主键
    pub id: i64,
    /// 上传/下载（见 transfer_direction 常量）
    pub direction: i32,
    /// 关联的 SyncItem fileId（可空，手动传输无对应项）
    pub file_id: Option<String>,
    /// 本地路径（可空）
    pub local_path: Option<String>,
    /// 文件名
    pub name: String,
    /// 总大小（字节）
    pub total_size: i64,
    /// 已传输（字节）
    pub transferred: i64,
    /// 传输状态（见 transfer_state 常量）
    pub state: i32,
    /// 失败原因
    pub error_message: Option<String>,
    /// 入队时间（毫秒）
    pub created_at: i64,
    /// 完成时间（毫秒）
    pub finished_at: Option<i64>,
    /// 华为 resume 上传会话标识（v2）
    pub server_id: Option<String>,
    /// 华为 uploadId（v2）
    pub upload_id: Option<String>,
    /// 已上传字节偏移（断点续传恢复点，v2）
    pub resume_offset: i64,
}

// ===== SyncItems 仓储 =====

impl SyncItem {
    /// 从行读取
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            file_id: row.get("file_id")?,
            local_path: row.get("local_path")?,
            parent_folder_id: row.get("parent_folder_id")?,
            name: row.get("name")?,
            is_folder: row.get::<_, i64>("is_folder")? != 0,
            size: row.get("size")?,
            local_size: row.get("local_size")?,
            sha256: row.get("sha256")?,
            local_mtime: row.get("local_mtime")?,
            cloud_edited_time: row.get("cloud_edited_time")?,
            last_sync_time: row.get("last_sync_time")?,
            status: row.get("status")?,
            error_message: row.get("error_message")?,
        })
    }
}

/// 按 fileId 查询单条同步记录。
pub fn find_by_file_id(conn: &Connection, file_id: &str) -> AppResult<Option<SyncItem>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM sync_items WHERE file_id = ?1 LIMIT 1")
    );
    let mut rows = db_err!("查询", stmt.query_map(params![file_id], SyncItem::from_row));
    match rows.next() {
        Some(Ok(item)) => Ok(Some(item)),
        Some(Err(_)) => Ok(None),
        None => Ok(None),
    }
}

/// 加载全部同步记录（按 local_path 索引）。对齐 dart `_loadDbRecords`。
/// 过滤 basename 以 `.hwcloud_` 开头的内部文件记录。
pub fn load_all(conn: &Connection) -> AppResult<Vec<SyncItem>> {
    let mut stmt = db_err!("查询", conn.prepare("SELECT * FROM sync_items"));
    let rows = db_err!("查询", stmt.query_map([], SyncItem::from_row));
    let mut items = Vec::new();
    for item in rows.flatten() {
        // 过滤内部文件（对齐 _loadDbRecords 跳过 .hwcloud_ 前缀）
        let basename = std::path::Path::new(&item.local_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !basename.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
            items.push(item);
        }
    }
    Ok(items)
}

/// 插入或更新（冲突时替换）。对齐 dart `insertOnConflictUpdate`。
pub fn upsert(conn: &Connection, item: &SyncItem) -> AppResult<()> {
    db_err!(
        "写入",
        conn.execute(
            "INSERT OR REPLACE INTO sync_items
                (file_id, local_path, parent_folder_id, name, is_folder, size, local_size,
                 sha256, local_mtime, cloud_edited_time, last_sync_time, status, error_message)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                item.file_id,
                item.local_path,
                item.parent_folder_id,
                item.name,
                item.is_folder as i64,
                item.size,
                item.local_size,
                item.sha256,
                item.local_mtime,
                item.cloud_edited_time,
                item.last_sync_time,
                item.status,
                item.error_message,
            ],
        )
    );
    Ok(())
}

/// 按 local_path 删除记录。
#[allow(dead_code)]
pub fn delete_by_local_path(conn: &Connection, local_path: &str) -> AppResult<()> {
    db_err!(
        "删除",
        conn.execute(
            "DELETE FROM sync_items WHERE local_path = ?1",
            params![local_path],
        )
    );
    Ok(())
}

/// 清空全部同步记录（退出登录/清空缓存用）。
pub fn delete_all(conn: &Connection) -> AppResult<()> {
    db_err!("清空", conn.execute("DELETE FROM sync_items", []));
    Ok(())
}

/// 重置过期状态：syncing(3)/failed(4) → 根据情况重置。
/// 对齐 dart `_resetStaleStatuses`：文件夹→synced；文件→缺失则 synced，
/// elif 占位则 cloudOnly，否则 synced。
pub fn reset_stale_statuses(conn: &Connection) -> AppResult<()> {
    // 简化实现：syncing→synced，failed→保留（需用户重试）。
    // 完整逻辑在 sync_engine 启动时根据本地文件存在性细化。
    db_err!(
        "重置状态",
        conn.execute(
            "UPDATE sync_items SET status = ?1 WHERE status = ?2",
            params![sync_status::SYNCED, sync_status::SYNCING],
        )
    );
    Ok(())
}

// ===== TransferQueue 仓储 =====

impl TransferTask {
    /// 从行读取
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            direction: row.get("direction")?,
            file_id: row.get("file_id")?,
            local_path: row.get("local_path")?,
            name: row.get("name")?,
            total_size: row.get("total_size")?,
            transferred: row.get("transferred")?,
            state: row.get("state")?,
            error_message: row.get("error_message")?,
            created_at: row.get("created_at")?,
            finished_at: row.get("finished_at")?,
            server_id: row.get("server_id")?,
            upload_id: row.get("upload_id")?,
            resume_offset: row.get("resume_offset")?,
        })
    }
}

/// 插入传输任务，返回自增 id。
pub fn insert_transfer(conn: &Connection, task: &TransferTask) -> AppResult<i64> {
    db_err!(
        "插入传输任务",
        conn.execute(
            "INSERT INTO transfer_queue
                (direction, file_id, local_path, name, total_size, transferred, state,
                 error_message, created_at, finished_at, server_id, upload_id, resume_offset)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                task.direction,
                task.file_id,
                task.local_path,
                task.name,
                task.total_size,
                task.transferred,
                task.state,
                task.error_message,
                task.created_at,
                task.finished_at,
                task.server_id,
                task.upload_id,
                task.resume_offset,
            ],
        )
    );
    Ok(conn.last_insert_rowid())
}

/// 按状态+方向查询传输任务（按 created_at 倒序）。对齐 dart 传输队列列表。
#[allow(dead_code)]
pub fn list_transfers(
    conn: &Connection,
    direction: Option<i32>,
    state_filter: Option<i32>,
) -> AppResult<Vec<TransferTask>> {
    match (direction, state_filter) {
        (Some(d), Some(s)) => {
            let mut stmt = db_err!(
                "查询",
                conn.prepare(
                    "SELECT * FROM transfer_queue WHERE direction = ?1 AND state = ?2 ORDER BY created_at DESC",
                )
            );
            collect_tasks(stmt.query_map(params![d, s], TransferTask::from_row))
        }
        (Some(d), None) => list_transfers_with_dir(conn, d),
        (None, Some(s)) => list_transfers_with_state(conn, s),
        (None, None) => list_all_transfers(conn),
    }
}

/// 收集迭代结果为 Vec<TransferTask>，跳过解析失败的行。
/// 接收 query_map 返回的 MappedRows（迭代产出 rusqlite::Result<TransferTask>）。
fn collect_tasks<I>(rows_result: rusqlite::Result<I>) -> AppResult<Vec<TransferTask>>
where
    I: Iterator<Item = rusqlite::Result<TransferTask>>,
{
    let rows = db_err!("查询", rows_result);
    let mut tasks = Vec::new();
    for t in rows.flatten() {
        tasks.push(t);
    }
    Ok(tasks)
}

#[allow(dead_code)]
fn list_transfers_with_dir(conn: &Connection, d: i32) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue WHERE direction = ?1 ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map(params![d], TransferTask::from_row))
}

#[allow(dead_code)]
fn list_transfers_with_state(conn: &Connection, s: i32) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue WHERE state = ?1 ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map(params![s], TransferTask::from_row))
}

/// 查询所有传输任务（created_at 倒序）。
pub fn list_all_transfers(conn: &Connection) -> AppResult<Vec<TransferTask>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM transfer_queue ORDER BY created_at DESC")
    );
    collect_tasks(stmt.query_map([], TransferTask::from_row))
}

/// 更新传输任务状态/进度。
#[allow(dead_code)]
pub fn update_transfer_state(
    conn: &Connection,
    id: i64,
    state: i32,
    transferred: i64,
    finished_at: Option<i64>,
    error_message: Option<&str>,
) -> AppResult<()> {
    db_err!(
        "更新传输任务",
        conn.execute(
            "UPDATE transfer_queue SET state = ?1, transferred = ?2, finished_at = ?3, error_message = ?4 WHERE id = ?5",
            params![state, transferred, finished_at, error_message, id],
        )
    );
    Ok(())
}

/// 清空传输队列表。
pub fn delete_all_transfers(conn: &Connection) -> AppResult<()> {
    db_err!("清空", conn.execute("DELETE FROM transfer_queue", []));
    Ok(())
}

/// 结算传输任务：成功 → COMPLETED + transferred=total_size；失败 → FAILED + transferred 保持。
///
/// 替代 commands.rs 中 3 处重复的结算 SQL（download_on_demand / folder_recursive 下载循环 / 上传循环）。
/// 错误仅忽略（与原内联实现一致——结算失败不应阻断主流程）。
pub fn settle_transfer_by_id(conn: &Connection, task_id: i64, success: bool, error_message: Option<&str>) {
    let (state, transferred_sql) = if success {
        (transfer_state::COMPLETED, "transferred = total_size")
    } else {
        (transfer_state::FAILED, "transferred = transferred")
    };
    let sql = format!(
        "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, {transferred_sql} WHERE id=?4"
    );
    let _ = conn.execute(
        &sql,
        params![state, error_message, chrono::Utc::now().timestamp_millis(), task_id],
    );
}

/// 修剪传输历史：保留最近 N 条已结束任务（completed/failed/canceled）。
/// 对齐 dart `_pruneTransferHistory`（保留最近 100 条）。
pub fn prune_transfer_history(conn: &Connection, keep: usize) -> AppResult<()> {
    db_err!(
        "修剪历史",
        conn.execute(
            "DELETE FROM transfer_queue
             WHERE id IN (
                SELECT id FROM transfer_queue
                WHERE state IN (?1, ?2, ?3)
                ORDER BY id DESC
                LIMIT -1 OFFSET ?4
             )",
            params![
                transfer_state::COMPLETED,
                transfer_state::FAILED,
                transfer_state::CANCELED,
                keep as i64,
            ],
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> Connection {
        // 注意：tempdir() 返回的 TempDir 在 drop 时会删除目录及文件，
        // 必须用 into_path() 固化为持久路径，否则连接在写入前文件已被删除 → readonly。
        let dir = tempfile::tempdir().unwrap().keep();
        let path = dir.join("test.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::data::migrations::run(&conn).unwrap();
        conn
    }

    fn sample_item(file_id: &str, status: i32) -> SyncItem {
        SyncItem {
            file_id: file_id.to_string(),
            local_path: format!("/tmp/{file_id}.txt"),
            parent_folder_id: None,
            name: format!("{file_id}.txt"),
            is_folder: false,
            size: 100,
            local_size: Some(100),
            sha256: None,
            local_mtime: Some(1000),
            cloud_edited_time: Some(1000),
            last_sync_time: Some(1000),
            status,
            error_message: None,
        }
    }

    #[test]
    fn test_upsert_and_find() {
        let conn = fresh_db();
        let item = sample_item("f1", sync_status::SYNCED);
        upsert(&conn, &item).unwrap();
        let found = find_by_file_id(&conn, "f1").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "f1.txt");
    }

    #[test]
    fn test_upsert_replaces() {
        let conn = fresh_db();
        let mut item = sample_item("f1", sync_status::SYNCED);
        upsert(&conn, &item).unwrap();
        item.status = sync_status::FAILED;
        item.error_message = Some("err".into());
        upsert(&conn, &item).unwrap();
        let found = find_by_file_id(&conn, "f1").unwrap().unwrap();
        assert_eq!(found.status, sync_status::FAILED);
        assert_eq!(found.error_message.as_deref(), Some("err"));
    }

    #[test]
    fn test_delete_by_path() {
        let conn = fresh_db();
        upsert(&conn, &sample_item("f1", sync_status::SYNCED)).unwrap();
        delete_by_local_path(&conn, "/tmp/f1.txt").unwrap();
        assert!(find_by_file_id(&conn, "f1").unwrap().is_none());
    }

    #[test]
    fn test_load_all_filters_internal() {
        let conn = fresh_db();
        let normal = sample_item("f1", sync_status::SYNCED);
        upsert(&conn, &normal).unwrap();
        // 内部文件（.hwcloud_ 前缀）应被 load_all 过滤
        let internal = SyncItem {
            file_id: "internal".into(),
            local_path: "/tmp/.hwcloud_cache.json".into(),
            name: ".hwcloud_cache.json".into(),
            ..sample_item("internal", sync_status::SYNCED)
        };
        upsert(&conn, &internal).unwrap();
        let all = load_all(&conn).unwrap();
        assert_eq!(all.len(), 1); // 仅 normal
        assert_eq!(all[0].file_id, "f1");
    }

    #[test]
    fn test_transfer_crud() {
        let conn = fresh_db();
        let task = TransferTask {
            id: 0,
            direction: transfer_direction::UPLOAD,
            file_id: Some("f1".into()),
            local_path: Some("/tmp/f1.txt".into()),
            name: "f1.txt".into(),
            total_size: 1000,
            transferred: 500,
            state: transfer_state::RUNNING,
            error_message: None,
            created_at: 1000,
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
        };
        let id = insert_transfer(&conn, &task).unwrap();
        assert!(id > 0);
        update_transfer_state(&conn, id, transfer_state::COMPLETED, 1000, Some(2000), None).unwrap();
        let all = list_all_transfers(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, transfer_state::COMPLETED);
        assert_eq!(all[0].transferred, 1000);
    }

    #[test]
    fn test_prune_history() {
        let conn = fresh_db();
        // 插入 5 条已完成 + 1 条运行中
        for i in 0..5 {
            let mut t = TransferTask {
                id: 0,
                direction: transfer_direction::UPLOAD,
                file_id: None,
                local_path: None,
                name: format!("t{i}"),
                total_size: 0,
                transferred: 0,
                state: transfer_state::COMPLETED,
                error_message: None,
                created_at: i,
                finished_at: Some(i),
                server_id: None,
                upload_id: None,
                resume_offset: 0,
            };
            insert_transfer(&conn, &t).unwrap();
            t.state = transfer_state::RUNNING;
            t.name = "running".into();
            insert_transfer(&conn, &t).unwrap();
        }
        // 保留最近 2 条已完成
        prune_transfer_history(&conn, 2).unwrap();
        let completed: Vec<_> = list_transfers(&conn, None, Some(transfer_state::COMPLETED))
            .unwrap();
        assert_eq!(completed.len(), 2);
    }
}
