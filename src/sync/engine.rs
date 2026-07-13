//! 同步引擎主循环 —— 核心编排（阶段 5 骨架，后续阶段逐步接入 mount/executor/cloud_tree 完成闭环）。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart`。

use parking_lot::Mutex;
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CycleRequest(u32);

impl CycleRequest {
    const LOCAL_RESCAN: Self = Self(1 << 0);
    const CLOUD_INCREMENTAL: Self = Self(1 << 1);
    const CLOUD_FULL: Self = Self(1 << 2);
    const ONLINE_RECOVERY: Self = Self(1 << 3);
    const STARTUP: Self = Self(1 << 4);
    const RETRY: Self = Self(1 << 5);

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for CycleRequest {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Per-engine single owner for every source of scan/recovery work. Requests are recorded before
/// ownership is awaited, so an edge arriving while a cycle is active (or in its release window)
/// remains sticky and is consumed by the current owner or the next waiter.
#[derive(Default)]
struct CycleCoordinator {
    state: Mutex<CycleCoordinatorState>,
    owner: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct CycleCoordinatorState {
    pending: u32,
    requested: u64,
    completed: u64,
    expired_result_through: u64,
    failures: Vec<(u64, u64, String)>,
}

impl CycleCoordinator {
    fn request(&self, request: CycleRequest) -> u64 {
        let mut state = self.state.lock();
        state.requested = state.requested.wrapping_add(1).max(1);
        state.pending |= request.0;
        state.requested
    }

    async fn lock_owner(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.owner.lock().await
    }

    #[cfg(test)]
    fn take_pending(&self) -> CycleRequest {
        self.take_pending_with_sequence().0
    }

    fn take_pending_with_sequence(&self) -> (CycleRequest, u64) {
        let mut state = self.state.lock();
        let request = CycleRequest(state.pending);
        state.pending = 0;
        (request, state.requested)
    }

    fn restore(&self, request: CycleRequest) {
        self.state.lock().pending |= request.0;
    }

    #[cfg(test)]
    fn requested_sequence(&self) -> u64 {
        self.state.lock().requested
    }

    fn complete(&self, through: u64, error: Option<&AppError>) {
        let mut state = self.state.lock();
        let previous = state.completed;
        state.completed = state.completed.max(through);
        if let Some(error) = error {
            state
                .failures
                .push((previous.saturating_add(1), through, error.to_string()));
            if state.failures.len() > 128 {
                let excess = state.failures.len() - 128;
                let expired_through = state
                    .failures
                    .iter()
                    .take(excess)
                    .map(|(_, end, _)| *end)
                    .max()
                    .unwrap_or(state.expired_result_through);
                state.expired_result_through = state.expired_result_through.max(expired_through);
                state.failures.drain(..excess);
            }
        }
    }

    fn result_if_completed(&self, sequence: u64) -> Option<AppResult<()>> {
        let state = self.state.lock();
        if state.completed < sequence {
            return None;
        }
        if sequence <= state.expired_result_through {
            return Some(Err(AppError::generic("同步周期结果历史已过期")));
        }
        let failure = state
            .failures
            .iter()
            .find(|(start, end, _)| *start <= sequence && sequence <= *end)
            .map(|(_, _, message)| message.clone());
        Some(match failure {
            Some(message) => Err(AppError::generic(message)),
            None => Ok(()),
        })
    }

    fn is_idle(&self) -> bool {
        self.state.lock().pending == 0 && self.owner.try_lock().is_ok()
    }

    fn has_pending(&self) -> bool {
        self.state.lock().pending != 0
    }

    fn has_uncompleted_request(&self) -> bool {
        let state = self.state.lock();
        state.requested > state.completed
    }

    #[cfg(test)]
    async fn run<F, Fut>(&self, request: CycleRequest, mut execute: F)
    where
        F: FnMut(CycleRequest) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        self.request(request);
        let _owner = self.lock_owner().await;
        loop {
            let pending = self.take_pending();
            if pending.is_empty() {
                break;
            }
            execute(pending).await;
        }
    }
}

#[derive(Default)]
struct ActivityState {
    accepting: bool,
    count: usize,
}

struct ActivityTracker {
    state: Mutex<ActivityState>,
    idle: tokio::sync::Notify,
}

impl Default for ActivityTracker {
    fn default() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                accepting: true,
                count: 0,
            }),
            idle: tokio::sync::Notify::new(),
        }
    }
}

impl ActivityTracker {
    fn begin(self: &Arc<Self>) -> AppResult<ActivityGuard> {
        let mut state = self.state.lock();
        if !state.accepting {
            return Err(AppError::generic("同步引擎已停止，拒绝新传输活动"));
        }
        state.count += 1;
        Ok(ActivityGuard {
            tracker: self.clone(),
        })
    }

    fn close(&self) {
        self.state.lock().accepting = false;
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.state.lock().count == 0 {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) struct ActivityGuard {
    tracker: Arc<ActivityTracker>,
}

pub(crate) struct FolderSyncGuard {
    engine: Arc<SyncEngine>,
}

impl Drop for FolderSyncGuard {
    fn drop(&mut self) {
        self.engine.end_folder_sync();
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut state = self.tracker.state.lock();
        state.count = state.count.saturating_sub(1);
        if state.count == 0 {
            self.tracker.idle.notify_waiters();
        }
    }
}

struct TaskRunnerActivityGate(Arc<ActivityTracker>);

impl crate::sync::task_runner::TaskActivityGate for TaskRunnerActivityGate {
    fn begin(&self) -> AppResult<Box<dyn Send>> {
        Ok(Box::new(self.0.begin()?))
    }
}

async fn network_listener_loop<L, R>(
    mut transitions: broadcast::Receiver<crate::core::net_guard::NetworkTransition>,
    is_online: L,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    reconcile_initial: bool,
    mut request_recovery: R,
) where
    L: Fn() -> bool,
    R: FnMut(),
{
    if *shutdown.borrow() {
        return;
    }
    // The caller creates the subscription before reading the current level. Reconcile the level
    // once here so an Online edge sent before listener startup is never required for progress.
    if reconcile_initial && is_online() {
        request_recovery();
    }
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            transition = transitions.recv() => {
                match transition {
                    Ok(crate::core::net_guard::NetworkTransition::Online) => {
                        if is_online() {
                            request_recovery();
                        }
                    }
                    Ok(crate::core::net_guard::NetworkTransition::Offline) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "网络转换 listener 滞后，按当前 level 收敛");
                        if is_online() {
                            request_recovery();
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Long-lived watcher receiver. A lagged broadcast means path-level details were lost, so the
/// only safe convergence action is one coalesced full rescan; unlike `Closed`, it must not kill
/// the listener because later filesystem batches are still authoritative triggers.
async fn watcher_listener_loop<R>(
    mut changes: broadcast::Receiver<crate::mount::local_watcher::ChangeSet>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    mut request_rescan: R,
) where
    R: FnMut(),
{
    if *shutdown.borrow() {
        return;
    }
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            change = changes.recv() => {
                match change {
                    Ok(_) => request_rescan(),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "watcher listener 滞后，请求完整补偿扫描");
                        request_rescan();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

use crate::data::repository;
use crate::drive::download_api::DownloadApi;
use crate::drive::files_api::FilesApi;
use crate::drive::models::DriveFile;
use crate::drive::upload_api::UploadApi;
use crate::error::{AppError, AppResult};
use crate::mount::local_watcher::LocalWatcher;
use crate::mount::manager::{LocalFileEntry, MountManager};
use crate::sync::cloud_tree;
use crate::sync::conflict::ConflictResolver;
use crate::sync::executor::SyncExecutor;
use crate::sync::planner::{DbSnapshotEntry, SyncPlanner, SyncSnapshot};
use crate::sync::state::{FreeUpCheckResult, SyncGlobalState};
use crate::sync::status_aggregator::{RuntimeStatus, StatusAggregator};
use crate::sync::task_runner::{TaskDisposition, TaskRunner};
use crate::sync::transfer_state::TransferState;

#[cfg(test)]
type RetryPreparedHook = Arc<dyn Fn(i64) + Send + Sync>;

#[async_trait::async_trait]
trait StartCursorSource: Send + Sync {
    async fn get_start_cursor(&self) -> AppResult<String>;
}

#[async_trait::async_trait]
impl StartCursorSource for crate::drive::changes_api::ChangesApi {
    async fn get_start_cursor(&self) -> AppResult<String> {
        crate::drive::changes_api::ChangesApi::get_start_cursor(self).await
    }
}

/// 增量同步安全网：连续走 N 次增量后强制一次全量 BFS，纠正改名/移动/新建文件的累积偏差。
/// 增量 merge 无法处理"已知 id 但 rel_path 变了"（改名/移动）和"全新文件"，需定期全量收敛。
/// 配合自动刷新间隔（默认 60s）：300 次 × 60s = 5 小时强制一次全量纠偏。
const INCREMENTAL_FORCED_FULL_THRESHOLD: u32 = 300;
const RECOVERABLE_CYCLE_RETRY_MAX_SECS: u64 = 32;

fn recoverable_cycle_retry_delay(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(5);
    Duration::from_secs((1_u64 << exponent).min(RECOVERABLE_CYCLE_RETRY_MAX_SECS))
}

/// Resets a lifecycle gate on every return path, including cancellation and panic unwind.
struct ResetFlag<'a> {
    flag: &'a Mutex<bool>,
}

impl<'a> ResetFlag<'a> {
    fn new(flag: &'a Mutex<bool>) -> Self {
        Self { flag }
    }
}

impl Drop for ResetFlag<'_> {
    fn drop(&mut self) {
        *self.flag.lock() = false;
    }
}

pub struct SyncEngine {
    files_api: Arc<FilesApi>,
    changes_api: Arc<crate::drive::changes_api::ChangesApi>,
    start_cursor_source: Arc<dyn StartCursorSource>,
    #[allow(dead_code)]
    download_api: Arc<DownloadApi>,
    #[allow(dead_code)]
    upload_api: Arc<UploadApi>,
    mount: Option<Arc<MountManager>>,
    db: Arc<Mutex<Connection>>,
    planner: SyncPlanner,
    #[allow(dead_code)]
    conflict: Arc<Mutex<ConflictResolver>>,
    executor: Option<SyncExecutor>,
    task_runner: Option<Arc<TaskRunner>>,
    cycle: CycleCoordinator,
    syncing: Mutex<bool>,
    /// 目录递归同步（sync_folder_recursive）互斥锁。
    /// 独立于 syncing：folder sync 不被启动/常规 sync cycle 的 syncing 锁阻塞
    /// （启动 scan 可能耗时几十秒，不该挡住用户点目录同步）；run_sync_cycle 会检查本锁跳过，
    /// 避免 watcher cycle 与 folder sync 并发竞争本地文件/DB。
    folder_syncing: Mutex<bool>,
    cloud_tree: Mutex<HashMap<String, DriveFile>>,
    path_to_id: Mutex<HashMap<String, String>>,
    root_folder_id: Mutex<Option<String>>,
    cloud_cursor: Mutex<Option<String>>,
    /// `true` only when the live tree came from a complete, crash-consistent checkpoint.
    /// Failed/partial refreshes keep the previous tree for display but revoke destructive trust.
    cloud_tree_trusted: AtomicBool,
    recently_deleted_paths: Mutex<HashMap<String, i64>>,
    state: Mutex<SyncGlobalState>,
    status_aggregator: Arc<StatusAggregator>,
    running: Mutex<bool>,
    mount_dir: Mutex<Option<String>>,
    skip_patterns: Vec<String>,
    debounce_secs: u32,
    /// 云端定时刷新间隔（秒）。0 = 关闭。到期后全量 BFS 重拉云端树，使云端变更自动同步到本地。
    poll_interval_secs: u32,
    state_tx: broadcast::Sender<SyncGlobalState>,
    #[allow(dead_code)]
    is_first_time: Mutex<bool>,
    /// 本地监听器句柄（保活，防止 FSEvents 提前释放）
    watcher: Mutex<Option<Arc<LocalWatcher>>>,
    /// 是否已 shutdown。detached watcher 任务每次 cycle 前检查此标志，
    /// 置位后退出循环，防止引擎被替换后旧 watcher 仍触发 sync cycle（误判上传）。
    shutdown: Mutex<bool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    backoff_changed: tokio::sync::Notify,
    schedule_revision: AtomicU64,
    started: AtomicBool,
    online_check: Arc<dyn Fn() -> bool + Send + Sync>,
    request_network_failure_reporter: Arc<dyn Fn() -> bool + Send + Sync>,
    known_waiting_count: Mutex<Option<(u64, u64)>>,
    cycle_observer: Arc<dyn Fn(&'static str) + Send + Sync>,
    #[cfg(test)]
    incremental_refresh_hook: Option<Arc<dyn Fn() -> AppResult<()> + Send + Sync>>,
    #[cfg(test)]
    startup_cloud_hook: Option<Arc<dyn Fn() -> AppResult<()> + Send + Sync>>,
    #[cfg(test)]
    retry_prepared_hook: Mutex<Option<RetryPreparedHook>>,
    activity: Arc<ActivityTracker>,
    background_scheduled: AtomicBool,
    /// 连续增量刷新计数。达 INCREMENTAL_FORCED_FULL_THRESHOLD 后强制一次全量 BFS，
    /// 纠正增量无法处理的改名/移动/新建文件累积偏差。全量后归零。
    incremental_since_full: AtomicU32,
}

impl SyncEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        files_api: Arc<FilesApi>,
        changes_api: Arc<crate::drive::changes_api::ChangesApi>,
        download_api: Arc<DownloadApi>,
        upload_api: Arc<UploadApi>,
        db: Arc<Mutex<Connection>>,
        status_aggregator: Arc<StatusAggregator>,
        skip_patterns: Vec<String>,
        debounce_secs: u32,
        poll_interval_secs: u32,
    ) -> Self {
        let (state_tx, _) = broadcast::channel(256);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let start_cursor_source = changes_api.clone();
        Self {
            files_api,
            changes_api,
            start_cursor_source,
            download_api,
            upload_api,
            mount: None,
            db,
            planner: SyncPlanner,
            conflict: Arc::new(Mutex::new(ConflictResolver::new())),
            executor: None,
            task_runner: None,
            cycle: CycleCoordinator::default(),
            syncing: Mutex::new(false),
            folder_syncing: Mutex::new(false),
            cloud_tree: Mutex::new(HashMap::new()),
            path_to_id: Mutex::new(HashMap::new()),
            root_folder_id: Mutex::new(None),
            cloud_cursor: Mutex::new(None),
            cloud_tree_trusted: AtomicBool::new(false),
            recently_deleted_paths: Mutex::new(HashMap::new()),
            state: Mutex::new(SyncGlobalState::default()),
            status_aggregator,
            running: Mutex::new(false),
            mount_dir: Mutex::new(None),
            skip_patterns,
            debounce_secs,
            poll_interval_secs,
            state_tx,
            is_first_time: Mutex::new(true),
            watcher: Mutex::new(None),
            shutdown: Mutex::new(false),
            shutdown_tx,
            backoff_changed: tokio::sync::Notify::new(),
            schedule_revision: AtomicU64::new(0),
            started: AtomicBool::new(false),
            online_check: Arc::new(crate::core::net_guard::is_online),
            request_network_failure_reporter: Arc::new(|| {
                crate::core::net_guard::report_request_network_failure()
            }),
            known_waiting_count: Mutex::new(None),
            cycle_observer: Arc::new(|_| {}),
            #[cfg(test)]
            incremental_refresh_hook: None,
            #[cfg(test)]
            startup_cloud_hook: None,
            #[cfg(test)]
            retry_prepared_hook: Mutex::new(None),
            activity: Arc::new(ActivityTracker::default()),
            background_scheduled: AtomicBool::new(false),
            incremental_since_full: AtomicU32::new(0),
        }
    }

    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        *self.mount_dir.lock() = Some(mount.mount_dir().to_string_lossy().to_string());
        self.mount = Some(mount);
    }

    pub fn set_executor(&mut self, mut executor: SyncExecutor) {
        let activity_gate = Arc::new(TaskRunnerActivityGate(self.activity.clone()));
        executor.set_action_activity_gate(activity_gate.clone());
        self.task_runner = executor.task_runner().ok();
        if let Some(task_runner) = &self.task_runner {
            task_runner.set_activity_gate(activity_gate);
        }
        self.executor = Some(executor);
    }

    pub fn set_online_check(&mut self, online_check: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.online_check = online_check;
    }

    #[cfg(test)]
    fn set_request_network_failure_reporter_for_test(
        &mut self,
        reporter: Arc<dyn Fn() -> bool + Send + Sync>,
    ) {
        self.request_network_failure_reporter = reporter;
    }

    pub fn set_cycle_observer(&mut self, cycle_observer: Arc<dyn Fn(&'static str) + Send + Sync>) {
        self.cycle_observer = cycle_observer;
    }

    #[cfg(test)]
    fn set_incremental_refresh_hook_for_test(
        &mut self,
        hook: Arc<dyn Fn() -> AppResult<()> + Send + Sync>,
    ) {
        self.incremental_refresh_hook = Some(hook);
    }

    #[cfg(test)]
    fn set_startup_cloud_hook_for_test(
        &mut self,
        hook: Arc<dyn Fn() -> AppResult<()> + Send + Sync>,
    ) {
        self.startup_cloud_hook = Some(hook);
    }

    #[cfg(test)]
    fn install_retry_prepared_hook_for_test(&self, hook: RetryPreparedHook) {
        *self.retry_prepared_hook.lock() = Some(hook);
    }

    fn is_online(&self) -> bool {
        (self.online_check)()
    }

    pub(crate) fn begin_external_activity(&self) -> AppResult<ActivityGuard> {
        self.activity.begin()
    }

    pub(crate) fn task_runner(&self) -> AppResult<Arc<TaskRunner>> {
        self.task_runner
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 未初始化"))
    }

    pub(crate) fn bind_task_runner_state_sink(self: &Arc<Self>, task_runner: &Arc<TaskRunner>) {
        let weak_engine = Arc::downgrade(self);
        task_runner.set_state_sink(Arc::new(move || {
            let engine = weak_engine
                .upgrade()
                .ok_or_else(|| AppError::generic("同步引擎已停止"))?;
            match engine.recompute_and_broadcast_state() {
                Ok(snapshot) => {
                    engine.note_transfer_state_changed(snapshot.revision, snapshot.waiting_network);
                    Ok(())
                }
                Err(error) => {
                    engine.notify_backoff_schedule_changed();
                    Err(error)
                }
            }
        }));
        self.initialize_known_waiting_count();
    }

    pub(crate) fn notify_backoff_schedule_changed(&self) {
        self.schedule_revision.fetch_add(1, Ordering::AcqRel);
        self.backoff_changed.notify_one();
    }

    fn initialize_known_waiting_count(&self) {
        let baseline = match self.recompute_and_broadcast_state() {
            Ok(snapshot) => (snapshot.revision, snapshot.waiting_network),
            Err(error) => {
                tracing::warn!(%error, "初始化等待网络任务边沿失败，使用保守基线");
                let has_waiting = repository::has_transfer_in_state(
                    &self.db.lock(),
                    TransferState::WaitingForNetwork,
                )
                .unwrap_or(true);
                (
                    self.current_state().revision,
                    if has_waiting { u64::MAX } else { 0 },
                )
            }
        };
        let mut known = self.known_waiting_count.lock();
        let is_newer = match *known {
            Some((revision, _)) => baseline.0 > revision,
            None => true,
        };
        if is_newer {
            *known = Some(baseline);
        }
    }

    pub(crate) fn note_transfer_state_changed(&self, revision: u64, waiting_count: u64) {
        self.notify_backoff_schedule_changed();
        let waiting_increased = {
            let mut known = self.known_waiting_count.lock();
            match *known {
                Some((known_revision, known_count)) if revision > known_revision => {
                    let increased = waiting_count > known_count;
                    *known = Some((revision, waiting_count));
                    increased
                }
                None => {
                    *known = Some((revision, waiting_count));
                    false
                }
                Some(_) => false,
            }
        };
        if waiting_increased && self.is_online() {
            (self.request_network_failure_reporter)();
        }
    }
    pub fn state_receiver(&self) -> broadcast::Receiver<SyncGlobalState> {
        self.state_tx.subscribe()
    }
    pub fn current_state(&self) -> SyncGlobalState {
        self.state.lock().clone()
    }

    /// Recompute and publish one complete authoritative snapshot.
    pub fn recompute_and_broadcast_state(&self) -> AppResult<SyncGlobalState> {
        self.update_runtime_and_broadcast(|_| {})
    }

    /// Apply an explicit runtime transition, then rebuild and publish every persisted field.
    pub(crate) fn update_runtime_and_broadcast(
        &self,
        update: impl FnOnce(&mut RuntimeStatus),
    ) -> AppResult<SyncGlobalState> {
        let _publish_guard = self.status_aggregator.lock_publication();
        if *self.shutdown.lock() {
            return Err(AppError::generic("同步引擎已停止，拒绝发布状态"));
        }
        let mut runtime = RuntimeStatus::from(&*self.state.lock());
        update(&mut runtime);
        let snapshot_result = {
            let conn = self.db.lock();
            self.status_aggregator.snapshot(&conn, runtime.clone())
        };
        let snapshot = match snapshot_result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                runtime.apply_to(&mut self.state.lock());
                return Err(error);
            }
        };
        *self.state.lock() = snapshot.clone();
        let _ = self.state_tx.send(snapshot.clone());
        Ok(snapshot)
    }

    fn restore_idle_runtime_after_error(&self) {
        let _ = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        });
    }

    /// Accept one permanent transfer failure for retry without touching its success baseline.
    ///
    /// The transfer lifecycle and compatibility sync status change in one SQLite transaction.
    /// Task execution remains the responsibility of the existing runner path.
    #[cfg(test)]
    fn accept_failed_transfer_retry(&self, task_id: i64) -> AppResult<repository::TransferTask> {
        let pending = {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始重试事务失败：{error}")))?;
            let current = transaction
                .query_row(
                    "SELECT * FROM transfer_queue WHERE id=?1",
                    params![task_id],
                    repository::TransferTask::from_row,
                )
                .optional()
                .map_err(|error| AppError::generic(format!("查询传输任务失败：{error}")))?
                .ok_or_else(|| AppError::generic("任务不存在"))?;
            let current_state = current
                .state_kind()
                .map_err(|error| AppError::generic(error.to_string()))?;
            if current_state != TransferState::Failed {
                return Err(AppError::generic("任务不存在或非失败状态"));
            }
            let relative_path = current
                .relative_path
                .as_deref()
                .ok_or_else(|| AppError::generic("任务缺少相对路径，无法安全重试"))?;
            crate::core::paths::validate_relative_path(relative_path, false)?;

            let changed = transaction
                .execute(
                    "UPDATE transfer_queue SET
                        state=?1,
                        error_kind=NULL,
                        error_message=NULL,
                        next_retry_at=NULL,
                        finished_at=NULL,
                        state_revision=state_revision+1
                     WHERE id=?2 AND state=?3 AND state_revision=?4",
                    params![
                        i32::from(TransferState::Pending),
                        task_id,
                        i32::from(TransferState::Failed),
                        current.state_revision,
                    ],
                )
                .map_err(|error| AppError::generic(format!("重置传输任务失败：{error}")))?;
            if changed != 1 {
                return Err(AppError::generic("传输任务状态已变化，请刷新后重试"));
            }
            transaction
                .execute(
                    "UPDATE sync_items
                     SET status=?1, error_message=NULL
                     WHERE local_path=?2 AND status=?3",
                    params![
                        repository::sync_status::SYNCING,
                        relative_path,
                        repository::sync_status::FAILED,
                    ],
                )
                .map_err(|error| AppError::generic(format!("重置同步失败状态失败：{error}")))?;
            let updated = transaction
                .query_row(
                    "SELECT * FROM transfer_queue WHERE id=?1",
                    params![task_id],
                    repository::TransferTask::from_row,
                )
                .map_err(|error| AppError::generic(format!("读取重试任务失败：{error}")))?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交重试事务失败：{error}")))?;
            updated
        };
        self.recompute_and_broadcast_state()?;
        Ok(pending)
    }

    /// Apply one optimistic typed lifecycle transition and publish the resulting full snapshot.
    #[cfg(test)]
    fn transition_transfer_and_broadcast(
        &self,
        task_id: i64,
        expected_revision: i64,
        next_state: TransferState,
        patch: repository::TransferPatch,
    ) -> AppResult<repository::TransferTask> {
        let updated = {
            let conn = self.db.lock();
            repository::transition_transfer(&conn, task_id, expected_revision, next_state, patch)
                .map_err(|error| AppError::generic(error.to_string()))?
        };
        self.recompute_and_broadcast_state()?;
        Ok(updated)
    }

    /// Persist a retry's permanent failure in both task history and compatibility sync status.
    #[cfg(test)]
    fn record_retry_failure_and_broadcast(
        &self,
        task_id: i64,
        expected_revision: i64,
        error_message: &str,
        finished_at: i64,
    ) -> AppResult<SyncGlobalState> {
        {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始重试失败结算事务失败：{error}")))?;
            let failed = repository::transition_transfer_in_transaction(
                &transaction,
                task_id,
                expected_revision,
                TransferState::Failed,
                repository::TransferPatch {
                    error_kind: repository::ColumnPatch::Set(
                        crate::sync::transfer_state::TransferErrorKind::Unknown,
                    ),
                    error_message: repository::ColumnPatch::Set(error_message.to_string()),
                    next_retry_at: repository::ColumnPatch::Clear,
                    finished_at: repository::ColumnPatch::Set(finished_at),
                    ..Default::default()
                },
            )
            .map_err(|error| AppError::generic(error.to_string()))?;
            let relative_path = failed
                .relative_path
                .as_deref()
                .ok_or_else(|| AppError::generic("任务缺少相对路径，无法记录重试失败"))?;
            transaction
                .execute(
                    "UPDATE sync_items
                 SET status=?1, error_message=?2
                 WHERE local_path=?3 AND status=?4",
                    params![
                        repository::sync_status::FAILED,
                        error_message,
                        relative_path,
                        repository::sync_status::SYNCING,
                    ],
                )
                .map_err(|error| AppError::generic(format!("记录重试失败状态失败：{error}")))?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交重试失败结算失败：{error}")))?;
        }
        self.recompute_and_broadcast_state()
    }

    /// Atomically confirm a retry task and replace its successful sync baseline.
    #[cfg(test)]
    fn settle_retry_success_and_broadcast(
        &self,
        task_id: i64,
        expected_revision: i64,
        cloud_file: &DriveFile,
        finished_at: i64,
    ) -> AppResult<SyncGlobalState> {
        {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始重试成功结算事务失败：{error}")))?;
            let completed = repository::transition_transfer_in_transaction(
                &transaction,
                task_id,
                expected_revision,
                TransferState::Completed,
                repository::TransferPatch {
                    error_kind: repository::ColumnPatch::Clear,
                    error_message: repository::ColumnPatch::Clear,
                    next_retry_at: repository::ColumnPatch::Clear,
                    finished_at: repository::ColumnPatch::Set(finished_at),
                    remote_result_file_id: repository::ColumnPatch::Set(cloud_file.id.clone()),
                    transferred: Some(cloud_file.size),
                    ..Default::default()
                },
            )
            .map_err(|error| AppError::generic(error.to_string()))?;
            let relative_path = completed
                .relative_path
                .as_deref()
                .ok_or_else(|| AppError::generic("任务缺少相对路径，无法结算重试"))?;
            transaction
                .execute(
                    "DELETE FROM sync_items WHERE local_path=?1",
                    params![relative_path],
                )
                .map_err(|error| AppError::generic(format!("替换重试成功基线失败：{error}")))?;
            repository::upsert(
                &transaction,
                &repository::SyncItem {
                    file_id: cloud_file.id.clone(),
                    local_path: relative_path.to_string(),
                    parent_folder_id: completed.parent_file_id.clone(),
                    name: completed.name.clone(),
                    is_folder: false,
                    size: cloud_file.size,
                    local_size: completed.source_size.or(Some(completed.total_size)),
                    sha256: None,
                    local_mtime: completed.source_mtime,
                    cloud_edited_time: cloud_file.edited_time.map(|time| time.timestamp_millis()),
                    last_sync_time: Some(finished_at),
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                },
            )?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交重试成功结算失败：{error}")))?;
        }
        self.recompute_and_broadcast_state()
    }

