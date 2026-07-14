//! TaskRunner 远端结果核验恢复的集成测试。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use petal_link_lib::drive::models::DriveFile;
use petal_link_lib::error::{AppError, AppResult};
use petal_link_lib::sync::task_runner::{
    RemoteVerification, TaskActivityGate, TaskExecutionError, TaskExecutionOutcome,
    TaskProgressReporter, TaskRunner, TransferOperations, TransferTask,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};
use rusqlite::{params, Connection};

/// 测试时钟的固定当前时间。
const NOW_MS: i64 = 10_000;
/// 远端写入确认后的稳定 fileId。
const FILE_ID: &str = "verified-file-id";
/// 测试任务使用的相对路径。
const RELATIVE_PATH: &str = "contracts/verified.docx";
/// 测试任务使用的文件名。
const FILE_NAME: &str = "verified.docx";
/// 测试任务上传的固定内容。
const FILE_CONTENT: &[u8] = b"data";
/// 上传方向的持久化协议值。
const UPLOAD_DIRECTION: i32 = 0;

/// 返回固定远端确认结果并记录核验次数。
struct CommittedOperations {
    cloud_file: DriveFile,
    verification_calls: Arc<AtomicUsize>,
    active_activities: Arc<AtomicUsize>,
}

#[async_trait]
impl TransferOperations for CommittedOperations {
    /// 恢复核验用例不允许重新执行远端写入。
    async fn execute(
        &self,
        _task: &TransferTask,
        _progress: &TaskProgressReporter,
    ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
        Err(TaskExecutionError::App(AppError::generic(
            "恢复核验不应重新执行远端写入",
        )))
    }

    /// 返回已提交结果，并确认远端 GET 期间持有路径许可。
    async fn verify_remote(&self, _task: &TransferTask) -> AppResult<RemoteVerification> {
        self.verification_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.active_activities.load(Ordering::SeqCst), 1);
        Ok(RemoteVerification::Committed(self.cloud_file.clone()))
    }
}

/// 统计任务路径许可的获取次数与当前持有数量。
struct CountingActivityGate {
    active_activities: Arc<AtomicUsize>,
    begin_calls: Arc<AtomicUsize>,
}

/// 第二次准入失败，用于验证单任务错误不会丢弃先前恢复结果。
struct RejectSecondActivityGate {
    active_activities: Arc<AtomicUsize>,
    begin_calls: Arc<AtomicUsize>,
}

impl TaskActivityGate for RejectSecondActivityGate {
    /// 首个任务正常获取许可，后续任务模拟并发排他路径冲突。
    fn begin(&self, relative_path: Option<&str>) -> AppResult<Box<dyn Send>> {
        assert_eq!(relative_path, Some(RELATIVE_PATH));
        let call = self.begin_calls.fetch_add(1, Ordering::SeqCst);
        if call > 0 {
            return Err(AppError::generic("模拟第二个任务路径许可失败"));
        }
        self.active_activities.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ActivityLease {
            active_activities: self.active_activities.clone(),
        }))
    }
}

impl TaskActivityGate for CountingActivityGate {
    /// 获取许可时增加活动计数，guard 释放时自动减少。
    fn begin(&self, relative_path: Option<&str>) -> AppResult<Box<dyn Send>> {
        assert_eq!(relative_path, Some(RELATIVE_PATH));
        self.begin_calls.fetch_add(1, Ordering::SeqCst);
        self.active_activities.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ActivityLease {
            active_activities: self.active_activities.clone(),
        }))
    }
}

/// 离开恢复临界区时释放测试活动计数。
struct ActivityLease {
    active_activities: Arc<AtomicUsize>,
}

impl Drop for ActivityLease {
    /// 保证每次成功获取的许可只释放一次。
    fn drop(&mut self) {
        let previous = self.active_activities.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(previous, 1);
    }
}

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

/// 创建与本地源快照一致的 VerifyingRemote 更新任务。
fn verifying_task(local_path: &Path, next_retry_at: i64) -> TransferTask {
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
        file_id: Some(FILE_ID.to_string()),
        local_path: Some(local_path.to_str().unwrap().to_string()),
        name: FILE_NAME.to_string(),
        total_size: FILE_CONTENT.len() as i64,
        transferred: FILE_CONTENT.len() as i64,
        state: i32::from(TransferState::VerifyingRemote),
        error_message: Some("远端响应不确定".to_string()),
        created_at: 1,
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(RELATIVE_PATH.to_string()),
        parent_file_id: Some("contracts-folder-id".to_string()),
        operation: Some(i32::from(TransferOperation::Update)),
        source_mtime: Some(source_mtime),
        source_size: Some(FILE_CONTENT.len() as i64),
        expected_cloud_edited_time: Some(2_000),
        attempt_count: 1,
        next_retry_at: Some(next_retry_at),
        error_kind: Some(i32::from(TransferErrorKind::RemoteAmbiguous)),
        remote_result_file_id: Some(FILE_ID.to_string()),
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
                source_size, expected_cloud_edited_time, attempt_count, next_retry_at,
                error_kind, remote_result_file_id, state_revision
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
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
                task.next_retry_at,
                task.error_kind,
                task.remote_result_file_id,
                task.state_revision,
            ],
        )
        .unwrap();
    connection.last_insert_rowid()
}

