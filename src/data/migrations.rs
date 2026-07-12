//! 数据库迁移 —— schemaVersion=5。
//!
//! 对齐 dart `MigrationStrategy`：
//! - v1 onCreate: 建全部表
//! - v2 onUpgrade from<2: TransferQueue 加 serverId/uploadId/resumeOffset（分片续传断点）
//! - v3 onUpgrade from<3: SyncItems 加 localSize（本地变更检测）
//! - v4 onUpgrade from<4: TransferQueue 加 session_url（华为 resume 上传 Location 头会话 URL）
//! - v5 onUpgrade from<5: TransferQueue 加任务状态机上下文、revision 与重试索引
//! - beforeOpen: PRAGMA foreign_keys = ON（已在 open 中处理）

use rusqlite::{params, Connection};

use crate::data::SCHEMA_VERSION;
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

/// 用户版本 PRAGMA key
const USER_VERSION_PRAGMA: &str = "PRAGMA user_version";

/// 运行迁移。读取当前 user_version，按需建表/升级。
pub fn run(conn: &Connection) -> AppResult<()> {
    let current: u32 = conn
        .query_row(USER_VERSION_PRAGMA, [], |row| row.get::<_, i64>(0))
        .map(|v| v as u32)
        .unwrap_or(0);

    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = conn
        .unchecked_transaction()
        .map_err(|e| AppError::generic(format!("开始数据库迁移事务失败：{e}")))?;

    if current == 0 {
        // 全新数据库：直接建 v5 终态，避免先建旧结构再 ALTER。
        create_all(&transaction)?;
    } else {
        // 旧数据库逐步升级，全部步骤与 user_version 写入同属一个事务。
        if current < 2 {
            upgrade_to_v2(&transaction)?;
        }
        if current < 3 {
            upgrade_to_v3(&transaction)?;
        }
        if current < 4 {
            upgrade_to_v4(&transaction)?;
        }
        if current < 5 {
            upgrade_to_v5(&transaction)?;
        }
    }

    set_version(&transaction, SCHEMA_VERSION)?;
    transaction
        .commit()
        .map_err(|e| AppError::generic(format!("提交数据库迁移事务失败：{e}")))?;
    Ok(())
}

/// 新库直接创建为 v5 终态结构。
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
            session_url   TEXT,
            relative_path TEXT,
            parent_file_id TEXT,
            operation INTEGER,
            source_mtime INTEGER,
            source_size INTEGER,
            expected_cloud_edited_time INTEGER,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_retry_at INTEGER,
            error_kind INTEGER,
            remote_result_file_id TEXT,
            state_revision INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_sync_items_file_id ON sync_items(file_id);
        CREATE INDEX IF NOT EXISTS idx_sync_items_status  ON sync_items(status);
        CREATE INDEX IF NOT EXISTS idx_transfer_state     ON transfer_queue(state);
        CREATE INDEX IF NOT EXISTS idx_transfer_state_retry
            ON transfer_queue(state, next_retry_at);
        CREATE INDEX IF NOT EXISTS idx_transfer_relative_state
            ON transfer_queue(relative_path, state);
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

