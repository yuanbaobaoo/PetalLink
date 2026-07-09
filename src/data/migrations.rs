//! 数据库迁移 —— schemaVersion=4。
//!
//! 对齐 dart `MigrationStrategy`：
//! - v1 onCreate: 建全部表
//! - v2 onUpgrade from<2: TransferQueue 加 serverId/uploadId/resumeOffset（分片续传断点）
//! - v3 onUpgrade from<3: SyncItems 加 localSize（本地变更检测）
//! - v4 onUpgrade from<4: TransferQueue 加 session_url（华为 resume 上传 Location 头会话 URL）
//! - beforeOpen: PRAGMA foreign_keys = ON（已在 open 中处理）

use rusqlite::Connection;

use crate::data::SCHEMA_VERSION;
use crate::error::{AppError, AppResult};

/// 用户版本 PRAGMA key
const USER_VERSION_PRAGMA: &str = "PRAGMA user_version";

/// 运行迁移。读取当前 user_version，按需建表/升级。
pub fn run(conn: &Connection) -> AppResult<()> {
    let current: u32 = conn
        .query_row(USER_VERSION_PRAGMA, [], |row| row.get::<_, i64>(0))
        .map(|v| v as u32)
        .unwrap_or(0);

    if current == 0 {
        // 全新数据库：建全部表（v4 终态）
        create_all(conn)?;
        set_version(conn, SCHEMA_VERSION)?;
        return Ok(());
    }

    // 逐步升级
    if current < 2 {
        upgrade_to_v2(conn)?;
    }
    if current < 3 {
        upgrade_to_v3(conn)?;
    }
    if current < 4 {
        upgrade_to_v4(conn)?;
    }
    set_version(conn, SCHEMA_VERSION)?;
    Ok(())
}

/// v1 onCreate：建全部表（直接建 v3 终态结构）。
fn create_all(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sync_items (
            file_id           TEXT    NOT NULL,
            local_path        TEXT    NOT NULL,
            parent_folder_id  TEXT,
            name              TEXT    NOT NULL,
            is_folder         INTEGER NOT NULL DEFAULT 0,
            size              INTEGER NOT NULL DEFAULT 0,
            local_size        INTEGER,
            sha256            TEXT,
            local_mtime       INTEGER,
            cloud_edited_time INTEGER,
            last_sync_time    INTEGER,
            status            INTEGER NOT NULL DEFAULT 0,
            error_message     TEXT,
            PRIMARY KEY (file_id, local_path)
        );

        CREATE TABLE IF NOT EXISTS transfer_queue (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            direction     INTEGER NOT NULL,
            file_id       TEXT,
            local_path    TEXT,
            name          TEXT    NOT NULL,
            total_size    INTEGER NOT NULL DEFAULT 0,
            transferred   INTEGER NOT NULL DEFAULT 0,
            state         INTEGER NOT NULL DEFAULT 0,
            error_message TEXT,
            created_at    INTEGER NOT NULL,
            finished_at   INTEGER,
            server_id     TEXT,
            upload_id     TEXT,
            resume_offset INTEGER NOT NULL DEFAULT 0,
            session_url   TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_sync_items_file_id ON sync_items(file_id);
        CREATE INDEX IF NOT EXISTS idx_sync_items_status  ON sync_items(status);
        CREATE INDEX IF NOT EXISTS idx_transfer_state     ON transfer_queue(state);
        ",
    )
    .map_err(|e| AppError::generic(format!("建表失败：{e}")))?;
    Ok(())
}