/// 构造核验后可安全结算的完整远端元数据。
fn committed_cloud_file() -> DriveFile {
    DriveFile {
        id: FILE_ID.to_string(),
        name: FILE_NAME.to_string(),
        size: FILE_CONTENT.len() as i64,
        parent_folder: Some(vec!["contracts-folder-id".to_string()]),
        edited_time: chrono::DateTime::from_timestamp_millis(4_000),
        ..Default::default()
    }
}

/// 创建测试文件并返回挂载根与文件绝对路径。
fn create_local_source(temp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let mount_root = temp.path().join("mount");
    let local_path = mount_root.join(RELATIVE_PATH);
    std::fs::create_dir_all(local_path.parent().unwrap()).unwrap();
    std::fs::write(&local_path, FILE_CONTENT).unwrap();
    (mount_root, local_path)
}

/// 尚未到达 next_retry_at 时不得占用路径许可或发起远端核验。
#[tokio::test]
async fn future_verification_deadline_skips_remote_check() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let task_id = insert_task(&database.lock(), &verifying_task(&local_path, NOW_MS + 1));
    let active_activities = Arc::new(AtomicUsize::new(0));
    let begin_calls = Arc::new(AtomicUsize::new(0));
    let verification_calls = Arc::new(AtomicUsize::new(0));
    let operations = Arc::new(CommittedOperations {
        cloud_file: committed_cloud_file(),
        verification_calls: verification_calls.clone(),
        active_activities: active_activities.clone(),
    });
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root,
        operations,
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    );
    runner.set_activity_gate(Arc::new(CountingActivityGate {
        active_activities: active_activities.clone(),
        begin_calls: begin_calls.clone(),
    }));

    let summary = runner.resume_verifying().await.unwrap();

    assert_eq!(summary.completed, 0);
    assert!(summary.recovered_cloud_files.is_empty());
    assert_eq!(verification_calls.load(Ordering::SeqCst), 0);
    assert_eq!(begin_calls.load(Ordering::SeqCst), 0);
    assert_eq!(active_activities.load(Ordering::SeqCst), 0);
    let persisted: (i32, Option<i64>, i64) = database
        .lock()
        .query_row(
            "SELECT state, next_retry_at, state_revision FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, i32::from(TransferState::VerifyingRemote));
    assert_eq!(persisted.1, Some(NOW_MS + 1));
    assert_eq!(persisted.2, 0);
}

