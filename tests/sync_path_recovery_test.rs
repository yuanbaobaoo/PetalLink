//! 远端路径恢复与持久传输状态协作的集成测试。

#![cfg(target_os = "macos")]

use std::collections::HashMap;

use petal_link_lib::drive::models::DriveFile;
use petal_link_lib::error::AppResult;
use petal_link_lib::sync::path_recovery::recover_verified_remote_path_changes;
use petal_link_lib::sync::transfer_state::{TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试使用的稳定远端身份。
const FILE_ID: &str = "rename-file-id";
/// 数据库中的旧相对路径。
const OLD_PATH: &str = "contracts/old.docx";
/// 云端与本地已经采用的新相对路径。
const NEW_PATH: &str = "contracts/new.docx";
/// 本地持久化远端身份使用的扩展属性。
const XATTR_FILE_ID: &str = "com.hwcloud.fileId";

/// 创建路径恢复所需的最小数据库结构。
fn open_database(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE sync_items (
                file_id TEXT NOT NULL,
                local_path TEXT NOT NULL,
                parent_folder_id TEXT,
                name TEXT NOT NULL,
                is_folder INTEGER NOT NULL DEFAULT 0,
                size INTEGER NOT NULL DEFAULT 0,
                local_size INTEGER,
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
                finished_at INTEGER,
                server_id TEXT,
                upload_id TEXT,
                resume_offset INTEGER NOT NULL DEFAULT 0,
                session_url TEXT,
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
            ",
        )
        .unwrap();
    connection
}

/// 插入远端路径变化前的内容基线。
fn insert_old_baseline(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO sync_items (
                file_id, local_path, parent_folder_id, name, is_folder, size,
                local_size, sha256, local_mtime, cloud_edited_time,
                last_sync_time, status, error_message
             ) VALUES (?1, ?2, ?3, ?4, 0, 4, 4, ?5, 1000, 2000, 3000, 0, NULL)",
            params![
                FILE_ID,
                OLD_PATH,
                "contracts-folder-id",
                "old.docx",
                "baseline-sha"
            ],
        )
        .unwrap();
}

/// 插入指定状态的路径相关传输任务。
fn insert_path_task(connection: &Connection, state: TransferState) {
    connection
        .execute(
            "INSERT INTO transfer_queue (
                direction, file_id, local_path, name, total_size, state, created_at,
                relative_path, parent_file_id, operation, source_mtime, source_size,
                expected_cloud_edited_time
             ) VALUES (0, ?1, ?2, ?3, 4, ?4, 1, ?2, ?5, ?6, 1000, 4, 2000)",
            params![
                FILE_ID,
                NEW_PATH,
                "new.docx",
                i32::from(state),
                "contracts-folder-id",
                i32::from(TransferOperation::Update)
            ],
        )
        .unwrap();
}

/// 返回指定 fileId 当前保存的唯一相对路径。
fn load_baseline_path(connection: &Connection) -> String {
    connection
        .query_row(
            "SELECT local_path FROM sync_items WHERE file_id=?1",
            [FILE_ID],
            |row| row.get(0),
        )
        .unwrap()
}

/// 构造只在新路径出现同一 fileId 的可信云端树。
fn renamed_cloud_tree() -> HashMap<String, DriveFile> {
    HashMap::from([(
        NEW_PATH.to_string(),
        DriveFile {
            id: FILE_ID.to_string(),
            name: "new.docx".to_string(),
            size: 4,
            parent_folder: Some(vec!["contracts-folder-id".to_string()]),
            edited_time: chrono::DateTime::from_timestamp_millis(4_000),
            ..Default::default()
        },
    )])
}

/// VerifyingRemote 仍可能改写远端，路径恢复必须隔离该身份。
#[test]
fn verifying_remote_blocks_path_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("state.db");
    let connection = open_database(&database_path);
    insert_old_baseline(&connection);
    insert_path_task(&connection, TransferState::VerifyingRemote);

    let summary = recover_verified_remote_path_changes(
        temp.path(),
        &connection,
        &renamed_cloud_tree(),
        |_, _| Ok::<(), petal_link_lib::error::AppError>(()),
    )
    .unwrap();

    assert_eq!(summary.rekeyed_roots, 0);
    assert_eq!(summary.blocked_changes.len(), 1);
    assert_eq!(summary.blocked_changes[0].old_path, OLD_PATH);
    assert_eq!(summary.blocked_changes[0].new_path, NEW_PATH);
    assert_eq!(load_baseline_path(&connection), OLD_PATH);
}