/// v2: TransferQueue 加分片续传断点字段（ALTER TABLE ADD COLUMN，幂等安全）。
fn upgrade_to_v2(conn: &Connection) -> AppResult<()> {
    add_column_if_missing(conn, "transfer_queue", "server_id", "TEXT")?;
    add_column_if_missing(conn, "transfer_queue", "upload_id", "TEXT")?;
    add_column_if_missing(
        conn,
        "transfer_queue",
        "resume_offset",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

/// v3: SyncItems 加 localSize（本地变更检测，避免 mtime 精度不足漏判）。
fn upgrade_to_v3(conn: &Connection) -> AppResult<()> {
    add_column_if_missing(conn, "sync_items", "local_size", "INTEGER")?;
    Ok(())
}

/// v4: TransferQueue 加 session_url（华为 resume 上传的 Location 头会话 URL，断点续传必需）。
fn upgrade_to_v4(conn: &Connection) -> AppResult<()> {
    add_column_if_missing(conn, "transfer_queue", "session_url", "TEXT")?;
    Ok(())
}

/// 幂等加列：列已存在时跳过（SQLite ALTER TABLE 不支持 IF NOT EXISTS）。
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> AppResult<()> {
    // 查询表结构判断列是否存在
    let exists: bool = {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| AppError::generic(format!("查询表结构失败：{e}")))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::generic(format!("读取列名失败：{e}")))?;
        let mut found = false;
        for r in rows {
            if r.map(|name| name == column).unwrap_or(false) {
                found = true;
                break;
            }
        }
        found
    };
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition};"
        ))
        .map_err(|e| AppError::generic(format!("加列失败：{e}")))?;
    }
    Ok(())
}

/// 写入 user_version。
fn set_version(conn: &Connection, version: u32) -> AppResult<()> {
    conn.execute_batch(&format!("{USER_VERSION_PRAGMA} = {version};"))
        .map_err(|e| AppError::generic(format!("写入版本失败：{e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        // 注意：tempdir() 返回的 TempDir 在 drop 时会删除目录及文件，
        // 必须用 into_path() 固化为持久路径，否则连接在写入前文件已被删除 → readonly。
        let dir = tempfile::tempdir().unwrap().keep();
        let path = dir.join("test.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    #[test]
    fn test_create_all_sets_version_4() {
        let conn = fresh_conn();
        run(&conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 4);
    }

    #[test]
    fn test_tables_exist() {
        let conn = fresh_conn();
        run(&conn).unwrap();
        // sync_items 可写入（复合主键 file_id + local_path）
        conn.execute(
            "INSERT INTO sync_items (file_id, local_path, name, is_folder, size, status) VALUES ('f1','/p','n',0,0,0)",
            [],
        )
        .unwrap();
        // transfer_queue 可写入（autoincrement id）
        conn.execute(
            "INSERT INTO transfer_queue (direction, name, created_at) VALUES (0,'t',1)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_v3_has_local_size() {
        let conn = fresh_conn();
        run(&conn).unwrap();
        // local_size 列应存在且可写
        conn.execute(
            "INSERT INTO sync_items (file_id, local_path, name, is_folder, size, local_size, status) VALUES ('f','/p','n',0,0,100,0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_upgrade_from_v1_to_v4() {
        let conn = fresh_conn();
        // 模拟 v1 表结构（无 resume 字段，无 local_size，无 session_url）
        conn.execute_batch(
            "CREATE TABLE sync_items (file_id TEXT NOT NULL, local_path TEXT NOT NULL, name TEXT NOT NULL, is_folder INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0, sha256 TEXT, local_mtime INTEGER, cloud_edited_time INTEGER, last_sync_time INTEGER, status INTEGER NOT NULL DEFAULT 0, error_message TEXT, PRIMARY KEY (file_id, local_path));
             CREATE TABLE transfer_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, direction INTEGER NOT NULL, file_id TEXT, local_path TEXT, name TEXT NOT NULL, total_size INTEGER NOT NULL DEFAULT 0, transferred INTEGER NOT NULL DEFAULT 0, state INTEGER NOT NULL DEFAULT 0, error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        run(&conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 4);
        // resume_offset 列应存在
        conn.execute(
            "INSERT INTO transfer_queue (direction, name, created_at, resume_offset) VALUES (0,'t',1,500)",
            [],
        )
        .unwrap();
        // session_url 列应存在（v4 新增）
        conn.execute(
            "INSERT INTO transfer_queue (direction, name, created_at, session_url) VALUES (0,'t2',1,'https://example/upload/session')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_migration_idempotent() {
        let conn = fresh_conn();
        run(&conn).unwrap();
        // 再跑一次不应报错
        run(&conn).unwrap();
    }
}
