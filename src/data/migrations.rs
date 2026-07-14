//! 数据库迁移 —— schemaVersion=5。
//!
//! 对齐 dart `MigrationStrategy`：
//! - v1 onCreate: 建全部表
//! - v2 onUpgrade from<2: TransferQueue 加 serverId/uploadId/resumeOffset（分片续传断点）
//! - v3 onUpgrade from<3: SyncItems 加 localSize（本地变更检测）
//! - v4 onUpgrade from<4: TransferQueue 加 session_url（华为 resume 上传 Location 头会话 URL）
//! - v5 onUpgrade from<5: TransferQueue 加任务状态机上下文、revision 与重试索引
//! - beforeOpen: PRAGMA foreign_keys = ON（已在 open 中处理）

use std::path::Path;

use rusqlite::{params, Connection};

use crate::data::SCHEMA_VERSION;
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferState};

/// 用户版本 PRAGMA key
const USER_VERSION_PRAGMA: &str = "PRAGMA user_version";

/// 运行迁移。读取当前 user_version，按需建表/升级。
#[allow(dead_code)]
pub fn run(conn: &Connection) -> AppResult<()> {
    run_with_mount(conn, None)
}

/// 使用可选的可信挂载根运行迁移，并据此安全恢复旧任务路径。
pub fn run_with_mount(conn: &Connection, mount_root: Option<&Path>) -> AppResult<()> {
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
            upgrade_to_v5(&transaction, mount_root)?;
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

/// v5：补充持久化任务上下文并归一化旧生命周期值。
fn upgrade_to_v5(conn: &Connection, mount_root: Option<&Path>) -> AppResult<()> {
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

    recover_legacy_relative_paths(conn, mount_root)?;

    // v1-v4 的旧 FAILED=4 没有结构化错误分类。
    conn.execute(
        "UPDATE transfer_queue SET error_kind=?1 WHERE state=4 AND error_kind IS NULL",
        params![i32::from(TransferErrorKind::Unknown)],
    )
    .map_err(|e| AppError::generic(format!("归一化旧传输错误类型失败：{e}")))?;
    // 旧 PENDING/RUNNING/PAUSED 保守地从 Pending 重启；终态历史在新数值表示中保留原语义。
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

/// 回填旧任务的相对路径，无法安全恢复的活动任务标记为验证失败。
fn recover_legacy_relative_paths(conn: &Connection, mount_root: Option<&Path>) -> AppResult<()> {
    let legacy_tasks = {
        let mut stmt = conn
            .prepare("SELECT id, state, local_path FROM transfer_queue")
            .map_err(|e| AppError::generic(format!("读取旧传输任务失败：{e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| AppError::generic(format!("读取旧传输任务失败：{e}")))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| AppError::generic(format!("解析旧传输任务失败：{e}")))?
    };

    for (task_id, legacy_state, local_path) in legacy_tasks {
        match derive_legacy_relative_path(mount_root, local_path.as_deref()) {
            Ok(relative_path) => {
                conn.execute(
                    "UPDATE transfer_queue SET relative_path=?1 WHERE id=?2",
                    params![relative_path, task_id],
                )
                .map_err(|e| AppError::generic(format!("回填旧传输相对路径失败：{e}")))?;
            }
            Err(reason) if matches!(legacy_state, 0..=2) => {
                let error_message = format!("旧传输任务无法安全恢复：{reason}");
                conn.execute(
                    "UPDATE transfer_queue
                     SET state=?1, error_kind=?2, error_message=?3
                     WHERE id=?4",
                    params![
                        i32::from(TransferState::Failed),
                        i32::from(TransferErrorKind::Validation),
                        error_message,
                        task_id,
                    ],
                )
                .map_err(|e| AppError::generic(format!("标记旧传输任务验证失败：{e}")))?;
            }
            Err(_) => {}
        }
    }
    Ok(())
}

/// 仅从可信挂载根内的绝对路径推导规范相对路径。
fn derive_legacy_relative_path(
    mount_root: Option<&Path>,
    local_path: Option<&str>,
) -> Result<String, String> {
    let mount_root = mount_root.ok_or_else(|| "未配置同步目录".to_string())?;
    if !mount_root.is_absolute() {
        return Err(format!("同步目录不是绝对路径：{}", mount_root.display()));
    }
    let local_path = local_path
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "缺少本地路径".to_string())?;
    let candidate = Path::new(local_path);
    if !candidate.is_absolute() {
        return Err(format!("本地路径不是绝对路径：{local_path}"));
    }
    let relative = candidate
        .strip_prefix(mount_root)
        .map_err(|_| format!("本地路径不在同步目录内：{local_path}"))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| format!("相对路径不是 UTF-8：{}", relative.display()))?;
    crate::core::paths::validate_relative_path(relative, false)
        .map_err(|error| format!("相对路径校验失败：{error}"))?;
    Ok(relative.to_string())
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

/// 更新 SQLite 结构版本；执行失败时返回数据库错误。
fn set_version(conn: &Connection, version: u32) -> AppResult<()> {
    conn.execute_batch(&format!("{USER_VERSION_PRAGMA} = {version};"))
        .map_err(|e| AppError::generic(format!("写入版本失败：{e}")))?;
    Ok(())
}
