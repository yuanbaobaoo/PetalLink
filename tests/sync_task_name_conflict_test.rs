//! 同名冲突（NameConflict）与用户决策操作的集成测试。
//!
//! 覆盖：冲突错误的持久化映射、replan 保留歧义远端结果 ID、
//! 含结果 ID 的重启任务升回核验态、用户取消/重新放行任务。
//! 自包含最小测试基建，与 `sync_task_replay_breaker_test.rs` 的 helper 刻意重复。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use petal_link_lib::sync::task_runner::{
    TaskDisposition, TaskExecutionError, TaskExecutionOutcome, TaskProgressReporter, TaskRunner,
    TransferOperations, TransferTask,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试时钟的固定当前时间。
const NOW_MS: i64 = 10_000;
/// 测试任务使用的相对路径。
const RELATIVE_PATH: &str = "contracts/report.docx";
/// 测试任务使用的文件名。
const FILE_NAME: &str = "report.docx";
/// 测试任务上传的固定内容。
const FILE_CONTENT: &[u8] = b"data";
/// 上传方向的持久化协议值。
const UPLOAD_DIRECTION: i32 = 0;

/// 创建恢复流程所需的最小临时数据库。
fn open_database(path: &Path) -> Arc<Mutex<Connection>> {
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
                verify_attempt_count INTEGER NOT NULL DEFAULT 0,
                next_retry_at INTEGER,
                error_kind INTEGER,
                remote_result_file_id TEXT,
                state_revision INTEGER NOT NULL DEFAULT 0
            );
            ",
        )
        .unwrap();
    Arc::new(Mutex::new(connection))
}

/// 创建等待执行的 Create 上传任务。
fn pending_create_task(local_path: &Path) -> TransferTask {
    let metadata = std::fs::metadata(local_path).unwrap();
    let source_mtime = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    TransferTask {
        id: 0,
        direction: UPLOAD_DIRECTION,
        file_id: None,
        local_path: Some(local_path.to_str().unwrap().to_string()),
        name: FILE_NAME.to_string(),
        total_size: FILE_CONTENT.len() as i64,
        transferred: 0,
        state: i32::from(TransferState::Pending),
        error_message: None,
        created_at: 1,
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(RELATIVE_PATH.to_string()),
        parent_file_id: Some("contracts-folder-id".to_string()),
        operation: Some(i32::from(TransferOperation::Create)),
        source_mtime: Some(source_mtime),
        source_size: Some(FILE_CONTENT.len() as i64),
        expected_cloud_edited_time: None,
        attempt_count: 0,
        verify_attempt_count: 0,
        next_retry_at: None,
        error_kind: None,
        remote_result_file_id: None,
        state_revision: 0,
    }
}

/// 插入完整任务合同并返回持久化 ID。
fn insert_task(connection: &Connection, task: &TransferTask) -> i64 {
    connection
        .execute(
            "INSERT INTO transfer_queue (
                direction, file_id, local_path, name, total_size, transferred, state,
                error_message, created_at, finished_at, server_id, upload_id, resume_offset,
                session_url, relative_path, parent_file_id, operation, source_mtime,
                source_size, expected_cloud_edited_time, attempt_count, verify_attempt_count,
                next_retry_at, error_kind, remote_result_file_id, state_revision
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
             )",
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
                task.session_url,
                task.relative_path,
                task.parent_file_id,
                task.operation,
                task.source_mtime,
                task.source_size,
                task.expected_cloud_edited_time,
                task.attempt_count,
                task.verify_attempt_count,
                task.next_retry_at,
                task.error_kind,
                task.remote_result_file_id,
                task.state_revision,
            ],
        )
        .unwrap();
    connection.last_insert_rowid()
}

/// 创建测试文件并返回挂载根与文件绝对路径。
fn create_local_source(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let mount_root = temp.path().join("mount");
    let local_path = mount_root.join(RELATIVE_PATH);
    std::fs::create_dir_all(local_path.parent().unwrap()).unwrap();
    std::fs::write(&local_path, FILE_CONTENT).unwrap();
    (mount_root, local_path)
}

/// 构造标准 TaskRunner（内存态操作集可注入）。
fn build_runner(
    database: Arc<Mutex<Connection>>,
    mount_root: std::path::PathBuf,
    operations: Arc<dyn TransferOperations>,
) -> TaskRunner {
    TaskRunner::new_with_clock(
        database,
        mount_root,
        operations,
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    )
}