    /// Delete selected terminal transfer history without changing sync baseline facts.
    pub(crate) fn clear_transfer_history_and_broadcast(
        &self,
        include_completed: bool,
        include_failed: bool,
    ) -> AppResult<SyncGlobalState> {
        {
            let conn = self.db.lock();
            conn.execute(
                "DELETE FROM transfer_queue
                 WHERE (?1=1 AND state=?2) OR (?3=1 AND state=?4)",
                params![
                    include_completed as i32,
                    i32::from(TransferState::Completed),
                    include_failed as i32,
                    i32::from(TransferState::Failed),
                ],
            )
            .map_err(|error| AppError::generic(format!("清除传输历史失败：{error}")))?;
        }
        self.recompute_and_broadcast_state()
    }
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }

    /// 尝试获取 folder_syncing 锁（供 sync_folder_recursive 防并发用，独立于 syncing）。
    /// 已有目录同步进行中 → false；否则置 true 并返回 true。调用方负责在 finally 调 end_folder_sync。
    pub fn try_begin_folder_sync(&self) -> bool {
        let mut g = self.folder_syncing.lock();
        if *g || *self.syncing.lock() || *self.shutdown.lock() {
            return false;
        }
        *g = true;
        true
    }

    pub(crate) fn try_begin_folder_sync_guard(self: &Arc<Self>) -> Option<FolderSyncGuard> {
        self.try_begin_folder_sync().then(|| FolderSyncGuard {
            engine: self.clone(),
        })
    }
    /// 释放 folder_syncing 锁（与 try_begin_folder_sync 配对）。
    pub fn end_folder_sync(self: &Arc<Self>) {
        *self.folder_syncing.lock() = false;
        if !self.cycle.is_idle() {
            self.request_cycle_background("local-watcher");
        }
    }

    /// 停止引擎：停 watcher（释放 FSEvents）+ 置 shutdown 标志（detached watcher 任务退出）。
    ///
    /// 必须在引擎被替换（换目录/换账号）或退出前调用。之前只 `drop_runtime()` 清全局指针，
    /// 但 detached watcher 任务持有 `Arc<SyncEngine>` 克隆，引擎永不被 drop → 旧 watcher
    /// 持续监听 FSEvents，向旧（cloud_tree 已过时的）引擎触发 sync cycle → 误判「本地新建」
    /// 疯狂上传。本方法确保旧 watcher 真正停止。
    pub async fn shutdown(&self) {
        let watcher = self.watcher.lock().take();
        self.activity.close();
        self.shutdown_sync();
        if let Some(watcher) = watcher {
            watcher.stop().await;
        }
        // If a remote call was already submitted, TaskRunner owns its settlement. Waiting for the
        // cycle owner here prevents a replacement from starting until that settlement converges.
        let _quiesced = self.cycle.lock_owner().await;
        self.activity.wait_idle().await;
        let late_watcher = self.watcher.lock().take();
        if let Some(late_watcher) = late_watcher {
            late_watcher.stop().await;
        }
    }

    /// shutdown 的同步子集：仅置 shutdown 标志 + drop watcher 句柄（同步释放 FSEvents）。
    /// 供不能 await 的同步上下文（drop_runtime / shutdown.rs 线程）调用。
    /// drop RecommendedWatcher 会同步关闭底层 FSEvents stream → 不再有事件回调，
    /// detached watcher 任务下次循环见 shutdown 标志退出。
    pub fn shutdown_sync(&self) {
        self.shutdown_sync_with_contention_hook(|| {});
    }

    fn shutdown_sync_with_contention_hook(&self, on_publication_contention: impl FnOnce()) {
        self.activity.close();
        // 与状态发布共用同一屏障：先开始的发布可以完成；本方法返回后，旧引擎的
        // 后续 closure 只能在屏障内看到 shutdown=true，并在分配 revision 前失败。
        {
            let _publish_guard = self
                .status_aggregator
                .lock_publication_with_contention_hook(on_publication_contention);
            *self.shutdown.lock() = true;
        }
        let _ = self.shutdown_tx.send(true);
        self.backoff_changed.notify_waiters();
        // take 出 watcher 并 drop（同步释放 FSEvents 句柄）
        let taken = self.watcher.lock().take();
        drop(taken);
        tracing::info!("SyncEngine shutdown_sync（shutdown 标志置位、FSEvents 释放）");
    }

    /// 向云端树缓存插入一项（folder sync 上传后更新，避免下个 cycle 因旧树误判重复上传/删除）。
    pub fn cloud_tree_insert(&self, rel: String, file: DriveFile) {
        self.cloud_tree.lock().insert(rel, file);
    }
    /// 向 path→id 映射插入一项（folder sync 上传后更新）。
    pub fn path_to_id_insert(&self, rel: String, id: String) {
        self.path_to_id.lock().insert(rel, id);
    }
    /// 从云端树缓存删除一项（客户端删除后放逐，防下一轮 cycle 重建占位符）。
    pub fn cloud_tree_remove(&self, rel: &str) {
        self.cloud_tree.lock().remove(rel);
    }
    /// 从 path→id 映射删除一项。
    pub fn path_to_id_remove(&self, rel: &str) {
        self.path_to_id.lock().remove(rel);
    }
    /// 记入防振荡集（客户端删除后加锁，防 watcher cycle 误判）。
    pub fn add_recently_deleted(&self, rel: &str) {
        self.recently_deleted_paths
            .lock()
            .insert(rel.to_string(), chrono::Utc::now().timestamp_millis());
    }
    /// 获取 cloud_tree 的可变锁（供 commands 层清理用）。
    pub fn cloud_tree_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, DriveFile>> {
        self.cloud_tree.lock()
    }
    pub(crate) fn cloud_tree_is_trusted(&self) -> bool {
        self.cloud_tree_trusted.load(Ordering::Acquire)
    }
    fn set_cloud_tree_trusted(&self, trusted: bool) {
        self.cloud_tree_trusted.store(trusted, Ordering::Release);
    }
    fn install_cloud_checkpoint(&self, checkpoint: cloud_tree::CloudTreeCache) {
        self.set_cloud_tree_trusted(false);
        *self.cloud_tree.lock() = checkpoint.tree;
        *self.path_to_id.lock() = checkpoint.path_to_id;
        *self.root_folder_id.lock() = checkpoint.root_folder_id;
        *self.cloud_cursor.lock() = checkpoint.cursor;
        self.set_cloud_tree_trusted(true);
    }
    /// 获取 path_to_id 的可变锁（供 commands 层清理用）。
    pub fn path_to_id_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, String>> {
        self.path_to_id.lock()
    }

    /// 启动引擎。
    pub async fn start(self: &Arc<Self>) -> AppResult<()> {
        // Subscribe before reading current network level. The receiver is started after startup
        // reconciliation so buffered edges cannot race the initial cloud snapshot.
        let network_transitions = crate::core::net_guard::subscribe();
        self.start_with_network_receiver(network_transitions, true)
            .await
    }

    async fn start_with_network_receiver(
        self: &Arc<Self>,
        network_transitions: broadcast::Receiver<crate::core::net_guard::NetworkTransition>,
        start_probe: bool,
    ) -> AppResult<()> {
        // 启动前检查 shutdown 标志
        if *self.shutdown.lock() {
            tracing::info!("引擎已 shutdown，跳过启动");
            return Ok(());
        }
        *self.running.lock() = true;

        if let Err(error) = self.update_runtime_and_broadcast(|runtime| runtime.is_running = true) {
            self.restore_idle_runtime_after_error();
            return Err(error);
        }

        // ★ 启动网络探测任务 + 初始化睡眠处理（必须在 tokio 运行时上下文中，
        //   start_probe_task 内部 tokio::spawn 需要 reactor，不能在同步 setup 闭包中调用）。
        if start_probe {
            crate::core::net_guard::start_probe_task();
            crate::core::net_guard::init_sleep_handling();
        }

        // Startup recovery, cloud refresh and first local reconciliation use the same owner as
        // every later trigger. Requests arriving before this point remain pending and are folded
        // into the startup or its single follow-up.
        self.run_sync_cycle("startup-resume").await?;
        self.ensure_cycle_active()?;

        // Publish startup idle before arming any source that can enqueue a new cycle. Buffered
        // network edges are preserved by the already-created receiver and run after this point.
        self.update_runtime_and_broadcast(|runtime| {
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        })?;
        self.started.store(true, Ordering::Release);

        self.start_network_listener(network_transitions);
        self.ensure_cycle_active()?;

        // BFS 后启动 watcher
        self.start_watcher().await;
        self.ensure_cycle_active()?;

        // 启动云端定时刷新任务（poll_interval_secs=0 时内部不启动）
        self.start_cloud_refresh_timer().await;
        self.start_backoff_scheduler();

        Ok(())
    }

    #[cfg(test)]
    async fn start_with_network_receiver_for_test(
        self: &Arc<Self>,
        network_transitions: broadcast::Receiver<crate::core::net_guard::NetworkTransition>,
    ) -> AppResult<()> {
        self.start_with_network_receiver(network_transitions, false)
            .await
    }

    /// 恢复/清理所有中断的传输任务（kill 后重启时调用）。
    /// - 下载：清理 .tmp → 标记 FAILED（planner 下轮重新创建下载任务）
    /// - 上传有断点（session_url 非空 + 本地文件在）：尝试续传，失败则标记 FAILED
    /// - 上传无断点：标记 FAILED（planner 下轮重新创建上传任务）
    /// - 删除：标记 FAILED
    async fn recover_interrupted_transfers(&self) {
        let Some(task_runner) = &self.task_runner else {
            tracing::warn!("TaskRunner 未初始化，跳过中断任务恢复");
            return;
        };
        match task_runner.recover_startup().await {
            Ok(summary) => tracing::info!(
                completed = summary.completed,
                waiting_network = summary.waiting_network,
                verifying_remote = summary.verifying_remote,
                failed = summary.failed,
                "中断传输已通过统一 TaskRunner 恢复"
            ),
            Err(error) => tracing::warn!(%error, "统一中断任务恢复失败"),
        }
    }
    /// Returns true when a trusted checkpoint was loaded and still needs one incremental catch-up;
    /// false when this call already built, replayed and committed a fresh full checkpoint.
    async fn load_or_refresh_cloud_tree(&self, mount_dir: &str) -> AppResult<bool> {
        let _activity = self.begin_external_activity()?;
        #[cfg(test)]
        if let Some(hook) = &self.startup_cloud_hook {
            let result = hook();
            self.set_cloud_tree_trusted(result.is_ok());
            return result.map(|_| false);
        }
        let abs_dir = crate::core::paths::expand_tilde(mount_dir);
        let loaded_from_cache = if let Some(cache) = cloud_tree::load_persisted_cloud_tree(&abs_dir)
        {
            self.install_cloud_checkpoint(cache);
            true
        } else {
            self.set_cloud_tree_trusted(false);
            self.update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = true;
                runtime.sync_phase = Some("indexing-startup".to_string());
            })?;
            let refresh_result = self.build_and_commit_full_checkpoint(&abs_dir).await;
            let reset_result = self.update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = false;
                runtime.sync_phase = None;
            });
            if refresh_result.is_err() {
                self.set_cloud_tree_trusted(false);
                self.restore_idle_runtime_after_error();
            }
            refresh_result?;
            reset_result?;
            false
        };
        // ★ 清理无效墓碑：云端树里已不存在的 DELETED 记录可以真删了
        if self.cloud_tree_is_trusted() {
            let conn = self.db.lock();
            let ct = self.cloud_tree.lock();
            // 收集所有 DELETED 但云端已不存在的路径
            let to_purge: Vec<String> = {
                let mut stmt = conn
                    .prepare("SELECT local_path FROM sync_items WHERE status=?1")
                    .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
                let rows = stmt
                    .query_map(rusqlite::params![repository::sync_status::DELETED], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
                rows.filter_map(|r| r.ok())
                    .filter(|p| !ct.contains_key(p))
                    .collect()
            };
            drop(ct);
            if !to_purge.is_empty() {
                for p in &to_purge {
                    let _ = conn.execute(
                        "DELETE FROM sync_items WHERE local_path=?1 AND status=?2",
                        rusqlite::params![p, repository::sync_status::DELETED],
                    );
                }
                tracing::info!(count = to_purge.len(), "已清理无效墓碑（云端已不存在）");
            }
        } else {
            tracing::warn!("云端树不可信，跳过墓碑清理");
        }
        Ok(loaded_from_cache)
    }

    async fn start_watcher(self: &Arc<Self>) {
        let _activity = match self.begin_external_activity() {
            Ok(activity) => activity,
            Err(_) => return,
        };
        if *self.shutdown.lock() {
            return;
        }
        if let Some(ref m) = self.mount {
            let watcher = Arc::new(LocalWatcher::new(
                m.mount_dir(),
                self.skip_patterns.clone(),
                self.debounce_secs,
            ));
            if let Err(e) = watcher.start().await {
                tracing::error!("watcher启动失败: {e}");
            } else {
                let installed = {
                    let shutdown_guard = self.shutdown.lock();
                    if *shutdown_guard {
                        false
                    } else {
                        *self.watcher.lock() = Some(watcher.clone());
                        true
                    }
                };
                if !installed {
                    watcher.stop().await;
                    return;
                }
                let rx = watcher.subscribe();
                // 持 Arc<SyncEngine> 共享实时 cloud_tree/syncing（不再用冻结快照克隆）
                let engine = self.clone();
                tokio::spawn(async move {
                    let shutdown = engine.shutdown_tx.subscribe();
                    let request_engine = engine.clone();
                    watcher_listener_loop(rx, shutdown, move || {
                        request_engine.request_cycle_background("local-watcher")
                    })
                    .await;
                });
            }
        }
    }

    fn request_cycle_background(self: &Arc<Self>, triggered_by: &'static str) {
        self.cycle
            .request(Self::cycle_request_for_trigger(triggered_by));
        self.schedule_background_drain();
    }

    fn schedule_background_drain(self: &Arc<Self>) {
        if self
            .background_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let mut recoverable_failures = 0_u32;
            loop {
                let (failed, retryable_failure) = match engine.drain_cycle_requests_for(None).await
                {
                    Ok(()) => (false, false),
                    Err(error) => {
                        tracing::warn!(%error, "后台协调周期失败");
                        (true, SyncEngine::is_recoverable_cycle_error(&error))
                    }
                };
                if retryable_failure && engine.cycle.has_pending() {
                    recoverable_failures = recoverable_failures.saturating_add(1);
                    let delay = recoverable_cycle_retry_delay(recoverable_failures);
                    let mut shutdown = engine.shutdown_tx.subscribe();
                    if *shutdown.borrow() {
                        engine.background_scheduled.store(false, Ordering::Release);
                        break;
                    }
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                engine.background_scheduled.store(false, Ordering::Release);
                                break;
                            }
                        }
                        _ = tokio::time::sleep(delay) => {}
                    }
                    if engine.is_online() && !*engine.shutdown.lock() {
                        continue;
                    }
                }
                if !failed {
                    recoverable_failures = 0;
                }
                engine.background_scheduled.store(false, Ordering::Release);
                // A restored request from the failed sequence must not hot-loop. A genuinely newer
                // sequence that arrived while this worker still owned the scheduled bit must get
                // one handoff, otherwise it would remain pending with no worker to drain it.
                let newer_request_after_failure = engine.cycle.has_uncompleted_request();
                let can_continue = (!failed || newer_request_after_failure)
                    && engine.started.load(Ordering::Acquire)
                    && engine.is_online()
                    && !*engine.folder_syncing.lock()
                    && !*engine.shutdown.lock()
                    && engine.cycle.has_pending();
                if !can_continue
                    || engine
                        .background_scheduled
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                {
                    break;
                }
            }
        });
    }

    fn is_recoverable_cycle_error(error: &AppError) -> bool {
        match error {
            AppError::DriveApi {
                status_code,
                transport_kind,
                ..
            } => {
                transport_kind.is_some()
                    || status_code.is_some_and(|status| status == 429 || status >= 500)
            }
            _ => false,
        }
    }

    fn start_network_listener(
        self: &Arc<Self>,
        transitions: broadcast::Receiver<crate::core::net_guard::NetworkTransition>,
    ) {
        let engine = self.clone();
        let shutdown = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let request_engine = engine.clone();
            let level_engine = engine.clone();
            network_listener_loop(
                transitions,
                move || level_engine.is_online(),
                shutdown,
                false,
                move || request_engine.request_cycle_background("network-recovery"),
            )
            .await;
        });
    }

    fn start_backoff_scheduler(self: &Arc<Self>) {
        let Some(task_runner) = self.task_runner.clone() else {
            return;
        };
        let engine = self.clone();
        let mut shutdown = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                let deadline = match task_runner.next_backoff_deadline_ms() {
                    Ok(deadline) => deadline,
                    Err(error) => {
                        tracing::warn!(%error, "读取退避 deadline 失败");
                        None
                    }
                };
                match deadline {
                    Some(deadline) => {
                        let remaining_ms = deadline
                            .saturating_sub(task_runner.current_time_ms())
                            .max(0) as u64;
                        if remaining_ms == 0 {
                            if !engine.is_online() {
                                tokio::select! {
                                    changed = shutdown.changed() => {
                                        if changed.is_err() || *shutdown.borrow() { break; }
                                    }
                                    _ = engine.backoff_changed.notified() => {}
                                }
                                continue;
                            }
                            if let Err(error) = engine.run_sync_cycle("backoff-deadline").await {
                                tracing::warn!(%error, "退避 deadline 恢复周期失败");
                            }
                            let still_blocked = task_runner
                                .next_backoff_deadline_ms()
                                .ok()
                                .flatten()
                                .is_some_and(|next| next <= task_runner.current_time_ms());
                            if still_blocked {
                                let observed = engine.schedule_revision.load(Ordering::Acquire);
                                loop {
                                    tokio::select! {
                                        changed = shutdown.changed() => {
                                            if changed.is_err() || *shutdown.borrow() { return; }
                                        }
                                        _ = engine.backoff_changed.notified() => {
                                            if engine.schedule_revision.load(Ordering::Acquire) != observed {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_err() || *shutdown.borrow() { break; }
                            }
                            _ = engine.backoff_changed.notified() => {}
                            _ = tokio::time::sleep(Duration::from_millis(remaining_ms)) => {
                                if let Err(error) = engine.run_sync_cycle("backoff-deadline").await {
                                    tracing::warn!(%error, "退避 deadline 恢复周期失败");
                                }
                            }
                        }
                    }
                    None => {
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_err() || *shutdown.borrow() { break; }
                            }
                            _ = engine.backoff_changed.notified() => {}
                        }
                    }
                }
            }
        });
    }

    /// 启动云端定时刷新后台任务（shutdown-aware）。
    /// 间隔由 `poll_interval_secs` 决定，0 = 不启动。完全模仿 watcher 循环的
    /// shutdown 检查模式：循环顶部与 sleep 唤醒后各检查一次，置位即退出。
    /// 用 `sleep`（非 `interval`）：避免引擎 busy/索引中时累积欠债 tick。
    /// 首次亦 sleep：`start()` 已在启动时做过一次 BFS + startup-resume cycle，无需立即再刷。
    async fn start_cloud_refresh_timer(self: &Arc<Self>) {
        if self.poll_interval_secs == 0 {
            tracing::info!("云端定时刷新已关闭（poll_interval_secs=0）");
            return;
        }
        let engine = self.clone();
        tracing::info!(
            interval_secs = engine.poll_interval_secs,
            "启动云端定时刷新任务"
        );
        let mut shutdown = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { break; }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(engine.poll_interval_secs as u64)) => {}
                }
                if *shutdown.borrow() {
                    break;
                }
                if !engine.is_online() {
                    tracing::info!("网络离线，跳过本次云端刷新");
                    continue;
                }
                engine.run_auto_cloud_refresh().await;
            }
        });
    }

    /// 执行一次同步周期。
    pub async fn run_sync_cycle(&self, triggered_by: &str) -> AppResult<()> {
        if triggered_by != "startup-resume" && !self.started.load(Ordering::Acquire) {
            return Err(AppError::generic("同步引擎正在启动，请稍后重试"));
        }
        let sequence = self
            .cycle
            .request(Self::cycle_request_for_trigger(triggered_by));
        if triggered_by == "manual-refresh" {
            (self.cycle_observer)("request-manual");
        }
        self.drain_cycle_requests_for(Some(sequence)).await?;
        self.cycle
            .result_if_completed(sequence)
            .unwrap_or_else(|| Err(AppError::generic("同步请求已排队，等待恢复条件")))
    }

    fn cycle_request_for_trigger(triggered_by: &str) -> CycleRequest {
        match triggered_by {
            "manual-refresh" => CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_FULL,
            "auto-cloud-refresh" => CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL,
            "network-recovery" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
            }
            "startup-resume" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
                    | CycleRequest::STARTUP
            }
            "retry-failed" => CycleRequest::LOCAL_RESCAN | CycleRequest::RETRY,
            "backoff-deadline" => CycleRequest::LOCAL_RESCAN | CycleRequest::ONLINE_RECOVERY,
            _ => CycleRequest::LOCAL_RESCAN,
        }
    }

    async fn drain_cycle_requests_for(&self, awaited: Option<u64>) -> AppResult<()> {
        let _owner = self.cycle.lock_owner().await;
        if let Some(sequence) = awaited {
            if let Some(result) = self.cycle.result_if_completed(sequence) {
                return result;
            }
        }
        *self.syncing.lock() = true;
        let _syncing_reset = ResetFlag::new(&self.syncing);
        loop {
            let (request, cycle_sequence) = self.cycle.take_pending_with_sequence();
            if request.is_empty() {
                return Ok(());
            }
            if *self.shutdown.lock() {
                return Ok(());
            }
            if *self.folder_syncing.lock() {
                self.cycle.restore(request);
                return Ok(());
            }
            if !request.contains(CycleRequest::STARTUP) && !self.is_online() {
                self.cycle.restore(request);
                tracing::info!("网络离线，保留同步请求等待 level recovery");
                return Ok(());
            }
            if let Err(error) = self.run_coordinated_cycle(request).await {
                self.cycle.complete(cycle_sequence, Some(&error));
                self.restore_idle_runtime_after_error();
                return Err(error);
            }
            self.cycle.complete(cycle_sequence, None);
        }
    }

    async fn run_coordinated_cycle(&self, request: CycleRequest) -> AppResult<()> {
        let triggered_by = if request.contains(CycleRequest::STARTUP) {
            "startup-resume"
        } else if request.contains(CycleRequest::CLOUD_FULL) {
            "manual-refresh"
        } else if request.contains(CycleRequest::ONLINE_RECOVERY) {
            "network-recovery"
        } else if request.contains(CycleRequest::CLOUD_INCREMENTAL) {
            "auto-cloud-refresh"
        } else if request.contains(CycleRequest::RETRY) {
            "retry-failed"
        } else {
            "local-watcher"
        };
        let result = async {
            let mut startup_needs_incremental = true;
            self.update_runtime_and_broadcast(|runtime| {
                runtime.is_running = true;
                if runtime.sync_phase.is_none() {
                    runtime.sync_phase = match triggered_by {
                        "local-watcher" => Some("syncing-local".to_string()),
                        "manual-refresh" => Some("syncing-manual".to_string()),
                        "retry-failed" => Some("syncing-retry".to_string()),
                        "startup-resume" => Some("syncing-startup".to_string()),
                        _ => None, // auto-cloud-refresh 由上层设好
                    };
                }
            })?;

            if request.contains(CycleRequest::RETRY) {
                let _activity = self.begin_external_activity()?;
                {
                    let conn = self.db.lock();
                    conn.execute(
                        "UPDATE sync_items SET status=?1, error_message=NULL WHERE status=?2",
                        rusqlite::params![
                            repository::sync_status::SYNCING,
                            repository::sync_status::FAILED
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("接受失败项重试失败：{error}")))?;
                }
                self.recompute_and_broadcast_state()?;
            }

            if request.contains(CycleRequest::STARTUP) {
                {
                    let _activity = self.begin_external_activity()?;
                    let conn = self.db.lock();
                    let _ = repository::reset_stale_statuses(&conn);
                }
                self.ensure_cycle_active()?;
                self.recover_interrupted_transfers().await;
            }

            if request.contains(CycleRequest::ONLINE_RECOVERY) {
                if !self.is_online() {
                    self.cycle.restore(
                        CycleRequest::LOCAL_RESCAN
                            | CycleRequest::CLOUD_INCREMENTAL
                            | CycleRequest::ONLINE_RECOVERY,
                    );
                    if !request.contains(CycleRequest::STARTUP) {
                        return Ok(());
                    }
                } else {
                    self.ensure_cycle_active()?;
                    if let Some(task_runner) = &self.task_runner {
                        (self.cycle_observer)("resume-waiting");
                        task_runner.resume_waiting().await?;
                        self.ensure_cycle_active()?;
                        (self.cycle_observer)("resume-due");
                        task_runner.resume_due_backoff().await?;
                    }
                }
            }
            if request.contains(CycleRequest::STARTUP)
                && request.contains(CycleRequest::CLOUD_INCREMENTAL)
            {
                let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
                match self.load_or_refresh_cloud_tree(&mount_dir).await {
                    Ok(loaded_from_cache) => {
                        startup_needs_incremental = loaded_from_cache;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "启动 owner 无法建立可信云端 checkpoint，禁止进入 planner");
                        self.cycle.restore(
                            CycleRequest::LOCAL_RESCAN
                                | CycleRequest::CLOUD_INCREMENTAL
                                | CycleRequest::ONLINE_RECOVERY,
                        );
                        return Err(error);
                    }
                }
                self.ensure_cycle_active()?;
            }
            self.ensure_cycle_active()?;
            if request.contains(CycleRequest::CLOUD_FULL) {
                (self.cycle_observer)("cloud-refresh");
                if let Err(error) = self.refresh_cloud_full_for_cycle().await {
                    self.cycle
                        .restore(CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_FULL);
                    return Err(error);
                }
            } else if request.contains(CycleRequest::CLOUD_INCREMENTAL)
                && (!request.contains(CycleRequest::STARTUP) || startup_needs_incremental)
            {
                if !self.is_online() {
                    self.cycle.restore(
                        CycleRequest::LOCAL_RESCAN
                            | CycleRequest::CLOUD_INCREMENTAL
                            | CycleRequest::ONLINE_RECOVERY,
                    );
                } else {
                    (self.cycle_observer)("cloud-refresh");
                    if let Err(error) = self.refresh_cloud_incremental_for_cycle().await {
                        if request.contains(CycleRequest::STARTUP) {
                            tracing::warn!(%error, "启动 owner 云端刷新失败，等待稳定 Online 补跑");
                            self.cycle.restore(
                                CycleRequest::LOCAL_RESCAN
                                    | CycleRequest::CLOUD_INCREMENTAL
                                    | CycleRequest::ONLINE_RECOVERY,
                            );
                        } else {
                            self.cycle.restore(
                                CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL,
                            );
                            return Err(error);
                        }
                    }
                }
            }
            self.ensure_cycle_active()?;
            (self.cycle_observer)("local-rescan");
            self.run_sync_cycle_inner(triggered_by).await
        }
        .await;
        result
    }

    pub(crate) fn ensure_cycle_active(&self) -> AppResult<()> {
        if *self.shutdown.lock() {
            Err(AppError::generic("同步引擎已停止，拒绝开始新副作用"))
        } else {
            Ok(())
        }
    }

    /// 设置当前同步阶段并广播（供前端状态条精确显示）。
    fn set_phase(&self, phase: &str) -> AppResult<()> {
        self.update_runtime_and_broadcast(|runtime| {
            runtime.sync_phase = Some(phase.to_string());
        })?;
        Ok(())
    }

    async fn run_sync_cycle_inner(&self, triggered_by: &str) -> AppResult<()> {
        let local = self.scan_local().await;
        (self.cycle_observer)("local-scan-complete");
        self.ensure_cycle_active()?;
        let planning_activity = self.begin_external_activity()?;
        let cloud = self.cloud_tree.lock().clone();
        let db = self.load_db_snapshot();

        // 诊断日志：统计三方数据
        let local_in_cloud_not_db: Vec<&str> = local
            .keys()
            .filter(|k| cloud.contains_key(*k) && !db.contains_key(*k))
            .map(|s| s.as_str())
            .collect();
        let in_cloud_db_not_local: Vec<&str> = cloud
            .keys()
            .filter(|k| db.contains_key(*k) && !local.contains_key(*k))
            .map(|s| s.as_str())
            .collect();
        if !local_in_cloud_not_db.is_empty() {
            tracing::debug!(count = local_in_cloud_not_db.len(), paths = ?local_in_cloud_not_db, "本地+云端有但DB无（reconcile 将补）");
        }
        if !in_cloud_db_not_local.is_empty() {
            tracing::info!(count = in_cloud_db_not_local.len(), paths = ?in_cloud_db_not_local, "云端+DB有但本地无（应生成 DeleteFromCloud）");
        }

        let cloud_tree_trusted = self.cloud_tree_is_trusted();

        // 只有可信云端快照才能据“云端存在/缺失”制造成功 baseline。
        if cloud_tree_trusted {
            self.reconcile_db_records(&local, &db);
        } else {
            tracing::warn!("云端树不可信，跳过 DB reconcile");
        }

        let db_len = db.len();
        let local_len = local.len();
        let cloud_len = cloud.len();
        let snapshot = SyncSnapshot {
            local: local.clone(),
            cloud: cloud.clone(),
            db,
            is_startup_resume: triggered_by == "startup-resume",
            cloud_tree_trusted,
        };
        let mut actions = self.planner.plan(&snapshot);
        // §2.8 改名检测：在本地新文件上检查 xattr fileId，匹配 → 改名而非 upload+delete
        if cloud_tree_trusted {
            self.detect_renames(&mut actions);
        }
        filter_anti_oscillation(&mut actions, &self.recently_deleted_paths.lock());
        fill_parent_file_ids(&mut actions, &self.path_to_id.lock());
        // 为"云端已删目录下有内容需救援"补建目录链（跳过用户主动删除的目录）
        add_rescue_folder_recreations(&mut actions, &snapshot, &self.recently_deleted_paths.lock());

        // §2.11 防误删校验：planner 判定 !local_exists 时可能因 scan_local 漏扫
        // （如下载刚完成、xattr 延迟）误生成 DeleteFromCloud。在 mount_dir 下实际
        // stat 文件，若文件存在 → 改为 Skip，防止"删了又删"振荡（288→删除→上传）。
        self.validate_delete_from_cloud(&mut actions);
        // 第三道防线：删除本地文件前云端复核（不可逆操作 last-chance check）。
        // 对有真实 fileId 的 DeleteFromLocal 调 GET /files/{id} 确认云端确实不存在，
        // 防止 cloud_tree 残缺导致 cloud_exists 误判为 false 而删掉本地真实文件。
        // ★ 仅 startup-resume 启用：启动期 cloud_tree 可能不可信；会话内 cloud_tree 是
        // 上轮 BFS 的可信基线，复核冗余且会拖慢批量删除（每个删除串行一次 GET）。
        if triggered_by == "startup-resume" {
            self.validate_delete_from_local(&mut actions).await?;
        }

        // §2.12 目录级联删除去重：删除一个目录时，华为 API 对目录设置 recycled=true
        // 会级联将整个子树移入回收站（保留目录层级）。若同时为目录和其子文件分别生成
        // DeleteFromCloud，子文件会作为独立条目进入回收站 → 目录层级丢失、用户只能逐个恢复。
        // 本过滤：检测到目录 DeleteFromCloud 时，移除其所有子孙的 DeleteFromCloud。
        dedupe_directory_deletes(&mut actions, &self.cloud_tree.lock(), &self.db);

        // DeleteFromLocal 同样需要祖先去重：删除目录时其子文件会被级联清掉，
        // 无需（也不应）单独执行文件删除——否则并发执行时文件删除报 No such file。
        dedupe_local_descendants(&mut actions);

        // §2.13 目录删除保护：若目录下有文件被 BackupBeforeCloudDelete（本地改过需备份），
        // 则保留该目录的 DeleteFromLocal，确保备份副本有栖身目录。其余目录正常删除。
        preserve_dirs_with_pending_backups(&mut actions);

        // #8 空操作短路（对齐 dart：无 action → 清零计数 + contentChanged=false → return）
        if actions.is_empty() {
            self.update_runtime_and_broadcast(|runtime| {
                runtime.editing = 0;
                runtime.content_changed = false;
                runtime.is_running = false;
                // 同步周期结束即非索引态：复位 is_indexing，防止此前某次 BFS/刷新
                // 的 is_indexing=true 因交错未被清，导致状态条永久卡在「正在读取云端索引」。
                runtime.is_indexing = false;
                runtime.sync_phase = None;
                runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            })?;
            tracing::info!(
                triggered_by,
                local = local_len,
                cloud = cloud_len,
                db = db_len,
                "sync cycle: 无操作，短路返回"
            );
            return Ok(());
        }

        tracing::info!(
            triggered_by,
            actions = actions.len(),
            "sync cycle: 开始执行动作"
        );

        drop(planning_activity);
        self.ensure_cycle_active()?;
        let results = if let Some(ref exec) = self.executor {
            self.execute_actions_ordered(exec, &mut actions).await?
        } else {
            Vec::new()
        };
        // Submitted remote writes are allowed to settle through TaskRunner. An engine that was
        // replaced while awaiting them must not begin the later engine-level DB/cache mutations.
        self.ensure_cycle_active()?;
        let _apply_activity = self.begin_external_activity()?;

        // 执行后回写：cloud_tree/path_to_id + DB（对齐 dart _updateDbFromResults + syncFolderRecursive）
        self.apply_results(&actions, &results);

        // ★ 清理残余 DB 记录：planner 不再为"双方都删了"生成 DeleteFromCloud（避免
        // 无意义的 404 API 调用），改为此处直接清理 local 和 cloud 都不存在的 DB 行。
        if cloud_tree_trusted {
            self.purge_stale_db_records(&local, &cloud);
        } else {
            tracing::warn!("云端树不可信，跳过 stale DB purge");
        }

        // #7 contentChanged 逻辑（对齐 dart：仅结构性操作成功才 true）
        let content_changed = actions.iter().zip(results.iter()).any(|(a, r)| {
            r.success
                && matches!(
                    a.action_type,
                    crate::sync::state::SyncActionType::Upload
                        | crate::sync::state::SyncActionType::Download
                        | crate::sync::state::SyncActionType::DeleteFromCloud
                        | crate::sync::state::SyncActionType::DeleteFromLocal
                        | crate::sync::state::SyncActionType::CreateFolder
                        | crate::sync::state::SyncActionType::CreateConflictCopy
                        | crate::sync::state::SyncActionType::CreatePlaceholder
                        | crate::sync::state::SyncActionType::BackupBeforeCloudDelete
                )
        });
        // 广播状态更新
        self.update_and_push_state(content_changed)?;

        tracing::info!(
            triggered_by,
            actions = actions.len(),
            content_changed,
            "sync cycle ok"
        );
        Ok(())
    }

    /// 按依赖顺序执行动作：本地新建目录先于其内文件/子目录。
    ///
    /// 背景：planner 对「本地新建目录」生成 CreateFolder（cloud_file=None），对目录内
    /// 新文件生成 Upload。若全部并发执行，Upload 的 parent_file_id 在执行前就已固定，
    /// 而此时新目录尚未创建、不在 path_to_id → 文件被上传到云端根目录而非新目录。
    ///
    /// 修复（对齐用户预期「建目录→拿 ID→上传文件→子目录递归」）：
    /// 1. 本地新建目录按路径深度升序**顺序**执行（父目录先建，子目录才能找到父 id）；
    /// 2. 每个创建成功后用 result.cloud_file.id 回填 cloud_tree/path_to_id；
    /// 3. 再为其余动作重新填充 parent（此时新目录已在 path_to_id），然后并发执行；
    /// 4. 其余动作（上传/下载/删除/占位/云端→本地建目录）保持原并发语义不变。
    async fn execute_actions_ordered(
        &self,
        exec: &SyncExecutor,
        actions: &mut [crate::sync::state::SyncAction],
    ) -> AppResult<Vec<crate::sync::state::ActionResult>> {
        use crate::sync::state::{ActionResult, SyncActionType};
        let n = actions.len();
        let mut results: Vec<Option<ActionResult>> = (0..n).map(|_| None).collect();

        // 本地新建目录（CreateFolder 且无 cloud_file）下标，按深度升序
        let mut folder_idxs: Vec<usize> = (0..n)
            .filter(|&i| {
                actions[i].action_type == SyncActionType::CreateFolder
                    && actions[i].cloud_file.is_none()
            })
            .collect();
        folder_idxs.sort_by_key(|&i| {
            actions[i]
                .relative_path
                .as_deref()
                .map(|p| p.matches('/').count())
                .unwrap_or(0)
        });

        // 阶段 1：顺序执行本地新建目录，成功后回填 path_to_id/cloud_tree
        for &i in &folder_idxs {
            self.ensure_cycle_active()?;
            let _activity = self.begin_external_activity()?;
            // 重新填充 parent（父目录可能刚被创建并回填到 path_to_id）
            fill_parent_file_ids(&mut actions[i..=i], &self.path_to_id.lock());
            let res = exec
                .execute_all(&[actions[i].clone()])
                .await
                .into_iter()
                .next()
                .unwrap_or_else(|| ActionResult {
                    success: false,
                    error_message: Some("目录创建未返回结果".into()),
                    deferred: false,
                    cloud_file: None,
                });
            self.ensure_cycle_active()?;
            if res.success {
                if let (Some(rel), Some(cf)) =
                    (actions[i].relative_path.clone(), res.cloud_file.clone())
                {
                    self.cloud_tree_insert(rel.clone(), cf.clone());
                    self.path_to_id_insert(rel, cf.id.clone());
                    tracing::info!(rel = %actions[i].relative_path.as_deref().unwrap_or("?"),
                        folder_id = %cf.id, "本地新建目录已上传，回填 path_to_id");
                }
            }
            results[i] = Some(res);
        }

        // ★ 目录创建完成，立即通知前端刷新列表和侧边栏
        if !folder_idxs.is_empty() {
            if let Err(error) = self.update_runtime_and_broadcast(|runtime| {
                runtime.content_changed = true;
            }) {
                tracing::warn!(%error, "目录创建后重算全局状态失败");
            }
        }

        // 阶段 2：为其余动作重新填充 parent，再并发执行
        fill_parent_file_ids(actions, &self.path_to_id.lock());
        let other_idxs: Vec<usize> = (0..n).filter(|&i| results[i].is_none()).collect();
        let other_actions: Vec<crate::sync::state::SyncAction> =
            other_idxs.iter().map(|&i| actions[i].clone()).collect();
        let other_results = if other_actions.is_empty() {
            Vec::new()
        } else {
            self.ensure_cycle_active()?;
            exec.execute_all(&other_actions).await
        };
        for (k, &i) in other_idxs.iter().enumerate() {
            if let Some(r) = other_results.get(k) {
                results[i] = Some(r.clone());
            }
        }

        Ok(results
            .into_iter()
            .map(|r| {
                r.unwrap_or_else(|| ActionResult {
                    success: false,
                    error_message: Some("动作未执行".into()),
                    deferred: false,
                    cloud_file: None,
                })
            })
            .collect())
    }

    /// 执行后回写：cloud_tree/path_to_id + DB。
    ///
    /// 对齐 dart `_updateDbFromResults`（DB 用执行结果回写 fileId/元数据/真实 mtime）
    /// + `syncFolderRecursive` 上传后更新 `_cloudTree`/`_pathToId`。
    ///
    /// 关键修复（振荡根因）：
    /// - 新文件 Upload 动作 `action.file_id=None`，云端 ID 仅存在于 `result.cloud_file`。
    ///   之前用 `action.file_id` 取键 → `continue` 跳过 → DB 永不记录 + cloud_tree 不更新
    ///   → 下轮 watcher cycle 误判为「本地新增」重复上传，或「云端已删除」误删本地。
    /// - 之前 `local_mtime/size` 写死 None → `is_local_changed` 恒 true → 每轮重传。
    pub fn apply_results(
        &self,
        actions: &[crate::sync::state::SyncAction],
        results: &[crate::sync::state::ActionResult],
    ) {
        use crate::sync::state::SyncActionType;

        // 1. 防振荡维护 + cloud_tree/path_to_id 回写
        {
            let mut rdp = self.recently_deleted_paths.lock();
            let mut ct = self.cloud_tree.lock();
            let mut p2i = self.path_to_id.lock();
            for (action, result) in actions.iter().zip(results.iter()) {
                let Some(rel) = &action.relative_path else {
                    continue;
                };
                if result.success && action.action_type == SyncActionType::DeleteFromCloud {
                    // 云端已删 → 记入防振荡集，并从 cloud_tree/path_to_id 移除
                    rdp.insert(rel.clone(), chrono::Utc::now().timestamp_millis());
                    ct.remove(rel);
                    p2i.remove(rel);
                    continue;
                }
                // 成功且产生/更新了云端条目（上传/建文件夹/冲突/下载/占位）→ 回写
                // cloud_tree/path_to_id。Tauri 的 watcher 会在执行动作（建占位/下载写文件）
                // 后再次触发 cycle；若 cloud_tree 不含刚上传的文件，会误判误删/重复上传。
                if result.success {
                    if let Some(cf) = result.cloud_file.as_ref().or(action.cloud_file.as_ref()) {
                        ct.insert(rel.clone(), cf.clone());
                        p2i.insert(rel.clone(), cf.id.clone());
                    }
                }
            }
            // ★ 过期清理：5 分钟前加入的条目自动移除（防止永久拦截云端恢复的目录重建）
            let expire_before = chrono::Utc::now().timestamp_millis() - 300_000;
            rdp.retain(|_, ts| *ts > expire_before);
        }

        // 2. 更新 DB（对齐 dart _updateDbFromResults：用执行结果回写 fileId/元数据/真实 mtime）
        let conn = self.db.lock();
        for (action, result) in actions.iter().zip(results.iter()) {
            let Some(rel) = &action.relative_path else {
                continue;
            };

            // 删除/备份动作成功 → 清 DB 记录（按 local_path；file_id 可选，覆盖"双方都删清理"file_id=None 场景）
            // BackupBeforeCloudDelete：原文件改名走，原路径腾空 + 云端已删 → 同样清掉原 DB 记录，
            // 让下轮该路径「全缺席」无动作；副本是全新路径，下轮正常 Upload。
            if result.success
                && matches!(
                    action.action_type,
                    SyncActionType::DeleteFromCloud
                        | SyncActionType::DeleteFromLocal
                        | SyncActionType::BackupBeforeCloudDelete
                )
            {
                let fid = action.file_id.as_deref().unwrap_or("");
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1 AND (?2='' OR file_id=?2)",
                    rusqlite::params![rel, fid],
                );
                // ★ 防振荡：所有 DeleteFromLocal/DeleteFromCloud/BackupBeforeCloudDelete 成功
                // 的路径都加入 recentlyDeletedPaths，防止 users 从云端回收站恢复后
                // planner 误生成 CreateFolder 导致 (1) 后缀副本。
                self.recently_deleted_paths
                    .lock()
                    .insert(rel.clone(), chrono::Utc::now().timestamp_millis());
                // ★ 若删除的是目录，级联清理所有子孙的 DB 记录。
                // dedupe_directory_deletes 将子文件删除合并为目录删除后，子文件的 DB 记录
                // 不会被执行器结算（动作已移除），导致残留。用户从回收站恢复目录后，
                // planner 看到"云端有 + DB有 + 本地无"→ 误删已恢复的文件。
                // 此处在目录删除成功后，按 local_path 前缀清理所有子孙。
                {
                    let mut ct = self.cloud_tree.lock();
                    if ct.get(rel).map(|f| f.is_folder()).unwrap_or(false) {
                        let prefix = format!("{}/", rel);
                        let _ = conn.execute(
                            "DELETE FROM sync_items WHERE local_path=?1 OR local_path LIKE ?2",
                            rusqlite::params![rel, format!("{}%", prefix)],
                        );
                        // 从 cloud_tree / path_to_id 移除整个子树
                        let to_remove: Vec<String> = ct
                            .keys()
                            .filter(|k| *k == rel || k.starts_with(&prefix))
                            .cloned()
                            .collect();
                        let mut p2i = self.path_to_id.lock();
                        for k in &to_remove {
                            ct.remove(k);
                            p2i.remove(k);
                        }
                        // 同时清理防振荡集
                        self.recently_deleted_paths
                            .lock()
                            .insert(rel.clone(), chrono::Utc::now().timestamp_millis());
                        tracing::info!(
                            rel = %rel,
                            descendants = to_remove.len().saturating_sub(1),
                            "目录删除：级联清理子树 DB / cloud_tree / path_to_id"
                        );
                    }
                }
                continue;
            }

            // A failed/deferred durable task must never advance the last confirmed-success
            // baseline. Permanent failures may update only compatibility status/message; all
            // mtime/size/fileId/cloud-version facts remain untouched and TaskRunner is the
            // authoritative failure source.
            if !result.success {
                if !result.deferred {
                    if let Some(file_id) = action
                        .file_id
                        .as_deref()
                        .filter(|file_id| !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX))
                    {
                        let _ = conn.execute(
                            "UPDATE sync_items SET status=?1, error_message=?2
                             WHERE file_id=?3 AND local_path=?4",
                            rusqlite::params![
                                repository::sync_status::FAILED,
                                result.error_message.as_deref(),
                                file_id,
                                rel,
                            ],
                        );
                    }
                }
                continue;
            }

            let status = if action.action_type == SyncActionType::CreatePlaceholder {
                repository::sync_status::CLOUD_ONLY // 占位符 → cloudOnly（非 synced）
            } else if action.action_type == SyncActionType::CreateConflictCopy {
                repository::sync_status::CONFLICT
            } else {
                repository::sync_status::SYNCED // upload/download/createFolder → synced
            };

            // 云端元数据：成功时优先用 executor 返回的（新上传/建文件夹的 fileId 由此得到），
            // 否则用 action 携带的（download/placeholder 的 cloud_file）。
            let cloud_file = result.cloud_file.as_ref().or(action.cloud_file.as_ref());
            // A confirmed success must have a real remote ID. TaskRunner routes incomplete
            // upload responses to VerifyingRemote, so synthesizing a pending ID here would create
            // a false success baseline.
            let file_id = cloud_file
                .map(|file| file.id.clone())
                .or_else(|| action.file_id.clone());
            let file_id = match file_id {
                Some(fid) => fid,
                None => {
                    tracing::warn!(rel = %rel, status, "跳过成功基线写入：缺少真实 fileId");
                    continue;
                }
            };

            // 读取本地真实 mtime/size（对齐 dart _updateDbFromResults 从本地文件 stat）。
            // 写死 None 会导致 is_local_changed 恒 true（db.local_mtime.is_none()），每轮重传。
            let (local_mtime, local_size) = match &action.local_path {
                Some(p) => std::fs::metadata(p)
                    .ok()
                    .map(|m| {
                        let mt = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64);
                        (mt, Some(m.len() as i64))
                    })
                    .unwrap_or((None, None)),
                None => (None, None),
            };

            // 重建云端已删目录（CreateFolder && cloud_file=None 成功）：新 folderId 与旧 db
            // 记录不同（旧目录已删，重建得新 id），先清掉同路径旧记录，避免 dual 记录污染。
            if result.success
                && action.action_type == SyncActionType::CreateFolder
                && action.cloud_file.is_none()
            {
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1",
                    rusqlite::params![rel],
                );
            }
            // 成功上传/覆盖上传 → 清掉同路径的 pending: 占位孤儿行。
            // PK 是 (file_id, local_path)，真实 fileId 与 pending: 占位 fileId 是不同主键，
            // upsert 不会覆盖占位行 → 必须显式删除，避免残留孤儿导致 planner 误判。
            if !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1 AND file_id=?2",
                    rusqlite::params![
                        rel,
                        format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel)
                    ],
                );
            }

            // upsert（对齐 dart insertOnConflictUpdate）
            let _ = repository::upsert(
                &conn,
                &repository::SyncItem {
                    file_id,
                    local_path: rel.clone(),
                    parent_folder_id: action.parent_file_id.clone(),
                    name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
                    is_folder: matches!(action.action_type, SyncActionType::CreateFolder),
                    size: cloud_file.map(|f| f.size).unwrap_or(0),
                    local_size,
                    sha256: None,
                    local_mtime,
                    cloud_edited_time: cloud_file
                        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis())),
                    last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                    status,
                    // 成功时清空 error_message（Skip 收敛等场景 result 可能带 reason，但已同步不应残留错误）
                    error_message: None,
                },
            );
        }
        drop(conn);
    }

    /// #4 DB 自愈（对齐 dart _reconcileDbRecords）：
    /// 本地有内容（非占位符）无 DB → upsert synced；
    /// 本地占位符无 DB → upsert cloudOnly。
    /// 防止孤儿记录导致 planner 误判。
    ///
    /// **安全阀**：仅当 xattr fileId 在 cloud_tree 中存在且路径一致时才创建 DB 记录。
    /// 若 fileId 存在但路径不同（用户复制了带 xattr 的已同步文件到新位置），
    /// 跳过——避免 planner 误判为"云端已删除"而触发 DeleteFromLocal（数据丢失）。
    fn reconcile_db_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        db: &HashMap<String, DbSnapshotEntry>,
    ) {
        let conn = self.db.lock();
        let ct = self.cloud_tree.lock();
        for (rel, entry) in local {
            if let Some(db_entry) = db.get(rel) {
                // 若 DB 记录标记为 DELETED 但用户重新粘贴了文件 → 复活为正常状态
                if db_entry.status == repository::sync_status::DELETED {
                    let status = if entry.is_placeholder {
                        repository::sync_status::CLOUD_ONLY
                    } else {
                        repository::sync_status::SYNCED
                    };
                    let _ = conn.execute(
                        "UPDATE sync_items SET status=?1 WHERE local_path=?2",
                        rusqlite::params![status, rel],
                    );
                    tracing::info!(rel = %rel, "DELETED 墓碑已复活");
                }
                continue;
            }
            let status = if entry.is_placeholder {
                repository::sync_status::CLOUD_ONLY
            } else {
                repository::sync_status::SYNCED
            };
            // 尝试从 xattr 获取 fileId（占位符有 xattr）
            let file_id = std::fs::metadata(&entry.absolute_path)
                .ok()
                .and_then(|_| {
                    use crate::mount::manager::XATTR_FILE_ID;
                    xattr::get(&entry.absolute_path, XATTR_FILE_ID)
                        .ok()
                        .flatten()
                        .and_then(|b| String::from_utf8(b).ok())
                })
                .unwrap_or_default();
            if file_id.is_empty() {
                continue; // 无 fileId 无法 upsert（本地新增文件由 planner Upload 处理）
            }
            // ★ 安全阀：xattr fileId 必须在 cloud_tree 中存在且路径一致，才创建 DB 记录。
            // 若 fileId 存在但路径不同 → 说明用户复制了带 xattr 的已同步文件到新位置，
            // 不能创建 DB 记录，否则 planner 会误判为 "云端已删除" → DeleteFromLocal（数据丢失）。
            // 新路径的文件应由 planner 的 Upload 分支处理（local_exists, !cloud_exists, !db_exists）。
            let Some(cloud_file) = ct.get(rel) else {
                tracing::info!(
                    rel = %rel,
                    xattr_fid = %file_id,
                    "reconcile 跳过：可信 cloud_tree 同路径不存在，禁止制造已同步 baseline"
                );
                continue;
            };
            if cloud_file.id != file_id {
                tracing::info!(
                    rel = %rel,
                    xattr_fid = %file_id,
                    cloud_fid = %cloud_file.id,
                    "reconcile 跳过：xattr fileId 与 cloud_tree 不一致（可能为复制文件），由 planner 正常处理"
                );
                continue;
            }
            let _ = repository::upsert(
                &conn,
                &repository::SyncItem {
                    file_id,
                    local_path: rel.clone(),
                    parent_folder_id: None,
                    name: entry
                        .relative_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.relative_path)
                        .to_string(),
                    is_folder: entry.is_folder,
                    size: 0,
                    local_size: if entry.is_placeholder {
                        None
                    } else {
                        Some(entry.size as i64)
                    },
                    sha256: None,
                    local_mtime: Some(entry.mtime),
                    cloud_edited_time: None,
                    last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                    status,
                    error_message: None,
                },
            );
        }
        drop(conn);
    }

    /// §2.8 改名检测：本地新文件若 xattr 含已知 fileId → 改名而非 upload+delete。
    /// 通过 xattr 匹配"本地新文件"与"缺失原路径的 DB 记录"，调用 update 同步改名到云端
    /// （先于内容同步），避免先删后传导致云端短暂不可用。
    fn detect_renames(&self, actions: &mut [crate::sync::state::SyncAction]) {
        use crate::mount::manager::XATTR_FILE_ID;
        let db = self.db.lock();
        // 先收集全体 DB 记录（按 fileId 索引）
        let db_by_id: std::collections::HashMap<String, crate::data::repository::SyncItem> =
            repository::load_all(&db)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| !r.file_id.is_empty())
                .map(|r| (r.file_id.clone(), r))
                .collect();
        drop(db);

        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        for action in actions.iter_mut() {
            if action.action_type != crate::sync::state::SyncActionType::Upload
                || action.file_id.is_some()
            {
                continue;
            }
            let local_path = match &action.local_path {
                Some(p) => std::path::PathBuf::from(p),
                None => continue,
            };
            let xattr_id = std::fs::metadata(&local_path).ok().and_then(|_| {
                xattr::get(&local_path, XATTR_FILE_ID)
                    .ok()
                    .flatten()
                    .and_then(|b| String::from_utf8(b).ok())
            });
            let Some(fid) = xattr_id else { continue };
            let Some(old_record) = db_by_id.get(&fid) else {
                continue;
            };
            if Some(&old_record.local_path) == action.relative_path.as_ref() {
                continue;
            }
            if !ct.contains_key(&old_record.local_path) {
                continue;
            }
            // 旧文件仍在本地 → 复制（非改名）：新文件应作为全新上传，
            // 不能复用旧 fileId 走 update/rename 路径（否则云端文件被移动/覆盖）。
            // 同时清除新文件上的旧 xattr fileId，避免下轮又被误判为改名。
            let old_abs = std::path::PathBuf::from(&mount_dir).join(&old_record.local_path);
            if old_abs.exists() {
                let _ = xattr::remove(&local_path, XATTR_FILE_ID);
                tracing::info!(
                    old = %old_record.local_path,
                    new = action.relative_path.as_deref().unwrap_or("?"),
                    "复制检测（旧文件仍在本地），已清除新文件旧 xattr，按全新文件上传");
                continue;
            }

            action.file_id = Some(fid.clone());
            if let Some(cloud_file) = ct.get(&old_record.local_path) {
                action.parent_file_id = cloud_file
                    .parent_folder
                    .as_ref()
                    .and_then(|v| v.first().cloned());
            }
            action.reason = Some(format!(
                "改名检测：{} → {}（fileId={}，先于内容同步）",
                old_record.local_path,
                action.relative_path.as_deref().unwrap_or("?"),
                fid,
            ));
            tracing::info!(reason = action.reason.as_deref(), "检测到文件改名");
        }
        drop(ct);
    }

    /// 更新并广播全局状态（聚合 DB 计数，对齐 dart _updateState）
    fn update_and_push_state(&self, content_changed: bool) -> AppResult<()> {
        self.update_runtime_and_broadcast(|runtime| {
            runtime.content_changed = content_changed;
            runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        })?;
        Ok(())
    }

    /// 委托权威聚合器重算完整状态（不重置 runtime-only fields）。
    ///
    /// 供少数非 TaskRunner 的兼容入口显式请求一次完整状态重算。
    /// 与 [`update_and_push_state`] 区别：后者只在周期结束调用且重置 is_running；
    /// 本方法保留 runtime fields，但不会保留任何过期 DB 计数。
    pub fn push_live_transfer_state(&self) {
        if let Err(error) = self.recompute_and_broadcast_state() {
            tracing::warn!(%error, "传输变化后重算全局状态失败");
        }
    }

    async fn scan_local(&self) -> HashMap<String, LocalFileEntry> {
        match &self.mount {
            Some(m) => m
                .scan_local(&self.skip_patterns)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|e| (e.relative_path.clone(), e))
                .collect(),
            None => HashMap::new(),
        }
    }

    fn load_db_snapshot(&self) -> HashMap<String, DbSnapshotEntry> {
        let conn = self.db.lock();
        repository::load_all(&conn)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                (
                    r.local_path.clone(),
                    DbSnapshotEntry {
                        file_id: r.file_id,
                        local_mtime: r.local_mtime,
                        local_size: r.local_size,
                        cloud_edited_time: r.cloud_edited_time,
                        status: r.status,
                        is_folder: r.is_folder,
                    },
                )
            })
            .collect()
    }

    /// 安全释放校验。
    pub fn can_safely_free_up(&self, rel_path: &str, file_id: &str) -> FreeUpCheckResult {
        let tree = self.cloud_tree.lock();
        if !tree.is_empty() && !tree.contains_key(rel_path) {
            return FreeUpCheckResult::NotInCloud;
        }
        drop(tree);
        let conn = self.db.lock();
        if let Ok(Some(record)) = repository::find_by_file_id(&conn, file_id) {
            let path = std::path::Path::new(&record.local_path);
            // 本地文件必须存在且与 DB 记录一致（已下载、无本地改动）才可释放。
            // 之前 metadata 失败（本地无文件/已释放为占位）会落到 Safe，导致未下载文件
            // 也被判成可释放——已修正为 NotSynced。
            let Ok(meta) = std::fs::metadata(path) else {
                return FreeUpCheckResult::NotSynced;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            if record.local_mtime != mtime || record.local_size != Some(meta.len() as i64) {
                FreeUpCheckResult::NotSynced
            } else {
                FreeUpCheckResult::Safe
            }
        } else {
            FreeUpCheckResult::NotSynced
        }
    }

    pub async fn trigger_manual_sync(&self) -> AppResult<()> {
        let result = self.run_sync_cycle("manual-refresh").await;
        (self.cycle_observer)("manual-cycle-returned");
        if result.is_ok() {
            self.update_runtime_and_broadcast(|runtime| runtime.content_changed = true)?;
        }
        result
    }

    /// Build a full candidate without touching the last trusted checkpoint, replay every change
    /// that occurred during BFS, persist tree/path/cursor as one unit, then install that unit.
    async fn build_and_commit_full_checkpoint(&self, abs_dir: &str) -> AppResult<()> {
        let result = async {
            self.ensure_cycle_active()?;
            let start_cursor = self.start_cursor_source.get_start_cursor().await?;
            self.ensure_cycle_active()?;
            let (mut tree, mut path_to_id, root_folder_id) =
                cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, abs_dir).await?;
            self.ensure_cycle_active()?;
            let (changes, final_cursor) = self.changes_api.list_all_changes(&start_cursor).await?;
            Self::apply_changes_to_candidate(
                &mut tree,
                &mut path_to_id,
                root_folder_id.as_deref(),
                &changes,
            )?;
            let checkpoint = cloud_tree::CloudTreeCache::new_trusted(
                root_folder_id,
                tree,
                path_to_id,
                final_cursor,
            )?;
            self.ensure_cycle_active()?;
            cloud_tree::persist_cloud_checkpoint(abs_dir, &checkpoint)?;
            self.ensure_cycle_active()?;
            self.install_cloud_checkpoint(checkpoint);
            if let Ok(legacy_cursor) = crate::core::cache_paths::changes_cursor_file(abs_dir) {
                let _ = std::fs::remove_file(legacy_cursor);
            }
            self.incremental_since_full.store(0, Ordering::Relaxed);
            Ok(())
        }
        .await;
        if result.is_err() {
            // Keep the old live candidate for non-destructive display, but cloud absence is now
            // unknown and must not drive delete/reconcile/purge decisions.
            self.set_cloud_tree_trusted(false);
        }
        result
    }

    /// Full refresh stage owned by the cycle coordinator.
    async fn refresh_cloud_full_for_cycle(&self) -> AppResult<()> {
        let _activity = self.begin_external_activity()?;
        // 对齐 dart triggerManualSync：先刷新云端树获取最新状态，再跑同步周期
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = crate::core::paths::expand_tilde(&mount_dir);

        // 广播 is_indexing=true（对齐 dart _refreshCloudTree 的 isIndexing 广播）。
        // trigger_manual_sync 直接调 refresh_cloud_tree，需在此补 is_indexing 广播，
        // 否则手动刷新期间前端全程收到 idle 态、状态条显示「同步完成」。
        self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = true;
            runtime.sync_phase = Some("indexing-manual".to_string());
        })?;
        // 无论成功失败都要复位 is_indexing，避免 BFS 出错后状态条卡在索引态
        let refresh_result = self.build_and_commit_full_checkpoint(&abs_dir).await;
        self.ensure_cycle_active()?;
        let reset_result = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        });
        match refresh_result {
            Ok(()) => {
                reset_result?;
            }
            Err(error) => {
                let _ = reset_result;
                return Err(error);
            }
        }
        Ok(())
    }

    /// 自动定时刷新云端树（定时轮询专用，静默、容错）。
    /// 对齐 `trigger_manual_sync_impl` 的核心流程（刷新 + 置换 + cycle），差异：
    /// - 复用 `manual_syncing` 互斥锁，与手动刷新互斥，避免并发 BFS；
    /// - 不强制 `content_changed = true`（静默刷新，真实变化由 planner 的
    ///   `folder_content_changed` 事件驱动前端刷新，避免每 15 分钟无谓全量重拉 UI）；
    /// - 失败仅 `warn` 不传播，后台任务不应因单次失败终止循环。
    async fn run_auto_cloud_refresh(self: &Arc<Self>) {
        let result = self.run_sync_cycle("auto-cloud-refresh").await;
        (self.cycle_observer)("auto-cycle-returned");
        if let Err(e) = result {
            tracing::warn!(error = %e, "自动云端刷新失败（忽略，下次定时重试）");
        }
    }

    /// Incremental-preferred cloud refresh stage owned by the cycle coordinator.
    async fn refresh_cloud_incremental_for_cycle(&self) -> AppResult<()> {
        let _activity = self.begin_external_activity()?;
        #[cfg(test)]
        if let Some(hook) = &self.incremental_refresh_hook {
            return hook();
        }
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = crate::core::paths::expand_tilde(&mount_dir);

        // 广播 is_indexing=true（phase 由 try_incremental_or_full_refresh 内部按增量/全量设）
        self.update_runtime_and_broadcast(|runtime| runtime.is_indexing = true)?;

        // 尝试增量（有持久化 cursor）；失败/无 cursor 回退全量 BFS
        let refresh_result = self.try_incremental_or_full_refresh(&abs_dir).await;

        // 无论成败复位 is_indexing + phase
        let reset_result = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = false;
            runtime.sync_phase = None;
            // ★ 云端刷新（增量或全量）可能引入了新文件/恢复文件，强制 contentChanged
            // 通知前端刷新文件列表，否则 BFS 回退路径下 sync cycle 无 action 时前端不更新
            runtime.content_changed = true;
        });

        match refresh_result {
            Ok(()) => {
                reset_result?;
            }
            Err(error) => {
                let _ = reset_result;
                return Err(error);
            }
        }
        // 增量 merge 完成后，cycle 阶段显示"同步云端变更"。
        self.set_phase("syncing-auto-incremental")?;
        Ok(())
    }

    /// 增量优先：有 cursor → changes API merge；失败/无 cursor → 全量 BFS。
    async fn try_incremental_or_full_refresh(&self, abs_dir: &str) -> AppResult<()> {
        let saved_cursor = self.cloud_cursor.lock().clone();
        let consecutive = self.incremental_since_full.load(Ordering::Relaxed);
        let force_full = consecutive >= INCREMENTAL_FORCED_FULL_THRESHOLD;
        if force_full {
            tracing::info!(
                consecutive,
                threshold = INCREMENTAL_FORCED_FULL_THRESHOLD,
                "连续增量达阈值，强制全量 BFS 纠偏"
            );
        }

        if !force_full && self.cloud_tree_is_trusted() {
            if let Some(cursor) = saved_cursor.filter(|cursor| !cursor.trim().is_empty()) {
                self.set_phase("querying-changes")?;
                let incremental = async {
                    let (changes, final_cursor) =
                        self.changes_api.list_all_changes(&cursor).await?;
                    self.ensure_cycle_active()?;
                    let mut tree = self.cloud_tree.lock().clone();
                    let mut path_to_id = self.path_to_id.lock().clone();
                    let root_folder_id = self.root_folder_id.lock().clone();
                    Self::apply_changes_to_candidate(
                        &mut tree,
                        &mut path_to_id,
                        root_folder_id.as_deref(),
                        &changes,
                    )?;
                    let checkpoint = cloud_tree::CloudTreeCache::new_trusted(
                        root_folder_id,
                        tree,
                        path_to_id,
                        final_cursor,
                    )?;
                    self.ensure_cycle_active()?;
                    cloud_tree::persist_cloud_checkpoint(abs_dir, &checkpoint)?;
                    self.ensure_cycle_active()?;
                    self.install_cloud_checkpoint(checkpoint);
                    self.incremental_since_full.fetch_add(1, Ordering::Relaxed);
                    Ok::<(), AppError>(())
                }
                .await;
                match incremental {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        self.set_cloud_tree_trusted(false);
                        tracing::warn!(%error, "增量 checkpoint 失败，保留旧盘并回退可信全量刷新");
                    }
                }
            }
        }

        self.set_phase("indexing-auto-full")?;
        self.build_and_commit_full_checkpoint(abs_dir).await
    }

    /// 把一批 Changes 全量应用到候选树。调用方只在整个批次成功且 checkpoint
    /// 已原子落盘后安装候选；任一无法解析的 parent/rename/move 都 fail closed。
    fn apply_changes_to_candidate(
        tree: &mut HashMap<String, DriveFile>,
        path_to_id: &mut HashMap<String, String>,
        root_folder_id: Option<&str>,
        changes: &[crate::drive::changes_api::Change],
    ) -> AppResult<()> {
        use crate::drive::changes_api::ChangeKind;
        let mut id_to_path: HashMap<String, String> = path_to_id
            .iter()
            .map(|(path, id)| (id.clone(), path.clone()))
            .collect();
        if let Some(root_id) = root_folder_id.filter(|id| !id.trim().is_empty()) {
            id_to_path.insert(root_id.to_string(), String::new());
        }

        for change in changes {
            match change.kind {
                ChangeKind::Removed => {
                    let Some(relative_path) = id_to_path.get(change.file_id()).cloned() else {
                        // 已经不在候选树中的 tombstone 是幂等 no-op，仍算完整消费。
                        continue;
                    };
                    if relative_path.is_empty() {
                        return Err(AppError::generic("Changes 试图删除云盘根目录"));
                    }

                    let prefix = format!("{relative_path}/");
                    let removed_paths: Vec<String> = tree
                        .keys()
                        .filter(|path| *path == &relative_path || path.starts_with(&prefix))
                        .cloned()
                        .collect();
                    for path in removed_paths {
                        tree.remove(&path);
                        if let Some(id) = path_to_id.remove(&path) {
                            id_to_path.remove(&id);
                        }
                    }
                }
                ChangeKind::Modified => {
                    let file = change.file().ok_or_else(|| {
                        AppError::generic(format!(
                            "非删除 Change 缺少完整文件：{}",
                            change.file_id()
                        ))
                    })?;
                    crate::core::paths::validate_path_segment(&file.name)?;
                    let parent_id = file
                        .parent_folder
                        .as_ref()
                        .and_then(|parents| parents.first())
                        .filter(|id| !id.trim().is_empty())
                        .ok_or_else(|| {
                            AppError::generic(format!(
                                "Change {} 缺少可解析 parentFolder",
                                change.file_id()
                            ))
                        })?;
                    if parent_id == change.file_id() {
                        return Err(AppError::generic("Change 的 parentFolder 指向自身"));
                    }
                    let parent_path = id_to_path.get(parent_id).cloned().ok_or_else(|| {
                        AppError::generic(format!(
                            "Change {} 的 parentFolder {} 无法映射到可信路径",
                            change.file_id(),
                            parent_id
                        ))
                    })?;
                    let desired_path = if parent_path.is_empty() {
                        file.name.clone()
                    } else {
                        format!("{parent_path}/{}", file.name)
                    };

                    if let Some(existing_path) = id_to_path.get(change.file_id()).cloned() {
                        if existing_path.is_empty() {
                            return Err(AppError::generic("Changes 不支持修改云盘根目录"));
                        }
                        if existing_path != desired_path {
                            if desired_path.starts_with(&format!("{existing_path}/")) {
                                return Err(AppError::generic("Change 试图把目录移动到自身子树"));
                            }
                            Self::rekey_candidate_subtree(
                                tree,
                                path_to_id,
                                &mut id_to_path,
                                &existing_path,
                                &desired_path,
                            )?;
                        }
                    } else if let Some(existing_id) = path_to_id.get(&desired_path) {
                        if existing_id != change.file_id() {
                            return Err(AppError::generic(format!(
                                "Change 目标路径冲突：{desired_path}"
                            )));
                        }
                    };

                    tree.insert(desired_path.clone(), file.clone());
                    path_to_id.insert(desired_path.clone(), change.file_id().to_string());
                    id_to_path.insert(change.file_id().to_string(), desired_path);
                }
            }
        }
        Ok(())
    }

    fn rekey_candidate_subtree(
        tree: &mut HashMap<String, DriveFile>,
        path_to_id: &mut HashMap<String, String>,
        id_to_path: &mut HashMap<String, String>,
        old_root: &str,
        new_root: &str,
    ) -> AppResult<()> {
        let old_prefix = format!("{old_root}/");
        let moved_paths: Vec<String> = tree
            .keys()
            .filter(|path| path.as_str() == old_root || path.starts_with(&old_prefix))
            .cloned()
            .collect();
        if moved_paths.is_empty() {
            return Err(AppError::generic(format!(
                "Change 引用的旧路径不在候选树：{old_root}"
            )));
        }
        let moved_set: std::collections::HashSet<&str> =
            moved_paths.iter().map(String::as_str).collect();
        let targets: Vec<(String, String)> = moved_paths
            .iter()
            .map(|old_path| {
                let suffix = old_path.strip_prefix(old_root).unwrap_or_default();
                (old_path.clone(), format!("{new_root}{suffix}"))
            })
            .collect();
        for (_, target) in &targets {
            if tree.contains_key(target) && !moved_set.contains(target.as_str()) {
                return Err(AppError::generic(format!(
                    "Change 移动/改名目标路径已存在：{target}"
                )));
            }
        }

        let mut moved = Vec::with_capacity(targets.len());
        for (old_path, new_path) in targets {
            let file = tree
                .remove(&old_path)
                .ok_or_else(|| AppError::generic(format!("候选树移动时路径消失：{old_path}")))?;
            let file_id = path_to_id.remove(&old_path).ok_or_else(|| {
                AppError::generic(format!("候选路径索引移动时路径消失：{old_path}"))
            })?;
            id_to_path.remove(&file_id);
            moved.push((new_path, file_id, file));
        }
        for (new_path, file_id, file) in moved {
            id_to_path.insert(file_id.clone(), new_path.clone());
            path_to_id.insert(new_path.clone(), file_id);
            tree.insert(new_path, file);
        }
        Ok(())
    }

    /// §2.11 防误删校验：对 DeleteFromCloud 动作，实际 stat 本地文件是否真的不存在。
    ///
    /// planner 的 `!local_exists && cloud_exists && db_exists` 分支在 watcher 周期
    /// 中可能因 scan_local 漏扫（下载刚完成、xattr 延迟等）而误生成 DeleteFromCloud。
    /// 本方法在 mount_dir 下拼接绝对路径并 stat，若文件存在则改为 Skip，防止振荡：
    /// watcher 周期删云端 → 下一轮本地文件又被 Upload → 传输队列出现多余的"删除+上传"。
    fn validate_delete_from_cloud(&self, actions: &mut [crate::sync::state::SyncAction]) {
        let mount_dir = match self.mount_dir.lock().clone() {
            Some(d) => std::path::PathBuf::from(crate::core::paths::expand_tilde(&d)),
            None => return,
        };
        for a in actions.iter_mut() {
            if a.action_type != crate::sync::state::SyncActionType::DeleteFromCloud {
                continue;
            }
            let Some(rel) = &a.relative_path else {
                continue;
            };
            let abs = mount_dir.join(rel);
            if abs.exists() {
                let meta = std::fs::metadata(&abs).ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                // 占位符（0 字节 + xattr state=placeholder）→ 仍执行删除（无实际内容）
                // 其余（size>0 或无 xattr）→ 本地文件实际存在，跳过删除
                if size == 0 && crate::mount::manager::is_placeholder_file(&abs) {
                    continue; // 空占位符，可安全删除
                }
                tracing::info!(
                    rel = %rel,
                    size,
                    reason = a.reason.as_deref().unwrap_or("?"),
                    "DeleteFromCloud 防误删：本地文件实际存在，改为 Skip"
                );
                a.action_type = crate::sync::state::SyncActionType::Skip;
                a.reason = Some(format!(
                    "防误删：本地文件实际存在（{} 字节），scan_local 漏扫 → 跳过 DeleteFromCloud",
                    size,
                ));
            }
        }
    }

    /// 删除本地文件前的云端复核（不可逆操作的最后一道防线）。
    /// 对有真实 fileId 的 DeleteFromLocal 动作，调 GET /files/{id} 确认云端确实不存在。
    /// 与 validate_delete_from_cloud 对称——删云端有 stat 校验，删本地更应有云端复核。
    async fn validate_delete_from_local(
        &self,
        actions: &mut [crate::sync::state::SyncAction],
    ) -> AppResult<()> {
        use crate::data::repository::PENDING_FILE_ID_PREFIX;
        for a in actions.iter_mut() {
            if a.action_type != crate::sync::state::SyncActionType::DeleteFromLocal {
                continue;
            }
            let Some(file_id) = &a.file_id else {
                continue;
            };
            // 占位项（pending: 前缀）无真实云端对应，不复核
            if file_id.starts_with(PENDING_FILE_ID_PREFIX) {
                continue;
            }
            let Some(rel) = &a.relative_path else {
                continue;
            };

            self.ensure_cycle_active()?;
            let _activity = self.begin_external_activity()?;
            match self.files_api.get(file_id).await {
                Ok(_) => {
                    // 云端仍存在 → cloud_exists=false 是误判（cloud_tree 残缺）→ 拦截删除
                    tracing::warn!(
                        rel = %rel,
                        fid = %file_id,
                        "删除前复核：云端仍存在该文件，跳过删除（cloud_tree 疑似残缺）"
                    );
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason =
                        Some("删除前复核：云端仍存在，跳过（cloud_tree 疑似残缺）".to_string());
                }
                Err(crate::error::AppError::DriveApi {
                    status_code: Some(404),
                    ..
                }) => {
                    // 云端确实不存在 → 允许删除（保持原动作）
                    tracing::debug!(rel = %rel, "删除前复核：云端确认不存在，允许删除");
                }
                Err(e) => {
                    // 复核请求本身失败（网络问题）→ 保守跳过，下轮再判
                    tracing::warn!(
                        rel = %rel,
                        error = %e,
                        "删除前复核请求失败，保守跳过删除"
                    );
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason = Some(format!("删除前复核失败，保守跳过：{e}"));
                }
            }
        }
        Ok(())
    }

    /// 清理残余 DB 记录：删除 local 和 cloud 都不再存在的 DB 行。
    /// planner 不再为"双方都删了"生成 DeleteFromCloud（避免无意义的 404 API 调用），
    /// 改为每轮 sync cycle 末尾统一在此清理。
    fn purge_stale_db_records(
        &self,
        local: &HashMap<String, crate::mount::manager::LocalFileEntry>,
        cloud: &HashMap<String, crate::drive::models::DriveFile>,
    ) {
        let conn = self.db.lock();
        // 收集 local 和 cloud 都缺失但 DB 有的路径
        let stale: Vec<String> = {
            let Ok(mut stmt) = conn.prepare("SELECT local_path FROM sync_items") else {
                tracing::warn!("purge_stale_db_records: prepare 失败，跳过本轮清理");
                return;
            };
            let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
                tracing::warn!("purge_stale_db_records: query_map 失败，跳过本轮清理");
                return;
            };
            rows.filter_map(|r| r.ok())
                .filter(|p| !local.contains_key(p) && !cloud.contains_key(p))
                .collect()
        };
        if stale.is_empty() {
            return;
        }
        for path in &stale {
            let _ = conn.execute(
                "DELETE FROM sync_items WHERE local_path=?1",
                rusqlite::params![path],
            );
        }
        tracing::info!(count = stale.len(), "清理残余 DB 记录（双方都已不存在）");
    }

    pub async fn retry_failed(&self) -> AppResult<()> {
        self.run_sync_cycle("retry-failed").await
    }

    fn request_retry_replan_if_restart_required(self: &Arc<Self>, task_id: i64) -> bool {
        let restart_required = match repository::get_transfer_by_id(&self.db.lock(), task_id) {
            Ok(Some(task)) => task.state_kind() == Ok(TransferState::RestartRequired),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(task_id, %error, "检查重试任务是否需要重规划失败");
                false
            }
        };
        if restart_required {
            self.request_cycle_background("retry-failed");
        }
        restart_required
    }

    /// Retry one durable transfer through the same runner used by automatic/startup work.
    pub async fn retry_transfer(self: &Arc<Self>, task_id: i64) -> AppResult<()> {
        let activity = self.begin_external_activity()?;
        let task_runner = self.task_runner()?;
        let pending = match task_runner.prepare_retry(task_id).await {
            Ok(pending) => pending,
            Err(error) => {
                if self.request_retry_replan_if_restart_required(task_id) {
                    return Ok(());
                }
                return Err(error);
            }
        };
        #[cfg(test)]
        if let Some(hook) = self.retry_prepared_hook.lock().clone() {
            hook(task_id);
        }
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let _activity = activity;
            match task_runner.run_prepared(pending.id).await {
                Ok(outcome) => {
                    if outcome.disposition == TaskDisposition::RestartRequired {
                        engine.request_retry_replan_if_restart_required(task_id);
                    } else if let Some(cloud_file) = outcome.cloud_file {
                        let relative_path = {
                            let conn = engine.db.lock();
                            repository::get_transfer_by_id(&conn, task_id)
                                .ok()
                                .flatten()
                                .and_then(|task| task.relative_path)
                        };
                        if let Some(relative_path) = relative_path {
                            engine.cloud_tree_insert(relative_path.clone(), cloud_file.clone());
                            engine.path_to_id_insert(relative_path, cloud_file.id.clone());
                        }
                    }
                }
                Err(error) => {
                    if !engine.request_retry_replan_if_restart_required(task_id) {
                        tracing::warn!(task_id, %error, "后台重试任务失败");
                    }
                }
            }
            engine.notify_backoff_schedule_changed();
        });
        Ok(())
    }
}