/// v5: add persistent task context and normalize legacy lifecycle values.
fn upgrade_to_v5(conn: &Connection) -> AppResult<()> {
    add_column_if_missing(conn, "transfer_queue", "relative_path", "TEXT")?;
    add_column_if_missing(conn, "transfer_queue", "parent_file_id", "TEXT")?;
    add_column_if_missing(conn, "transfer_queue", "operation", "INTEGER")?;
    add_column_if_missing(conn, "transfer_queue", "source_mtime", "INTEGER")?;
    add_column_if_missing(conn, "transfer_queue", "source_size", "INTEGER")?;
    add_column_if_missing(
        conn,
        "transfer_queue",
        "expected_cloud_edited_time",
        "INTEGER",
    )?;
    add_column_if_missing(
        conn,
        "transfer_queue",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(conn, "transfer_queue", "next_retry_at", "INTEGER")?;
    add_column_if_missing(conn, "transfer_queue", "error_kind", "INTEGER")?;
    add_column_if_missing(conn, "transfer_queue", "remote_result_file_id", "TEXT")?;
    add_column_if_missing(
        conn,
        "transfer_queue",
        "state_revision",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    // Legacy FAILED=4 has no structured classification in schemas v1-v4.
    conn.execute(
        "UPDATE transfer_queue SET error_kind=?1 WHERE state=4 AND error_kind IS NULL",
        params![i32::from(TransferErrorKind::Unknown)],
    )
    .map_err(|e| AppError::generic(format!("归一化旧传输错误类型失败：{e}")))?;
    // Legacy PENDING/RUNNING/PAUSED restart conservatively from Pending. Terminal
    // history retains its semantic meaning under the new numeric representation.
    conn.execute(
        "UPDATE transfer_queue
         SET state = CASE state
            WHEN 0 THEN ?1
            WHEN 1 THEN ?1
            WHEN 2 THEN ?1
            WHEN 3 THEN ?2
            WHEN 4 THEN ?3
            WHEN 5 THEN ?4
            ELSE state
         END",
        params![
            i32::from(TransferState::Pending),
            i32::from(TransferState::Completed),
            i32::from(TransferState::Failed),
            i32::from(TransferState::Canceled),
        ],
    )
    .map_err(|e| AppError::generic(format!("归一化旧传输状态失败：{e}")))?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_transfer_state_retry
            ON transfer_queue(state, next_retry_at);
         CREATE INDEX IF NOT EXISTS idx_transfer_relative_state
            ON transfer_queue(relative_path, state);",
    )
    .map_err(|e| AppError::generic(format!("创建 v5 传输索引失败：{e}")))?;
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

    const V5_COLUMNS: [&str; 11] = [
        "relative_path",
        "parent_file_id",
        "operation",
        "source_mtime",
        "source_size",
        "expected_cloud_edited_time",
        "attempt_count",
        "next_retry_at",
        "error_kind",
        "remote_result_file_id",
        "state_revision",
    ];

    fn fresh_conn() -> Connection {
        // 注意：tempdir() 返回的 TempDir 在 drop 时会删除目录及文件，
        // 必须用 into_path() 固化为持久路径，否则连接在写入前文件已被删除 → readonly。
        let dir = tempfile::tempdir().unwrap().keep();
        let path = dir.join("test.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    fn create_legacy_schema(conn: &Connection, version: u32) {
        assert!((1..=4).contains(&version));
        conn.execute_batch(
            "CREATE TABLE sync_items (
                file_id TEXT NOT NULL,
                local_path TEXT NOT NULL,
                parent_folder_id TEXT,
                name TEXT NOT NULL,
                is_folder INTEGER NOT NULL DEFAULT 0,
                size INTEGER NOT NULL DEFAULT 0,
                sha256 TEXT,
                local_mtime INTEGER,
                cloud_edited_time INTEGER,
                last_sync_time INTEGER,
                status INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                PRIMARY KEY (file_id, local_path)
             );
             CREATE TABLE transfer_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction INTEGER NOT NULL,
                file_id TEXT,
                local_path TEXT,
                name TEXT NOT NULL,
                total_size INTEGER NOT NULL DEFAULT 0,
                transferred INTEGER NOT NULL DEFAULT 0,
                state INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at INTEGER NOT NULL,
                finished_at INTEGER
             );",
        )
        .unwrap();
        if version >= 2 {
            conn.execute_batch(
                "ALTER TABLE transfer_queue ADD COLUMN server_id TEXT;
                 ALTER TABLE transfer_queue ADD COLUMN upload_id TEXT;
                 ALTER TABLE transfer_queue ADD COLUMN resume_offset INTEGER NOT NULL DEFAULT 0;",
            )
            .unwrap();
        }
        if version >= 3 {
            conn.execute_batch("ALTER TABLE sync_items ADD COLUMN local_size INTEGER;")
                .unwrap();
        }
        if version >= 4 {
            conn.execute_batch("ALTER TABLE transfer_queue ADD COLUMN session_url TEXT;")
                .unwrap();
        }
        conn.pragma_update(None, "user_version", version).unwrap();
    }

    fn transfer_columns(conn: &Connection) -> Vec<(String, i64, Option<String>)> {
        let mut stmt = conn.prepare("PRAGMA table_info(transfer_queue)").unwrap();
        stmt.query_map([], |row| Ok((row.get(1)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn normalized_transfer_index_definitions(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT sql FROM sqlite_master
                 WHERE type='index' AND tbl_name='transfer_queue' AND sql IS NOT NULL",
            )
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .into_iter()
            .map(|sql| {
                sql.to_ascii_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn fresh_database_is_created_directly_at_v5() {
        let conn = fresh_conn();
        run(&conn).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 5);

        let columns = transfer_columns(&conn);
        for expected in V5_COLUMNS {
            assert!(
                columns.iter().any(|(name, _, _)| name == expected),
                "missing v5 column {expected}"
            );
        }
        assert!(columns.iter().any(|(name, not_null, default)| {
            name == "attempt_count" && *not_null == 1 && default.as_deref() == Some("0")
        }));
        assert!(columns.iter().any(|(name, not_null, default)| {
            name == "state_revision" && *not_null == 1 && default.as_deref() == Some("0")
        }));

        let indexes = normalized_transfer_index_definitions(&conn);
        assert!(indexes
            .iter()
            .any(|sql| sql.contains("(state,next_retry_at)")));
        assert!(indexes
            .iter()
            .any(|sql| sql.contains("(relative_path,state)")));
    }

    #[test]
    fn every_legacy_version_migrates_in_place_to_v5() {
        for version in 1..=4 {
            let conn = fresh_conn();
            create_legacy_schema(&conn, version);
            conn.execute(
                "INSERT INTO transfer_queue
                 (direction, file_id, local_path, name, state, created_at)
                 VALUES (0, 'file-1', '/mount/legacy.txt', 'legacy.txt', 0, 123)",
                [],
            )
            .unwrap();

            run(&conn).unwrap();

            let migrated_version: i64 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            let row_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
                .unwrap();
            assert_eq!(migrated_version, 5, "failed to migrate v{version}");
            assert_eq!(row_count, 1, "lost transfer row from v{version}");
            for expected in V5_COLUMNS {
                assert!(
                    transfer_columns(&conn)
                        .iter()
                        .any(|(name, _, _)| name == expected),
                    "v{version} missing v5 column {expected}"
                );
            }
        }
    }

    #[test]
    fn v4_to_v5_preserves_sync_baseline_and_transfer_history() {
        let conn = fresh_conn();
        create_legacy_schema(&conn, 4);
        conn.execute(
            "INSERT INTO sync_items
             (file_id, local_path, parent_folder_id, name, size, local_size, local_mtime,
              cloud_edited_time, last_sync_time, status)
             VALUES ('cloud-1', 'folder/file.txt', 'parent-1', 'file.txt', 42, 41, 100,
                     200, 300, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transfer_queue
             (id, direction, file_id, local_path, name, total_size, transferred, state,
              error_message, created_at, finished_at, server_id, upload_id, resume_offset,
              session_url)
             VALUES (91, 0, 'cloud-1', '/mount/folder/file.txt', 'folder/file.txt',
                     42, 21, 3, NULL, 111, 222, 'server', 'upload', 21, 'https://session')",
            [],
        )
        .unwrap();

        run(&conn).unwrap();

        let sync_row = conn
            .query_row(
                "SELECT file_id, local_path, parent_folder_id, size, local_size,
                        local_mtime, cloud_edited_time, last_sync_time
                 FROM sync_items WHERE file_id='cloud-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            sync_row,
            (
                "cloud-1".to_string(),
                "folder/file.txt".to_string(),
                Some("parent-1".to_string()),
                42,
                Some(41),
                Some(100),
                Some(200),
                Some(300),
            )
        );

        let transfer_row = conn
            .query_row(
                "SELECT id, local_path, name, total_size, transferred, state, resume_offset,
                        session_url, state_revision
                 FROM transfer_queue WHERE id=91",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i32>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            transfer_row,
            (
                91,
                Some("/mount/folder/file.txt".to_string()),
                "folder/file.txt".to_string(),
                42,
                21,
                6,
                21,
                Some("https://session".to_string()),
                0,
            )
        );
    }

    #[test]
    fn legacy_transfer_states_are_normalized_conservatively() {
        let conn = fresh_conn();
        create_legacy_schema(&conn, 4);
        for state in 0..=5 {
            conn.execute(
                "INSERT INTO transfer_queue (direction, name, state, created_at)
                 VALUES (0, ?1, ?2, ?3)",
                rusqlite::params![format!("state-{state}"), state, state],
            )
            .unwrap();
        }

        run(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT state, error_kind FROM transfer_queue ORDER BY created_at")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i32>(0)?, row.get::<_, Option<i32>>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (0, None),
                (0, None),
                (0, None),
                (6, None),
                (7, Some(11)),
                (8, None),
            ]
        );
    }

    #[test]
    fn migration_is_idempotent() {
        let conn = fresh_conn();
        create_legacy_schema(&conn, 1);
        conn.execute(
            "INSERT INTO transfer_queue (direction, name, state, created_at)
             VALUES (0, 'failed', 4, 1)",
            [],
        )
        .unwrap();

        run(&conn).unwrap();
        let columns_after_first = transfer_columns(&conn);
        run(&conn).unwrap();
        let columns_after_second = transfer_columns(&conn);

        assert_eq!(columns_after_second, columns_after_first);
        let row: (i32, Option<i32>, i64) = conn
            .query_row(
                "SELECT state, error_kind, state_revision FROM transfer_queue",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, (7, Some(11), 0));
    }

    #[test]
    fn failed_migration_rolls_back_all_v5_changes() {
        let conn = fresh_conn();
        conn.execute_batch(
            "CREATE TABLE sync_items (
                file_id TEXT NOT NULL,
                local_path TEXT NOT NULL,
                name TEXT NOT NULL,
                is_folder INTEGER NOT NULL DEFAULT 0,
                size INTEGER NOT NULL DEFAULT 0,
                status INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (file_id, local_path)
             );
             CREATE TABLE transfer_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction INTEGER NOT NULL,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );
             PRAGMA user_version = 4;",
        )
        .unwrap();

        assert!(run(&conn).is_err());

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
        let columns = transfer_columns(&conn);
        for v5_column in V5_COLUMNS {
            assert!(
                !columns.iter().any(|(name, _, _)| name == v5_column),
                "transaction leaked column {v5_column}"
            );
        }
    }
}