/// 执行始终报同名冲突的操作集。
struct NameConflictOperations {
    execute_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TransferOperations for NameConflictOperations {
    async fn execute(
        &self,
        _task: &TransferTask,
        _progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        Err(TaskExecutionError::NameConflict(
            "目标目录已存在同名远端文件且内容不一致，请选择处理方式".to_string(),
        ))
    }
}

/// 冲突错误必须落库为 RestartRequired + NameConflict，等待用户决策而不是无限重试。
#[tokio::test]
async fn name_conflict_error_lands_as_restart_required_with_kind() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let execute_calls = Arc::new(AtomicUsize::new(0));
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: execute_calls.clone(),
        }),
    );

    let intent = pending_create_task(&local_path);
    let outcome = runner.enqueue_and_run(intent).await.unwrap();
    assert_eq!(
        outcome.outcome.disposition,
        TaskDisposition::RestartRequired
    );
    assert_eq!(execute_calls.load(Ordering::SeqCst), 1);

    let persisted: (i32, Option<i32>, Option<String>) = database
        .lock()
        .query_row(
            "SELECT state, error_kind, error_message FROM transfer_queue WHERE id=?1",
            [outcome.task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, i32::from(TransferState::RestartRequired));
    assert_eq!(
        persisted.1,
        Some(i32::from(TransferErrorKind::NameConflict)),
        "同名冲突必须携带 NameConflict 分类，前端据此渲染决策菜单"
    );
    assert!(persisted.2.unwrap().contains("内容不一致"));
}

/// replan 必须保留 remote_result_file_id：它是歧义远端写入的唯一身份证据，
/// 清空会让核验调和失效并形成「重试 → 撞同名 → 再重启」死循环（防回归）。
#[tokio::test]
async fn replan_preserves_remote_result_file_id() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    // 既有阻塞态旧意图，持有歧义写入的远端结果 ID。
    let mut stale = pending_create_task(&local_path);
    stale.total_size = FILE_CONTENT.len() as i64 + 100;
    stale.source_size = Some(FILE_CONTENT.len() as i64 + 100);
    stale.remote_result_file_id = Some("ambiguous-result-id".to_string());
    let stale_id = insert_task(&database.lock(), &stale);
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: Arc::new(AtomicUsize::new(0)),
        }),
    );

    // 新意图同路径但内容不同，触发 replan 替换旧意图。
    let intent = pending_create_task(&local_path);
    let outcome = runner.enqueue_and_run(intent).await.unwrap();
    assert_eq!(outcome.task_id, stale_id, "replan 必须复用原任务 ID");

    let persisted: Option<String> = database
        .lock()
        .query_row(
            "SELECT remote_result_file_id FROM transfer_queue WHERE id=?1",
            [stale_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        persisted.as_deref(),
        Some("ambiguous-result-id"),
        "replan 不得清空歧义远端结果 ID"
    );
}

/// 持有远端结果 ID 的 RestartRequired 任务，新意图准入时必须升回核验态精确核验，
/// 而不是盲目重放（盲目重放会被同名碰撞检查拦回，形成死循环）。
#[tokio::test]
async fn ambiguous_restart_is_promoted_to_verifying_instead_of_blind_replay() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let mut restart = pending_create_task(&local_path);
    restart.state = i32::from(TransferState::RestartRequired);
    restart.error_kind = Some(i32::from(TransferErrorKind::RemoteAmbiguous));
    restart.error_message = Some("上次上传结果待核验".to_string());
    restart.remote_result_file_id = Some("ambiguous-result-id".to_string());
    let restart_id = insert_task(&database.lock(), &restart);
    let execute_calls = Arc::new(AtomicUsize::new(0));
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: execute_calls.clone(),
        }),
    );

    let intent = pending_create_task(&local_path);
    let outcome = runner.enqueue_and_run(intent).await.unwrap();

    assert_eq!(outcome.task_id, restart_id);
    assert_eq!(
        outcome.outcome.disposition,
        TaskDisposition::VerifyingRemote,
        "含远端结果 ID 的重启任务应升回核验态"
    );
    assert_eq!(
        execute_calls.load(Ordering::SeqCst),
        0,
        "核验前不得盲目重放上传"
    );
    let persisted: (i32, Option<String>) = database
        .lock()
        .query_row(
            "SELECT state, remote_result_file_id FROM transfer_queue WHERE id=?1",
            [restart_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, i32::from(TransferState::VerifyingRemote));
    assert_eq!(persisted.1.as_deref(), Some("ambiguous-result-id"));
}