/// 防振荡过滤。
fn filter_anti_oscillation(
    actions: &mut Vec<crate::sync::state::SyncAction>,
    rdp: &HashMap<String, i64>,
) {
    use crate::sync::state::SyncActionType;
    actions.retain(|a| {
        let rel = match &a.relative_path {
            Some(p) => p,
            None => return true,
        };
        !rdp.contains_key(rel) || matches!(a.action_type, SyncActionType::DeleteFromCloud)
    });
}

/// 填充 parent_file_id。
fn fill_parent_file_ids(
    actions: &mut [crate::sync::state::SyncAction],
    p2i: &HashMap<String, String>,
) {
    for a in actions {
        if a.parent_file_id.is_some() || a.relative_path.is_none() {
            continue;
        }
        let rel = a.relative_path.as_ref().unwrap();
        if let Some(pos) = rel.rfind('/') {
            if let Some(pid) = p2i.get(&rel[..pos]) {
                a.parent_file_id = Some(pid.clone());
            }
        }
    }
}

/// 为「云端已删除目录下、有云端内容要创建」的动作补建目录链到云端。
///
/// 场景：云端删了整个目录 B（含 B/sub），本地改过 B/sub/f2.txt → f2 走
/// BackupBeforeCloudDelete 改名备份（本地目录链已保留）。但副本下轮 Upload、
/// 或本轮有 Upload/冲突副本/本地新目录落在被删目录下时，父目录已不在云端
/// path_to_id → 内容会落到云端根目录。本方法为这些「被删但有内容要放进去」的
/// 祖先目录补 CreateFolder（cloud_file=None，本地→云端重建），execute_actions_ordered
/// 阶段 1 会先建它们并回填 path_to_id，内容即落到正确目录。
fn add_rescue_folder_recreations(
    actions: &mut Vec<crate::sync::state::SyncAction>,
    snapshot: &crate::sync::planner::SyncSnapshot,
    recently_deleted: &std::collections::HashMap<String, i64>,
) {
    use crate::sync::state::{SyncAction, SyncActionType};

    // 仅对「创建云端内容」的动作（上传/备份副本/冲突副本/本地新建目录）补建父目录链；
    // 下载/删除/占位不创建云端内容，无需为其父目录重建（避免误重建正在清理的目录）。
    let rescue_paths: Vec<String> = actions
        .iter()
        .filter(|a| {
            matches!(
                a.action_type,
                SyncActionType::Upload
                    | SyncActionType::BackupBeforeCloudDelete
                    | SyncActionType::CreateConflictCopy
            ) || (a.action_type == SyncActionType::CreateFolder && a.cloud_file.is_none())
        })
        .filter_map(|a| a.relative_path.clone())
        .collect();
    if rescue_paths.is_empty() {
        return;
    }

    // 已有动作的路径（owned，避免与下方 push 的可变借用冲突）
    let existing: std::collections::HashSet<String> = actions
        .iter()
        .filter_map(|a| a.relative_path.clone())
        .collect();

    let mut to_recreate: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in &rescue_paths {
        // 枚举所有祖先目录前缀
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let ancestor = parts[..i].join("/");
            if existing.contains(&ancestor) {
                continue;
            }
            // 祖先是「云端已删除的本地目录」：本地有目录 + 云端无 + db 有
            // ★ 跳过用户主动删除的目录（已在 recently_deleted 中），避免"救援重建"已删除目录
            let is_deleted_folder = snapshot
                .local
                .get(&ancestor)
                .map(|e| e.is_folder)
                .unwrap_or(false)
                && !snapshot.cloud.contains_key(&ancestor)
                && snapshot.db.contains_key(&ancestor)
                && !recently_deleted.contains_key(&ancestor);
            if is_deleted_folder {
                to_recreate.insert(ancestor);
            }
        }
    }
    if to_recreate.is_empty() {
        return;
    }

    // 按深度升序加入（父先建）；execute_actions_ordered 阶段 1 会再排一次并回填 path_to_id
    let mut folders: Vec<String> = to_recreate.into_iter().collect();
    folders.sort_by_key(|p| p.matches('/').count());
    for rel in folders {
        let Some(entry) = snapshot.local.get(&rel) else {
            continue;
        };
        actions.push(SyncAction {
            action_type: SyncActionType::CreateFolder,
            relative_path: Some(rel.clone()),
            file_id: None,
            parent_file_id: None,
            local_path: Some(entry.absolute_path.to_string_lossy().to_string()),
            cloud_file: None,
            reason: Some("云端已删除但内有内容需救援 → 重建目录到云端".to_string()),
        });
        tracing::info!(rel = %rel, "为救援内容补建云端目录");
    }
}

