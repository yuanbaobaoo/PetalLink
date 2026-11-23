//! 用户重试入队合同测试（B1）：幂等入队、只入队不执行、一键重试汇总计数。
//!
//! 自包含最小测试基建（DB 建表/任务构造/落库），与 `sync_task_recovery_test.rs`
//! 的 helper 刻意重复：集成测试文件按领域独立，避免跨文件私有依赖。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use petal_link_lib::error::AppResult;
use petal_link_lib::sync::task_runner::{
    RemoteVerification, RetryEnqueued, TaskExecutionError, TaskExecutionOutcome,
    TaskProgressReporter, TaskRunner, TransferOperations, TransferTask,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试任务使用的相对路径。
const RELATIVE_PATH: &str = "enqueue/retry.bin";
/// 测试任务使用的文件名。
const FILE_NAME: &str = "retry.bin";
/// 测试任务上传的固定内容。
const FILE_CONTENT: &[u8] = b"payload";
/// 上传方向的持久化协议值。
const UPLOAD_DIRECTION: i32 = 0;
/// 测试时钟的固定当前时间。
const NOW_MS: i64 = 1_000;

/// 创建入队测试所需的最小临时数据库。
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

/// 每次执行都会被计数的操作集，用于证明「入队不等于执行」。
struct CountingOperations {
    execute_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl TransferOperations for CountingOperations {
    /// 记录一次执行请求；测试中不应被入队路径触发。
    async fn execute(
        &self,
        _task: &TransferTask,
        _progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        Err(TaskExecutionError::App(
            petal_link_lib::error::AppError::generic("测试不应真正执行传输"),
        ))
    }

    /// 核验不参与本测试。
    async fn verify_remote(&self, _task: &TransferTask) -> AppResult<RemoteVerification> {
        Ok(RemoteVerification::NotCommitted)
    }
}

/// 创建可写入数据库的上传任务模板并落库。
fn insert_task(
    connection: &Connection,
    local_path: &Path,
    state: TransferState,
    operation: TransferOperation,
    error_kind: Option<TransferErrorKind>,
) -> i64 {
    let metadata = std::fs::metadata(local_path).unwrap();
    let source_mtime = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let task = TransferTask {
        id: 0,
        direction: UPLOAD_DIRECTION,
        file_id: None,
        local_path: Some(local_path.to_str().unwrap().to_string()),
        name: FILE_NAME.to_string(),
        total_size: FILE_CONTENT.len() as i64,
        transferred: 0,
        state: i32::from(state),
        error_message: Some("历史失败原因".to_string()),
        created_at: 1,
        finished_at: Some(NOW_MS),
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: Some("https://uploadserver.example/session".to_string()),
        relative_path: Some(RELATIVE_PATH.to_string()),
        parent_file_id: Some("parent-folder-id".to_string()),
        operation: Some(i32::from(operation)),
        source_mtime: Some(source_mtime),
        source_size: Some(FILE_CONTENT.len() as i64),
        expected_cloud_edited_time: None,
        attempt_count: 5,
        verify_attempt_count: 5,
        next_retry_at: None,
        error_kind: error_kind.map(i32::from),
        remote_result_file_id: None,
        state_revision: 0,
    };
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

/// 构造只计数、不真正执行传输的 TaskRunner。
fn build_runner(
    database: &Arc<Mutex<Connection>>,
    mount_root: &Path,
) -> (TaskRunner, Arc<AtomicUsize>) {
    let execute_calls = Arc::new(AtomicUsize::new(0));
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root.to_path_buf(),
        Arc::new(CountingOperations {
            execute_calls: execute_calls.clone(),
        }),
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    );
    (runner, execute_calls)
}

/// 入队必须把失败任务推进到 Pending、清空错误与预算，但绝不执行传输。
#[tokio::test]
async fn enqueue_retry_marks_pending_without_executing() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let task_id = insert_task(
        &database.lock(),
        &local_path,
        TransferState::Failed,
        TransferOperation::Create,
        Some(TransferErrorKind::RemoteAmbiguous),
    );
    let (runner, execute_calls) = build_runner(&database, &mount_root);

    let enqueued = runner.enqueue_retry(task_id).await.unwrap();

    assert!(matches!(enqueued, RetryEnqueued::Queued { task_id: id } if id == task_id));
    assert_eq!(
        execute_calls.load(Ordering::SeqCst),
        0,
        "入队路径不得触发传输执行"
    );
    let row: (i32, Option<String>, i64, i64, Option<i64>) = database
        .lock()
        .query_row(
            "SELECT state, error_message, attempt_count, verify_attempt_count, finished_at
             FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, i32::from(TransferState::Pending));
    assert_eq!(row.1, None, "入队必须清理历史错误文案");
    assert_eq!((row.2, row.3), (0, 0), "人工重试开启新一轮预算");
    assert_eq!(row.4, None, "重新入队后任务不再是已完成状态");
}

/// 重复点击同一条失败项必须幂等：返回 AlreadyQueued 而不是报错或排队两次。
#[tokio::test]
async fn enqueue_retry_is_idempotent_for_queued_task() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let task_id = insert_task(
        &database.lock(),
        &local_path,
        TransferState::Failed,
        TransferOperation::Create,
        None,
    );
    let (runner, _execute_calls) = build_runner(&database, &mount_root);

    runner.enqueue_retry(task_id).await.unwrap();
    let second = runner.enqueue_retry(task_id).await.unwrap();

    assert!(matches!(second, RetryEnqueued::AlreadyQueued { task_id: id } if id == task_id));
    let count: i64 = database
        .lock()
        .query_row("SELECT COUNT(*) FROM transfer_queue", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "幂等入队不得插入第二条任务");
}