/// 用户取消仅允许 RestartRequired/Failed；取消后落终态、带说明与完成时间。
#[tokio::test]
async fn user_cancel_transitions_restart_and_failed_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: Arc::new(AtomicUsize::new(0)),
        }),
    );

    // RestartRequired + NameConflict 可取消。
    let mut conflict = pending_create_task(&local_path);
    conflict.state = i32::from(TransferState::RestartRequired);
    conflict.error_kind = Some(i32::from(TransferErrorKind::NameConflict));
    let conflict_id = insert_task(&database.lock(), &conflict);
    let canceled = runner
        .cancel_user_task(conflict_id, "用户已取消该任务")
        .unwrap();
    assert_eq!(canceled.state, i32::from(TransferState::Canceled));
    assert_eq!(canceled.error_message.as_deref(), Some("用户已取消该任务"));
    assert_eq!(canceled.finished_at, Some(NOW_MS));

    // 活动状态（Pending）不允许取消。
    let pending_id = insert_task(&database.lock(), &pending_create_task(&local_path));
    assert!(runner.cancel_user_task(pending_id, "x").is_err());
}

/// 冲突任务重新放行：清空错误、计数归零、迁移回 Pending。
#[tokio::test]
async fn requeue_name_conflict_task_resets_and_pends() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let mut conflict = pending_create_task(&local_path);
    conflict.state = i32::from(TransferState::RestartRequired);
    conflict.error_kind = Some(i32::from(TransferErrorKind::NameConflict));
    conflict.error_message = Some("同名冲突".to_string());
    conflict.attempt_count = 3;
    conflict.verify_attempt_count = 2;
    let conflict_id = insert_task(&database.lock(), &conflict);

    // 守卫：非冲突的 RestartRequired 不得走冲突放行通道。
    let mut plain_restart = pending_create_task(&local_path);
    plain_restart.state = i32::from(TransferState::RestartRequired);
    plain_restart.error_kind = Some(i32::from(TransferErrorKind::LocalChanged));
    plain_restart.relative_path = Some("contracts/other.docx".to_string());
    let plain_id = insert_task(&database.lock(), &plain_restart);
    assert!(runner.load_name_conflict_task(plain_id).is_err());

    let task = runner.load_name_conflict_task(conflict_id).unwrap();
    let pending = runner.requeue_name_conflict_task(&task).unwrap();
    assert_eq!(pending.state, i32::from(TransferState::Pending));
    let persisted: (Option<i32>, Option<String>, i64, i64) = database
        .lock()
        .query_row(
            "SELECT error_kind, error_message, attempt_count, verify_attempt_count
             FROM transfer_queue WHERE id=?1",
            [conflict_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, None);
    assert_eq!(persisted.1, None);
    assert_eq!(persisted.2, 0, "用户显式操作开启新一轮预算周期");
    assert_eq!(persisted.3, 0);
}

/// RestartRequired 任务必须能直接重试：同名碰撞场景下 planner 对该路径只会 Skip
/// （本地有+云端有+无基线），等重规划会永久滞留（v1.1.10 两条滞留记录的根因）。
#[tokio::test]
async fn restart_required_task_can_retry_directly() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let runner = build_runner(
        database.clone(),
        mount_root,
        Arc::new(NameConflictOperations {
            execute_calls: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let mut restart = pending_create_task(&local_path);
    restart.state = i32::from(TransferState::RestartRequired);
    restart.error_kind = Some(i32::from(TransferErrorKind::LocalChanged));
    restart.error_message = Some("目标目录已存在同名远端文件，拒绝重复创建".to_string());
    restart.attempt_count = 2;
    let restart_id = insert_task(&database.lock(), &restart);

    let retried = runner.prepare_retry(restart_id).await.unwrap();
    assert_eq!(retried.state, i32::from(TransferState::Pending));
    let persisted: (Option<i32>, Option<String>, i64) = database
        .lock()
        .query_row(
            "SELECT error_kind, error_message, attempt_count FROM transfer_queue WHERE id=?1",
            [restart_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, None);
    assert_eq!(persisted.1, None);
    assert_eq!(persisted.2, 0, "直接重试同样开启新一轮预算周期");
}