/// 无远端结果的 RestartRequired 必须让出路径恢复入口，并取消已经失效的旧意图。
#[test]
fn restart_required_does_not_permanently_block_path_recovery() -> AppResult<()> {
    let temp = tempfile::tempdir()?;
    let database_path = temp.path().join("state.db");
    let connection = open_database(&database_path);
    insert_old_baseline(&connection);
    insert_path_task(&connection, TransferState::RestartRequired);

    let target = temp.path().join(NEW_PATH);
    std::fs::create_dir_all(target.parent().unwrap())?;
    std::fs::write(&target, b"data")?;
    xattr::set(&target, XATTR_FILE_ID, FILE_ID.as_bytes())?;

    let summary = recover_verified_remote_path_changes(
        temp.path(),
        &connection,
        &renamed_cloud_tree(),
        |_, _| Ok::<(), petal_link_lib::error::AppError>(()),
    )?;

    assert_eq!(summary.rekeyed_roots, 1);
    assert!(summary.blocked_changes.is_empty());
    assert_eq!(load_baseline_path(&connection), NEW_PATH);
    let preserved: (Option<String>, Option<i64>) = connection
        .query_row(
            "SELECT sha256, local_mtime FROM sync_items WHERE file_id=?1",
            [FILE_ID],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(preserved, (Some("baseline-sha".to_string()), Some(1_000)));
    let task_state: i32 = connection
        .query_row("SELECT state FROM transfer_queue", [], |row| row.get(0))
        .unwrap();
    assert_eq!(task_state, i32::from(TransferState::Canceled));
    Ok(())
}

/// 保存了远端结果的 RestartRequired 仍有重复写风险，不能直接重键。
#[test]
fn ambiguous_restart_required_still_blocks_path_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("state.db");
    let connection = open_database(&database_path);
    insert_old_baseline(&connection);
    insert_path_task(&connection, TransferState::RestartRequired);
    connection
        .execute(
            "UPDATE transfer_queue SET remote_result_file_id=?1",
            [FILE_ID],
        )
        .unwrap();

    let summary = recover_verified_remote_path_changes(
        temp.path(),
        &connection,
        &renamed_cloud_tree(),
        |_, _| Ok::<(), petal_link_lib::error::AppError>(()),
    )
    .unwrap();

    assert_eq!(summary.rekeyed_roots, 0);
    assert_eq!(summary.blocked_changes.len(), 1);
    assert_eq!(load_baseline_path(&connection), OLD_PATH);
}

/// 单个路径冲突只能隔离自身，不能阻止其他独立路径完成恢复。
#[test]
fn path_conflict_does_not_block_independent_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database_path = temp.path().join("state.db");
    let connection = open_database(&database_path);
    insert_old_baseline(&connection);

    let second_file_id = "independent-file-id";
    let second_old_path = "reports/old.pdf";
    let second_new_path = "reports/new.pdf";
    connection.execute(
        "INSERT INTO sync_items (
            file_id, local_path, parent_folder_id, name, is_folder, size,
            local_size, sha256, local_mtime, cloud_edited_time,
            last_sync_time, status, error_message
         ) VALUES (?1, ?2, ?3, ?4, 0, 4, 4, ?5, 1000, 2000, 3000, 0, NULL)",
        params![
            second_file_id,
            second_old_path,
            "reports-folder-id",
            "old.pdf",
            "second-baseline-sha"
        ],
    )?;

    let first_source = temp.path().join(OLD_PATH);
    let first_target = temp.path().join(NEW_PATH);
    std::fs::create_dir_all(first_source.parent().unwrap())?;
    std::fs::write(&first_source, b"old")?;
    std::fs::write(&first_target, b"conflict")?;

    let second_target = temp.path().join(second_new_path);
    std::fs::create_dir_all(second_target.parent().unwrap())?;
    std::fs::write(&second_target, b"data")?;
    xattr::set(&second_target, XATTR_FILE_ID, second_file_id.as_bytes())?;

    let mut cloud_tree = renamed_cloud_tree();
    cloud_tree.insert(
        second_new_path.to_string(),
        DriveFile {
            id: second_file_id.to_string(),
            name: "new.pdf".to_string(),
            size: 4,
            parent_folder: Some(vec!["reports-folder-id".to_string()]),
            edited_time: chrono::DateTime::from_timestamp_millis(5_000),
            ..Default::default()
        },
    );

    let summary =
        recover_verified_remote_path_changes(temp.path(), &connection, &cloud_tree, |_, _| {
            Ok::<(), petal_link_lib::error::AppError>(())
        })?;

    assert_eq!(summary.rekeyed_roots, 1);
    assert_eq!(summary.blocked_changes.len(), 1);
    assert_eq!(summary.blocked_changes[0].file_id, FILE_ID);
    assert_eq!(load_baseline_path(&connection), OLD_PATH);
    let recovered_path: String = connection.query_row(
        "SELECT local_path FROM sync_items WHERE file_id=?1",
        [second_file_id],
        |row| row.get(0),
    )?;
    assert_eq!(recovered_path, second_new_path);
    Ok(())
}