/// 已结束的任务不能被重试，且错误文案要指向真实原因。
#[tokio::test]
async fn enqueue_retry_rejects_finished_task() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let task_id = insert_task(
        &database.lock(),
        &local_path,
        TransferState::Completed,
        TransferOperation::Create,
        None,
    );
    let (runner, _execute_calls) = build_runner(&database, &mount_root);

    let error = runner.enqueue_retry(task_id).await.unwrap_err();

    assert!(
        error.to_string().contains("任务已结束"),
        "实际错误：{error}"
    );
}

/// 一键重试必须跳过同名冲突与结构动作，只对可重放的传输任务计数。
#[tokio::test]
async fn enqueue_all_failed_skips_conflicts_and_structural_operations() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    // 四类失败任务在同一段同步代码里插入，避免跨 await 持有锁。
    let (first, second, conflict, structural) = {
        let database_lock = database.lock();
        (
            insert_task(
                &database_lock,
                &local_path,
                TransferState::Failed,
                TransferOperation::Create,
                Some(TransferErrorKind::Network),
            ),
            insert_task(
                &database_lock,
                &local_path,
                TransferState::Failed,
                TransferOperation::Create,
                Some(TransferErrorKind::Server),
            ),
            // 同名冲突必须由用户三选一，不能自动重放。
            insert_task(
                &database_lock,
                &local_path,
                TransferState::Failed,
                TransferOperation::Create,
                Some(TransferErrorKind::NameConflict),
            ),
            // 删除类结构动作没有可重放的传输后端。
            insert_task(
                &database_lock,
                &local_path,
                TransferState::Failed,
                TransferOperation::Delete,
                Some(TransferErrorKind::Unknown),
            ),
        )
    };
    let (runner, _execute_calls) = build_runner(&database, &mount_root);

    let plan = runner.enqueue_all_failed().await.unwrap();

    assert_eq!(plan.summary.queued, 2);
    assert_eq!(plan.summary.skipped, 2);
    assert_eq!(plan.summary.rejected, 0);
    assert_eq!(plan.queued_task_ids.len(), 2);
    assert!(plan.queued_task_ids.contains(&first));
    assert!(plan.queued_task_ids.contains(&second));
    // 被跳过的任务保持原状，等待用户显式决策。
    let skipped_state: i32 = database
        .lock()
        .query_row(
            "SELECT state FROM transfer_queue WHERE id=?1",
            [conflict],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(skipped_state, i32::from(TransferState::Failed));
    let structural_state: i32 = database
        .lock()
        .query_row(
            "SELECT state FROM transfer_queue WHERE id=?1",
            [structural],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(structural_state, i32::from(TransferState::Failed));
}

/// 自动重规划不得把重试计数清零，否则熔断会被每一轮规划反复重置。
///
/// 人工重试（`enqueue_retry`）才是「开启新一轮预算」的唯一入口。
#[tokio::test]
async fn replan_preserves_attempt_counters() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    // 一条已消耗预算、正在退避的既有任务。
    {
        let connection = database.lock();
        let id = insert_task(
            &connection,
            &local_path,
            TransferState::BackingOff,
            TransferOperation::Create,
            Some(TransferErrorKind::Server),
        );
        connection
            .execute(
                "UPDATE transfer_queue SET attempt_count=3, verify_attempt_count=2,
                        next_retry_at=?1 WHERE id=?2",
                params![NOW_MS + 60_000, id],
            )
            .unwrap();
    }
    let (runner, _execute_calls) = build_runner(&database, &mount_root);

    // 规划产生同一路径但源快照已变的新意图：应原子替换意图并保留计数。
    let enqueued = runner.enqueue(replan_intent(&local_path, 1_000)).unwrap();
    assert_ne!(enqueued.task_id, 0);

    let row: (i32, i64, i64) = database
        .lock()
        .query_row(
            "SELECT state, attempt_count, verify_attempt_count FROM transfer_queue WHERE id=?1",
            [enqueued.task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, i32::from(TransferState::Pending));
    assert_eq!(row.1, 3, "自动重规划必须保留重试预算");
    assert_eq!(row.2, 2, "自动重规划必须保留核验预算");
}

/// 构造同一路径的规划意图（id=0/revision=0 的新意图）。
///
/// `mtime_offset_ms` 用来制造「源快照已变」：完全相同的意图会被准入层判定为
/// 同一任务并直接复用，不会走重规划分支。
fn replan_intent(local_path: &Path, mtime_offset_ms: i64) -> TransferTask {
    // 源快照与既有任务不同，才会触发重规划。
    let metadata = std::fs::metadata(local_path).unwrap();
    let source_mtime = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + mtime_offset_ms;
    TransferTask {
        direction: UPLOAD_DIRECTION,
        local_path: Some(local_path.to_str().unwrap().to_string()),
        name: FILE_NAME.to_string(),
        total_size: FILE_CONTENT.len() as i64,
        relative_path: Some(RELATIVE_PATH.to_string()),
        parent_file_id: Some("parent-folder-id".to_string()),
        operation: Some(i32::from(TransferOperation::Create)),
        source_mtime: Some(source_mtime),
        source_size: Some(FILE_CONTENT.len() as i64),
        created_at: 1,
        ..TransferTask::fresh_intent()
    }
}
