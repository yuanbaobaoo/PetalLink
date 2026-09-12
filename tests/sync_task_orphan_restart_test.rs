//! TaskRunner 孤儿 RestartRequired 任务回收的集成测试。

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use petal_link_lib::error::{AppError, AppResult};
use petal_link_lib::sync::task_runner::{
    TaskActivityGate, TaskProgressReporter, TaskRunner, TransferOperations, TransferTask,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试时钟的固定当前时间（100 小时）。
const NOW_MS: i64 = 100 * 60 * 60 * 1000;
/// 测试任务使用的相对路径。
const RELATIVE_PATH: &str = "contracts/stale.pdf";
/// 上传方向的持久化协议值。
const UPLOAD_DIRECTION: i32 = 0;
/// 下载方向的持久化协议值。
const DOWNLOAD_DIRECTION: i32 = 2;

/// 回收流程不应触发任何远端执行；误调用即测试失败。
struct UnreachableOperations;

#[async_trait]
impl TransferOperations for UnreachableOperations {
    /// 远端执行不应发生在回收流程中。
    async fn execute(
        &self,
        _task: &TransferTask,
        _progress: &TaskProgressReporter,
    ) -> Result<
        petal_link_lib::sync::task_runner::TaskExecutionOutcome,
        petal_link_lib::sync::task_runner::TaskExecutionError,
    > {
        Err(petal_link_lib::sync::task_runner::TaskExecutionError::App(
            AppError::generic("回收流程不应执行远端写入"),
        ))
    }

    /// 远端核验不应发生在回收流程中。
    async fn verify_remote(
        &self,
        _task: &TransferTask,
    ) -> AppResult<petal_link_lib::sync::task_runner::RemoteVerification> {
        Err(AppError::generic("回收流程不应触发远端核验"))
    }
}

/// 恒拒绝的活动许可门：回收流程不应获取路径许可。
struct RejectingActivityGate;

impl TaskActivityGate for RejectingActivityGate {
    /// 任何许可获取都视为流程越界。
    fn begin(&self, _relative_path: Option<&str>) -> AppResult<Box<dyn Send>> {
        Err(AppError::generic("回收流程不应获取路径许可"))
    }
}

/// 创建回收流程所需的最小临时数据库。
fn open_database(path: &std::path::Path) -> Arc<Mutex<Connection>> {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "
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

/// 构造一条 RestartRequired 任务记录。
fn restart_task(direction: i32, operation: TransferOperation, created_at: i64) -> TransferTask {
    TransferTask {
        id: 0,
        direction,
        file_id: None,
        local_path: None,
        name: "stale.pdf".to_string(),
        total_size: 100,
        transferred: 0,
        state: i32::from(TransferState::RestartRequired),
        error_message: Some("文件尚不稳定，等待重新规划".to_string()),
        created_at,
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(RELATIVE_PATH.to_string()),
        parent_file_id: None,
        operation: Some(i32::from(operation)),
        source_mtime: None,
        source_size: None,
        expected_cloud_edited_time: None,
        attempt_count: 0,
        verify_attempt_count: 0,
        next_retry_at: None,
        error_kind: Some(i32::from(TransferErrorKind::LocalChanged)),
        remote_result_file_id: None,
        state_revision: 0,
    }
}

/// 插入任务并返回持久化 ID。
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

/// 读取任务的状态三元组（state, error_kind, finished_at）。
fn read_state(connection: &Connection, task_id: i64) -> (i32, Option<i32>, Option<i64>) {
    connection
        .query_row(
            "SELECT state, error_kind, finished_at FROM transfer_queue WHERE id=?1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, Option<i32>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .unwrap()
}

/// 构造使用固定时钟的任务执行器。
fn build_runner(database: Arc<Mutex<Connection>>, mount_root: std::path::PathBuf) -> TaskRunner {
    TaskRunner::new_with_clock(
        database,
        mount_root,
        Arc::new(UnreachableOperations),
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    )
}

/// 核心合同：上传任务在源消失、云端无同路径对象且超龄时被取消为 Canceled 终态。
#[test]
fn cancels_stale_upload_restart_when_source_and_cloud_missing() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let task_id = insert_task(
        &database.lock(),
        &restart_task(
            UPLOAD_DIRECTION,
            TransferOperation::Update,
            NOW_MS - 25 * 60 * 60 * 1000,
        ),
    );
    let runner = build_runner(database.clone(), temp.path().join("mount"));
    runner.set_activity_gate(Arc::new(RejectingActivityGate));

    let cancelled = runner.cancel_orphaned_restart_tasks(&|_| false).unwrap();

    assert_eq!(cancelled, 1);
    let (state, error_kind, finished_at) = read_state(&database.lock(), task_id);
    assert_eq!(state, i32::from(TransferState::Canceled));
    assert_eq!(error_kind, Some(i32::from(TransferErrorKind::LocalChanged)));
    assert_eq!(finished_at, Some(NOW_MS));
}

/// 下载任务的本地路径缺失是执行中的常态，绝不允许被回收。
#[test]
fn keeps_download_restart_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let task_id = insert_task(
        &database.lock(),
        &restart_task(
            DOWNLOAD_DIRECTION,
            TransferOperation::Download,
            NOW_MS - 25 * 60 * 60 * 1000,
        ),
    );
    let runner = build_runner(database.clone(), temp.path().join("mount"));

    assert_eq!(runner.cancel_orphaned_restart_tasks(&|_| false).unwrap(), 0);
    assert_eq!(
        read_state(&database.lock(), task_id).0,
        i32::from(TransferState::RestartRequired)
    );
}

/// 本地源文件仍存在时保留任务：可能只是等待网络恢复重试。
#[test]
fn keeps_restart_tasks_with_existing_local_source() {
    let temp = tempfile::tempdir().unwrap();
    let mount_root = temp.path().join("mount");
    std::fs::create_dir_all(mount_root.join("contracts")).unwrap();
    std::fs::write(mount_root.join(RELATIVE_PATH), b"data").unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let task_id = insert_task(
        &database.lock(),
        &restart_task(
            UPLOAD_DIRECTION,
            TransferOperation::Update,
            NOW_MS - 25 * 60 * 60 * 1000,
        ),
    );
    let runner = build_runner(database.clone(), mount_root);

    assert_eq!(runner.cancel_orphaned_restart_tasks(&|_| false).unwrap(), 0);
    assert_eq!(
        read_state(&database.lock(), task_id).0,
        i32::from(TransferState::RestartRequired)
    );
}

/// 云端同路径对象仍存在时保留任务：该路径仍可能产生重新规划意图。
#[test]
fn keeps_restart_tasks_when_cloud_still_has_path() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let task_id = insert_task(
        &database.lock(),
        &restart_task(
            UPLOAD_DIRECTION,
            TransferOperation::Update,
            NOW_MS - 25 * 60 * 60 * 1000,
        ),
    );
    let runner = build_runner(database.clone(), temp.path().join("mount"));

    assert_eq!(
        runner
            .cancel_orphaned_restart_tasks(&|path| path == RELATIVE_PATH)
            .unwrap(),
        0
    );
    assert_eq!(
        read_state(&database.lock(), task_id).0,
        i32::from(TransferState::RestartRequired)
    );
}