/// §2.12 目录级联删除去重：若 DeleteFromCloud 目标的某个祖先目录也在本次删除列表中，
/// 则跳过该条删除（目录级 API 调用会自动级联处理整个子树）。
///
/// 此外，若某个云端目录下的**全部文件**都在删除列表中（但目录自身不在），
/// 则补建目录的 DeleteFromCloud 并移除所有子文件删除——避免回收站文件被打平。
///
/// 华为 API 对目录设置 `recycled: true` 会将整个子树移入回收站（保留目录层级）。
fn dedupe_directory_deletes(
    actions: &mut Vec<crate::sync::state::SyncAction>,
    cloud_tree: &std::collections::HashMap<String, crate::drive::models::DriveFile>,
    db: &std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>,
) {
    use crate::sync::state::{SyncAction, SyncActionType};

    // ── 第一遍：收集所有 DeleteFromCloud 的路径，按深度升序排列 ──
    let mut delete_paths: Vec<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::DeleteFromCloud)
        .filter_map(|a| a.relative_path.clone())
        .collect();
    delete_paths.sort_by_key(|p| p.matches('/').count()); // 浅路径在前（祖先优先）

    if delete_paths.is_empty() {
        return;
    }

    // ── 第二遍：祖先前缀过滤 ──
    // 若路径 X 的某个祖先也在删除列表中 → X 被祖先的级联删除覆盖 → 移除
    let ancestor_set: std::collections::HashSet<&str> =
        delete_paths.iter().map(|s| s.as_str()).collect();
    // 收集被跳过的路径，用于立即清理 DB 记录
    let mut skipped_rel_paths: Vec<String> = Vec::new();
    actions.retain(|a| {
        if a.action_type != SyncActionType::DeleteFromCloud {
            return true;
        }
        let Some(rel) = &a.relative_path else {
            return true;
        };
        // 检查是否有祖先目录也在删除列表中
        let has_ancestor = (0..rel.len())
            .any(|i| rel.as_bytes().get(i) == Some(&b'/') && ancestor_set.contains(&rel[..i]));
        if has_ancestor {
            skipped_rel_paths.push(rel.clone());
            return false;
        }
        // ★ 即使祖先不在当前删除列表中，若祖先在 cloud_tree 中也不存在了
        //（说明祖先前一轮已被级联回收），同样跳过 API 调用（只清 DB），
        // 避免回收站平铺、层级丢失。
        {
            let has_deleted_ancestor = (0..rel.len()).any(|i| {
                rel.as_bytes().get(i) == Some(&b'/') && !cloud_tree.contains_key(&rel[..i])
            });
            if has_deleted_ancestor {
                skipped_rel_paths.push(rel.clone());
                return false;
            }
        }
        true
    });

    // ★ 立即清理被跳过文件的 DB 记录：虽然跳过了 API 调用（由目录级联覆盖），
    // 但 DB 记录必须同步清除，否则之后 planner 会误认为文件仍在云端。
    if !skipped_rel_paths.is_empty() {
        let conn = db.lock();
        for rel in &skipped_rel_paths {
            let _ = conn.execute(
                "DELETE FROM sync_items WHERE local_path=?1",
                rusqlite::params![rel],
            );
        }
        tracing::info!(
            count = skipped_rel_paths.len(),
            "目录级联删除：跳过 {} 个子条目的 API 调用，同步清除 DB 记录",
            skipped_rel_paths.len(),
        );
    }

    // ── 第三遍：收集剩余的文件删除，按云端父目录分组 ──
    // 若一个云端目录下所有文件都要被删除（但目录本身不在删除列表），则补建目录删除
    let remaining_deletes: Vec<&SyncAction> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::DeleteFromCloud)
        .collect();

    // 按最近云端父目录分组
    let mut dir_files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for a in &remaining_deletes {
        let Some(rel) = &a.relative_path else {
            continue;
        };
        // 跳过已经是目录的删除（在上一步已保留）
        if let Some(entry) = cloud_tree.get(rel.as_str()) {
            if entry.is_folder() {
                continue;
            }
        }
        // 找最近云端父目录
        let mut parent = rel.as_str();
        while let Some(pos) = parent.rfind('/') {
            parent = &parent[..pos];
            if let Some(entry) = cloud_tree.get(parent) {
                if entry.is_folder() {
                    dir_files
                        .entry(parent.to_string())
                        .or_default()
                        .push(rel.clone());
                    break;
                }
            }
        }
    }

    // 对每组，检查是否该目录下所有云端文件都在删除列表中
    let mut merged = 0usize;
    for (dir, deleting) in &dir_files {
        // 统计 cloud_tree 中该目录下的文件总数（不含子目录）
        let total_in_cloud = cloud_tree
            .keys()
            .filter(|k| {
                k.starts_with(&format!("{}/", dir))
                    && !k[dir.len() + 1..].contains('/') // 仅直接子项
                    && cloud_tree.get(*k).map(|f| !f.is_folder()).unwrap_or(false)
            })
            .count();
        // 只有目录下**全部**文件都要删除时，才合并为目录删除
        if total_in_cloud == 0 || deleting.len() < total_in_cloud {
            continue;
        }
        // 移除所有子文件删除
        let before = actions.len();
        let mut removed_paths: Vec<String> = Vec::new();
        actions.retain(|a| {
            if a.action_type != SyncActionType::DeleteFromCloud {
                return true;
            }
            let Some(rel) = &a.relative_path else {
                return true;
            };
            if rel == dir {
                return true; // 保留目录自身
            }
            if deleting.contains(rel) {
                removed_paths.push(rel.clone());
                return false;
            }
            true
        });
        let removed = before - actions.len();
        merged += removed;

        // ★ 同步清理被合并文件的 DB 记录
        if !removed_paths.is_empty() {
            let conn = db.lock();
            for rel in &removed_paths {
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1",
                    rusqlite::params![rel],
                );
            }
        }

        // 补建目录 DeleteFromCloud（如果还没有）
        let has_dir = actions.iter().any(|a| {
            a.action_type == SyncActionType::DeleteFromCloud
                && a.relative_path.as_deref() == Some(dir.as_str())
        });
        if !has_dir {
            if let Some(dir_entry) = cloud_tree.get(dir.as_str()) {
                actions.push(SyncAction {
                    action_type: SyncActionType::DeleteFromCloud,
                    relative_path: Some(dir.clone()),
                    file_id: Some(dir_entry.id.clone()),
                    parent_file_id: dir_entry
                        .parent_folder
                        .as_ref()
                        .and_then(|v| v.first().cloned()),
                    local_path: None,
                    cloud_file: None,
                    reason: Some(format!(
                        "合并 {} 个子文件为目录级删除（目录共 {} 文件，全部删除）",
                        deleting.len(),
                        total_in_cloud,
                    )),
                });
            }
        }
        tracing::info!(
            dir = %dir,
            files = deleting.len(),
            cloud_total = total_in_cloud,
            "目录级联删除：{} 个文件合并为目录删除",
            deleting.len(),
        );
    }

    if !skipped_rel_paths.is_empty() || merged > 0 {
        tracing::info!(
            skipped_by_ancestor = skipped_rel_paths.len(),
            merged_files_to_dirs = merged,
            "目录级联删除去重完成"
        );
    }
}

