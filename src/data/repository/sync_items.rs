//! 同步基线记录的 SQLite 查询与写入。

use rusqlite::{params, Connection};

use super::{sync_status, SyncItem};
use crate::error::{AppError, AppResult};

/// 按 fileId 查询单条同步记录。
pub fn find_by_file_id(conn: &Connection, file_id: &str) -> AppResult<Option<SyncItem>> {
    let mut stmt = db_err!(
        "查询",
        conn.prepare("SELECT * FROM sync_items WHERE file_id = ?1 LIMIT 2")
    );
    let mut rows = db_err!("查询", stmt.query_map(params![file_id], SyncItem::from_row));
    let first = match rows.next() {
        Some(Ok(item)) => item,
        Some(Err(error)) => return Err(AppError::generic(format!("读取同步记录失败：{error}"))),
        None => return Ok(None),
    };
    if let Some(second) = rows.next() {
        second.map_err(|error| AppError::generic(format!("读取同步记录失败：{error}")))?;
        return Err(AppError::generic(format!(
            "fileId {file_id} 对应多条本地路径，拒绝使用歧义同步基线"
        )));
    }
    Ok(Some(first))
}

/// 加载全部同步记录（按 local_path 索引）。对齐 dart `_loadDbRecords`。
/// 过滤 basename 以 `.hwcloud_` 开头的内部文件记录。
pub fn load_all(conn: &Connection) -> AppResult<Vec<SyncItem>> {
    let mut stmt = db_err!("查询", conn.prepare("SELECT * FROM sync_items"));
    let rows = db_err!("查询", stmt.query_map([], SyncItem::from_row));
    let mut items = Vec::new();
    for item in rows {
        let item = item.map_err(|error| AppError::generic(format!("读取同步记录失败：{error}")))?;
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
