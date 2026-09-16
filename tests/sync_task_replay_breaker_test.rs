//! 「核验确认未提交 → 重放」环路的熔断与人工重试预算重置测试（2026-09-15 事故防回归）。
//!
//! 自包含最小测试基建（DB 建表/任务构造/落库），与 `sync_task_recovery_test.rs`
//! 的 helper 刻意重复：集成测试文件按领域独立，避免跨文件私有依赖。

use std::path::Path;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use petal_link_lib::error::{AppError, AppResult, DriveTransportKind};
use petal_link_lib::sync::task_runner::{
    RemoteVerification, TaskExecutionError, TaskExecutionOutcome, TaskProgressReporter, TaskRunner,
    TransferOperations, TransferTask,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试时钟的固定当前时间。
const NOW_MS: i64 = 10_000;
/// 测试任务使用的相对路径。
const RELATIVE_PATH: &str = "contracts/replay.docx";
/// 测试任务使用的文件名。
const FILE_NAME: &str = "replay.docx";
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

/// 核验始终未提交且重放始终歧义失败的操作集，复现 2026-09-15 事故环路。
struct NotCommittedReplayOperations {
    execute_calls: Arc<AtomicUsize>,
    verification_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TransferOperations for NotCommittedReplayOperations {
    /// 重放执行始终因远端响应歧义失败。
    async fn execute(
        &self,
        _task: &TransferTask,
        _progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        Err(TaskExecutionError::App(
            AppError::drive_transport_with_submission(
                DriveTransportKind::Decode,
                true,
                false,
                Some("finalize response rejected"),
            ),
        ))
    }

    /// 远端核验始终确认写入未提交。
    async fn verify_remote(&self, _task: &TransferTask) -> AppResult<RemoteVerification> {
        self.verification_calls.fetch_add(1, Ordering::SeqCst);
        Ok(RemoteVerification::NotCommitted)
    }
}

/// 核验-重放环路必须有界：每轮重放累积计数，达上限后终态 Failed 并清理续传会话，
/// 人工重试因此从全新会话开始（2026-09-15 五个大文件任务无限循环事故）。
#[tokio::test]
async fn not_committed_replay_is_bounded_and_clears_upload_session() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    // 构造带续传会话的 VerifyingRemote Create 任务，对齐事故现场。
    let mut task = pending_create_task(&local_path);
    task.state = i32::from(TransferState::VerifyingRemote);
    task.error_kind = Some(i32::from(TransferErrorKind::RemoteAmbiguous));
    task.error_message = Some("云端响应异常".to_string());
    task.session_url = Some("https://uploadserver.example/session".to_string());
    task.resume_offset = 2;
    task.next_retry_at = Some(NOW_MS);
    let task_id = insert_task(&database.lock(), &task);
    let execute_calls = Arc::new(AtomicUsize::new(0));
    let verification_calls = Arc::new(AtomicUsize::new(0));
    let clock = Arc::new(AtomicI64::new(NOW_MS));
    let runner_clock = clock.clone();
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root,
        Arc::new(NotCommittedReplayOperations {
            execute_calls: execute_calls.clone(),
            verification_calls: verification_calls.clone(),
        }),
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(move || runner_clock.load(Ordering::SeqCst)),
    );

    // 第一轮：计数累积而不是清零，重放后回到 VerifyingRemote 等待下一轮。
    runner.resume_verifying().await.unwrap();
    let first_round: (i32, i64, Option<String>) = database
        .lock()
        .query_row(
            "SELECT state, verify_attempt_count, session_url FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(first_round.0, i32::from(TransferState::VerifyingRemote));
    assert_eq!(first_round.1, 1, "核验计数不得清零，否则环路熔断永不触发");
    assert!(first_round.2.is_some(), "未达上限时会话必须保留供续传");

    // 后续轮次：第 5 轮核验后到达上限，终态失败，不再重放。
    for _ in 2..=5 {
        clock.fetch_add(4_000, Ordering::SeqCst);
        runner.resume_verifying().await.unwrap();
    }

    assert_eq!(verification_calls.load(Ordering::SeqCst), 5);
    assert_eq!(
        execute_calls.load(Ordering::SeqCst),
        4,
        "达上限后不得再重放"
    );
    // 状态与计数。
    let state_row: (i32, i64, Option<i64>, Option<i32>) = database
        .lock()
        .query_row(
            "SELECT state, verify_attempt_count, next_retry_at, error_kind
             FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state_row.0, i32::from(TransferState::Failed));
    assert_eq!(state_row.1, 5);
    assert_eq!(state_row.2, None, "终态失败不得再被退避调度唤醒");
    assert_eq!(
        state_row.3,
        Some(i32::from(TransferErrorKind::RemoteAmbiguous))
    );
    // 会话清理与终态字段。
    let session_row: (Option<String>, i64, Option<i64>, Option<String>) = database
        .lock()
        .query_row(
            "SELECT session_url, resume_offset, finished_at, error_message
             FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(session_row.0, None, "终态失败必须清理续传会话");
    assert_eq!(session_row.1, 0, "终态失败必须清零断点偏移");
    assert!(session_row.2.is_some());
    assert!(session_row.3.unwrap().contains("请手动重试"));
}

/// 人工重试开启新一轮预算周期：耗尽而终态失败的任务重试后计数器归零，
/// 否则加固后的歧义预算会让重试在第一次失败时立即再次终态失败。
#[tokio::test]
async fn manual_retry_resets_attempt_and_verify_counters() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    // 构造预算耗尽而终态失败的任务。
    let mut task = pending_create_task(&local_path);
    task.state = i32::from(TransferState::Failed);
    task.attempt_count = 5;
    task.verify_attempt_count = 5;
    task.error_kind = Some(i32::from(TransferErrorKind::RemoteAmbiguous));
    task.error_message = Some("多次重传仍无法确认云端结果".to_string());
    task.finished_at = Some(NOW_MS);
    let task_id = insert_task(&database.lock(), &task);
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root,
        Arc::new(NotCommittedReplayOperations {
            execute_calls: Arc::new(AtomicUsize::new(0)),
            verification_calls: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    );

    let retried = runner.prepare_retry(task_id).await.unwrap();

    assert_eq!(retried.state, i32::from(TransferState::Pending));
    let persisted: (i64, i64, Option<i32>, Option<i64>) = database
        .lock()
        .query_row(
            "SELECT attempt_count, verify_attempt_count, error_kind, finished_at
             FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, 0, "人工重试必须重置尝试预算");
    assert_eq!(persisted.1, 0, "人工重试必须重置核验计数");
    assert_eq!(persisted.2, None);
    assert_eq!(persisted.3, None);
}
