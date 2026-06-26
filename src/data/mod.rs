//! SQLite 数据层 —— 同步状态表 + 传输队列表。
//!
//! 对齐 `legacy/lib/data/`（database.dart + tables/sync_items.dart + tables/transfer_queue.dart）。
//!
//! 使用 rusqlite（bundled），schemaVersion=3，启用外键约束。
//! DB 文件：`<Application Support>/io.github.yuanbaobaoo.PetalLink/petal_link.db`。

pub mod migrations;
pub mod repository;

use std::path::PathBuf;

use rusqlite::Connection;

use crate::core::config_store::support_dir;
use crate::error::{AppError, AppResult};

/// 当前 schema 版本（对齐 dart `schemaVersion = 3`）
pub const SCHEMA_VERSION: u32 = 3;

/// DB 文件名
pub const DB_FILE_NAME: &str = "petal_link.db";

/// 打开数据库连接（运行迁移 + 启用外键）。
/// 对齐 dart `AppDatabase`：`PRAGMA foreign_keys = ON` + 迁移策略。
pub fn open() -> AppResult<Connection> {
    open_at(&db_file_path()?)
}

/// 在指定路径打开数据库（测试用，可指向临时文件）。
pub fn open_at(path: &PathBuf) -> AppResult<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(path)
        .map_err(|e| AppError::generic(format!("打开数据库失败：{e}")))?;

    // 启用外键约束（SQLite 默认关闭，对齐 dart beforeOpen）
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| AppError::generic(format!("启用外键约束失败：{e}")))?;

    migrations::run(&conn)?;
    Ok(conn)
}

/// DB 文件完整路径
pub fn db_file_path() -> AppResult<PathBuf> {
    Ok(support_dir()?.join(DB_FILE_NAME))
}