/// 低龄任务不回收：规避改名/替换瞬间文件短暂消失的监听器竞态。
#[test]
fn keeps_young_restart_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let task_id = insert_task(
        &database.lock(),
        &restart_task(
            UPLOAD_DIRECTION,
            TransferOperation::Update,
            NOW_MS - 60 * 60 * 1000,
        ),
    );
    let runner = build_runner(database.clone(), temp.path().join("mount"));

    assert_eq!(runner.cancel_orphaned_restart_tasks(&|_| false).unwrap(), 0);
    assert_eq!(
        read_state(&database.lock(), task_id).0,
        i32::from(TransferState::RestartRequired)
    );
}

/// 同路径存在其他非终态任务时保留：避免误杀核验/重放流程中的任务。
#[test]
fn keeps_restart_tasks_with_active_sibling_on_same_path() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let conn = database.lock();
    let task_id = insert_task(
        &conn,
        &restart_task(
            UPLOAD_DIRECTION,
            TransferOperation::Update,
            NOW_MS - 25 * 60 * 60 * 1000,
        ),
    );
    let mut sibling = restart_task(
        UPLOAD_DIRECTION,
        TransferOperation::Update,
        NOW_MS - 60 * 1000,
    );
    sibling.state = i32::from(TransferState::Pending);
    insert_task(&conn, &sibling);
    drop(conn);
    let runner = build_runner(database.clone(), temp.path().join("mount"));

    assert_eq!(runner.cancel_orphaned_restart_tasks(&|_| false).unwrap(), 0);
    assert_eq!(
        read_state(&database.lock(), task_id).0,
        i32::from(TransferState::RestartRequired)
    );
}

/// 终态与非上传操作任务不在回收范围。
#[test]
fn ignores_terminal_and_non_upload_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let database = open_database(&temp.path().join("db.sqlite"));
    let conn = database.lock();
    let mut failed = restart_task(
        UPLOAD_DIRECTION,
        TransferOperation::Update,
        NOW_MS - 25 * 60 * 60 * 1000,
    );
    failed.state = i32::from(TransferState::Failed);
    let failed_id = insert_task(&conn, &failed);
    let mut completed = restart_task(
        UPLOAD_DIRECTION,
        TransferOperation::Create,
        NOW_MS - 25 * 60 * 60 * 1000,
    );
    completed.state = i32::from(TransferState::Completed);
    let completed_id = insert_task(&conn, &completed);
    drop(conn);
    let runner = build_runner(database.clone(), temp.path().join("mount"));

    assert_eq!(runner.cancel_orphaned_restart_tasks(&|_| false).unwrap(), 0);
    assert_eq!(
        read_state(&database.lock(), failed_id).0,
        i32::from(TransferState::Failed)
    );
    assert_eq!(
        read_state(&database.lock(), completed_id).0,
        i32::from(TransferState::Completed)
    );
}