/// §2.13 目录删除保护：若云端目录下有文件被 BackupBeforeCloudDelete（本地修改过
/// 需要备份保存），则移除该目录的 DeleteFromLocal，保留目录作为备份副本的栖身之所。
/// 其余无本地修改的目录正常删除。
fn preserve_dirs_with_pending_backups(actions: &mut Vec<crate::sync::state::SyncAction>) {
    use crate::sync::state::SyncActionType;
    // 收集所有 BackupBeforeCloudDelete 的目标路径（owned，避免 borrow 冲突）
    let backup_paths: std::collections::HashSet<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete)
        .filter_map(|a| a.relative_path.clone())
        .collect();
    if backup_paths.is_empty() {
        return;
    }
    // 找出哪些 DeleteFromLocal 目标是目录，且其下有文件需要备份
    let mut preserved = 0usize;
    actions.retain(|a| {
        if a.action_type != SyncActionType::DeleteFromLocal {
            return true;
        }
        let Some(rel) = &a.relative_path else {
            return true;
        };
        // 检查是否有 BackupBeforeCloudDelete 的文件在此目录下
        let has_backup_child = backup_paths
            .iter()
            .any(|bp| bp.starts_with(&format!("{}/", rel)));
        if has_backup_child {
            tracing::info!(
                dir = %rel,
                "保留本地目录：目录下有文件需 BackupBeforeCloudDelete（备份副本需要栖身目录）"
            );
            preserved += 1;
            return false; // 移除 DeleteFromLocal
        }
        true
    });
    if preserved > 0 {
        tracing::info!(
            preserved,
            "目录删除保护：保留 {} 个有备份子文件的目录",
            preserved
        );
    }
}

/// DeleteFromLocal 祖先去重：若目录自身已在 DeleteFromLocal 列表中，
/// 则其子孙的文件删除动作是多余的（目录 delete 会级联清空）。移除它们，
/// 避免并发执行时报 "No such file or directory"。
fn dedupe_local_descendants(actions: &mut Vec<crate::sync::state::SyncAction>) {
    use crate::sync::state::SyncActionType;
    // 收集所有 DeleteFromLocal 的路径（owned，避免 borrow 冲突）
    let delete_paths: Vec<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::DeleteFromLocal)
        .filter_map(|a| a.relative_path.clone())
        .collect();
    let ancestor_set: std::collections::HashSet<&str> =
        delete_paths.iter().map(|s| s.as_str()).collect();
    let mut skipped = 0usize;
    actions.retain(|a| {
        if a.action_type != SyncActionType::DeleteFromLocal {
            return true;
        }
        let Some(rel) = &a.relative_path else {
            return true;
        };
        let has_ancestor = (0..rel.len())
            .any(|i| rel.as_bytes().get(i) == Some(&b'/') && ancestor_set.contains(&rel[..i]));
        if has_ancestor {
            skipped += 1;
            return false;
        }
        true
    });
    if skipped > 0 {
        tracing::info!(
            skipped,
            "DeleteFromLocal 祖先去重：跳过 {} 个被子目录删除覆盖的文件",
            skipped
        );
    }
}

#[cfg(test)]
mod tests {
    //! apply_results 回归测试：覆盖本地新增/删除后 DB 与 cloud_tree 的回写。
    use super::{
        network_listener_loop, watcher_listener_loop, ActivityTracker, CycleCoordinator,
        CycleRequest, StartCursorSource, SyncEngine,
    };
    use crate::auth::service::AuthService;
    use crate::data::repository;
    use crate::drive::client::DriveClient;
    use crate::drive::download_api::DownloadApi;
    use crate::drive::files_api::FilesApi;
    use crate::drive::models::DriveFile;
    use crate::drive::upload_api::UploadApi;
    use crate::sync::state::{ActionResult, SyncAction, SyncActionType};
    use tempfile::tempdir;

    struct SchedulerBackend {
        calls: std::sync::Mutex<Vec<i64>>,
    }

    struct ArmGapBackend {
        calls: std::sync::Mutex<Vec<i64>>,
        submitted: tokio::sync::Notify,
    }

    struct RequestRecoveryBackend {
        calls: std::sync::Mutex<Vec<i64>>,
        attempts: std::sync::atomic::AtomicUsize,
        completed_attempt: tokio::sync::Notify,
    }

    struct AmbiguousBarrierBackend {
        calls: std::sync::Mutex<Vec<i64>>,
        submitted: tokio::sync::Notify,
        release_response: tokio::sync::Notify,
    }

    struct BatchShutdownBackend {
        calls: std::sync::Mutex<Vec<i64>>,
        first_submitted: tokio::sync::Notify,
        release_first: tokio::sync::Notify,
    }

    struct RecordingStartCursorSource {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StartCursorSource for RecordingStartCursorSource {
        async fn get_start_cursor(&self) -> crate::error::AppResult<String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("cursor-after-shutdown".to_string())
        }
    }

