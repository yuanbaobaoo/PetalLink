//! SyncEngine 公开行为集成测试。

use std::sync::Arc;

use parking_lot::Mutex;
use petal_link_lib::auth::service::AuthService;
use petal_link_lib::drive::changes_api::ChangesApi;
use petal_link_lib::drive::client::DriveClient;
use petal_link_lib::drive::files_api::FilesApi;
use petal_link_lib::drive::models::{DriveFile, FileCategory};
use petal_link_lib::sync::engine::SyncEngine;
use petal_link_lib::sync::state::{ActionResult, SyncAction, SyncActionType};
use petal_link_lib::sync::status_aggregator::StatusAggregator;
use rusqlite::{params, Connection};

/// 持久化同步成功状态值。
const SYNCED: i32 = 0;
/// 持久化同步失败状态值。
const FAILED: i32 = 4;
/// 持久化同步冲突状态值。
const CONFLICT: i32 = 5;
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
    let changes_api = Arc::new(ChangesApi::new(client));
    let engine = SyncEngine::new(
        files_api,
        changes_api,
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

/// 插入带可区分内容字段的目录基线。
fn insert_folder_baseline(
    connection: &Connection,
    file_id: &str,
    local_path: &str,
    parent_folder_id: &str,
) {
    connection
        .execute(
            "INSERT INTO sync_items (
                file_id, local_path, parent_folder_id, name, is_folder, size,
                local_size, sha256, local_mtime, cloud_edited_time,
                last_sync_time, status, error_message
             ) VALUES (?1, ?2, ?3, ?4, 1, 17, 0, ?5, 1111, 2222, 3333, ?6, ?7)",
            params![
                file_id,
                local_path,
                parent_folder_id,
                local_path.rsplit('/').next().unwrap_or(local_path),
                format!("folder-baseline-{file_id}"),
                SYNCED,
                format!("preserved-{file_id}"),
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

/// 本地优先冲突覆盖后必须用 PATCH 返回的新版本结算，不能沿用动作中的旧云快照。
#[test]
fn apply_results_local_wins_conflict_uses_updated_cloud_version() {
    let (engine, db) = new_engine();
    insert_baseline(
        &db.lock(),
        "conflict-file-id",
        "docs/report.txt",
        SYNCED,
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let local_path = temp.path().join("report.txt");
    std::fs::write(&local_path, b"new-local-version").unwrap();
    let old_cloud = DriveFile {
        id: "conflict-file-id".into(),
        name: "report.txt".into(),
        size: 333,
        parent_folder: Some(vec!["old-parent".into()]),
        edited_time: chrono::DateTime::from_timestamp_millis(2_222),
        content_hash: Some("old-cloud-hash".into()),
        ..Default::default()
    };
    let updated_cloud = DriveFile {
        id: "conflict-file-id".into(),
        name: "report.txt".into(),
        size: b"new-local-version".len() as i64,
        parent_folder: Some(vec!["updated-parent".into()]),
        edited_time: chrono::DateTime::from_timestamp_millis(9_999),
        content_hash: Some("updated-cloud-hash".into()),
        ..Default::default()
    };
    let action = SyncAction {
        action_type: SyncActionType::CreateConflictCopy,
        relative_path: Some("docs/report.txt".into()),
        file_id: Some("conflict-file-id".into()),
        parent_file_id: Some("old-parent".into()),
        local_path: Some(local_path.to_string_lossy().into_owned()),
        cloud_file: Some(old_cloud),
        reason: Some("本地优先冲突覆盖".into()),
    };
    let result = ActionResult {
        success: true,
        error_message: None,
        deferred: false,
        cloud_file: Some(updated_cloud.clone()),
    };

    engine.apply_results(&[action], &[result]).unwrap();

    let (parent, size, edited_time, status): (Option<String>, i64, Option<i64>, i32) = db
        .lock()
        .query_row(
            "SELECT parent_folder_id, size, cloud_edited_time, status
             FROM sync_items WHERE file_id=?1 AND local_path=?2",
            params!["conflict-file-id", "docs/report.txt"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(parent.as_deref(), Some("updated-parent"));
    assert_eq!(size, updated_cloud.size);
    assert_eq!(edited_time, Some(9_999));
    assert_eq!(status, CONFLICT);
    let cached = engine
        .cloud_tree_lock()
        .get("docs/report.txt")
        .cloned()
        .unwrap();
    assert_eq!(cached.content_hash.as_deref(), Some("updated-cloud-hash"));
    assert_eq!(cached.edited_time, updated_cloud.edited_time);
}

/// 验证同目录改名按结构变化结算，并保留最后确认的内容基线。
#[test]
fn apply_results_same_folder_rename_rekeys_without_advancing_content_baseline() {
    let (engine, db) = new_engine();
    insert_baseline(
        &db.lock(),
        "rename-file-id",
        "contracts/old.docx",
        SYNCED,
        None,
    );
    engine.cloud_tree_insert(
        "contracts/old.docx".into(),
        DriveFile {
            id: "rename-file-id".into(),
            name: "old.docx".into(),
            size: 333,
            parent_folder: Some(vec!["contracts-folder-id".into()]),
            ..Default::default()
        },
    );
    engine.path_to_id_insert("contracts/old.docx".into(), "rename-file-id".into());
    let edited_time = chrono::DateTime::from_timestamp_millis(4_444).unwrap();
    let renamed = DriveFile {
        id: "rename-file-id".into(),
        name: "new.docx".into(),
        size: 333,
        parent_folder: Some(vec!["contracts-folder-id".into()]),
        edited_time: Some(edited_time),
        ..Default::default()
    };
    let action = SyncAction {
        action_type: SyncActionType::MoveInCloud,
        relative_path: Some("contracts/new.docx".into()),
        file_id: Some("rename-file-id".into()),
        parent_file_id: Some("contracts-folder-id".into()),
        local_path: Some("/mount/contracts/new.docx".into()),
        cloud_file: None,
        reason: Some("同目录改名检测".into()),
    };
    let result = ActionResult {
        success: true,
        error_message: None,
        deferred: false,
        cloud_file: Some(renamed.clone()),
    };

    engine.apply_results(&[action], &[result]).unwrap();

    let connection = db.lock();
    let old_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_items WHERE file_id=?1 AND local_path=?2",
            params!["rename-file-id", "contracts/old.docx"],
            |row| row.get(0),
        )
        .unwrap();
    let new_baseline = baseline_snapshot(&connection, "rename-file-id", "contracts/new.docx");
    assert_eq!(old_count, 0, "改名成功后必须清理旧路径基线");
    assert_eq!(
        new_baseline,
        (
            Some("contracts-folder-id".to_string()),
            "new.docx".to_string(),
            0,
            333,
            Some(222),
            Some("baseline-sha".to_string()),
            Some(1_111),
            Some(4_444),
            Some(3_333),
            SYNCED,
            None,
        ),
        "结构改名不得伪造新的内容同步基线"
    );
    drop(connection);

    assert!(!engine.cloud_tree_lock().contains_key("contracts/old.docx"));
    assert_eq!(
        engine
            .cloud_tree_lock()
            .get("contracts/new.docx")
            .map(|file| file.id.as_str()),
        Some("rename-file-id")
    );
    assert!(!engine.path_to_id_lock().contains_key("contracts/old.docx"));
    assert_eq!(
        engine
            .path_to_id_lock()
            .get("contracts/new.docx")
            .map(String::as_str),
        Some("rename-file-id")
    );
}

/// 运行一次嵌套目录根移动结算，并核验 DB 与内存云树整棵重键。
fn assert_directory_root_move_rekeys_nested_subtree(
    old_root: &str,
    new_root: &str,
    source_parent_id: &str,
    target_parent_id: &str,
) {
    let (engine, db) = new_engine();
    let old_nested = format!("{old_root}/nested");
    let old_file = format!("{old_nested}/document.txt");
    let new_nested = format!("{new_root}/nested");
    let new_file = format!("{new_nested}/document.txt");
    {
        let connection = db.lock();
        insert_folder_baseline(&connection, "folder-root-id", old_root, source_parent_id);
        insert_folder_baseline(
            &connection,
            "folder-nested-id",
            &old_nested,
            "folder-root-id",
        );
        insert_baseline(
            &connection,
            "nested-file-id",
            &old_file,
            FAILED,
            Some("keep-me"),
        );
    }
    let child_before = baseline_snapshot(&db.lock(), "nested-file-id", &old_file);
    let nested_before = baseline_snapshot(&db.lock(), "folder-nested-id", &old_nested);

    let old_root_cloud = DriveFile {
        id: "folder-root-id".into(),
        name: old_root.rsplit('/').next().unwrap().into(),
        category: FileCategory::Folder,
        parent_folder: Some(vec![source_parent_id.into()]),
        ..Default::default()
    };
    let nested_cloud = DriveFile {
        id: "folder-nested-id".into(),
        name: "nested".into(),
        category: FileCategory::Folder,
        parent_folder: Some(vec!["folder-root-id".into()]),
        ..Default::default()
    };
    let file_cloud = DriveFile {
        id: "nested-file-id".into(),
        name: "document.txt".into(),
        size: 333,
        parent_folder: Some(vec!["folder-nested-id".into()]),
        ..Default::default()
    };
    for (path, file) in [
        (old_root.to_string(), old_root_cloud.clone()),
        (old_nested.clone(), nested_cloud),
        (old_file.clone(), file_cloud),
    ] {
        engine.path_to_id_insert(path.clone(), file.id.clone());
        engine.cloud_tree_insert(path, file);
    }

    let moved_root = DriveFile {
        id: "folder-root-id".into(),
        name: new_root.rsplit('/').next().unwrap().into(),
        category: FileCategory::Folder,
        size: 99,
        parent_folder: Some(vec![target_parent_id.into()]),
        edited_time: chrono::DateTime::from_timestamp_millis(4_444),
        ..Default::default()
    };
    let action = SyncAction {
        action_type: SyncActionType::MoveInCloud,
        relative_path: Some(new_root.into()),
        file_id: Some("folder-root-id".into()),
        parent_file_id: Some(target_parent_id.into()),
        local_path: Some(format!("/mount/{new_root}")),
        cloud_file: Some(old_root_cloud),
        reason: Some("目录根路径变化".into()),
    };
    let result = ActionResult {
        success: true,
        error_message: None,
        deferred: false,
        cloud_file: Some(moved_root),
    };

    engine.apply_results(&[action], &[result]).unwrap();

    let connection = db.lock();
    let remaining_old: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sync_items
             WHERE local_path=?1 OR local_path LIKE ?2",
            params![old_root, format!("{old_root}/%")],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_old, 0, "旧目录子树基线必须全部移除");
    assert_eq!(
        baseline_snapshot(&connection, "folder-nested-id", &new_nested),
        nested_before,
        "后代目录的内容与状态基线不得因根移动而推进"
    );
    assert_eq!(
        baseline_snapshot(&connection, "nested-file-id", &new_file),
        child_before,
        "后代文件的内容与失败状态基线必须原样保留"
    );
    let moved_root_baseline = baseline_snapshot(&connection, "folder-root-id", new_root);
    assert_eq!(moved_root_baseline.0.as_deref(), Some(target_parent_id));
    assert_eq!(moved_root_baseline.1, new_root.rsplit('/').next().unwrap());
    assert_eq!(moved_root_baseline.2, 1);
    assert_eq!(moved_root_baseline.3, 99);
    assert_eq!(moved_root_baseline.4, Some(0));
    assert_eq!(
        moved_root_baseline.5.as_deref(),
        Some("folder-baseline-folder-root-id")
    );
    assert_eq!(moved_root_baseline.6, Some(1_111));
    assert_eq!(moved_root_baseline.7, Some(4_444));
    assert_eq!(moved_root_baseline.8, Some(3_333));
    assert_eq!(moved_root_baseline.9, SYNCED);
    assert_eq!(
        moved_root_baseline.10.as_deref(),
        Some("preserved-folder-root-id")
    );
    drop(connection);

    let cloud = engine.cloud_tree_lock();
    assert!(!cloud.contains_key(old_root));
    assert!(!cloud.contains_key(&old_nested));
    assert!(!cloud.contains_key(&old_file));
    assert_eq!(
        cloud.get(new_root).map(|file| file.id.as_str()),
        Some("folder-root-id")
    );
    assert_eq!(
        cloud.get(&new_nested).map(|file| file.id.as_str()),
        Some("folder-nested-id")
    );
    assert_eq!(
        cloud.get(&new_file).map(|file| file.id.as_str()),
        Some("nested-file-id")
    );
    drop(cloud);
    let path_to_id = engine.path_to_id_lock();
    assert!(!path_to_id.contains_key(old_root));
    assert!(!path_to_id.contains_key(&old_nested));
    assert!(!path_to_id.contains_key(&old_file));
    assert_eq!(
        path_to_id.get(&new_file).map(String::as_str),
        Some("nested-file-id")
    );
}

/// 同一父目录内改名也必须作为一个目录根移动结算。
#[test]
fn apply_results_same_parent_directory_rename_rekeys_nested_subtree() {
    assert_directory_root_move_rekeys_nested_subtree(
        "projects/old",
        "projects/new",
        "projects-id",
        "projects-id",
    );
}

/// 跨父目录移动必须整棵重键，同时只更新根的 parentFolder。
#[test]
fn apply_results_cross_parent_directory_move_rekeys_nested_subtree() {
    assert_directory_root_move_rekeys_nested_subtree(
        "projects/old",
        "archive/new",
        "projects-id",
        "archive-id",
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
