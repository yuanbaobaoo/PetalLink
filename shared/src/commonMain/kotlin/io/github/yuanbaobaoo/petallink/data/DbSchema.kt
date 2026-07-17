package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.config.AppConfig

/**
 * 数据库 Schema（对标原项目 src/data/migrations.rs）
 *
 * schemaVersion = 6（inode 方案后）。完整建表 SQL 在此集中声明，
 * 平台层（macosMain 的 SQLDelight / expect Connection）执行这些语句。
 * 详见 docs/04-数据模型与持久化.md / docs/11 §3。
 */
object DbSchema {

    /**
     * 当前 schema 版本（v6 = inode 身份方案）
     */
    const val VERSION: Int = AppConfig.SCHEMA_VERSION

    // ------------------------------------------------------------------
    // 同步项基线表
    // ------------------------------------------------------------------
    const val CREATE_SYNC_ITEMS = """
        CREATE TABLE IF NOT EXISTS sync_items (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id         TEXT    NOT NULL,
            local_path      TEXT    NOT NULL,
            parent_file_id  TEXT,
            is_folder       INTEGER NOT NULL DEFAULT 0,
            size            INTEGER NOT NULL DEFAULT 0,
            mtime           INTEGER NOT NULL DEFAULT 0,
            etag            TEXT,
            sync_status     INTEGER NOT NULL DEFAULT 0,
            state_revision  INTEGER NOT NULL DEFAULT 0,
            last_error      TEXT,
            UNIQUE(file_id)
        );
        CREATE INDEX IF NOT EXISTS idx_sync_items_local_path ON sync_items(local_path);
        CREATE INDEX IF NOT EXISTS idx_sync_items_parent ON sync_items(parent_file_id);
    """

    // ------------------------------------------------------------------
    // 传输队列表（九态状态机 + CAS revision）
    // ------------------------------------------------------------------
    const val CREATE_TRANSFER_QUEUE = """
        CREATE TABLE IF NOT EXISTS transfer_queue (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id         TEXT    NOT NULL,
            local_path      TEXT    NOT NULL,
            direction       INTEGER NOT NULL,           -- 0=upload, 1=download
            state           INTEGER NOT NULL DEFAULT 0, -- TransferState 序号
            state_revision  INTEGER NOT NULL DEFAULT 0, -- CAS 乐观锁
            attempt         INTEGER NOT NULL DEFAULT 0,
            bytes_total     INTEGER NOT NULL DEFAULT 0,
            bytes_done      INTEGER NOT NULL DEFAULT 0,
            error_message   TEXT,
            upload_session_url TEXT,                    -- 断点续传 session
            created_at      INTEGER NOT NULL DEFAULT 0,
            updated_at      INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_transfer_queue_state ON transfer_queue(state);
        CREATE INDEX IF NOT EXISTS idx_transfer_queue_file_id ON transfer_queue(file_id);
    """

    // ------------------------------------------------------------------
    // 云端增量游标
    // ------------------------------------------------------------------
    const val CREATE_SYNC_CURSOR = """
        CREATE TABLE IF NOT EXISTS sync_cursor (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL
        );
    """

    // ------------------------------------------------------------------
    // inode 身份映射表（v6 新增，docs/11 §3.1）
    // 取代 fileId xattr，作为文件身份识别的核心数据结构。
    // ------------------------------------------------------------------
    const val CREATE_LOCAL_INODE_MAP = """
        CREATE TABLE IF NOT EXISTS local_inode_map (
            inode         INTEGER NOT NULL,
            relative_path TEXT    NOT NULL,
            file_id       TEXT    NOT NULL,
            scanned_at    INTEGER NOT NULL,
            PRIMARY KEY (inode)
        );
        CREATE INDEX IF NOT EXISTS idx_inode_map_path ON local_inode_map(relative_path);
        CREATE INDEX IF NOT EXISTS idx_inode_map_fid  ON local_inode_map(file_id);
    """

    // ------------------------------------------------------------------
    // 释放空间暂存表（v6 新增，docs/11 §3.2）
    // 替代 XATTR_FREE_UP_RELATIVE_PATH，恢复记录走 DB 事务。
    // ------------------------------------------------------------------
    const val CREATE_FREE_UP_STAGING = """
        CREATE TABLE IF NOT EXISTS free_up_staging (
            staging_name   TEXT    NOT NULL PRIMARY KEY,   -- 暂存文件名
            relative_path  TEXT    NOT NULL,               -- 原始相对路径
            file_id        TEXT    NOT NULL,               -- 云端文件 ID
            source_mtime   INTEGER,                        -- 回滚恢复用
            source_size    INTEGER,                        -- 回滚恢复用
            created_at     INTEGER NOT NULL
        );
    """

    /**
     * 全部建表语句（按依赖顺序）
     */
    val ALL_CREATE: List<String> = listOf(
        CREATE_SYNC_ITEMS,
        CREATE_TRANSFER_QUEUE,
        CREATE_SYNC_CURSOR,
        CREATE_LOCAL_INODE_MAP,
        CREATE_FREE_UP_STAGING,
    )
}