/// 到期且远端已提交时，任务与同步基线必须在路径许可内原子结算并返回云端结果。
#[tokio::test]
async fn due_committed_verification_settles_while_activity_is_held() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let task_id = insert_task(&database.lock(), &verifying_task(&local_path, NOW_MS));
    let active_activities = Arc::new(AtomicUsize::new(0));
    let begin_calls = Arc::new(AtomicUsize::new(0));
    let verification_calls = Arc::new(AtomicUsize::new(0));
    let state_sink_calls = Arc::new(AtomicUsize::new(0));
    let settled_while_active = Arc::new(AtomicBool::new(false));
    let sink_database = database.clone();
    let sink_active_activities = active_activities.clone();
    let sink_state_calls = state_sink_calls.clone();
    let sink_settled_while_active = settled_while_active.clone();
    let state_sink = Arc::new(move || {
        sink_state_calls.fetch_add(1, Ordering::SeqCst);
        let connection = sink_database.lock();
        let task_state: i32 = connection
            .query_row(
                "SELECT state FROM transfer_queue WHERE id=?1",
                [task_id],
                |row| row.get(0),
            )
            .map_err(|error| AppError::generic(format!("状态发布读取任务失败：{error}")))?;
        let baseline: (String, Option<i64>) = connection
            .query_row(
                "SELECT local_path, cloud_edited_time FROM sync_items WHERE file_id=?1",
                [FILE_ID],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| AppError::generic(format!("状态发布读取同步基线失败：{error}")))?;
        if sink_active_activities.load(Ordering::SeqCst) == 1
            && task_state == i32::from(TransferState::Completed)
            && baseline.0 == RELATIVE_PATH
            && baseline.1 == Some(4_000)
        {
            sink_settled_while_active.store(true, Ordering::SeqCst);
        }
        Ok(())
    });
    let cloud_file = committed_cloud_file();
    let operations = Arc::new(CommittedOperations {
        cloud_file: cloud_file.clone(),
        verification_calls: verification_calls.clone(),
        active_activities: active_activities.clone(),
    });
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root,
        operations,
        Arc::new(|| true),
        state_sink,
        None,
        Arc::new(|| NOW_MS),
    );
    runner.set_activity_gate(Arc::new(CountingActivityGate {
        active_activities: active_activities.clone(),
        begin_calls: begin_calls.clone(),
    }));

    let summary = runner.resume_verifying().await.unwrap();

    assert_eq!(summary.completed, 1);
    assert_eq!(summary.recovered_cloud_files.len(), 1);
    assert_eq!(
        summary.recovered_cloud_files[0].relative_path,
        RELATIVE_PATH
    );
    assert_eq!(summary.recovered_cloud_files[0].file.id, cloud_file.id);
    assert_eq!(
        summary.recovered_cloud_files[0]
            .file
            .edited_time
            .map(|time| time.timestamp_millis()),
        Some(4_000)
    );
    assert_eq!(verification_calls.load(Ordering::SeqCst), 1);
    assert_eq!(begin_calls.load(Ordering::SeqCst), 1);
    assert!(state_sink_calls.load(Ordering::SeqCst) >= 1);
    assert!(settled_while_active.load(Ordering::SeqCst));
    assert_eq!(active_activities.load(Ordering::SeqCst), 0);
    let connection = database.lock();
    let persisted: (i32, i64, Option<i64>) = connection
        .query_row(
            "SELECT state, transferred, next_retry_at FROM transfer_queue WHERE id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(persisted.0, i32::from(TransferState::Completed));
    assert_eq!(persisted.1, FILE_CONTENT.len() as i64);
    assert_eq!(persisted.2, None);
    let baseline: (String, String, Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT local_path, name, local_size, cloud_edited_time
             FROM sync_items WHERE file_id=?1",
            [FILE_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(baseline.0, RELATIVE_PATH);
    assert_eq!(baseline.1, FILE_NAME);
    assert_eq!(baseline.2, Some(FILE_CONTENT.len() as i64));
    assert_eq!(baseline.3, Some(4_000));
}

/// 后续任务准入失败时，先前已完成的任务及其云端结果仍必须返回给 checkpoint 提交方。
#[tokio::test]
async fn later_activity_failure_preserves_completed_recovery_summary() {
    let temp = tempfile::tempdir().unwrap();
    let (mount_root, local_path) = create_local_source(&temp);
    let database = open_database(&temp.path().join("state.db"));
    let mut first_task = verifying_task(&local_path, NOW_MS);
    first_task.created_at = 2;
    let first_task_id = insert_task(&database.lock(), &first_task);
    let mut second_task = verifying_task(&local_path, NOW_MS);
    second_task.created_at = 1;
    let second_task_id = insert_task(&database.lock(), &second_task);
    let active_activities = Arc::new(AtomicUsize::new(0));
    let begin_calls = Arc::new(AtomicUsize::new(0));
    let verification_calls = Arc::new(AtomicUsize::new(0));
    let operations = Arc::new(CommittedOperations {
        cloud_file: committed_cloud_file(),
        verification_calls: verification_calls.clone(),
        active_activities: active_activities.clone(),
    });
    let runner = TaskRunner::new_with_clock(
        database.clone(),
        mount_root,
        operations,
        Arc::new(|| true),
        Arc::new(|| Ok(())),
        None,
        Arc::new(|| NOW_MS),
    );
    runner.set_activity_gate(Arc::new(RejectSecondActivityGate {
        active_activities: active_activities.clone(),
        begin_calls: begin_calls.clone(),
    }));

    let summary = runner.resume_verifying().await.unwrap();

    assert_eq!(summary.completed, 1);
    assert_eq!(summary.recovered_cloud_files.len(), 1);
    assert_eq!(verification_calls.load(Ordering::SeqCst), 1);
    assert_eq!(begin_calls.load(Ordering::SeqCst), 2);
    assert_eq!(active_activities.load(Ordering::SeqCst), 0);
    let connection = database.lock();
    let first_state: i32 = connection
        .query_row(
            "SELECT state FROM transfer_queue WHERE id=?1",
            [first_task_id],
            |row| row.get(0),
        )
        .unwrap();
    let second_state: i32 = connection
        .query_row(
            "SELECT state FROM transfer_queue WHERE id=?1",
            [second_task_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_state, i32::from(TransferState::Completed));
    assert_eq!(second_state, i32::from(TransferState::VerifyingRemote));
}
