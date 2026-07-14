//! SyncEngine 公开行为集成测试。

use std::sync::Arc;

use parking_lot::Mutex;
use petal_link_lib::auth::service::AuthService;
use petal_link_lib::drive::changes_api::ChangesApi;
use petal_link_lib::drive::client::DriveClient;
use petal_link_lib::drive::download_api::DownloadApi;
use petal_link_lib::drive::files_api::FilesApi;
use petal_link_lib::drive::models::DriveFile;
use petal_link_lib::drive::upload_api::UploadApi;
use petal_link_lib::sync::engine::SyncEngine;
use petal_link_lib::sync::state::{ActionResult, SyncAction, SyncActionType};
use petal_link_lib::sync::status_aggregator::StatusAggregator;
use rusqlite::{params, Connection};

/// 持久化同步成功状态值。
const SYNCED: i32 = 0;
/// 持久化同步失败状态值。
const FAILED: i32 = 4;
/// 同步基线测试表结构。
const SYNC_ITEMS_DDL: &str = "
    CREATE TABLE sync_items (
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
";

/// 用于比较同步基线全部可变字段的快照。
type BaselineSnapshot = (
    Option<String>,
    String,
    i32,
    i64,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i32,
    Option<String>,
);

/// 用公开构造器创建尚未启动的引擎，并保留调用方持有的数据库连接。
fn new_engine() -> (SyncEngine, Arc<Mutex<Connection>>) {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(SYNC_ITEMS_DDL).unwrap();
    let db = Arc::new(Mutex::new(connection));
    let auth = Arc::new(AuthService::new());
    let client = Arc::new(DriveClient::new(auth));
    let files_api = Arc::new(FilesApi::new(client.clone()));
    let changes_api = Arc::new(ChangesApi::new(client.clone()));
    let download_api = Arc::new(DownloadApi::new(client.clone()));
    let upload_api = Arc::new(UploadApi::new(client));
    let engine = SyncEngine::new(
        files_api,
        changes_api,
        download_api,
        upload_api,
        db.clone(),
        Arc::new(StatusAggregator::default()),
        Vec::new(),
        3,
        0,
    );

    (engine, db)
}

/// 插入一条可区分字段变化的同步基线。
fn insert_baseline(
    connection: &Connection,
    file_id: &str,
    local_path: &str,
    status: i32,
    error_message: Option<&str>,
) {
    connection
        .execute(
            "INSERT INTO sync_items (
                file_id, local_path, parent_folder_id, name, is_folder, size,
                local_size, sha256, local_mtime, cloud_edited_time,
                last_sync_time, status, error_message
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                file_id,
                local_path,
                "baseline-parent",
                "baseline-name",
                0,
                333_i64,
                222_i64,
                "baseline-sha",
                1_111_i64,
                2_222_i64,
                3_333_i64,
                status,
                error_message,
            ],
        )
        .unwrap();
}