    #[async_trait::async_trait]
    impl crate::sync::task_runner::TransferOperations for SchedulerBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &crate::sync::task_runner::TaskProgressReporter,
        ) -> Result<
            crate::sync::task_runner::TaskExecutionOutcome,
            crate::sync::task_runner::TaskExecutionError,
        > {
            self.calls.lock().unwrap().push(task.id);
            if let Some(path) = task.local_path.as_deref() {
                std::fs::write(path, b"payload")
                    .map_err(|error| crate::error::AppError::generic(error.to_string()))?;
            }
            Ok(crate::sync::task_runner::TaskExecutionOutcome::default())
        }
    }

    #[async_trait::async_trait]
    impl crate::sync::task_runner::TransferOperations for ArmGapBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &crate::sync::task_runner::TaskProgressReporter,
        ) -> Result<
            crate::sync::task_runner::TaskExecutionOutcome,
            crate::sync::task_runner::TaskExecutionError,
        > {
            self.calls.lock().unwrap().push(task.id);
            self.submitted.notify_one();
            Ok(crate::sync::task_runner::TaskExecutionOutcome {
                cloud_file: Some(DriveFile {
                    id: format!("uploaded-{}", task.id),
                    name: task.name.clone(),
                    size: task.total_size,
                    edited_time: chrono::DateTime::from_timestamp_millis(20_000),
                    ..Default::default()
                }),
                disposition: crate::sync::task_runner::TaskDisposition::Completed,
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::sync::task_runner::TransferOperations for RequestRecoveryBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &crate::sync::task_runner::TaskProgressReporter,
        ) -> Result<
            crate::sync::task_runner::TaskExecutionOutcome,
            crate::sync::task_runner::TaskExecutionError,
        > {
            self.calls.lock().unwrap().push(task.id);
            let attempt = self
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if attempt == 0 {
                return Err(crate::sync::task_runner::TaskExecutionError::App(
                    crate::error::AppError::drive_transport(
                        crate::error::DriveTransportKind::Connect,
                        crate::error::RequestSemantics::Read,
                        false,
                        Some("request layer disconnected"),
                    ),
                ));
            }
            if let Some(path) = task.local_path.as_deref() {
                std::fs::write(path, b"payload")
                    .map_err(|error| crate::error::AppError::generic(error.to_string()))?;
            }
            self.completed_attempt.notify_one();
            Ok(crate::sync::task_runner::TaskExecutionOutcome::default())
        }
    }

    #[async_trait::async_trait]
    impl crate::sync::task_runner::TransferOperations for AmbiguousBarrierBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &crate::sync::task_runner::TaskProgressReporter,
        ) -> Result<
            crate::sync::task_runner::TaskExecutionOutcome,
            crate::sync::task_runner::TaskExecutionError,
        > {
            self.calls.lock().unwrap().push(task.id);
            self.submitted.notify_one();
            self.release_response.notified().await;
            Ok(crate::sync::task_runner::TaskExecutionOutcome {
                cloud_file: Some(DriveFile {
                    id: format!("ambiguous-remote-{}", task.id),
                    name: task.name.clone(),
                    size: task.total_size,
                    ..Default::default()
                }),
                disposition: crate::sync::task_runner::TaskDisposition::VerifyingRemote,
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::sync::task_runner::TransferOperations for BatchShutdownBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &crate::sync::task_runner::TaskProgressReporter,
        ) -> Result<
            crate::sync::task_runner::TaskExecutionOutcome,
            crate::sync::task_runner::TaskExecutionError,
        > {
            let first = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(task.id);
                calls.len() == 1
            };
            if first {
                self.first_submitted.notify_one();
                self.release_first.notified().await;
            }
            if let Some(path) = task.local_path.as_deref() {
                std::fs::write(path, b"payload")
                    .map_err(|error| crate::error::AppError::generic(error.to_string()))?;
            }
            Ok(crate::sync::task_runner::TaskExecutionOutcome::default())
        }
    }

    #[tokio::test]
    async fn watcher_and_manual_barrier_run_one_plus_one_followup() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let coordinator = std::sync::Arc::new(CycleCoordinator::default());
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let cycles = std::sync::Arc::new(AtomicUsize::new(0));
        let in_flight = std::sync::Arc::new(AtomicUsize::new(0));
        let max_in_flight = std::sync::Arc::new(AtomicUsize::new(0));

        let owner = {
            let coordinator = coordinator.clone();
            let entered = entered.clone();
            let release = release.clone();
            let cycles = cycles.clone();
            let in_flight = in_flight.clone();
            let max_in_flight = max_in_flight.clone();
            tokio::spawn(async move {
                coordinator
                    .run(CycleRequest::LOCAL_RESCAN, |request| {
                        let entered = entered.clone();
                        let release = release.clone();
                        let cycles = cycles.clone();
                        let in_flight = in_flight.clone();
                        let max_in_flight = max_in_flight.clone();
                        async move {
                            assert!(request.contains(CycleRequest::LOCAL_RESCAN));
                            let active = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                            max_in_flight.fetch_max(active, Ordering::SeqCst);
                            let turn = cycles.fetch_add(1, Ordering::SeqCst);
                            if turn == 0 {
                                entered.notify_one();
                                release.notified().await;
                            }
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                        }
                    })
                    .await;
            })
        };
        entered.notified().await;

        let follower = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_FULL,
                        |_| async {},
                    )
                    .await;
            })
        };
        tokio::task::yield_now().await;
        release.notify_one();
        owner.await.unwrap();
        follower.await.unwrap();

        assert_eq!(cycles.load(Ordering::SeqCst), 2);
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 1);
        assert!(coordinator.is_idle());
    }

    #[tokio::test]
    async fn many_busy_triggers_coalesce_to_one_followup() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let coordinator = std::sync::Arc::new(CycleCoordinator::default());
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let cycles = std::sync::Arc::new(AtomicUsize::new(0));
        let owner = {
            let coordinator = coordinator.clone();
            let entered = entered.clone();
            let release = release.clone();
            let cycles = cycles.clone();
            tokio::spawn(async move {
                coordinator
                    .run(CycleRequest::LOCAL_RESCAN, |_| {
                        let entered = entered.clone();
                        let release = release.clone();
                        let cycles = cycles.clone();
                        async move {
                            if cycles.fetch_add(1, Ordering::SeqCst) == 0 {
                                entered.notify_one();
                                release.notified().await;
                            }
                        }
                    })
                    .await;
            })
        };
        entered.notified().await;
        for _ in 0..100 {
            coordinator.request(CycleRequest::LOCAL_RESCAN);
        }
        release.notify_one();
        owner.await.unwrap();

        assert_eq!(cycles.load(Ordering::SeqCst), 2);
        assert!(coordinator.is_idle());
    }

    #[tokio::test]
    async fn background_request_arriving_during_nonrecoverable_failure_is_not_stranded() {
        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);

        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let local_scans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let engine_slot =
            std::sync::Arc::new(std::sync::Mutex::new(None::<std::sync::Weak<SyncEngine>>));
        engine.set_incremental_refresh_hook_for_test({
            let attempts = attempts.clone();
            let engine_slot = engine_slot.clone();
            std::sync::Arc::new(move || {
                let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if attempt == 1 {
                    // Inject the newer request synchronously while the only scheduled worker is
                    // still inside the failing production cycle.
                    engine_slot
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .upgrade()
                        .unwrap()
                        .request_cycle_background("auto-cloud-refresh");
                    Err(crate::error::AppError::generic(
                        "deterministic non-recoverable refresh failure",
                    ))
                } else {
                    Ok(())
                }
            })
        });
        engine.set_cycle_observer({
            let local_scans = local_scans.clone();
            std::sync::Arc::new(move |stage| {
                if stage == "local-rescan" {
                    local_scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        });
        let engine = std::sync::Arc::new(engine);
        *engine_slot.lock().unwrap() = Some(std::sync::Arc::downgrade(&engine));

        engine.request_cycle_background("auto-cloud-refresh");

        for _ in 0..64 {
            tokio::task::yield_now().await;
        }

        let diagnostic = {
            let coordinator_state = engine.cycle.state.lock();
            (
                coordinator_state.pending,
                coordinator_state.requested,
                coordinator_state.completed,
                engine
                    .background_scheduled
                    .load(std::sync::atomic::Ordering::Acquire),
            )
        };
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "coordinator diagnostic: {diagnostic:?}"
        );
        assert_eq!(local_scans.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!engine.cycle.has_pending());
        assert!(!engine
            .background_scheduled
            .load(std::sync::atomic::Ordering::Acquire));
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn nonrecoverable_background_failure_without_new_sequence_does_not_hot_loop() {
        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        engine.set_incremental_refresh_hook_for_test({
            let attempts = attempts.clone();
            std::sync::Arc::new(move || {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(crate::error::AppError::generic(
                    "deterministic non-recoverable refresh failure",
                ))
            })
        });
        let engine = std::sync::Arc::new(engine);

        engine.request_cycle_background("auto-cloud-refresh");
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(engine.cycle.has_pending());
        assert!(!engine
            .background_scheduled
            .load(std::sync::atomic::Ordering::Acquire));

        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn production_watcher_background_and_manual_run_one_plus_one_followup() {
        let (mut engine, db) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let local_cycles = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let manual_cycles = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (first_local_tx, first_local_rx) = std::sync::mpsc::channel();
        let (release_local_tx, release_local_rx) = std::sync::mpsc::channel();
        let (manual_requested_tx, manual_requested_rx) = std::sync::mpsc::channel();
        let first_local_tx = std::sync::Mutex::new(Some(first_local_tx));
        let release_local_rx = std::sync::Mutex::new(release_local_rx);
        let manual_requested_tx = std::sync::Mutex::new(Some(manual_requested_tx));
        engine.set_cycle_observer({
            let db = db.clone();
            let local_cycles = local_cycles.clone();
            let manual_cycles = manual_cycles.clone();
            std::sync::Arc::new(move |stage| match stage {
                "local-rescan" => {
                    if local_cycles.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        first_local_tx
                            .lock()
                            .unwrap()
                            .take()
                            .unwrap()
                            .send(())
                            .unwrap();
                        release_local_rx.lock().unwrap().recv().unwrap();
                    }
                }
                "request-manual" => {
                    if let Some(tx) = manual_requested_tx.lock().unwrap().take() {
                        tx.send(()).unwrap();
                    }
                }
                "cloud-refresh" => {
                    manual_cycles.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // Stop before real cloud I/O with a deterministic non-recoverable error.
                    db.lock()
                        .execute_batch("DROP TABLE transfer_queue;")
                        .unwrap();
                }
                _ => {}
            })
        });
        let engine = std::sync::Arc::new(engine);

        engine.request_cycle_background("local-watcher");
        first_local_rx.recv().unwrap();
        let manual = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.trigger_manual_sync().await })
        };
        manual_requested_rx.recv().unwrap();
        release_local_tx.send(()).unwrap();

        let error = manual.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("transfer_queue"));
        assert_eq!(
            local_cycles.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the watcher request must run exactly once"
        );
        assert_eq!(
            manual_cycles.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the busy manual request must become exactly one coordinated follow-up"
        );
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn production_background_hundred_busy_triggers_coalesce_to_one_followup() {
        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let local_cycles = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (first_local_tx, first_local_rx) = std::sync::mpsc::channel();
        let (release_local_tx, release_local_rx) = std::sync::mpsc::channel();
        let (followup_tx, followup_rx) = std::sync::mpsc::channel();
        let first_local_tx = std::sync::Mutex::new(Some(first_local_tx));
        let release_local_rx = std::sync::Mutex::new(release_local_rx);
        let followup_tx = std::sync::Mutex::new(Some(followup_tx));
        engine.set_cycle_observer({
            let local_cycles = local_cycles.clone();
            std::sync::Arc::new(move |stage| {
                if stage != "local-rescan" {
                    return;
                }
                match local_cycles.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    0 => {
                        first_local_tx
                            .lock()
                            .unwrap()
                            .take()
                            .unwrap()
                            .send(())
                            .unwrap();
                        release_local_rx.lock().unwrap().recv().unwrap();
                    }
                    1 => {
                        followup_tx
                            .lock()
                            .unwrap()
                            .take()
                            .unwrap()
                            .send(())
                            .unwrap();
                    }
                    _ => {}
                }
            })
        });
        let engine = std::sync::Arc::new(engine);

        engine.request_cycle_background("local-watcher");
        first_local_rx.recv().unwrap();
        for _ in 0..100 {
            engine.request_cycle_background("local-watcher");
        }
        release_local_tx.send(()).unwrap();
        followup_rx.recv().unwrap();

        let owner = engine.cycle.lock_owner().await;
        drop(owner);
        assert_eq!(local_cycles.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(!engine.cycle.has_pending());
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn release_window_trigger_is_not_lost() {
        let coordinator = CycleCoordinator::default();
        coordinator.request(CycleRequest::LOCAL_RESCAN);
        let owner = coordinator.lock_owner().await;
        assert!(coordinator
            .take_pending()
            .contains(CycleRequest::LOCAL_RESCAN));
        assert!(coordinator.take_pending().is_empty());

        coordinator.request(CycleRequest::ONLINE_RECOVERY);
        drop(owner);

        let owner = coordinator.lock_owner().await;
        assert!(coordinator
            .take_pending()
            .contains(CycleRequest::ONLINE_RECOVERY));
        drop(owner);
        assert!(coordinator.is_idle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_after_first_non_transfer_action_never_starts_the_second_callback() {
        use crate::sync::conflict::ConflictResolver;
        use crate::sync::executor::SyncExecutor;

        let (mut engine, _) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let first_path = root.path().join("first.txt");
        let second_path = root.path().join("second.txt");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();

        let conflict = std::sync::Arc::new(std::sync::Mutex::new(ConflictResolver::new()));
        let (conflict_held_tx, conflict_held_rx) = std::sync::mpsc::channel();
        let (release_conflict_tx, release_conflict_rx) = std::sync::mpsc::channel();
        let conflict_for_thread = conflict.clone();
        let poisoner = std::thread::spawn(move || {
            let _guard = conflict_for_thread.lock().unwrap();
            conflict_held_tx.send(()).unwrap();
            release_conflict_rx.recv().unwrap();
            panic!("poison conflict callback after the admitted action is released");
        });
        conflict_held_rx.recv().unwrap();

        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_conflict(conflict);
        engine.set_executor(executor);
        let engine = std::sync::Arc::new(engine);
        let actions = [first_path, second_path].map(|path| SyncAction {
            action_type: SyncActionType::CreateConflictCopy,
            relative_path: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            file_id: Some("remote-conflict".to_string()),
            parent_file_id: None,
            local_path: Some(path.to_string_lossy().into_owned()),
            cloud_file: Some(DriveFile {
                id: "remote-conflict".to_string(),
                name: "conflict.txt".to_string(),
                ..Default::default()
            }),
            reason: Some("deterministic shutdown admission test".to_string()),
        });

        let batch = {
            let engine = engine.clone();
            tokio::spawn(async move {
                engine
                    .executor
                    .as_ref()
                    .expect("executor installed through production set_executor")
                    .execute_all(&actions)
                    .await
            })
        };
        let first_action_admitted =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                loop {
                    if engine.activity.state.lock().count == 1 {
                        break;
                    }
                    // Give the spawned batch a real scheduling interval; a fixed number of
                    // yields can finish before Tokio polls a newly spawned task on another worker.
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            })
            .await
            .is_ok();

        engine.shutdown_sync();
        release_conflict_tx.send(()).unwrap();
        assert!(poisoner.join().is_err());
        let results = batch.await.unwrap();

        assert!(
            first_action_admitted,
            "the first action must acquire the engine activity permit after executor admission"
        );
        assert!(
            !results[0].deferred,
            "the admitted first callback may settle"
        );
        assert!(
            results[1].deferred,
            "the queued second action must be canceled without entering its callback"
        );
        assert!(results[1]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("已停止")));
    }

    #[tokio::test]
    async fn shutdown_before_start_cursor_admission_skips_remote_and_cache_work() {
        let (mut engine, _) = build_engine();
        let source = std::sync::Arc::new(RecordingStartCursorSource {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        engine.start_cursor_source = source.clone();
        let mount = tempfile::tempdir().unwrap();
        let mount_dir = mount.path().to_string_lossy().into_owned();
        let checkpoint_path = crate::core::cache_paths::cloud_tree_cache_file(&mount_dir).unwrap();
        let _ = std::fs::remove_file(&checkpoint_path);

        engine.shutdown_sync();
        assert!(engine
            .build_and_commit_full_checkpoint(&mount_dir)
            .await
            .is_err());

        assert_eq!(
            source.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "closed engines must not start getStartCursor"
        );
        assert!(
            !checkpoint_path.exists(),
            "closed engines must not write cloud checkpoint"
        );
    }

    #[tokio::test]
    async fn shutdown_activity_gate_rejects_new_work_and_waits_for_settlement() {
        let tracker = std::sync::Arc::new(ActivityTracker::default());
        let submitted = tracker.begin().unwrap();
        tracker.close();
        assert!(tracker.begin().is_err());

        let waiter = {
            let tracker = tracker.clone();
            tokio::spawn(async move { tracker.wait_idle().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        drop(submitted);
        waiter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn last_activity_wakes_all_registered_idle_waiters() {
        let tracker = std::sync::Arc::new(ActivityTracker::default());
        let activity = tracker.begin().unwrap();
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();

        let first = {
            let tracker = tracker.clone();
            let started_tx = started_tx.clone();
            tokio::spawn(async move {
                started_tx.send(()).unwrap();
                tracker.wait_idle().await;
            })
        };
        let second = {
            let tracker = tracker.clone();
            tokio::spawn(async move {
                started_tx.send(()).unwrap();
                tracker.wait_idle().await;
            })
        };
        started_rx.recv().await.expect("first waiter started");
        started_rx.recv().await.expect("second waiter started");
        tokio::task::yield_now().await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());

        drop(activity);

        tokio::time::timeout(std::time::Duration::from_millis(1), async {
            first.await.unwrap();
            second.await.unwrap();
        })
        .await
        .expect("all registered idle waiters must wake when the final activity ends");
    }

    #[tokio::test]
    async fn coalesced_manual_waiter_observes_owner_failure() {
        let coordinator = CycleCoordinator::default();
        let watcher_sequence = coordinator.request(CycleRequest::LOCAL_RESCAN);
        let _owner = coordinator.lock_owner().await;
        let request = coordinator.take_pending();
        assert!(request.contains(CycleRequest::LOCAL_RESCAN));

        let manual_sequence = coordinator.request(CycleRequest::CLOUD_FULL);
        let merged = coordinator.take_pending();
        assert!(merged.contains(CycleRequest::CLOUD_FULL));
        let through = coordinator.requested_sequence();
        let error = crate::error::AppError::generic("cloud refresh failed");
        coordinator.complete(through, Some(&error));

        assert!(coordinator
            .result_if_completed(watcher_sequence)
            .unwrap()
            .is_err());
        assert!(coordinator
            .result_if_completed(manual_sequence)
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cloud refresh failed"));
    }

    #[test]
    fn evicted_cycle_failure_is_reported_as_expired_never_success() {
        let coordinator = CycleCoordinator::default();
        let mut first_sequence = 0;
        let mut latest_sequence = 0;
        for index in 0..129 {
            let sequence = coordinator.request(CycleRequest::LOCAL_RESCAN);
            if index == 0 {
                first_sequence = sequence;
            }
            latest_sequence = sequence;
            let error = crate::error::AppError::generic(format!("cycle failure {index}"));
            coordinator.complete(sequence, Some(&error));
        }

        let expired = coordinator
            .result_if_completed(first_sequence)
            .expect("the first sequence completed")
            .expect_err("an evicted failure must never be reported as success");
        assert!(expired.to_string().contains("结果历史已过期"));

        let latest = coordinator
            .result_if_completed(latest_sequence)
            .expect("the latest sequence completed")
            .expect_err("the latest retained failure must remain observable");
        assert!(latest.to_string().contains("cycle failure 128"));
    }

    #[tokio::test]
    async fn initial_online_and_lagged_network_receiver_level_reconcile() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let (tx, rx) = tokio::sync::broadcast::channel(2);
        let online = std::sync::Arc::new(AtomicBool::new(true));
        // Force the first recv after initial reconciliation to observe Lagged.
        tx.send(crate::core::net_guard::NetworkTransition::Offline)
            .unwrap();
        tx.send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        tx.send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        let coordinator = std::sync::Arc::new(CycleCoordinator::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let listener = {
            let online = online.clone();
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                network_listener_loop(
                    rx,
                    move || online.load(Ordering::SeqCst),
                    stop_rx,
                    true,
                    move || {
                        coordinator.request(CycleRequest::ONLINE_RECOVERY);
                    },
                )
                .await;
            })
        };
        tokio::task::yield_now().await;

        let pending = coordinator.take_pending();
        assert!(pending.contains(CycleRequest::ONLINE_RECOVERY));
        assert!(coordinator.take_pending().is_empty());

        tx.send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        tokio::task::yield_now().await;
        assert!(coordinator
            .take_pending()
            .contains(CycleRequest::ONLINE_RECOVERY));

        stop_tx.send(true).unwrap();
        listener.await.unwrap();
    }

    #[tokio::test]
    async fn watcher_lagged_batch_requests_rescan_and_listener_survives() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::TransferState;

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);

        std::fs::write(root.path().join("lagged.txt"), b"lagged payload").unwrap();
        let (changes_tx, changes_rx) = tokio::sync::broadcast::channel(2);
        let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Overflow the receiver before listener startup. The callback must run once for Lagged
        // and once for each retained tail batch, with all three routed through the real engine.
        changes_tx.send(vec!["one".into()]).unwrap();
        changes_tx.send(vec!["two".into()]).unwrap();
        changes_tx.send(vec!["three".into()]).unwrap();
        let listener = {
            let request_engine = engine.clone();
            tokio::spawn(async move {
                watcher_listener_loop(changes_rx, shutdown_rx, move || {
                    request_engine.request_cycle_background("local-watcher");
                    request_tx.send(()).unwrap();
                })
                .await;
            })
        };

        for _ in 0..3 {
            request_rx
                .recv()
                .await
                .expect("Lagged plus both retained batches are observed");
        }
        assert!(matches!(
            request_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        backend.submitted.notified().await;
        let owner = engine.cycle.lock_owner().await;
        drop(owner);

        let initial_tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(initial_tasks.len(), 1);
        assert_eq!(
            initial_tasks[0].relative_path.as_deref(),
            Some("lagged.txt")
        );
        assert_eq!(
            initial_tasks[0].state_kind().unwrap(),
            TransferState::Completed
        );
        let lagged_id = initial_tasks[0].id;
        assert_eq!(&*backend.calls.lock().unwrap(), &[lagged_id]);
        let lagged_baseline =
            repository::find_by_file_id(&db.lock(), &format!("uploaded-{lagged_id}"))
                .unwrap()
                .expect("Lagged rescan writes the confirmed success baseline");
        assert_eq!(lagged_baseline.local_path, "lagged.txt");
        assert_eq!(lagged_baseline.status, repository::sync_status::SYNCED);

        std::fs::write(root.path().join("after-lag.txt"), b"later payload").unwrap();
        changes_tx.send(vec!["after-lag".into()]).unwrap();
        request_rx
            .recv()
            .await
            .expect("listener survives Lagged and handles the next batch");
        assert!(matches!(
            request_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        backend.submitted.notified().await;
        let owner = engine.cycle.lock_owner().await;
        drop(owner);

        let final_tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(final_tasks.len(), 2);
        let after_lag = final_tasks
            .iter()
            .find(|task| task.relative_path.as_deref() == Some("after-lag.txt"))
            .expect("the post-Lagged batch reaches the planner and runner");
        assert_eq!(after_lag.state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*backend.calls.lock().unwrap(), &[lagged_id, after_lag.id]);
        let later_baseline =
            repository::find_by_file_id(&db.lock(), &format!("uploaded-{}", after_lag.id))
                .unwrap()
                .expect("the post-Lagged batch writes its success baseline");
        assert_eq!(later_baseline.local_path, "after-lag.txt");
        assert_eq!(later_baseline.status, repository::sync_status::SYNCED);
        assert!(!listener.is_finished());

        shutdown_tx.send(true).unwrap();
        listener.await.unwrap();
        engine.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn watcher_warmup_arm_gap_runs_engine_plan_and_creates_one_task() {
        use crate::mount::local_watcher::LocalWatcher;
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::TransferState;

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner);
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);

        // This is the completed startup scan. The file appears only in the scan-to-watch arm gap.
        assert!(engine.scan_local().await.is_empty());
        std::fs::write(root.path().join("arm-gap.txt"), b"payload").unwrap();

        let watcher = std::sync::Arc::new(LocalWatcher::new(root.path(), vec![], 3));
        let changes = watcher.subscribe();
        let (_event_tx, event_rx) = tokio::sync::mpsc::channel(2);
        watcher.start_event_loop_for_receiver(event_rx, true).await;
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let listener = {
            let request_engine = engine.clone();
            tokio::spawn(async move {
                watcher_listener_loop(changes, shutdown_rx, move || {
                    request_engine.request_cycle_background("local-watcher");
                })
                .await;
            })
        };

        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        backend.submitted.notified().await;
        for _ in 0..32 {
            tokio::task::yield_now().await;
            let completed = repository::list_all_transfers(&db.lock()).is_ok_and(|tasks| {
                tasks.len() == 1 && tasks[0].state_kind() == Ok(TransferState::Completed)
            });
            if completed {
                break;
            }
        }

        let tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].relative_path.as_deref(), Some("arm-gap.txt"));
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*backend.calls.lock().unwrap(), &[tasks[0].id]);

        shutdown_tx.send(true).unwrap();
        listener.await.unwrap();
        watcher.stop().await;
        engine.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn recoverable_refresh_failure_retries_without_edge_with_bounded_exponential_delay() {
        use crate::error::RequestSemantics;

        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (attempt_tx, mut attempt_rx) = tokio::sync::mpsc::unbounded_channel();
        engine.set_incremental_refresh_hook_for_test({
            let attempts = attempts.clone();
            std::sync::Arc::new(move || {
                let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                attempt_tx.send(attempt).unwrap();
                if attempt < 3 {
                    Err(crate::error::AppError::drive_from_response(
                        503,
                        "{}",
                        None,
                        RequestSemantics::Read,
                        false,
                    ))
                } else {
                    Ok(())
                }
            })
        });
        let scans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (scan_tx, mut scan_rx) = tokio::sync::mpsc::unbounded_channel();
        engine.set_cycle_observer({
            let scans = scans.clone();
            std::sync::Arc::new(move |stage| {
                if stage == "local-rescan" {
                    scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    scan_tx.send(()).unwrap();
                }
            })
        });
        let engine = std::sync::Arc::new(engine);

        engine.request_cycle_background("network-recovery");
        assert_eq!(attempt_rx.recv().await, Some(1));
        assert_eq!(scans.load(std::sync::atomic::Ordering::SeqCst), 0);

        tokio::time::advance(std::time::Duration::from_millis(999)).await;
        tokio::task::yield_now().await;
        assert!(attempt_rx.try_recv().is_err());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert_eq!(attempt_rx.recv().await, Some(2));

        // Second recoverable failure backs off for 2 seconds, rather than scanning/calling every
        // fixed second forever. No external trigger is injected between these attempts.
        tokio::time::advance(std::time::Duration::from_millis(1_999)).await;
        tokio::task::yield_now().await;
        assert!(attempt_rx.try_recv().is_err());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert_eq!(attempt_rx.recv().await, Some(3));
        scan_rx
            .recv()
            .await
            .expect("successful refresh reaches local scan");

        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(scans.load(std::sync::atomic::Ordering::SeqCst), 1);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(engine.cycle.is_idle());
        engine.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn offline_watcher_event_poll_zero_recovers_at_stable_online_without_second_event() {
        use crate::core::net_guard::NetworkStateMachine;
        use crate::mount::local_watcher::LocalWatcher;
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::TransferState;
        use notify::{Event, EventKind};

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            {
                let online = online.clone();
                std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
            },
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner);
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        engine.set_cycle_observer({
            let order = order.clone();
            std::sync::Arc::new(move |stage| order.lock().unwrap().push(stage))
        });
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);

        let watcher = std::sync::Arc::new(LocalWatcher::new(root.path(), vec![], 3));
        let changes = watcher.subscribe();
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(2);
        watcher.start_event_loop_for_receiver(event_rx, false).await;
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let watcher_listener = {
            let request_engine = engine.clone();
            let stop_rx = stop_rx.clone();
            tokio::spawn(async move {
                watcher_listener_loop(changes, stop_rx, move || {
                    request_engine.request_cycle_background("local-watcher");
                })
                .await;
            })
        };

        let (network_tx, network_rx) = tokio::sync::broadcast::channel(4);
        let network_listener = {
            let request_engine = engine.clone();
            let online = online.clone();
            tokio::spawn(async move {
                network_listener_loop(
                    network_rx,
                    move || online.load(std::sync::atomic::Ordering::SeqCst),
                    stop_rx,
                    false,
                    move || request_engine.request_cycle_background("network-recovery"),
                )
                .await;
            })
        };

        std::fs::write(root.path().join("offline.txt"), b"offline payload").unwrap();
        event_tx
            .send(
                Event::new(EventKind::Create(notify::event::CreateKind::File))
                    .add_path(root.path().join("offline.txt")),
            )
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(3)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(engine.cycle.has_pending());
        assert!(backend.calls.lock().unwrap().is_empty());

        let mut network = NetworkStateMachine::new(false);
        for (index, probe_succeeded) in [true, false, true, true].into_iter().enumerate() {
            if let Some(transition) = network.observe(probe_succeeded) {
                online.store(network.is_online(), std::sync::atomic::Ordering::SeqCst);
                network_tx.send(transition).unwrap();
            }
            if index < 3 {
                for _ in 0..4 {
                    tokio::task::yield_now().await;
                }
                assert!(backend.calls.lock().unwrap().is_empty());
            }
        }

        backend.submitted.notified().await;
        for _ in 0..32 {
            tokio::task::yield_now().await;
            if repository::list_all_transfers(&db.lock()).is_ok_and(|tasks| {
                tasks.len() == 1 && tasks[0].state_kind() == Ok(TransferState::Completed)
            }) {
                break;
            }
        }
        let tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].relative_path.as_deref(), Some("offline.txt"));
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        {
            let order = order.lock().unwrap();
            let waiting = order
                .iter()
                .position(|stage| *stage == "resume-waiting")
                .unwrap();
            let cloud = order
                .iter()
                .position(|stage| *stage == "cloud-refresh")
                .unwrap();
            let local = order
                .iter()
                .position(|stage| *stage == "local-rescan")
                .unwrap();
            assert!(waiting < cloud && cloud < local);
        }

        stop_tx.send(true).unwrap();
        watcher_listener.await.unwrap();
        network_listener.await.unwrap();
        watcher.stop().await;
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn manual_retry_source_changed_sticky_replan_completes_same_id_without_second_event() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let relative_path = "manual-replan.bin";
        let source = root.path().join(relative_path);
        std::fs::write(&source, b"old").unwrap();
        let old_metadata = std::fs::metadata(&source).unwrap();
        let old_mtime = old_metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let old_size = old_metadata.len() as i64;
        let pending_file_id = format!("{}{relative_path}", repository::PENDING_FILE_ID_PREFIX);
        repository::upsert(
            &db.lock(),
            &repository::SyncItem {
                file_id: pending_file_id.clone(),
                local_path: relative_path.into(),
                parent_folder_id: None,
                name: relative_path.into(),
                is_folder: false,
                size: old_size,
                local_size: Some(old_size),
                sha256: None,
                local_mtime: Some(old_mtime),
                cloud_edited_time: None,
                last_sync_time: None,
                status: repository::sync_status::FAILED,
                error_message: Some("old sync failure".into()),
            },
        )
        .unwrap();
        let task_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::UPLOAD,
                file_id: None,
                local_path: Some(source.to_string_lossy().into_owned()),
                name: relative_path.into(),
                total_size: old_size,
                transferred: 0,
                state: TransferState::Failed.into(),
                error_message: Some("old transfer failure".into()),
                created_at: 1,
                finished_at: Some(2),
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some(relative_path.into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Create.into()),
                source_mtime: Some(old_mtime),
                source_size: Some(old_size),
                expected_cloud_edited_time: None,
                attempt_count: 1,
                next_retry_at: None,
                error_kind: Some(TransferErrorKind::Unknown.into()),
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();
        std::fs::write(&source, b"new source content").unwrap();

        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);

        let owner = engine.cycle.lock_owner().await;
        engine.retry_transfer(task_id).await.unwrap();
        let still_failed = repository::find_by_file_id(&db.lock(), &pending_file_id)
            .unwrap()
            .unwrap();
        assert_eq!(still_failed.status, repository::sync_status::FAILED);
        assert!(still_failed.error_message.is_some());
        assert_eq!(engine.current_state().failed, 1);
        drop(owner);
        backend.submitted.notified().await;
        let owner = engine.cycle.lock_owner().await;
        drop(owner);

        let tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task_id);
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*backend.calls.lock().unwrap(), &[task_id]);
        let sync_item = repository::find_by_file_id(&db.lock(), &format!("uploaded-{task_id}"))
            .unwrap()
            .unwrap();
        assert_eq!(sync_item.status, repository::sync_status::SYNCED);
        assert_eq!(sync_item.error_message, None);
        let state = engine.current_state();
        assert_eq!(state.failed, 0);
        assert_eq!(state.transfer_failed, 0);
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn offline_manual_retry_source_changed_waits_for_production_online_recovery() {
        use crate::sync::transfer_state::TransferState;

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fixture = build_manual_retry_replan_fixture(online.clone());
        std::fs::write(&fixture.source, b"changed while offline").unwrap();

        fixture
            .engine
            .retry_transfer(fixture.task_id)
            .await
            .unwrap();

        let deferred = repository::get_transfer_by_id(&fixture.db.lock(), fixture.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            deferred.state_kind().unwrap(),
            TransferState::RestartRequired
        );
        let failed_sync = repository::find_by_file_id(
            &fixture.db.lock(),
            &format!(
                "{}{}",
                repository::PENDING_FILE_ID_PREFIX,
                fixture.relative_path
            ),
        )
        .unwrap()
        .unwrap();
        assert_eq!(failed_sync.status, repository::sync_status::FAILED);
        assert!(failed_sync.error_message.is_some());
        assert!(fixture.backend.calls.lock().unwrap().is_empty());
        assert!(fixture.engine.cycle.has_pending());

        online.store(true, std::sync::atomic::Ordering::SeqCst);
        fixture.engine.request_cycle_background("network-recovery");
        fixture.backend.submitted.notified().await;
        let owner = fixture.engine.cycle.lock_owner().await;
        drop(owner);

        let tasks = repository::list_all_transfers(&fixture.db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, fixture.task_id);
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*fixture.backend.calls.lock().unwrap(), &[fixture.task_id]);
        let synced = repository::find_by_file_id(
            &fixture.db.lock(),
            &format!("uploaded-{}", fixture.task_id),
        )
        .unwrap()
        .unwrap();
        assert_eq!(synced.status, repository::sync_status::SYNCED);
        assert_eq!(synced.error_message, None);
        fixture.engine.shutdown().await;
    }

    #[tokio::test]
    async fn source_changed_after_retry_prepare_is_automatically_replanned() {
        use crate::sync::transfer_state::TransferState;

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let fixture = build_manual_retry_replan_fixture(online);
        fixture.engine.install_retry_prepared_hook_for_test({
            let source = fixture.source.clone();
            std::sync::Arc::new(move |_| {
                std::fs::write(&source, b"changed after prepare").unwrap();
            })
        });

        fixture
            .engine
            .retry_transfer(fixture.task_id)
            .await
            .unwrap();
        fixture.backend.submitted.notified().await;
        let owner = fixture.engine.cycle.lock_owner().await;
        drop(owner);

        let tasks = repository::list_all_transfers(&fixture.db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, fixture.task_id);
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*fixture.backend.calls.lock().unwrap(), &[fixture.task_id]);
        fixture.engine.shutdown().await;
    }

    #[tokio::test]
    async fn bound_production_task_state_sink_reports_waiting_once_and_ignores_nonwaiting() {
        use crate::sync::task_runner::{TaskDisposition, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let reports = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (mut engine, db) = build_engine();
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine.set_request_network_failure_reporter_for_test({
            let online = online.clone();
            let reports = reports.clone();
            std::sync::Arc::new(move || {
                reports.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                online.store(false, std::sync::atomic::Ordering::SeqCst);
                true
            })
        });
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(RequestRecoveryBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            attempts: std::sync::atomic::AtomicUsize::new(0),
            completed_attempt: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend,
            {
                let online = online.clone();
                std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
            },
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);

        let transfer = |name: &str| repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::DOWNLOAD,
            file_id: Some(format!("remote-{name}")),
            local_path: Some(root.path().join(name).to_string_lossy().into_owned()),
            name: name.into(),
            total_size: 7,
            transferred: 0,
            state: TransferState::Pending.into(),
            error_message: None,
            created_at: 1,
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(name.into()),
            parent_file_id: None,
            operation: Some(TransferOperation::Download.into()),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: Some(20_000),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };

        let waiting = runner
            .enqueue_and_run(transfer("waiting.bin"))
            .await
            .unwrap();
        assert_eq!(
            waiting.outcome.disposition,
            TaskDisposition::WaitingForNetwork
        );
        assert_eq!(reports.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!online.load(std::sync::atomic::Ordering::SeqCst));

        repository::delete_all_transfers(&db.lock()).unwrap();
        online.store(true, std::sync::atomic::Ordering::SeqCst);
        let completed = runner
            .enqueue_and_run(transfer("completed.bin"))
            .await
            .unwrap();
        assert_eq!(completed.outcome.disposition, TaskDisposition::Completed);
        assert_eq!(reports.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn bound_production_sink_resumes_two_initial_waiting_rows_on_one_online_level() {
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let reports = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (mut engine, db) = build_engine();
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine.set_request_network_failure_reporter_for_test({
            let online = online.clone();
            let reports = reports.clone();
            std::sync::Arc::new(move || {
                reports.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                online.store(false, std::sync::atomic::Ordering::SeqCst);
                true
            })
        });
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(SchedulerBackend {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            {
                let online = online.clone();
                std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
            },
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let insert_waiting = |name: &str, created_at: i64| {
            repository::insert_transfer(
                &db.lock(),
                &repository::TransferTask {
                    id: 0,
                    direction: repository::transfer_direction::DOWNLOAD,
                    file_id: Some(format!("remote-{name}")),
                    local_path: Some(root.path().join(name).to_string_lossy().into_owned()),
                    name: name.into(),
                    total_size: 7,
                    transferred: 0,
                    state: TransferState::WaitingForNetwork.into(),
                    error_message: Some("persisted offline".into()),
                    created_at,
                    finished_at: None,
                    server_id: None,
                    upload_id: None,
                    resume_offset: 0,
                    session_url: None,
                    relative_path: Some(name.into()),
                    parent_file_id: None,
                    operation: Some(TransferOperation::Download.into()),
                    source_mtime: None,
                    source_size: None,
                    expected_cloud_edited_time: Some(20_000),
                    attempt_count: 0,
                    next_retry_at: None,
                    error_kind: None,
                    remote_result_file_id: None,
                    state_revision: 0,
                },
            )
            .unwrap()
        };
        let first_id = insert_waiting("waiting-first.bin", 2);
        let second_id = insert_waiting("waiting-second.bin", 1);
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);

        let resumed = runner.resume_waiting().await.unwrap();

        assert_eq!(resumed, 2, "one Online level must drain both Waiting rows");
        assert_eq!(reports.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(online.load(std::sync::atomic::Ordering::SeqCst));
        let rows = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|task| task.state_kind() == Ok(TransferState::Completed)));
        assert_eq!(&*backend.calls.lock().unwrap(), &[first_id, second_id]);
    }

    #[tokio::test]
    async fn request_waiting_reports_offline_then_stable_probes_resume_same_id_before_rescan() {
        use crate::core::net_guard::{NetworkStateMachine, NetworkTransition};
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::{TaskDisposition, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let network = std::sync::Arc::new(parking_lot::Mutex::new(NetworkStateMachine::new(true)));
        let (network_tx, network_rx) = tokio::sync::broadcast::channel(8);
        let mut audit_rx = network_tx.subscribe();
        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(RequestRecoveryBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            attempts: std::sync::atomic::AtomicUsize::new(0),
            completed_attempt: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            {
                let online = online.clone();
                std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
            },
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        runner.set_state_sink({
            let db = db.clone();
            let online = online.clone();
            let network = network.clone();
            let network_tx = network_tx.clone();
            std::sync::Arc::new(move || {
                let has_waiting = repository::list_all_transfers(&db.lock())?
                    .into_iter()
                    .any(|task| task.state_kind() == Ok(TransferState::WaitingForNetwork));
                if has_waiting {
                    let mut state = network.lock();
                    if let Some(transition) = state.observe(false) {
                        online.store(state.is_online(), std::sync::atomic::Ordering::SeqCst);
                        let _ = network_tx.send(transition);
                    }
                }
                Ok(())
            })
        });
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        let recovery_order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        engine.set_cycle_observer({
            let recovery_order = recovery_order.clone();
            std::sync::Arc::new(move |stage| recovery_order.lock().unwrap().push(stage))
        });
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let edited_time = chrono::DateTime::from_timestamp_millis(20_000);
        engine.cloud_tree.lock().insert(
            "request-waiting.bin".into(),
            DriveFile {
                id: "remote-request-waiting".into(),
                name: "request-waiting.bin".into(),
                size: 7,
                edited_time,
                ..Default::default()
            },
        );
        engine.path_to_id.lock().insert(
            "request-waiting.bin".into(),
            "remote-request-waiting".into(),
        );
        let engine = std::sync::Arc::new(engine);
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let listener = {
            let request_engine = engine.clone();
            let online = online.clone();
            tokio::spawn(async move {
                network_listener_loop(
                    network_rx,
                    move || online.load(std::sync::atomic::Ordering::SeqCst),
                    stop_rx,
                    false,
                    move || request_engine.request_cycle_background("network-recovery"),
                )
                .await;
            })
        };

        let destination = root.path().join("request-waiting.bin");
        let first = runner
            .enqueue_and_run(repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::DOWNLOAD,
                file_id: Some("remote-request-waiting".into()),
                local_path: Some(destination.to_string_lossy().into_owned()),
                name: "request-waiting.bin".into(),
                total_size: 7,
                transferred: 0,
                state: TransferState::Pending.into(),
                error_message: None,
                created_at: 1,
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some("request-waiting.bin".into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Download.into()),
                source_mtime: None,
                source_size: None,
                expected_cloud_edited_time: Some(20_000),
                attempt_count: 0,
                next_retry_at: None,
                error_kind: None,
                remote_result_file_id: None,
                state_revision: 0,
            })
            .await
            .unwrap();
        assert_eq!(
            first.outcome.disposition,
            TaskDisposition::WaitingForNetwork
        );
        assert_eq!(audit_rx.recv().await.unwrap(), NetworkTransition::Offline);
        assert!(!online.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(&*backend.calls.lock().unwrap(), &[first.task_id]);

        for (index, probe_succeeded) in [true, false, true, true].into_iter().enumerate() {
            let transition = {
                let mut state = network.lock();
                let transition = state.observe(probe_succeeded);
                online.store(state.is_online(), std::sync::atomic::Ordering::SeqCst);
                transition
            };
            if let Some(transition) = transition {
                network_tx.send(transition).unwrap();
            }
            if index < 3 {
                for _ in 0..4 {
                    tokio::task::yield_now().await;
                }
                assert_eq!(backend.calls.lock().unwrap().len(), 1);
            }
        }

        backend.completed_attempt.notified().await;
        for _ in 0..32 {
            tokio::task::yield_now().await;
            if repository::get_transfer_by_id(&db.lock(), first.task_id)
                .unwrap()
                .is_some_and(|task| task.state_kind() == Ok(TransferState::Completed))
            {
                break;
            }
        }
        let tasks = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, first.task_id);
        assert_eq!(tasks[0].state_kind().unwrap(), TransferState::Completed);
        assert_eq!(
            &*backend.calls.lock().unwrap(),
            &[first.task_id, first.task_id]
        );
        {
            let recovery_order = recovery_order.lock().unwrap();
            let waiting = recovery_order
                .iter()
                .position(|stage| *stage == "resume-waiting")
                .unwrap();
            let cloud = recovery_order
                .iter()
                .position(|stage| *stage == "cloud-refresh")
                .unwrap();
            let local = recovery_order
                .iter()
                .position(|stage| *stage == "local-rescan")
                .unwrap();
            assert!(waiting < cloud && cloud < local);
        }

        stop_tx.send(true).unwrap();
        listener.await.unwrap();
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn lagged_network_listener_recovers_same_waiting_id_once_and_survives() {
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(SchedulerBackend {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let refreshes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        engine.set_cycle_observer({
            let refreshes = refreshes.clone();
            std::sync::Arc::new(move |stage| {
                if stage == "cloud-refresh" {
                    refreshes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        });
        let destination = root.path().join("lagged-network.bin");
        let waiting_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::DOWNLOAD,
                file_id: Some("remote-lagged-network".into()),
                local_path: Some(destination.to_string_lossy().into_owned()),
                name: "lagged-network.bin".into(),
                total_size: 7,
                transferred: 0,
                state: TransferState::WaitingForNetwork.into(),
                error_message: Some("persisted offline".into()),
                created_at: 1,
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some("lagged-network.bin".into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Download.into()),
                source_mtime: None,
                source_size: None,
                expected_cloud_edited_time: Some(1),
                attempt_count: 0,
                next_retry_at: None,
                error_kind: None,
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();
        let engine = std::sync::Arc::new(engine);
        let (network_tx, network_rx) = tokio::sync::broadcast::channel(2);

        // Overflow the already-subscribed receiver before the production listener is armed.
        // Lagged must be reconciled as a level and the retained Online tail must coalesce.
        network_tx
            .send(crate::core::net_guard::NetworkTransition::Offline)
            .unwrap();
        network_tx
            .send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        network_tx
            .send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        engine.start_network_listener(network_rx);

        for _ in 0..64 {
            tokio::task::yield_now().await;
            if repository::get_transfer_by_id(&db.lock(), waiting_id)
                .unwrap()
                .is_some_and(|task| task.state_kind() == Ok(TransferState::Completed))
            {
                break;
            }
        }
        let task = repository::get_transfer_by_id(&db.lock(), waiting_id)
            .unwrap()
            .unwrap();
        assert_eq!(task.state_kind().unwrap(), TransferState::Completed);
        assert_eq!(&*backend.calls.lock().unwrap(), &[waiting_id]);

        let refreshes_before_survival_probe = refreshes.load(std::sync::atomic::Ordering::SeqCst);
        network_tx
            .send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        for _ in 0..64 {
            tokio::task::yield_now().await;
            if refreshes.load(std::sync::atomic::Ordering::SeqCst) > refreshes_before_survival_probe
            {
                break;
            }
        }
        assert!(
            refreshes.load(std::sync::atomic::Ordering::SeqCst) > refreshes_before_survival_probe,
            "listener must handle an Online transition after the Lagged recovery"
        );
        assert_eq!(&*backend.calls.lock().unwrap(), &[waiting_id]);
        assert_eq!(repository::list_all_transfers(&db.lock()).unwrap().len(), 1);

        engine.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_during_waiting_batch_does_not_submit_the_next_row() {
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(BatchShutdownBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            first_submitted: tokio::sync::Notify::new(),
            release_first: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);

        let make_waiting = |name: &str, created_at: i64| repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::DOWNLOAD,
            file_id: Some(format!("remote-{name}")),
            local_path: Some(root.path().join(name).to_string_lossy().into_owned()),
            name: name.into(),
            total_size: 7,
            transferred: 0,
            state: TransferState::WaitingForNetwork.into(),
            error_message: Some("persisted offline".into()),
            created_at,
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(name.into()),
            parent_file_id: None,
            operation: Some(TransferOperation::Download.into()),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: Some(created_at),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };
        let first_id =
            repository::insert_transfer(&db.lock(), &make_waiting("first.bin", 1)).unwrap();
        let second_id =
            repository::insert_transfer(&db.lock(), &make_waiting("second.bin", 2)).unwrap();
        let engine = std::sync::Arc::new(engine);
        let cycle = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_sync_cycle("network-recovery").await })
        };

        backend.first_submitted.notified().await;
        let submitted_id = *backend.calls.lock().unwrap().first().unwrap();
        assert!(submitted_id == first_id || submitted_id == second_id);
        let unsubmitted_id = if submitted_id == first_id {
            second_id
        } else {
            first_id
        };
        engine.shutdown_sync();
        backend.release_first.notify_one();
        let error = cycle.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("已停止"));

        assert_eq!(&*backend.calls.lock().unwrap(), &[submitted_id]);
        let submitted = repository::get_transfer_by_id(&db.lock(), submitted_id)
            .unwrap()
            .unwrap();
        let unsubmitted = repository::get_transfer_by_id(&db.lock(), unsubmitted_id)
            .unwrap()
            .unwrap();
        assert_eq!(submitted.state_kind().unwrap(), TransferState::Completed);
        assert_eq!(
            unsubmitted.state_kind().unwrap(),
            TransferState::WaitingForNetwork
        );
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn old_auto_error_caller_cannot_overwrite_successor_owner_running_state() {
        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let refresh_attempt = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        engine.set_incremental_refresh_hook_for_test({
            let refresh_attempt = refresh_attempt.clone();
            std::sync::Arc::new(move || {
                if refresh_attempt.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Err(crate::error::AppError::drive_from_response(
                        503,
                        "{}",
                        None,
                        crate::error::RequestSemantics::Read,
                        false,
                    ))
                } else {
                    Ok(())
                }
            })
        });
        let (old_returned_tx, old_returned_rx) = std::sync::mpsc::channel();
        let (release_old_tx, release_old_rx) = std::sync::mpsc::channel();
        let release_old_rx = std::sync::Mutex::new(release_old_rx);
        let (successor_running_tx, successor_running_rx) = std::sync::mpsc::channel();
        let (release_successor_tx, release_successor_rx) = std::sync::mpsc::channel();
        let release_successor_rx = std::sync::Mutex::new(release_successor_rx);
        let old_signal = std::sync::Mutex::new(Some(old_returned_tx));
        let successor_signal = std::sync::Mutex::new(Some(successor_running_tx));
        engine.set_cycle_observer(std::sync::Arc::new(move |stage| {
            if stage == "auto-cycle-returned" {
                if let Some(tx) = old_signal.lock().unwrap().take() {
                    tx.send(()).unwrap();
                    release_old_rx.lock().unwrap().recv().unwrap();
                }
            }
            if stage == "local-rescan" {
                if let Some(tx) = successor_signal.lock().unwrap().take() {
                    tx.send(()).unwrap();
                    release_successor_rx.lock().unwrap().recv().unwrap();
                }
            }
        }));
        let engine = std::sync::Arc::new(engine);

        let old = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_auto_cloud_refresh().await })
        };
        old_returned_rx.recv().unwrap();
        let successor = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_sync_cycle("local-watcher").await })
        };
        successor_running_rx.recv().unwrap();
        assert!(engine.current_state().is_running);

        release_old_tx.send(()).unwrap();
        old.await.unwrap();
        assert!(
            engine.current_state().is_running,
            "the released old caller must not publish idle over its running successor"
        );

        release_successor_tx.send(()).unwrap();
        successor.await.unwrap().unwrap();
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_after_scan_prevents_old_cycle_db_and_backend_side_effects() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("shutdown-gap.txt"), b"payload").unwrap();
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner);
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let (scan_done_tx, scan_done_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let signal = std::sync::Mutex::new(Some(scan_done_tx));
        engine.set_cycle_observer(std::sync::Arc::new(move |stage| {
            if stage == "local-scan-complete" {
                if let Some(tx) = signal.lock().unwrap().take() {
                    tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
            }
        }));
        let engine = std::sync::Arc::new(engine);
        let cycle = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_sync_cycle("local-watcher").await })
        };

        scan_done_rx.recv().unwrap();
        engine.shutdown_sync();
        release_tx.send(()).unwrap();
        let error = cycle.await.unwrap().unwrap_err();

        assert!(error.to_string().contains("已停止"));
        assert!(backend.calls.lock().unwrap().is_empty());
        assert!(repository::list_all_transfers(&db.lock())
            .unwrap()
            .is_empty());
        assert!(repository::load_all(&db.lock()).unwrap().is_empty());
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn inflight_ambiguous_write_settles_before_replacement_can_start_new_owner() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::{TaskDisposition, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("ambiguous.txt");
        std::fs::write(&source, b"payload").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let source_mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let backend = std::sync::Arc::new(AmbiguousBarrierBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
            release_response: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        let engine = std::sync::Arc::new(engine);

        let transfer = {
            let runner = runner.clone();
            let source = source.clone();
            tokio::spawn(async move {
                runner
                    .enqueue_and_run(repository::TransferTask {
                        id: 0,
                        direction: repository::transfer_direction::UPLOAD,
                        file_id: None,
                        local_path: Some(source.to_string_lossy().into_owned()),
                        name: "ambiguous.txt".into(),
                        total_size: metadata.len() as i64,
                        transferred: 0,
                        state: TransferState::Pending.into(),
                        error_message: None,
                        created_at: 1,
                        finished_at: None,
                        server_id: None,
                        upload_id: None,
                        resume_offset: 0,
                        session_url: None,
                        relative_path: Some("ambiguous.txt".into()),
                        parent_file_id: None,
                        operation: Some(TransferOperation::Create.into()),
                        source_mtime,
                        source_size: Some(metadata.len() as i64),
                        expected_cloud_edited_time: None,
                        attempt_count: 0,
                        next_retry_at: None,
                        error_kind: None,
                        remote_result_file_id: None,
                        state_revision: 0,
                    })
                    .await
            })
        };
        backend.submitted.notified().await;
        let new_owner_started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let replacement = {
            let engine = engine.clone();
            let db = db.clone();
            let new_owner_started = new_owner_started.clone();
            tokio::spawn(async move {
                engine.shutdown().await;
                let rows = repository::list_all_transfers(&db.lock()).unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].state_kind().unwrap(),
                    TransferState::VerifyingRemote
                );
                new_owner_started.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        };
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(!replacement.is_finished());
        assert!(!new_owner_started.load(std::sync::atomic::Ordering::SeqCst));

        backend.release_response.notify_one();
        let outcome = transfer.await.unwrap().unwrap();
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::VerifyingRemote
        );
        replacement.await.unwrap();

        assert!(new_owner_started.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(&*backend.calls.lock().unwrap(), &[outcome.task_id]);
        let rows = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, outcome.task_id);
        assert_eq!(
            rows[0].state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn production_start_recovers_initial_online_waiting_before_arming_running_successor() {
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(SchedulerBackend {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        engine.task_runner = Some(runner);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine.set_startup_cloud_hook_for_test(std::sync::Arc::new(|| Ok(())));
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        let waiting_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::DOWNLOAD,
                file_id: Some("startup-online-remote".into()),
                local_path: Some(
                    root.path()
                        .join("startup-online.bin")
                        .to_string_lossy()
                        .into(),
                ),
                name: "startup-online.bin".into(),
                total_size: 7,
                transferred: 0,
                state: TransferState::WaitingForNetwork.into(),
                error_message: Some("persisted offline".into()),
                created_at: 1,
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some("startup-online.bin".into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Download.into()),
                source_mtime: None,
                source_size: None,
                expected_cloud_edited_time: Some(1),
                attempt_count: 0,
                next_retry_at: None,
                error_kind: None,
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();
        let local_scans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (successor_running_tx, successor_running_rx) = std::sync::mpsc::channel();
        let (release_successor_tx, release_successor_rx) = std::sync::mpsc::channel();
        let release_successor_rx = std::sync::Mutex::new(release_successor_rx);
        let successor_signal = std::sync::Mutex::new(Some(successor_running_tx));
        engine.set_cycle_observer({
            let local_scans = local_scans.clone();
            std::sync::Arc::new(move |stage| {
                if stage == "local-rescan"
                    && local_scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
                {
                    if let Some(tx) = successor_signal.lock().unwrap().take() {
                        tx.send(()).unwrap();
                        release_successor_rx.lock().unwrap().recv().unwrap();
                    }
                }
            })
        });
        let engine = std::sync::Arc::new(engine);
        let (network_tx, network_rx) = tokio::sync::broadcast::channel(4);
        network_tx
            .send(crate::core::net_guard::NetworkTransition::Online)
            .unwrap();
        let start = {
            let engine = engine.clone();
            tokio::spawn(async move {
                engine
                    .start_with_network_receiver_for_test(network_rx)
                    .await
            })
        };

        successor_running_rx.recv().unwrap();
        let start_result = start.await.unwrap();
        assert!(start_result.is_ok());
        assert!(engine.current_state().is_running);
        assert_eq!(&*backend.calls.lock().unwrap(), &[waiting_id]);
        let rows = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, waiting_id);
        assert_eq!(rows[0].state_kind().unwrap(), TransferState::Completed);

        release_successor_tx.send(()).unwrap();
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        engine.shutdown().await;
    }

    #[test]
    fn shutdown_closes_the_cycle_side_effect_gate() {
        let (engine, _) = build_engine();
        assert!(engine.ensure_cycle_active().is_ok());
        engine.shutdown_sync();
        assert!(engine.ensure_cycle_active().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn managed_backoff_deadline_wakes_exact_same_task_once() {
        use crate::sync::task_runner::{NowMs, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let start = tokio::time::Instant::now();
        let clock: NowMs = std::sync::Arc::new(move || {
            10_000 + start.elapsed().as_millis().try_into().unwrap_or(i64::MAX)
        });
        let backend = std::sync::Arc::new(SchedulerBackend {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let runner = std::sync::Arc::new(TaskRunner::new_with_clock(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
            clock,
        ));
        let destination = root.path().join("due.bin");
        let task_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::DOWNLOAD,
                file_id: Some("remote-due".into()),
                local_path: Some(destination.to_string_lossy().into_owned()),
                name: "due.bin".into(),
                total_size: 7,
                transferred: 0,
                state: TransferState::BackingOff.into(),
                error_message: Some("retry later".into()),
                created_at: 1,
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some("due.bin".into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Download.into()),
                source_mtime: None,
                source_size: None,
                expected_cloud_edited_time: Some(1),
                attempt_count: 1,
                next_retry_at: Some(12_000),
                error_kind: None,
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();
        engine.task_runner = Some(runner);
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);
        engine.start_backoff_scheduler();
        tokio::task::yield_now().await;

        tokio::time::advance(std::time::Duration::from_millis(1_999)).await;
        tokio::task::yield_now().await;
        assert!(backend.calls.lock().unwrap().is_empty());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        assert_eq!(&*backend.calls.lock().unwrap(), &[task_id]);
        assert_eq!(
            repository::get_transfer_by_id(&db.lock(), task_id)
                .unwrap()
                .unwrap()
                .state_kind()
                .unwrap(),
            TransferState::Completed
        );
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(backend.calls.lock().unwrap().len(), 1);
        engine.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn ordinary_cycle_does_not_duplicate_future_backoff_and_due_cycle_reuses_id() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::{NowMs, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let relative_path = "future-upload.bin";
        let source = root.path().join(relative_path);
        std::fs::write(&source, b"future upload payload").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let source_mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let source_size = metadata.len() as i64;
        let started_at = tokio::time::Instant::now();
        let clock: NowMs = std::sync::Arc::new(move || {
            10_000
                + started_at
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(i64::MAX)
        });
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new_with_clock(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
            clock,
        ));
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);

        let initial_revision = 7;
        let task_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::UPLOAD,
                file_id: None,
                local_path: Some(source.to_string_lossy().into_owned()),
                name: relative_path.into(),
                total_size: source_size,
                transferred: 0,
                state: TransferState::BackingOff.into(),
                error_message: Some("retry at exact deadline".into()),
                created_at: 1,
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some(relative_path.into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Create.into()),
                source_mtime: Some(source_mtime),
                source_size: Some(source_size),
                expected_cloud_edited_time: None,
                attempt_count: 1,
                next_retry_at: Some(10_001),
                error_kind: None,
                remote_result_file_id: None,
                state_revision: initial_revision,
            },
        )
        .unwrap();
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);

        engine.run_sync_cycle("local-watcher").await.unwrap();

        let before_deadline = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(before_deadline.len(), 1);
        assert_eq!(before_deadline[0].id, task_id);
        assert_eq!(
            before_deadline[0].state_kind().unwrap(),
            TransferState::BackingOff
        );
        assert_eq!(before_deadline[0].state_revision, initial_revision);
        assert_eq!(before_deadline[0].next_retry_at, Some(10_001));
        assert!(backend.calls.lock().unwrap().is_empty());

        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        engine.run_sync_cycle("backoff-deadline").await.unwrap();

        let after_deadline = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(after_deadline.len(), 1);
        assert_eq!(after_deadline[0].id, task_id);
        assert_eq!(
            after_deadline[0].state_kind().unwrap(),
            TransferState::Completed
        );
        assert_eq!(&*backend.calls.lock().unwrap(), &[task_id]);
        engine.shutdown().await;
    }

    #[tokio::test]
    async fn offline_watcher_request_stays_sticky_until_level_recovery_with_polling_off() {
        let (mut engine, _) = build_engine();
        let online = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);

        let deferred = engine.run_sync_cycle("local-watcher").await.unwrap_err();
        assert!(deferred.to_string().contains("排队"));
        assert!(!engine.cycle.is_idle());

        online.store(true, std::sync::atomic::Ordering::SeqCst);
        engine.run_sync_cycle("backoff-deadline").await.unwrap();
        assert!(engine.cycle.is_idle());
    }

    #[tokio::test]
    async fn folder_owner_blocks_cycle_then_release_wakes_sticky_request() {
        let (mut engine, _) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);
        let folder = engine.try_begin_folder_sync_guard().unwrap();

        let deferred = engine.run_sync_cycle("local-watcher").await.unwrap_err();
        assert!(deferred.to_string().contains("排队"));
        assert!(engine.cycle.has_pending());

        drop(folder);
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(engine.cycle.is_idle());
    }

    #[tokio::test]
    async fn manual_sync_before_start_returns_error_without_false_content_change() {
        let (engine, _) = build_engine();

        let error = engine.trigger_manual_sync().await.unwrap_err();

        assert!(error.to_string().contains("正在启动"));
        assert!(!engine.current_state().content_changed);
        assert!(!engine.cycle.has_pending());
    }

    #[tokio::test]
    async fn bulk_retry_before_start_rejects_without_mutating_failed_sync_items() {
        let (engine, db) = build_engine();
        let before = failed_sync_item("bulk/prestart.txt");
        repository::upsert(&db.lock(), &before).unwrap();

        let error = engine.retry_failed().await.unwrap_err();

        assert!(error.to_string().contains("正在启动"));
        let after = repository::find_by_file_id(&db.lock(), &before.file_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.status, repository::sync_status::FAILED);
        assert_eq!(after.error_message, before.error_message);
        assert!(!engine.cycle.has_pending());
    }

    #[tokio::test]
    async fn bulk_retry_offline_keeps_failed_fact_until_sticky_owner_can_run() {
        let (mut engine, db) = build_engine();
        engine.set_online_check(std::sync::Arc::new(|| false));
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let before = failed_sync_item("bulk/offline.txt");
        repository::upsert(&db.lock(), &before).unwrap();

        let deferred = engine.retry_failed().await.unwrap_err();

        assert!(deferred.to_string().contains("排队"));
        let after = repository::find_by_file_id(&db.lock(), &before.file_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.status, repository::sync_status::FAILED);
        assert_eq!(after.error_message, before.error_message);
        assert!(engine.cycle.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn startup_resumes_waiting_and_only_due_backoff_before_first_local_plan() {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::{NowMs, TaskRunner};
        use crate::sync::transfer_state::{TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let backend = std::sync::Arc::new(SchedulerBackend {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let started_at = tokio::time::Instant::now();
        let clock: NowMs = std::sync::Arc::new(move || {
            10_000
                + started_at
                    .elapsed()
                    .as_millis()
                    .try_into()
                    .unwrap_or(i64::MAX)
        });
        let runner = std::sync::Arc::new(TaskRunner::new_with_clock(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
            clock,
        ));
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount);
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_executor(executor);
        engine.set_online_check(std::sync::Arc::new(|| true));
        engine.set_startup_cloud_hook_for_test(std::sync::Arc::new(|| Ok(())));
        engine.set_incremental_refresh_hook_for_test(std::sync::Arc::new(|| Ok(())));
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        engine.set_cycle_observer({
            let order = order.clone();
            std::sync::Arc::new(move |stage| order.lock().unwrap().push(stage))
        });
        let insert = |state: TransferState, rel: &str, deadline: Option<i64>| {
            repository::insert_transfer(
                &db.lock(),
                &repository::TransferTask {
                    id: 0,
                    direction: repository::transfer_direction::DOWNLOAD,
                    file_id: Some(format!("remote-{rel}")),
                    local_path: Some(root.path().join(rel).to_string_lossy().into_owned()),
                    name: rel.into(),
                    total_size: 7,
                    transferred: 0,
                    state: state.into(),
                    error_message: Some("recover".into()),
                    created_at: 1,
                    finished_at: None,
                    server_id: None,
                    upload_id: None,
                    resume_offset: 0,
                    session_url: None,
                    relative_path: Some(rel.into()),
                    parent_file_id: None,
                    operation: Some(TransferOperation::Download.into()),
                    source_mtime: None,
                    source_size: None,
                    expected_cloud_edited_time: Some(1),
                    attempt_count: 1,
                    next_retry_at: deadline,
                    error_kind: None,
                    remote_result_file_id: None,
                    state_revision: 0,
                },
            )
            .unwrap()
        };
        let waiting_id = insert(TransferState::WaitingForNetwork, "waiting.bin", None);
        let due_id = insert(TransferState::BackingOff, "due.bin", Some(10_000));
        let future_id = insert(TransferState::BackingOff, "future.bin", Some(10_001));
        let future_revision = repository::get_transfer_by_id(&db.lock(), future_id)
            .unwrap()
            .unwrap()
            .state_revision;
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);
        let (_network_tx, network_rx) = tokio::sync::broadcast::channel(4);

        engine
            .start_with_network_receiver_for_test(network_rx)
            .await
            .unwrap();

        assert_eq!(&*backend.calls.lock().unwrap(), &[waiting_id, due_id]);
        let after_start = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(after_start.len(), 3);
        for completed_id in [waiting_id, due_id] {
            let completed = after_start
                .iter()
                .find(|task| task.id == completed_id)
                .expect("startup keeps each recovered task on its original row");
            assert_eq!(completed.state_kind().unwrap(), TransferState::Completed);
        }
        let future = after_start
            .iter()
            .find(|task| task.id == future_id)
            .expect("future Backoff remains on its original row");
        assert_eq!(future.state_kind().unwrap(), TransferState::BackingOff);
        assert_eq!(future.state_revision, future_revision);
        assert_eq!(future.next_retry_at, Some(10_001));
        {
            let order = order.lock().unwrap();
            let waiting = order
                .iter()
                .position(|stage| *stage == "resume-waiting")
                .unwrap();
            let due = order
                .iter()
                .position(|stage| *stage == "resume-due")
                .unwrap();
            let local = order
                .iter()
                .position(|stage| *stage == "local-rescan")
                .unwrap();
            assert!(waiting < due && due < local);
        }

        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
            if repository::get_transfer_by_id(&db.lock(), future_id)
                .unwrap()
                .is_some_and(|task| task.state_kind() == Ok(TransferState::Completed))
            {
                break;
            }
        }
        assert_eq!(
            &*backend.calls.lock().unwrap(),
            &[waiting_id, due_id, future_id]
        );
        assert_eq!(
            repository::get_transfer_by_id(&db.lock(), future_id)
                .unwrap()
                .unwrap()
                .state_kind()
                .unwrap(),
            TransferState::Completed
        );
        let final_rows = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(final_rows.len(), 3);
        assert_eq!(
            final_rows
                .iter()
                .filter(|task| task.id == future_id)
                .count(),
            1
        );

        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            &*backend.calls.lock().unwrap(),
            &[waiting_id, due_id, future_id]
        );
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn coalesced_manual_run_observes_the_owner_cloud_failure() {
        let (mut engine, db) = build_engine();
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        engine.set_online_check(std::sync::Arc::new(|| true));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = std::sync::Mutex::new(release_rx);
        let (manual_enqueued_tx, manual_enqueued_rx) = std::sync::mpsc::channel();
        let manual_enqueued_tx = std::sync::Mutex::new(Some(manual_enqueued_tx));
        let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        engine.set_cycle_observer({
            let first = first.clone();
            let db = db.clone();
            std::sync::Arc::new(move |stage| {
                if stage == "local-rescan" && first.swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    entered_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                }
                if stage == "request-manual" {
                    if let Some(tx) = manual_enqueued_tx.lock().unwrap().take() {
                        tx.send(()).unwrap();
                    }
                }
                if stage == "cloud-refresh" {
                    db.lock()
                        .execute_batch("DROP TABLE transfer_queue;")
                        .unwrap();
                }
            })
        });
        let engine = std::sync::Arc::new(engine);
        let watcher = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_sync_cycle("local-watcher").await })
        };
        entered_rx.recv().unwrap();
        let manual = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.run_sync_cycle("manual-refresh").await })
        };
        manual_enqueued_rx.recv().unwrap();
        release_tx.send(()).unwrap();

        let owner_error = watcher.await.unwrap().unwrap_err().to_string();
        let manual_error = manual.await.unwrap().unwrap_err().to_string();
        assert_eq!(manual_error, owner_error);
        assert!(manual_error.contains("transfer_queue"));
    }

    /// 构造测试用引擎：in-memory SQLite（含 sync_items 表）+ 桩 API。
    /// apply_results 不触网，故 API Arc 仅需可构造。
    fn build_engine_with_aggregator(
        status_aggregator: std::sync::Arc<crate::sync::status_aggregator::StatusAggregator>,
    ) -> (
        SyncEngine,
        std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>,
    ) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&conn).unwrap();
        let db = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        let auth = std::sync::Arc::new(AuthService::new());
        let client = std::sync::Arc::new(DriveClient::new(auth));
        let files_api = std::sync::Arc::new(FilesApi::new(client.clone()));
        let changes_api =
            std::sync::Arc::new(crate::drive::changes_api::ChangesApi::new(client.clone()));
        let download_api = std::sync::Arc::new(DownloadApi::new(client.clone()));
        let upload_api = std::sync::Arc::new(UploadApi::new(client));
        let eng = SyncEngine::new(
            files_api,
            changes_api,
            download_api,
            upload_api,
            db.clone(),
            status_aggregator,
            vec![],
            3,
            0,
        );
        (eng, db)
    }

    fn build_engine() -> (
        SyncEngine,
        std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>,
    ) {
        build_engine_with_aggregator(std::sync::Arc::new(
            crate::sync::status_aggregator::StatusAggregator::default(),
        ))
    }

    struct ManualRetryReplanFixture {
        engine: std::sync::Arc<SyncEngine>,
        db: std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>,
        _root: tempfile::TempDir,
        backend: std::sync::Arc<ArmGapBackend>,
        task_id: i64,
        source: std::path::PathBuf,
        relative_path: &'static str,
    }

    fn build_manual_retry_replan_fixture(
        online: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ManualRetryReplanFixture {
        use crate::mount::manager::MountManager;
        use crate::sync::executor::SyncExecutor;
        use crate::sync::task_runner::TaskRunner;
        use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

        let (mut engine, db) = build_engine();
        let root = tempfile::tempdir().unwrap();
        let relative_path = "manual-replan-fixture.bin";
        let source = root.path().join(relative_path);
        std::fs::write(&source, b"source snapshot").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let source_mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let source_size = metadata.len() as i64;
        repository::upsert(
            &db.lock(),
            &repository::SyncItem {
                file_id: format!("{}{}", repository::PENDING_FILE_ID_PREFIX, relative_path),
                local_path: relative_path.into(),
                parent_folder_id: None,
                name: relative_path.into(),
                is_folder: false,
                size: source_size,
                local_size: Some(source_size),
                sha256: None,
                local_mtime: Some(source_mtime),
                cloud_edited_time: None,
                last_sync_time: None,
                status: repository::sync_status::FAILED,
                error_message: Some("old sync failure".into()),
            },
        )
        .unwrap();
        let task_id = repository::insert_transfer(
            &db.lock(),
            &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::UPLOAD,
                file_id: None,
                local_path: Some(source.to_string_lossy().into_owned()),
                name: relative_path.into(),
                total_size: source_size,
                transferred: 0,
                state: TransferState::Failed.into(),
                error_message: Some("old transfer failure".into()),
                created_at: 1,
                finished_at: Some(2),
                server_id: None,
                upload_id: None,
                resume_offset: 0,
                session_url: None,
                relative_path: Some(relative_path.into()),
                parent_file_id: None,
                operation: Some(TransferOperation::Create.into()),
                source_mtime: Some(source_mtime),
                source_size: Some(source_size),
                expected_cloud_edited_time: None,
                attempt_count: 1,
                next_retry_at: None,
                error_kind: Some(TransferErrorKind::Unknown.into()),
                remote_result_file_id: None,
                state_revision: 0,
            },
        )
        .unwrap();
        let backend = std::sync::Arc::new(ArmGapBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
        });
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            {
                let online = online.clone();
                std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
            },
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let mut executor = SyncExecutor::new(
            1,
            engine.files_api.clone(),
            engine.download_api.clone(),
            engine.upload_api.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        engine.set_mount(mount);
        engine.set_executor(executor);
        engine.set_online_check({
            let online = online.clone();
            std::sync::Arc::new(move || online.load(std::sync::atomic::Ordering::SeqCst))
        });
        engine
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let engine = std::sync::Arc::new(engine);
        engine.bind_task_runner_state_sink(&runner);
        ManualRetryReplanFixture {
            engine,
            db,
            _root: root,
            backend,
            task_id,
            source,
            relative_path,
        }
    }

    fn failed_sync_item(relative_path: &str) -> repository::SyncItem {
        repository::SyncItem {
            file_id: "baseline-file-id".into(),
            local_path: relative_path.into(),
            parent_folder_id: Some("baseline-parent".into()),
            name: "retry.txt".into(),
            is_folder: false,
            size: 333,
            local_size: Some(222),
            sha256: Some("baseline-sha".into()),
            local_mtime: Some(1_111),
            cloud_edited_time: Some(2_222),
            last_sync_time: Some(3_333),
            status: repository::sync_status::FAILED,
            error_message: Some("old sync failure".into()),
        }
    }

    fn failed_transfer_task(relative_path: Option<&str>) -> repository::TransferTask {
        use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

        repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::UPLOAD,
            file_id: Some("baseline-file-id".into()),
            local_path: Some("/mount/folder/retry.txt".into()),
            name: "retry.txt".into(),
            total_size: 222,
            transferred: 100,
            state: TransferState::Failed.into(),
            error_message: Some("old transfer failure".into()),
            created_at: 1_000,
            finished_at: Some(2_000),
            server_id: Some("server".into()),
            upload_id: Some("upload".into()),
            resume_offset: 100,
            session_url: Some("https://upload/session".into()),
            relative_path: relative_path.map(str::to_string),
            parent_file_id: Some("parent".into()),
            operation: Some(TransferOperation::Create.into()),
            source_mtime: Some(1_000),
            source_size: Some(222),
            expected_cloud_edited_time: Some(2_000),
            attempt_count: 2,
            next_retry_at: Some(4_000),
            error_kind: Some(TransferErrorKind::Permission.into()),
            remote_result_file_id: None,
            state_revision: 0,
        }
    }

    #[test]
    fn accepting_failed_retry_uses_relative_path_and_preserves_success_baseline() {
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let before = failed_sync_item("folder/retry.txt");
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &before).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(Some("folder/retry.txt")))
                .unwrap()
        };
        let mut state_rx = eng.state_receiver();

        let pending = eng.accept_failed_transfer_retry(task_id).unwrap();

        assert_eq!(pending.state_kind().unwrap(), TransferState::Pending);
        assert_eq!(pending.state_revision, 1);
        assert_eq!(pending.error_message, None);
        assert_eq!(pending.error_kind, None);
        assert_eq!(pending.next_retry_at, None);
        assert_eq!(pending.finished_at, None);
        let snapshot = state_rx.try_recv().unwrap();
        assert!(snapshot.revision > 0);
        assert_eq!(snapshot.failed, 0);
        assert_eq!(snapshot.transfer_failed, 0);
        assert_eq!(snapshot.uploading, 0);

        let after = {
            let conn = db.lock();
            repository::find_by_file_id(&conn, &before.file_id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(after.status, repository::sync_status::SYNCING);
        assert_eq!(after.error_message, None);
        assert_eq!(after.file_id, before.file_id);
        assert_eq!(after.local_mtime, before.local_mtime);
        assert_eq!(after.local_size, before.local_size);
        assert_eq!(after.cloud_edited_time, before.cloud_edited_time);
        assert_eq!(after.last_sync_time, before.last_sync_time);
    }

    #[test]
    fn accepting_retry_without_relative_path_is_rejected_without_mutation() {
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let before = failed_sync_item("folder/retry.txt");
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &before).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(None)).unwrap()
        };
        let mut state_rx = eng.state_receiver();

        let error = eng.accept_failed_transfer_retry(task_id).unwrap_err();

        assert!(error.to_string().contains("相对路径"));
        let (task, sync_item) = {
            let conn = db.lock();
            (
                repository::get_transfer_by_id(&conn, task_id)
                    .unwrap()
                    .unwrap(),
                repository::find_by_file_id(&conn, &before.file_id)
                    .unwrap()
                    .unwrap(),
            )
        };
        assert_eq!(task.state_kind().unwrap(), TransferState::Failed);
        assert_eq!(task.state_revision, 0);
        assert_eq!(sync_item.status, repository::sync_status::FAILED);
        assert_eq!(sync_item.error_message, before.error_message);
        assert!(state_rx.try_recv().is_err());
    }

    #[test]
    fn runtime_broadcasts_recompute_complete_state_with_increasing_revisions() {
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            let mut waiting = failed_transfer_task(Some("folder/waiting.txt"));
            waiting.state = TransferState::WaitingForNetwork.into();
            waiting.error_kind = None;
            waiting.error_message = Some("offline".into());
            waiting.finished_at = None;
            repository::insert_transfer(&conn, &waiting).unwrap();
        }

        let first = eng
            .update_runtime_and_broadcast(|runtime| {
                runtime.is_running = true;
                runtime.sync_phase = Some("syncing-local".into());
            })
            .unwrap();
        let second = eng
            .update_runtime_and_broadcast(|runtime| {
                runtime.is_running = false;
                runtime.sync_phase = None;
                runtime.content_changed = true;
            })
            .unwrap();

        for snapshot in [&first, &second] {
            assert_eq!(snapshot.total, 1);
            assert_eq!(snapshot.failed, 1);
            assert_eq!(snapshot.failed_items.len(), 1);
            assert_eq!(snapshot.waiting_network, 1);
            assert_eq!(snapshot.transfer_failed, 0);
        }
        assert!(first.is_running);
        assert_eq!(first.sync_phase.as_deref(), Some("syncing-local"));
        assert!(!second.is_running);
        assert_eq!(second.sync_phase, None);
        assert!(second.content_changed);
        assert!(second.revision > first.revision);
        assert_eq!(eng.current_state().revision, second.revision);
    }

    #[test]
    fn shutdown_is_a_publication_barrier_across_engine_replacement() {
        use std::sync::{mpsc, Arc};

        let aggregator = Arc::new(crate::sync::status_aggregator::StatusAggregator::default());
        let (old_engine, _) = build_engine_with_aggregator(aggregator.clone());
        let old_engine = Arc::new(old_engine);
        let mut old_rx = old_engine.state_receiver();
        let initial = old_engine.recompute_and_broadcast_state().unwrap();
        assert_eq!(old_rx.try_recv().unwrap().revision, initial.revision);

        // The runtime closure executes while publication is held. Keep it there until shutdown has
        // explicitly observed contention on that exact mutex (no scheduler timeout inference).
        let (publish_entered_tx, publish_entered_rx) = mpsc::channel();
        let (release_publish_tx, release_publish_rx) = mpsc::channel();
        let old_for_publish = old_engine.clone();
        let publish_handle = std::thread::spawn(move || {
            old_for_publish.update_runtime_and_broadcast(|runtime| {
                runtime.sync_phase = Some("old-in-flight".into());
                publish_entered_tx.send(()).unwrap();
                release_publish_rx.recv().unwrap();
            })
        });
        publish_entered_rx.recv().unwrap();

        let (shutdown_contended_tx, shutdown_contended_rx) = mpsc::channel();
        let (shutdown_done_tx, shutdown_done_rx) = mpsc::channel();
        let old_for_shutdown = old_engine.clone();
        let shutdown_handle = std::thread::spawn(move || {
            old_for_shutdown.shutdown_sync_with_contention_hook(|| {
                shutdown_contended_tx.send(()).unwrap();
            });
            shutdown_done_tx.send(()).unwrap();
        });
        shutdown_contended_rx.recv().unwrap();
        assert!(!*old_engine.shutdown.lock());
        assert!(shutdown_done_rx.try_recv().is_err());

        release_publish_tx.send(()).unwrap();
        let in_flight = publish_handle.join().unwrap().unwrap();
        shutdown_done_rx.recv().unwrap();
        shutdown_handle.join().unwrap();
        assert_eq!(in_flight.revision, initial.revision + 1);
        assert_eq!(old_rx.try_recv().unwrap().revision, in_flight.revision);

        let (new_engine, _) = build_engine_with_aggregator(aggregator);
        let mut new_rx = new_engine.state_receiver();
        let new_snapshot = new_engine
            .update_runtime_and_broadcast(|runtime| {
                runtime.sync_phase = Some("new-engine".into());
            })
            .unwrap();
        assert_eq!(new_snapshot.revision, in_flight.revision + 1);
        assert_eq!(new_rx.try_recv().unwrap().revision, new_snapshot.revision);

        let old_revision = old_engine.current_state().revision;
        let error = old_engine.recompute_and_broadcast_state().unwrap_err();
        assert!(error.to_string().contains("已停止"));
        assert_eq!(old_engine.current_state().revision, old_revision);
        assert!(old_rx.try_recv().is_err());
        assert_eq!(new_engine.current_state().revision, new_snapshot.revision);
        assert_eq!(
            new_engine.current_state().sync_phase.as_deref(),
            Some("new-engine")
        );

        old_engine.push_live_transfer_state();
        assert_eq!(old_engine.current_state().revision, old_revision);
        assert!(old_rx.try_recv().is_err());

        let next_new_snapshot = new_engine.recompute_and_broadcast_state().unwrap();
        assert_eq!(next_new_snapshot.revision, new_snapshot.revision + 1);
    }

    #[test]
    fn runtime_cleanup_is_applied_in_memory_when_aggregation_fails() {
        let (eng, db) = build_engine();
        let mut state_rx = eng.state_receiver();
        {
            let conn = db.lock();
            conn.execute_batch("DROP TABLE transfer_queue;").unwrap();
        }

        assert!(eng
            .update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = true;
                runtime.sync_phase = Some("indexing-manual".into());
            })
            .is_err());
        assert!(eng.current_state().is_indexing);
        assert_eq!(
            eng.current_state().sync_phase.as_deref(),
            Some("indexing-manual")
        );

        assert!(eng
            .update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = false;
                runtime.sync_phase = None;
            })
            .is_err());
        assert!(!eng.current_state().is_indexing);
        assert_eq!(eng.current_state().sync_phase, None);
        assert_eq!(eng.current_state().revision, 0);
        assert!(state_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn aggregation_failure_releases_sync_cycle_and_indexing_gates() {
        let (eng, db) = build_engine();
        {
            let conn = db.lock();
            conn.execute_batch("DROP TABLE transfer_queue;").unwrap();
        }

        assert!(eng.run_sync_cycle("startup-resume").await.is_err());
        assert!(!*eng.syncing.lock());
        assert!(!eng.current_state().is_running);

        let mount_dir = tempfile::tempdir().unwrap();
        assert!(eng
            .load_or_refresh_cloud_tree(&mount_dir.path().to_string_lossy())
            .await
            .is_err());
        assert!(!*eng.syncing.lock());
        assert!(!eng.current_state().is_indexing);
        assert_eq!(eng.current_state().sync_phase, None);
    }

    #[test]
    fn single_retry_transitions_publish_consistent_running_and_completed_snapshots() {
        use crate::data::repository::TransferPatch;
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(Some("folder/retry.txt")))
                .unwrap()
        };

        let pending = eng.accept_failed_transfer_retry(task_id).unwrap();
        let running = eng
            .transition_transfer_and_broadcast(
                task_id,
                pending.state_revision,
                TransferState::Running,
                TransferPatch::default(),
            )
            .unwrap();
        let running_snapshot = eng.current_state();
        assert_eq!(running.state_kind().unwrap(), TransferState::Running);
        assert_eq!(running_snapshot.uploading, 1);
        assert_eq!(running_snapshot.failed, 0);
        assert_eq!(running_snapshot.transfer_failed, 0);
        assert!(running_snapshot.failed_items.is_empty());

        let cloud_file = DriveFile {
            id: "confirmed-file-id".into(),
            name: "retry.txt".into(),
            size: 222,
            edited_time: chrono::DateTime::from_timestamp_millis(8_000),
            ..Default::default()
        };
        let completed_snapshot = eng
            .settle_retry_success_and_broadcast(task_id, running.state_revision, &cloud_file, 9_000)
            .unwrap();
        let (completed, settled_sync_item, sync_item_count) = {
            let conn = db.lock();
            (
                repository::get_transfer_by_id(&conn, task_id)
                    .unwrap()
                    .unwrap(),
                repository::find_by_file_id(&conn, "confirmed-file-id")
                    .unwrap()
                    .unwrap(),
                conn.query_row(
                    "SELECT COUNT(*) FROM sync_items WHERE local_path=?1",
                    rusqlite::params!["folder/retry.txt"],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            )
        };
        assert_eq!(completed.state_kind().unwrap(), TransferState::Completed);
        assert_eq!(settled_sync_item.local_path, "folder/retry.txt");
        assert_eq!(
            settled_sync_item.parent_folder_id.as_deref(),
            Some("parent")
        );
        assert_eq!(settled_sync_item.local_mtime, Some(1_000));
        assert_eq!(settled_sync_item.local_size, Some(222));
        assert_eq!(settled_sync_item.cloud_edited_time, Some(8_000));
        assert_eq!(settled_sync_item.last_sync_time, Some(9_000));
        assert_eq!(settled_sync_item.status, repository::sync_status::SYNCED);
        assert_eq!(sync_item_count, 1);
        assert_eq!(completed_snapshot.uploading, 0);
        assert_eq!(completed_snapshot.failed, 0);
        assert_eq!(completed_snapshot.transfer_failed, 0);
        assert!(completed_snapshot.failed_items.is_empty());
        assert!(completed_snapshot.revision > running_snapshot.revision);
    }

    #[test]
    fn retry_success_settlement_rolls_back_task_when_baseline_write_fails() {
        use crate::data::repository::TransferPatch;
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(Some("folder/retry.txt")))
                .unwrap()
        };
        let pending = eng.accept_failed_transfer_retry(task_id).unwrap();
        let running = eng
            .transition_transfer_and_broadcast(
                task_id,
                pending.state_revision,
                TransferState::Running,
                TransferPatch::default(),
            )
            .unwrap();
        let revision_before = eng.current_state().revision;
        {
            let conn = db.lock();
            conn.execute_batch(
                "CREATE TRIGGER force_retry_baseline_delete_failure
                 BEFORE DELETE ON sync_items
                 BEGIN SELECT RAISE(ABORT, 'forced baseline failure'); END;",
            )
            .unwrap();
        }
        let cloud_file = DriveFile {
            id: "confirmed-file-id".into(),
            name: "retry.txt".into(),
            size: 222,
            ..Default::default()
        };

        let error = eng
            .settle_retry_success_and_broadcast(task_id, running.state_revision, &cloud_file, 9_000)
            .unwrap_err();

        assert!(error.to_string().contains("forced baseline failure"));
        let (task, sync_item) = {
            let conn = db.lock();
            (
                repository::get_transfer_by_id(&conn, task_id)
                    .unwrap()
                    .unwrap(),
                repository::find_by_file_id(&conn, "baseline-file-id")
                    .unwrap()
                    .unwrap(),
            )
        };
        assert_eq!(task.state_kind().unwrap(), TransferState::Running);
        assert_eq!(task.state_revision, running.state_revision);
        assert_eq!(sync_item.status, repository::sync_status::SYNCING);
        assert_eq!(eng.current_state().revision, revision_before);
    }

    #[test]
    fn retry_failure_settlement_rolls_back_task_when_sync_write_fails() {
        use crate::data::repository::TransferPatch;
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(Some("folder/retry.txt")))
                .unwrap()
        };
        let pending = eng.accept_failed_transfer_retry(task_id).unwrap();
        let running = eng
            .transition_transfer_and_broadcast(
                task_id,
                pending.state_revision,
                TransferState::Running,
                TransferPatch::default(),
            )
            .unwrap();
        let revision_before = eng.current_state().revision;
        {
            let conn = db.lock();
            conn.execute_batch(
                "CREATE TRIGGER force_retry_sync_failure
                 BEFORE UPDATE ON sync_items
                 BEGIN SELECT RAISE(ABORT, 'forced sync failure'); END;",
            )
            .unwrap();
        }

        let error = eng
            .record_retry_failure_and_broadcast(
                task_id,
                running.state_revision,
                "permission denied",
                9_000,
            )
            .unwrap_err();

        assert!(error.to_string().contains("forced sync failure"));
        let (task, sync_item) = {
            let conn = db.lock();
            (
                repository::get_transfer_by_id(&conn, task_id)
                    .unwrap()
                    .unwrap(),
                repository::find_by_file_id(&conn, "baseline-file-id")
                    .unwrap()
                    .unwrap(),
            )
        };
        assert_eq!(task.state_kind().unwrap(), TransferState::Running);
        assert_eq!(task.state_revision, running.state_revision);
        assert_eq!(sync_item.status, repository::sync_status::SYNCING);
        assert_eq!(eng.current_state().revision, revision_before);
    }

    #[test]
    fn retry_failure_explicitly_restores_both_failure_facts() {
        use crate::data::repository::TransferPatch;
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        let task_id = {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            repository::insert_transfer(&conn, &failed_transfer_task(Some("folder/retry.txt")))
                .unwrap()
        };
        let pending = eng.accept_failed_transfer_retry(task_id).unwrap();
        let running = eng
            .transition_transfer_and_broadcast(
                task_id,
                pending.state_revision,
                TransferState::Running,
                TransferPatch::default(),
            )
            .unwrap();

        let snapshot = eng
            .record_retry_failure_and_broadcast(
                task_id,
                running.state_revision,
                "permission denied",
                9_000,
            )
            .unwrap();

        assert_eq!(snapshot.failed, 1);
        assert_eq!(snapshot.transfer_failed, 1);
        assert_eq!(snapshot.waiting_network, 0);
        assert_eq!(snapshot.failed_items.len(), 1);
        assert_eq!(
            snapshot.failed_items[0].error_message.as_deref(),
            Some("permission denied")
        );
        let task = {
            let conn = db.lock();
            repository::get_transfer_by_id(&conn, task_id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(task.state_kind().unwrap(), TransferState::Failed);
        assert_eq!(task.state_revision, running.state_revision + 1);
    }

    #[test]
    fn clearing_failed_transfer_history_preserves_current_sync_failure() {
        use crate::sync::transfer_state::TransferState;

        let (eng, db) = build_engine();
        {
            let conn = db.lock();
            repository::upsert(&conn, &failed_sync_item("folder/retry.txt")).unwrap();
            repository::insert_transfer(
                &conn,
                &failed_transfer_task(Some("folder/failed-transfer.txt")),
            )
            .unwrap();
            let mut waiting = failed_transfer_task(Some("folder/waiting.txt"));
            waiting.state = TransferState::WaitingForNetwork.into();
            waiting.error_kind = None;
            waiting.finished_at = None;
            repository::insert_transfer(&conn, &waiting).unwrap();
        }

        let snapshot = eng
            .clear_transfer_history_and_broadcast(false, true)
            .unwrap();

        assert_eq!(snapshot.failed, 1);
        assert_eq!(snapshot.failed_items.len(), 1);
        assert_eq!(snapshot.failed_items[0].relative_path, "folder/retry.txt");
        assert_eq!(snapshot.transfer_failed, 0);
        assert_eq!(snapshot.waiting_network, 1);
        let remaining = {
            let conn = db.lock();
            repository::list_all_transfers(&conn).unwrap()
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].state_kind().unwrap(),
            TransferState::WaitingForNetwork
        );
    }

    /// 新文件上传成功后，DB/cloud_tree/path_to_id 必须用 result.cloud_file 回写。
    /// 回归：之前用 action.file_id（None）取键 → DB 跳过 + cloud_tree 不更新
    ///       → 下轮 watcher cycle 误判「本地新增」重复上传（288 振荡根因）。
    #[test]
    fn test_apply_results_new_upload_writes_db_and_cloud_tree() {
        let (eng, db) = build_engine();
        let dir = tempdir().unwrap();
        let local_file = dir.path().join("new.txt");
        std::fs::write(&local_file, b"hello").unwrap();

        let cloud = DriveFile {
            id: "cloud-id-1".into(),
            name: "new.txt".into(),
            size: 5,
            edited_time: chrono::DateTime::from_timestamp_millis(1_700_000_000_000),
            ..Default::default()
        };
        // 新文件 Upload：action.file_id = None（云端 ID 仅上传后存在于 result.cloud_file）
        let action = SyncAction {
            action_type: SyncActionType::Upload,
            relative_path: Some("A/new.txt".into()),
            file_id: None,
            parent_file_id: Some("folder-A".into()),
            local_path: Some(local_file.to_string_lossy().to_string()),
            cloud_file: None,
            reason: Some("本地新文件 → 上传".into()),
        };
        let result = ActionResult {
            success: true,
            error_message: None,
            deferred: false,
            cloud_file: Some(cloud.clone()),
        };

        eng.apply_results(&[action], &[result]);

        // DB：写入 result 的 fileId（非 action 的 None）、synced、真实 mtime/size
        let conn = db.lock();
        let row: (String, i32, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT file_id, status, local_mtime, local_size FROM sync_items WHERE local_path=?1",
                rusqlite::params!["A/new.txt"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, "cloud-id-1", "DB fileId 应取自 result.cloud_file");
        assert_eq!(row.1, repository::sync_status::SYNCED);
        assert!(
            row.2.is_some(),
            "local_mtime 必须写入真实值（None 会致每轮重传）"
        );
        assert_eq!(row.3, Some(5));
        drop(conn);

        // cloud_tree / path_to_id：含刚上传的文件，避免下轮误判为「云端已删除」误删本地
        assert!(eng.cloud_tree.lock().contains_key("A/new.txt"));
        assert_eq!(
            eng.path_to_id.lock().get("A/new.txt").cloned(),
            Some("cloud-id-1".to_string())
        );
    }

    /// DeleteFromCloud 成功 → 清 DB 记录 + 从 cloud_tree/path_to_id 移除。
    #[test]
    fn test_apply_results_delete_from_cloud_clears_state() {
        let (eng, db) = build_engine();
        // 预置 cloud_tree / path_to_id / DB 记录
        eng.cloud_tree_insert(
            "old.txt".into(),
            DriveFile {
                id: "c-old".into(),
                ..Default::default()
            },
        );
        eng.path_to_id_insert("old.txt".into(), "c-old".into());
        {
            let conn = db.lock();
            let _ = repository::upsert(
                &conn,
                &repository::SyncItem {
                    file_id: "c-old".into(),
                    local_path: "old.txt".into(),
                    parent_folder_id: None,
                    name: "old.txt".into(),
                    is_folder: false,
                    size: 0,
                    local_size: None,
                    sha256: None,
                    local_mtime: None,
                    cloud_edited_time: None,
                    last_sync_time: None,
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                },
            );
        }

        let action = SyncAction {
            action_type: SyncActionType::DeleteFromCloud,
            relative_path: Some("old.txt".into()),
            file_id: Some("c-old".into()),
            parent_file_id: None,
            local_path: None,
            cloud_file: None,
            reason: Some("会话内删除 → 双向删除云端".into()),
        };
        let result = ActionResult {
            success: true,
            error_message: None,
            deferred: false,
            cloud_file: None,
        };
        eng.apply_results(&[action], &[result]);

        assert!(!eng.cloud_tree.lock().contains_key("old.txt"));
        assert!(!eng.path_to_id.lock().contains_key("old.txt"));
        let conn = db.lock();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE local_path=?1",
                rusqlite::params!["old.txt"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "DeleteFromCloud 成功后 DB 记录应被清理");
    }

    /// 新建目录创建后回填 path_to_id，目录内文件的 parent 应指向该目录 id。
    /// 对应用户预期「建目录→拿 ID→上传文件」：execute_actions_ordered 阶段 1 创建目录
    /// 后调用 path_to_id_insert，阶段 2 重新 fill_parent_file_ids 即命中此处逻辑。
    #[test]
    fn test_fill_parent_file_ids_picks_up_newly_created_folder() {
        // 模拟「NEWDIR 已创建，folderId 已回填 path_to_id」
        let mut p2i = std::collections::HashMap::new();
        p2i.insert("NEWDIR".to_string(), "folder-id-1".to_string());

        let mut actions = vec![
            // NEWDIR 内的文件（新建目录内的文件，parent 尚未填充）
            SyncAction {
                action_type: SyncActionType::Upload,
                relative_path: Some("NEWDIR/file.txt".into()),
                file_id: None,
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: None,
            },
            // 嵌套子目录内的文件（NEWDIR/sub，sub 也已回填）
            SyncAction {
                action_type: SyncActionType::Upload,
                relative_path: Some("NEWDIR/sub/deep.txt".into()),
                file_id: None,
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: None,
            },
        ];
        p2i.insert("NEWDIR/sub".to_string(), "folder-id-2".to_string());

        super::fill_parent_file_ids(&mut actions, &p2i);
        assert_eq!(
            actions[0].parent_file_id.as_deref(),
            Some("folder-id-1"),
            "NEWDIR/file.txt 的 parent 应指向新建目录 NEWDIR 的 id"
        );
        assert_eq!(
            actions[1].parent_file_id.as_deref(),
            Some("folder-id-2"),
            "NEWDIR/sub/deep.txt 的 parent 应指向新建子目录 NEWDIR/sub 的 id"
        );
    }

    /// 云端删了 B、B/sub，本地改过 B/sub/f2.txt → BackupBeforeCloudDelete。
    /// add_rescue_folder_recreations 应为被删的祖先目录 B、B/sub 补 CreateFolder，
    /// 使副本下轮 Upload 时父目录已在云端 path_to_id、落到正确目录而非根。
    #[test]
    fn test_add_rescue_folder_recreations_for_backup_in_deleted_folder() {
        use crate::mount::manager::LocalFileEntry;
        use crate::sync::planner::{DbSnapshotEntry, SyncSnapshot};
        use std::path::PathBuf;

        let mk_folder = |rel: &str| LocalFileEntry {
            absolute_path: PathBuf::from(format!("/mount/{rel}")),
            relative_path: rel.into(),
            size: 0,
            mtime: 1000,
            is_folder: true,
            is_placeholder: false,
        };
        let mk_file = |rel: &str, size: u64, mtime: i64| LocalFileEntry {
            absolute_path: PathBuf::from(format!("/mount/{rel}")),
            relative_path: rel.into(),
            size,
            mtime,
            is_folder: false,
            is_placeholder: false,
        };
        let mk_db_folder = |fid: &str| DbSnapshotEntry {
            file_id: fid.into(),
            local_mtime: None,
            local_size: None,
            cloud_edited_time: Some(1000),
            status: 0,
            is_folder: true,
        };
        let mk_db_file = |fid: &str, mtime: i64, size: i64| DbSnapshotEntry {
            file_id: fid.into(),
            local_mtime: Some(mtime),
            local_size: Some(size),
            cloud_edited_time: Some(1000),
            status: 0,
            is_folder: false,
        };

        let mut local = std::collections::HashMap::new();
        local.insert("B".into(), mk_folder("B"));
        local.insert("B/sub".into(), mk_folder("B/sub"));
        local.insert("B/sub/f2.txt".into(), mk_file("B/sub/f2.txt", 300, 9000)); // 改过

        let mut db = std::collections::HashMap::new();
        db.insert("B".into(), mk_db_folder("fb"));
        db.insert("B/sub".into(), mk_db_folder("fbs"));
        db.insert("B/sub/f2.txt".into(), mk_db_file("fid2", 1000, 100)); // db mtime=1000，local=9000

        let snapshot = SyncSnapshot {
            local,
            cloud: std::collections::HashMap::new(),
            db,
            cloud_tree_trusted: true,
            is_startup_resume: false,
        };
        // 模拟 planner 产出的备份动作
        let mut actions = vec![SyncAction {
            action_type: SyncActionType::BackupBeforeCloudDelete,
            relative_path: Some("B/sub/f2.txt".into()),
            file_id: Some("fid2".into()),
            parent_file_id: None,
            local_path: Some("/mount/B/sub/f2.txt".into()),
            cloud_file: None,
            reason: None,
        }];

        super::add_rescue_folder_recreations(
            &mut actions,
            &snapshot,
            &std::collections::HashMap::new(),
        );

        let has_create = |rel: &str| {
            actions.iter().any(|a| {
                a.action_type == SyncActionType::CreateFolder
                    && a.relative_path.as_deref() == Some(rel)
            })
        };
        assert!(has_create("B"), "应为被删祖先目录 B 补 CreateFolder");
        assert!(
            has_create("B/sub"),
            "应为被删祖先目录 B/sub 补 CreateFolder"
        );
        assert!(
            actions
                .iter()
                .any(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete),
            "原备份动作应保留"
        );
    }

    /// 反例：祖先目录仍在云端（非被删）→ 不应补 CreateFolder。
    #[test]
    fn test_add_rescue_folder_recreations_skips_when_folder_on_cloud() {
        use crate::drive::models::{DriveFile, FileCategory};
        use crate::mount::manager::LocalFileEntry;
        use crate::sync::planner::{DbSnapshotEntry, SyncSnapshot};
        use std::path::PathBuf;

        let mut local = std::collections::HashMap::new();
        local.insert(
            "A".into(),
            LocalFileEntry {
                absolute_path: PathBuf::from("/mount/A"),
                relative_path: "A".into(),
                size: 0,
                mtime: 1000,
                is_folder: true,
                is_placeholder: false,
            },
        );
        local.insert(
            "A/f.txt".into(),
            LocalFileEntry {
                absolute_path: PathBuf::from("/mount/A/f.txt"),
                relative_path: "A/f.txt".into(),
                size: 200,
                mtime: 9000,
                is_folder: false,
                is_placeholder: false,
            },
        );
        // A 仍在云端
        let mut cloud = std::collections::HashMap::new();
        cloud.insert(
            "A".into(),
            DriveFile {
                id: "fa".into(),
                name: "A".into(),
                category: FileCategory::Folder,
                ..Default::default()
            },
        );
        let mut db = std::collections::HashMap::new();
        db.insert(
            "A".into(),
            DbSnapshotEntry {
                file_id: "fa".into(),
                local_mtime: None,
                local_size: None,
                cloud_edited_time: Some(1000),
                status: 0,
                is_folder: true,
            },
        );
        db.insert(
            "A/f.txt".into(),
            DbSnapshotEntry {
                file_id: "fid".into(),
                local_mtime: Some(1000),
                local_size: Some(100),
                cloud_edited_time: Some(1000),
                status: 0,
                is_folder: false,
            },
        );
        let snapshot = SyncSnapshot {
            local,
            cloud,
            db,
            cloud_tree_trusted: true,
            is_startup_resume: false,
        };
        let mut actions = vec![SyncAction {
            action_type: SyncActionType::BackupBeforeCloudDelete,
            relative_path: Some("A/f.txt".into()),
            file_id: Some("fid".into()),
            parent_file_id: None,
            local_path: Some("/mount/A/f.txt".into()),
            cloud_file: None,
            reason: None,
        }];

        super::add_rescue_folder_recreations(
            &mut actions,
            &snapshot,
            &std::collections::HashMap::new(),
        );

        assert!(
            !actions
                .iter()
                .any(|a| a.action_type == SyncActionType::CreateFolder
                    && a.relative_path.as_deref() == Some("A")),
            "A 仍在云端 → 不应补建"
        );
        assert_eq!(actions.len(), 1, "仅保留原备份动作");
    }

    #[test]
    fn failed_action_updates_only_exact_baseline_identity() {
        let (eng, db) = build_engine();
        let mut first = failed_sync_item("same/path.txt");
        first.file_id = "file-a".into();
        first.status = repository::sync_status::SYNCED;
        first.error_message = None;
        let mut second = first.clone();
        second.file_id = "file-b".into();
        {
            let conn = db.lock();
            repository::upsert(&conn, &first).unwrap();
            repository::upsert(&conn, &second).unwrap();
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
        eng.apply_results(
            &[action],
            &[ActionResult {
                success: false,
                error_message: Some("failed".into()),
                deferred: false,
                cloud_file: None,
            }],
        );

        let conn = db.lock();
        let status_a: i32 = conn
            .query_row(
                "SELECT status FROM sync_items WHERE file_id='file-a' AND local_path='same/path.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let status_b: i32 = conn
            .query_row(
                "SELECT status FROM sync_items WHERE file_id='file-b' AND local_path='same/path.txt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status_a, repository::sync_status::FAILED);
        assert_eq!(status_b, repository::sync_status::SYNCED);
    }
}