/// 读取除复合主键外的全部基线字段，便于验证没有重复结算。
fn baseline_snapshot(connection: &Connection, file_id: &str, local_path: &str) -> BaselineSnapshot {
    connection
        .query_row(
            "SELECT parent_folder_id, name, is_folder, size, local_size, sha256,
                    local_mtime, cloud_edited_time, last_sync_time, status, error_message
             FROM sync_items WHERE file_id=?1 AND local_path=?2",
            params![file_id, local_path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .unwrap()
}

/// 验证引擎启动前手动同步报错且不伪造内容变化。
#[tokio::test]
async fn manual_sync_before_start_returns_error_without_false_content_change() {
    let (engine, _) = new_engine();

    let error = engine.trigger_manual_sync().await.unwrap_err();

    assert!(error.to_string().contains("正在启动"));
    assert!(!engine.current_state().content_changed);
}

/// 验证启动前批量重试失败且不改写失败基线。
#[tokio::test]
async fn bulk_retry_before_start_rejects_without_mutating_failed_sync_items() {
    let (engine, db) = new_engine();
    insert_baseline(
        &db.lock(),
        "baseline-file-id",
        "bulk/prestart.txt",
        FAILED,
        Some("old sync failure"),
    );

    let error = engine.retry_failed().await.unwrap_err();

    assert!(error.to_string().contains("正在启动"));
    let after: (i32, Option<String>) = db
        .lock()
        .query_row(
            "SELECT status, error_message FROM sync_items
             WHERE file_id=?1 AND local_path=?2",
            params!["baseline-file-id", "bulk/prestart.txt"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, (FAILED, Some("old sync failure".to_string())));
}

/// TaskRunner 已持久化上传基线；Engine 只发布执行结果中的云端缓存。
#[test]
fn apply_results_upload_preserves_task_runner_baseline_and_updates_cloud_cache() {
    let (engine, db) = new_engine();
    insert_baseline(&db.lock(), "cloud-id-1", "A/new.txt", SYNCED, None);
    let before = baseline_snapshot(&db.lock(), "cloud-id-1", "A/new.txt");
    let cloud = DriveFile {
        id: "cloud-id-1".into(),
        name: "new.txt".into(),
        size: 5,
        edited_time: chrono::DateTime::from_timestamp_millis(1_700_000_000_000),
        ..Default::default()
    };
    let action = SyncAction {
        action_type: SyncActionType::Upload,
        relative_path: Some("A/new.txt".into()),
        file_id: None,
        parent_file_id: Some("folder-A".into()),
        local_path: Some("/mount/A/new.txt".into()),
        cloud_file: None,
        reason: Some("本地新文件上传".into()),
    };
    let result = ActionResult {
        success: true,
        error_message: None,
        deferred: false,
        cloud_file: Some(cloud.clone()),
    };

    engine.apply_results(&[action], &[result]).unwrap();

    let connection = db.lock();
    let after = baseline_snapshot(&connection, "cloud-id-1", "A/new.txt");
    let row_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_items WHERE local_path=?1",
            params!["A/new.txt"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, before, "apply_results 不应重复结算上传基线");
    assert_eq!(row_count, 1, "上传结果不应新增第二条基线");
    drop(connection);

    let cached = engine
        .cloud_tree_lock()
        .get("A/new.txt")
        .cloned()
        .expect("上传结果应写入 cloud_tree");
    assert_eq!(cached.id, cloud.id);
    assert_eq!(cached.name, cloud.name);
    assert_eq!(cached.size, cloud.size);
    assert_eq!(cached.edited_time, cloud.edited_time);
    assert_eq!(
        engine.path_to_id_lock().get("A/new.txt").cloned(),
        Some("cloud-id-1".to_string())
    );
}

/// 云端删除成功后，同一路径的持久基线和缓存都应被清理。
#[test]
fn test_apply_results_delete_from_cloud_clears_state() {
    let (engine, db) = new_engine();
    insert_baseline(&db.lock(), "c-old", "old.txt", SYNCED, None);
    engine.cloud_tree_insert(
        "old.txt".into(),
        DriveFile {
            id: "c-old".into(),
            name: "old.txt".into(),
            ..Default::default()
        },
    );
    engine.path_to_id_insert("old.txt".into(), "c-old".into());
    let action = SyncAction {
        action_type: SyncActionType::DeleteFromCloud,
        relative_path: Some("old.txt".into()),
        file_id: Some("c-old".into()),
        parent_file_id: None,
        local_path: None,
        cloud_file: None,
        reason: Some("会话内删除".into()),
    };
    let result = ActionResult {
        success: true,
        error_message: None,
        deferred: false,
        cloud_file: None,
    };

    engine.apply_results(&[action], &[result]).unwrap();

    assert!(!engine.cloud_tree_lock().contains_key("old.txt"));
    assert!(!engine.path_to_id_lock().contains_key("old.txt"));
    let row_count: i64 = db
        .lock()
        .query_row(
            "SELECT COUNT(*) FROM sync_items WHERE local_path=?1",
            params!["old.txt"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(row_count, 0);
}

/// 验证失败动作只更新复合身份完全匹配的基线。
#[test]
fn failed_action_updates_only_exact_baseline_identity() {
    let (engine, db) = new_engine();
    {
        let connection = db.lock();
        insert_baseline(&connection, "file-a", "same/path.txt", SYNCED, None);
        insert_baseline(&connection, "file-b", "same/path.txt", SYNCED, None);
    }
    let action = SyncAction {
        action_type: SyncActionType::Upload,
        relative_path: Some("same/path.txt".into()),
        file_id: Some("file-a".into()),
        parent_file_id: Some("parent".into()),
        local_path: Some("/mount/same/path.txt".into()),
        cloud_file: None,
        reason: None,
    };
    let result = ActionResult {
        success: false,
        error_message: Some("failed".into()),
        deferred: false,
        cloud_file: None,
    };

    engine.apply_results(&[action], &[result]).unwrap();

    let connection = db.lock();
    let status_a: (i32, Option<String>) = connection
        .query_row(
            "SELECT status, error_message FROM sync_items
             WHERE file_id=?1 AND local_path=?2",
            params!["file-a", "same/path.txt"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let status_b: (i32, Option<String>) = connection
        .query_row(
            "SELECT status, error_message FROM sync_items
             WHERE file_id=?1 AND local_path=?2",
            params!["file-b", "same/path.txt"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status_a, (FAILED, Some("failed".to_string())));
    assert_eq!(status_b, (SYNCED, None));
}
