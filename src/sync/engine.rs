//! 同步引擎主循环 —— 核心编排（阶段 5 骨架，后续阶段逐步接入 mount/executor/cloud_tree 完成闭环）。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart`。

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

mod action_filters;
mod coordination;

use action_filters::{
    add_rescue_folder_recreations, dedupe_directory_deletes, dedupe_local_descendants,
    fill_parent_file_ids, filter_anti_oscillation, preserve_dirs_with_pending_backups,
};
use coordination::{
    network_listener_loop, watcher_listener_loop, ActivityTracker, CycleCoordinator, CycleRequest,
    TaskRunnerActivityGate,
};
pub(crate) use coordination::{ActivityGuard, FolderSyncGuard};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FailedRecordReconciliation {
    healed: usize,
    purged: usize,
    remaining_failed: usize,
    pending_id: usize,
    missing_side: usize,
    id_mismatch: usize,
    type_conflict: usize,
    baseline_changed: usize,
    transfer_blocked: usize,
    stale_transfer_blocked: usize,
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

    pub fn set_cycle_observer(&mut self, cycle_observer: Arc<dyn Fn(&'static str) + Send + Sync>) {
        self.cycle_observer = cycle_observer;
    }

    fn is_online(&self) -> bool {
        (self.online_check)()
    }

    pub(crate) fn begin_external_activity(&self) -> AppResult<ActivityGuard> {
        self.activity.begin(None)
    }

    pub(crate) fn begin_exclusive_path_activity(
        &self,
        relative_path: &str,
    ) -> AppResult<ActivityGuard> {
        crate::core::paths::validate_relative_path(relative_path, false)?;
        self.activity.begin_exclusive(relative_path)
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
        // 远端调用若已提交，其结算由 TaskRunner 负责。此处等待 cycle owner，确保结算收敛前
        // 替代引擎不会启动；随后活动关闭屏障等待所有已登记活动释放。
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

    pub(crate) fn root_folder_id(&self) -> Option<String> {
        self.root_folder_id.lock().clone()
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
        let startup_deferred = match self.run_sync_cycle("startup-resume").await {
            Ok(()) => false,
            Err(error) if !self.is_online() || Self::is_recoverable_cycle_error(&error) => {
                tracing::warn!(%error, "启动同步暂不可执行，保留 STARTUP 等待后台恢复");
                true
            }
            Err(error) => return Err(error),
        };
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

        // A temporary cloud failure can happen while the network level remains Online, so no
        // transition edge is guaranteed to wake the listener. Explicitly drain the preserved
        // STARTUP request; the background worker applies bounded retry backoff.
        if startup_deferred && self.is_online() && self.cycle.has_pending() {
            self.schedule_background_drain();
        }

        Ok(())
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
        let abs_dir = crate::core::paths::expand_tilde(mount_dir);
        let loaded_from_cache = if let Some(cache) = cloud_tree::load_persisted_cloud_tree(&abs_dir)
        {
            self.install_cloud_checkpoint(cache);
            // The serialized checkpoint is a complete incremental baseline, but it is not the
            // current cloud fact until Changes catch-up succeeds. Keep it available for replay
            // while blocking every planner/write decision that requires current remote state.
            self.set_cloud_tree_trusted(false);
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
        Ok(loaded_from_cache)
    }

    fn purge_deleted_tombstones_if_trusted(&self) -> AppResult<()> {
        if !self.cloud_tree_is_trusted() {
            return Err(AppError::generic("云端树尚未 catch-up，拒绝清理墓碑"));
        }
        let conn = self.db.lock();
        let cloud = self.cloud_tree.lock();
        let to_purge: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT local_path FROM sync_items WHERE status=?1")
                .map_err(|error| AppError::generic(format!("查询墓碑失败：{error}")))?;
            let rows = statement
                .query_map(rusqlite::params![repository::sync_status::DELETED], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| AppError::generic(format!("读取墓碑失败：{error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::generic(format!("读取墓碑失败：{error}")))?
                .into_iter()
                .filter(|path| !cloud.contains_key(path))
                .collect()
        };
        drop(cloud);
        for path in &to_purge {
            conn.execute(
                "DELETE FROM sync_items WHERE local_path=?1 AND status=?2",
                rusqlite::params![path, repository::sync_status::DELETED],
            )
            .map_err(|error| AppError::generic(format!("清理墓碑失败：{error}")))?;
        }
        if !to_purge.is_empty() {
            tracing::info!(count = to_purge.len(), "已清理可信云树中不存在的墓碑");
        }
        Ok(())
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
            "retry-failed" => {
                CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL | CycleRequest::RETRY
            }
            "retry-replan" => {
                CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL | CycleRequest::REPLAN
            }
            "backoff-deadline" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
            }
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
        } else if request.contains(CycleRequest::RETRY) {
            "retry-failed"
        } else if request.contains(CycleRequest::REPLAN) {
            "retry-replan"
        } else if request.contains(CycleRequest::ONLINE_RECOVERY) {
            "network-recovery"
        } else if request.contains(CycleRequest::CLOUD_INCREMENTAL) {
            "auto-cloud-refresh"
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
                        "retry-failed" | "retry-replan" => {
                            Some("syncing-retry".to_string())
                        }
                        "startup-resume" => Some("syncing-startup".to_string()),
                        _ => None, // auto-cloud-refresh 由上层设好
                    };
                }
            })?;

            if request.contains(CycleRequest::STARTUP) {
                {
                    let _activity = self.begin_external_activity()?;
                    let conn = self.db.lock();
                    let _ = repository::reset_stale_statuses(&conn);
                }
                self.ensure_cycle_active()?;
            }

            // Startup installs a complete serialized baseline first, but deliberately keeps it
            // untrusted until Changes catch-up. No interrupted upload/download may be resumed
            // before current cloud state is known.
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
                        self.cycle.restore(request);
                        return Err(error);
                    }
                }
                self.ensure_cycle_active()?;
            }
            self.ensure_cycle_active()?;
            if request.contains(CycleRequest::CLOUD_FULL) {
                (self.cycle_observer)("cloud-refresh");
                if let Err(error) = self.refresh_cloud_full_for_cycle().await {
                    self.cycle.restore(request);
                    return Err(error);
                }
            } else if request.contains(CycleRequest::CLOUD_INCREMENTAL)
                && (!request.contains(CycleRequest::STARTUP) || startup_needs_incremental)
            {
                if !self.is_online() {
                    self.cycle.restore(request);
                    if request.contains(CycleRequest::STARTUP) {
                        return Err(AppError::generic(
                            "启动云端追平等待网络恢复",
                        ));
                    }
                    return Ok(());
                } else {
                    (self.cycle_observer)("cloud-refresh");
                    if let Err(error) = self.refresh_cloud_incremental_for_cycle().await {
                        tracing::warn!(%error, "云端刷新失败，完整保留当前周期意图等待补跑");
                        self.cycle.restore(request);
                        return Err(error);
                    }
                }
            }

            // A stale or failed checkpoint is display-only. Blocking the whole planner here also
            // blocks non-delete uploads/updates that could overwrite a newer remote version.
            if !self.cloud_tree_is_trusted() {
                self.cycle.restore(request);
                tracing::warn!("云端 checkpoint 尚未追平，跳过任务恢复与同步规划");
                if request.contains(CycleRequest::STARTUP) {
                    return Err(AppError::generic(
                        "启动云端 checkpoint 尚未追平，等待恢复",
                    ));
                }
                return Ok(());
            }

            // A direct rename/move may have reached Huawei before the process exited. Finish that
            // structural change from the trusted fileId/path view before any local scan can
            // misclassify the old path as a delete and the new path as unrelated content.
            let path_recovery = {
                let _activity = self.begin_external_activity()?;
                let mount_dir = self
                    .mount_dir
                    .lock()
                    .clone()
                    .ok_or_else(|| AppError::generic("同步挂载尚未初始化，无法恢复路径变更"))?;
                let mount_root = std::path::PathBuf::from(crate::core::paths::expand_tilde(
                    &mount_dir,
                ));
                let cloud = self.cloud_tree.lock().clone();
                let conn = self.db.lock();
                crate::sync::path_recovery::recover_verified_remote_path_changes(
                    &mount_root,
                    &conn,
                    &cloud,
                    |old_path, new_path| {
                        let old_lease = self.begin_exclusive_path_activity(old_path)?;
                        let new_lease = self.begin_exclusive_path_activity(new_path)?;
                        Ok((old_lease, new_lease))
                    },
                )
            };
            match path_recovery {
                Ok(summary) => {
                    if summary.rekeyed_roots > 0 {
                        tracing::info!(
                            recovered = summary.rekeyed_roots,
                            "已在同步规划前收敛中断的远端路径变更"
                        );
                    }
                }
                Err(error) => {
                    // Fail closed: do not let the normal planner interpret a disputed target as
                    // an upload/download/delete. A later watcher/cloud cycle can retry after the
                    // conflict is removed; startup itself remains available to the user.
                    tracing::error!(%error, "远端路径变更恢复被安全条件阻止，跳过本轮同步规划");
                    if request.contains(CycleRequest::STARTUP) {
                        // Do not strand old Running rows merely because a separate path conflict
                        // needs user resolution. Recovery remains version-checked and cannot make
                        // the disputed path eligible for normal planner deletes in this cycle.
                        self.recover_interrupted_transfers().await;
                    }
                    return Ok(());
                }
            }
            self.ensure_cycle_active()?;
            self.purge_deleted_tombstones_if_trusted()?;

            if request.contains(CycleRequest::STARTUP) {
                self.ensure_cycle_active()?;
                self.recover_interrupted_transfers().await;
            }

            // Stable-online recovery is ordered after cloud catch-up. This prevents a queued
            // update from replaying against a checkpoint that predates a remote edit.
            if request.contains(CycleRequest::ONLINE_RECOVERY) {
                if !self.is_online() {
                    self.cycle.restore(request);
                    return Ok(());
                }
                self.ensure_cycle_active()?;
                if let Some(task_runner) = &self.task_runner {
                    (self.cycle_observer)("verify-remote");
                    task_runner.resume_verifying().await?;
                    self.ensure_cycle_active()?;
                    (self.cycle_observer)("resume-waiting");
                    task_runner.resume_waiting().await?;
                    self.ensure_cycle_active()?;
                    (self.cycle_observer)("resume-due");
                    task_runner.resume_due_backoff().await?;
                }
            }

            // Accept a global retry only after the cloud view is current and recovery preflight
            // has succeeded. A single RestartRequired replan carries REPLAN instead and must not
            // clear unrelated FAILED rows.
            if request.contains(CycleRequest::RETRY) {
                let _activity = self.begin_external_activity()?;
                if let Some(task_runner) = &self.task_runner {
                    let failed_task_ids = {
                        let conn = self.db.lock();
                        repository::list_all_transfers(&conn)?
                            .into_iter()
                            .filter(|task| task.state_kind() == Ok(TransferState::Failed))
                            .map(|task| task.id)
                            .collect::<Vec<_>>()
                    };
                    for task_id in failed_task_ids {
                        match task_runner.prepare_retry(task_id).await {
                            Ok(prepared) => {
                                if let Err(error) = task_runner.run_prepared(prepared.id).await {
                                    tracing::warn!(task_id, %error, "全局重试执行失败，状态已由任务机保留");
                                }
                            }
                            Err(error) => {
                                tracing::warn!(task_id, %error, "失败任务未通过重试前置校验");
                            }
                        }
                        self.ensure_cycle_active()?;
                    }
                }
                {
                    let conn = self.db.lock();
                    conn.execute(
                        "UPDATE sync_items
                         SET status=?1, error_message=NULL
                         WHERE status=?2
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue AS task
                               WHERE task.relative_path=sync_items.local_path AND task.state=?3
                           )",
                        rusqlite::params![
                            repository::sync_status::SYNCING,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Failed),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("接受失败项重试失败：{error}")))?;
                }
                self.recompute_and_broadcast_state()?;
            }
            self.ensure_cycle_active()?;
            (self.cycle_observer)("local-rescan");
            self.run_sync_cycle_inner(triggered_by).await
        }
        .await;
        let needs_idle_restore = {
            let state = self.state.lock();
            state.is_running || state.is_indexing || state.sync_phase.is_some()
        };
        if needs_idle_restore {
            self.restore_idle_runtime_after_error();
        }
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
        let local = self.scan_local().await?;
        (self.cycle_observer)("local-scan-complete");
        self.ensure_cycle_active()?;
        let planning_activity = self.begin_external_activity()?;
        let cloud = self.cloud_tree.lock().clone();
        let mut db = self.load_db_snapshot()?;

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
            self.reconcile_db_records(&local, &db)?;
            let reconciliation = self.reconcile_failed_and_purge_stale_records(&local, &cloud)?;
            db = self.load_db_snapshot()?;
            tracing::info!(
                healed = reconciliation.healed,
                purged = reconciliation.purged,
                remaining_failed = reconciliation.remaining_failed,
                "可信同步周期已完成失败状态复核与残余清理"
            );
        } else {
            tracing::warn!("云端树不可信，跳过 DB reconcile、失败复核与残余清理");
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
            self.detect_renames(&mut actions)?;
        }
        filter_anti_oscillation(&mut actions, &self.recently_deleted_paths.lock());
        fill_parent_file_ids(&mut actions, &self.path_to_id.lock());
        // 为"云端已删目录下有内容需救援"补建目录链（跳过用户主动删除的目录）
        add_rescue_folder_recreations(&mut actions, &snapshot, &self.recently_deleted_paths.lock());

        // §2.11 防误删校验：planner 判定 !local_exists 时可能因 scan_local 漏扫
        // （如下载刚完成、xattr 延迟）误生成 DeleteFromCloud。在 mount_dir 下实际
        // stat 文件，若文件存在 → 改为 Skip，防止"删了又删"振荡（288→删除→上传）。
        self.validate_delete_from_cloud(&mut actions);
        // DeleteFromLocal 的远端删除事实与本地版本会由 executor 在真正 unlink 前复核；
        // 不在规划阶段重复串行 GET，避免扩大扫描到执行之间的竞态窗口。

        // §2.12 目录级联删除去重：删除一个目录时，华为 API 对目录设置 recycled=true
        // 会级联将整个子树移入回收站（保留目录层级）。若同时为目录和其子文件分别生成
        // DeleteFromCloud，子文件会作为独立条目进入回收站 → 目录层级丢失、用户只能逐个恢复。
        // 本过滤：检测到目录 DeleteFromCloud 时，移除其所有子孙的 DeleteFromCloud。
        dedupe_directory_deletes(&mut actions, &self.cloud_tree.lock());

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
        self.apply_results(&actions, &results)?;
        if actions.iter().zip(results.iter()).any(|(action, result)| {
            result.success && action.action_type == crate::sync::state::SyncActionType::MoveInCloud
        }) {
            // Move settles only the structural fact and deliberately preserves the previous
            // content baseline. Re-scan immediately so a file edited during/around the move is
            // uploaded in a second, version-checked step instead of being falsely marked synced.
            self.cycle.request(CycleRequest::LOCAL_RESCAN);
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
                        | crate::sync::state::SyncActionType::MoveInCloud
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
            let mut res = exec
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
                // Child actions need the new parent ID, but publishing it before the durable
                // baseline is committed creates a cache/DB split-brain on write failure. Settle
                // this one dependency first; `apply_results` publishes cache only after commit.
                if let Err(error) = self.apply_results(
                    std::slice::from_ref(&actions[i]),
                    std::slice::from_ref(&res),
                ) {
                    res.success = false;
                    res.deferred = true;
                    res.error_message = Some(format!(
                        "云端目录已创建，但本地基线结算失败，等待重新收敛：{error}"
                    ));
                } else if let Some(cloud_file) = res.cloud_file.as_ref() {
                    tracing::info!(rel = %actions[i].relative_path.as_deref().unwrap_or("?"),
                        folder_id = %cloud_file.id, "本地新建目录已持久化并回填 path_to_id");
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
    ) -> AppResult<()> {
        use crate::sync::state::SyncActionType;

        // Capture explicit directory subtrees before any cache mutation. The matching DB
        // settlement happens first; only successfully settled roots are removed from memory.
        let delete_subtrees: std::collections::HashMap<String, (bool, Vec<String>)> = {
            let cloud = self.cloud_tree.lock();
            actions
                .iter()
                .zip(results.iter())
                .filter(|(action, result)| {
                    result.success && action.action_type == SyncActionType::DeleteFromCloud
                })
                .filter_map(|(action, _)| {
                    let root = action.relative_path.as_ref()?;
                    let prefix = format!("{root}/");
                    let is_directory = cloud.get(root).is_some_and(|file| file.is_folder());
                    let paths = if is_directory {
                        cloud
                            .keys()
                            .filter(|path| *path == root || path.starts_with(&prefix))
                            .cloned()
                            .collect()
                    } else {
                        vec![root.clone()]
                    };
                    Some((root.clone(), (is_directory, paths)))
                })
                .collect()
        };
        let mut settled_cloud_deletes = std::collections::HashSet::new();

        // Update the durable baseline before changing in-memory caches.
        let conn = self.db.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始同步结果结算事务失败：{error}")))?;
        let move_baselines = if actions
            .iter()
            .any(|action| action.action_type == SyncActionType::MoveInCloud)
        {
            repository::load_all(&transaction)?
        } else {
            Vec::new()
        };
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
                let delete_result = if action.action_type == SyncActionType::DeleteFromCloud
                    && delete_subtrees
                        .get(rel)
                        .is_some_and(|(is_directory, _)| *is_directory)
                {
                    let prefix = format!("{rel}/");
                    transaction.execute(
                        "DELETE FROM sync_items
                         WHERE local_path=?1 OR substr(local_path, 1, length(?2))=?2",
                        rusqlite::params![rel, prefix],
                    )
                } else {
                    transaction.execute(
                        "DELETE FROM sync_items WHERE local_path=?1 AND (?2='' OR file_id=?2)",
                        rusqlite::params![rel, fid],
                    )
                };
                delete_result.map_err(|error| {
                    AppError::generic(format!("删除已确认但基线结算失败（{rel}）：{error}"))
                })?;
                if action.action_type == SyncActionType::DeleteFromCloud {
                    settled_cloud_deletes.insert(rel.clone());
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
                        transaction
                            .execute(
                                "UPDATE sync_items SET status=?1, error_message=?2
                             WHERE file_id=?3 AND local_path=?4",
                                rusqlite::params![
                                    repository::sync_status::FAILED,
                                    result.error_message.as_deref(),
                                    file_id,
                                    rel,
                                ],
                            )
                            .map_err(|error| {
                                AppError::generic(format!("记录同步失败状态失败：{error}"))
                            })?;
                    }
                }
                continue;
            }

            // Skip is normally a deliberate no-op (startup delete guard, access error, tombstone,
            // or reconcile handoff) and must never manufacture a successful baseline. The sole
            // settlement case is legacy `pending:<path>` upload recovery with a concrete cloud
            // file at the same path; that row is replaced only after local metadata is re-read.
            if action.action_type == SyncActionType::Skip {
                let pending_file_id = format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel);
                let pending_exists = transaction
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM sync_items WHERE file_id=?1 AND local_path=?2
                         )",
                        rusqlite::params![pending_file_id, rel],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|error| {
                        AppError::generic(format!("核验待确认同步基线失败：{error}"))
                    })?;
                if !pending_exists || action.cloud_file.is_none() {
                    continue;
                }
            }

            // Upload/Download are already atomically settled by TaskRunner against task ID,
            // revision and source snapshot. Re-statting them here would let an edit made after
            // the remote response overwrite the verified success baseline.
            if matches!(
                action.action_type,
                SyncActionType::Upload | SyncActionType::Download
            ) {
                if action.action_type == SyncActionType::Upload
                    && action
                        .reason
                        .as_deref()
                        .is_some_and(|reason| reason.starts_with("同目录改名检测："))
                {
                    let file_id = action
                        .file_id
                        .as_deref()
                        .ok_or_else(|| AppError::generic("同目录改名已完成但动作缺少 fileId"))?;
                    transaction
                        .execute(
                            "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                            rusqlite::params![file_id, rel],
                        )
                        .map_err(|error| {
                            AppError::generic(format!("清理同目录改名旧路径基线失败：{error}"))
                        })?;
                }
                continue;
            }

            let default_status = if action.action_type == SyncActionType::CreatePlaceholder {
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
                    tracing::warn!(rel = %rel, status = default_status, "跳过成功基线写入：缺少真实 fileId");
                    continue;
                }
            };

            // A structural move must preserve the last content version that was actually synced.
            // Re-statting the destination here would falsely acknowledge edits made before/during
            // the remote move. The immediate follow-up cycle uploads any content delta.
            let move_baseline = if action.action_type == SyncActionType::MoveInCloud {
                Some(
                    move_baselines
                        .iter()
                        .find(|item| item.file_id == file_id && item.local_path != rel.as_str())
                        .or_else(|| move_baselines.iter().find(|item| item.file_id == file_id))
                        .ok_or_else(|| {
                            AppError::generic(format!(
                                "云端移动已确认但缺少原内容基线（fileId={file_id}）"
                            ))
                        })?,
                )
            } else {
                None
            };

            // 读取本地真实 mtime/size（对齐 dart _updateDbFromResults 从本地文件 stat）。
            // 写死 None 会导致 is_local_changed 恒 true（db.local_mtime.is_none()），每轮重传。
            let (local_mtime, local_size, sha256, is_folder) = match move_baseline {
                Some(baseline) => (
                    baseline.local_mtime,
                    baseline.local_size,
                    baseline.sha256.clone(),
                    baseline.is_folder,
                ),
                None => {
                    let settlement_path = if let Some(path) = action.local_path.as_ref() {
                        Some(std::path::PathBuf::from(path))
                    } else if matches!(
                        action.action_type,
                        SyncActionType::CreatePlaceholder
                            | SyncActionType::CreateFolder
                            | SyncActionType::Skip
                    ) {
                        Some(
                            self.mount
                                .as_ref()
                                .ok_or_else(|| {
                                    AppError::generic("同步挂载未初始化，无法结算本地动作")
                                })?
                                .mount_dir()
                                .join(rel),
                        )
                    } else {
                        None
                    };
                    let (local_mtime, local_size) = match settlement_path {
                        Some(p) => {
                            let metadata = std::fs::symlink_metadata(p).map_err(|error| {
                                AppError::generic(format!(
                                    "成功动作结算时无法读取本地目标（{rel}）：{error}"
                                ))
                            })?;
                            let expects_folder = action.action_type == SyncActionType::CreateFolder;
                            if metadata.file_type().is_symlink()
                                || (expects_folder && !metadata.is_dir())
                                || (!expects_folder && !metadata.is_file())
                            {
                                return Err(AppError::generic(format!(
                                    "成功动作结算时本地目标类型不一致：{rel}"
                                )));
                            }
                            let modified = metadata
                                .modified()
                                .ok()
                                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|duration| duration.as_millis() as i64)
                                .ok_or_else(|| {
                                    AppError::generic(format!(
                                        "成功动作结算时无法读取本地修改时间：{rel}"
                                    ))
                                })?;
                            if action.action_type == SyncActionType::Skip
                                && cloud_file.is_some_and(|file| file.size != metadata.len() as i64)
                            {
                                return Err(AppError::generic(format!(
                                    "待确认上传的本地大小与云端结果不一致，拒绝收敛成功：{rel}"
                                )));
                            }
                            (Some(modified), Some(metadata.len() as i64))
                        }
                        None => (None, None),
                    };
                    (
                        local_mtime,
                        local_size,
                        None,
                        matches!(action.action_type, SyncActionType::CreateFolder),
                    )
                }
            };
            let status = move_baseline
                .map(|baseline| baseline.status)
                .unwrap_or(default_status);
            let error_message = move_baseline.and_then(|baseline| baseline.error_message.clone());
            let last_sync_time = match move_baseline {
                Some(baseline) => baseline.last_sync_time,
                None => Some(chrono::Utc::now().timestamp_millis()),
            };

            // 重建云端已删目录（CreateFolder && cloud_file=None 成功）：新 folderId 与旧 db
            // 记录不同（旧目录已删，重建得新 id），先清掉同路径旧记录，避免 dual 记录污染。
            if result.success
                && action.action_type == SyncActionType::CreateFolder
                && action.cloud_file.is_none()
            {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE local_path=?1",
                        rusqlite::params![rel],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理重建目录旧基线失败：{error}"))
                    })?;
            }
            if action.action_type == SyncActionType::MoveInCloud {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                        rusqlite::params![file_id.as_str(), rel],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理云端移动旧路径基线失败：{error}"))
                    })?;
            }
            // 成功上传/覆盖上传 → 清掉同路径的 pending: 占位孤儿行。
            // PK 是 (file_id, local_path)，真实 fileId 与 pending: 占位 fileId 是不同主键，
            // upsert 不会覆盖占位行 → 必须显式删除，避免残留孤儿导致 planner 误判。
            if !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE local_path=?1 AND file_id=?2",
                        rusqlite::params![
                            rel,
                            format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel)
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("清理待确认基线失败：{error}")))?;
            }

            // upsert（对齐 dart insertOnConflictUpdate）
            repository::upsert(
                &transaction,
                &repository::SyncItem {
                    file_id,
                    local_path: rel.clone(),
                    parent_folder_id: cloud_file
                        .and_then(|file| {
                            file.parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned())
                        })
                        .or_else(|| action.parent_file_id.clone()),
                    name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
                    is_folder,
                    size: cloud_file.map(|f| f.size).unwrap_or(0),
                    local_size,
                    sha256,
                    local_mtime,
                    cloud_edited_time: cloud_file
                        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis())),
                    last_sync_time,
                    status,
                    // Structural moves preserve any existing content failure. Other confirmed
                    // successes clear stale compatibility errors.
                    error_message,
                },
            )?;
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交同步结果结算失败：{error}")))?;
        drop(conn);

        // Publish cache deltas only after their durable baseline writes succeeded.
        {
            let mut recently_deleted = self.recently_deleted_paths.lock();
            let mut cloud = self.cloud_tree.lock();
            let mut path_to_id = self.path_to_id.lock();
            for (action, result) in actions.iter().zip(results.iter()) {
                let Some(relative_path) = &action.relative_path else {
                    continue;
                };
                if result.success
                    && matches!(
                        action.action_type,
                        SyncActionType::DeleteFromCloud
                            | SyncActionType::DeleteFromLocal
                            | SyncActionType::BackupBeforeCloudDelete
                    )
                {
                    recently_deleted
                        .insert(relative_path.clone(), chrono::Utc::now().timestamp_millis());
                }
                if result.success && action.action_type == SyncActionType::DeleteFromCloud {
                    if !settled_cloud_deletes.contains(relative_path) {
                        continue;
                    }
                    for path in delete_subtrees
                        .get(relative_path)
                        .map(|(_, paths)| paths)
                        .into_iter()
                        .flatten()
                    {
                        cloud.remove(path);
                        path_to_id.remove(path);
                    }
                    recently_deleted
                        .insert(relative_path.clone(), chrono::Utc::now().timestamp_millis());
                    continue;
                }
                if result.success {
                    if let Some(file) = result.cloud_file.as_ref().or(action.cloud_file.as_ref()) {
                        let stale_paths: Vec<String> = cloud
                            .iter()
                            .filter(|(path, existing)| {
                                path.as_str() != relative_path && existing.id == file.id
                            })
                            .map(|(path, _)| path.clone())
                            .collect();
                        for stale_path in stale_paths {
                            cloud.remove(&stale_path);
                            path_to_id.remove(&stale_path);
                        }
                        cloud.insert(relative_path.clone(), file.clone());
                        path_to_id.insert(relative_path.clone(), file.id.clone());
                    }
                }
            }
            let expire_before = chrono::Utc::now().timestamp_millis() - 300_000;
            recently_deleted.retain(|_, timestamp| *timestamp > expire_before);
        }
        Ok(())
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
    ) -> AppResult<()> {
        let conn = self.db.lock();
        let durable_records = repository::load_all(&conn)?;
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        for (rel, entry) in local {
            if let Some(db_entry) = db.get(rel) {
                // 若 DB 记录标记为 DELETED 但用户重新粘贴了文件 → 复活为正常状态
                if db_entry.status == repository::sync_status::DELETED {
                    let status = if entry.is_placeholder {
                        repository::sync_status::CLOUD_ONLY
                    } else {
                        repository::sync_status::SYNCED
                    };
                    conn.execute(
                        "UPDATE sync_items SET status=?1 WHERE local_path=?2",
                        rusqlite::params![status, rel],
                    )
                    .map_err(|error| AppError::generic(format!("复活删除墓碑失败：{error}")))?;
                    tracing::info!(rel = %rel, "DELETED 墓碑已复活");
                }
                continue;
            }

            // A trusted cloud tree plus the same local directory path/type is sufficient folder
            // identity. This closes a crash window after CreateFolder committed remotely but
            // before its DB baseline was written; folders do not have a file xattr identity.
            if entry.is_folder {
                if let Some(cloud_folder) = ct.get(rel).filter(|file| file.is_folder()) {
                    let metadata =
                        std::fs::symlink_metadata(&entry.absolute_path).map_err(|error| {
                            AppError::generic(format!("读取待恢复目录基线失败：{error}"))
                        })?;
                    repository::upsert(
                        &conn,
                        &repository::SyncItem {
                            file_id: cloud_folder.id.clone(),
                            local_path: rel.clone(),
                            parent_folder_id: cloud_folder
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned()),
                            name: cloud_folder.name.clone(),
                            is_folder: true,
                            size: cloud_folder.size,
                            local_size: Some(metadata.len() as i64),
                            sha256: None,
                            local_mtime: Some(entry.mtime),
                            cloud_edited_time: cloud_folder
                                .edited_time
                                .map(|time| time.timestamp_millis()),
                            last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                            status: repository::sync_status::SYNCED,
                            error_message: None,
                        },
                    )?;
                    continue;
                }
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

            // Crash/response-loss recovery for a confirmed remote move. If this fileId already
            // has a baseline at another path and that source path is truly absent locally, migrate
            // the key while preserving the old content version. Never stamp the destination's
            // current mtime/size as synced merely because the remote structural move committed.
            if let Some(previous) = durable_records
                .iter()
                .find(|record| record.file_id == file_id && record.local_path != rel.as_str())
            {
                let previous_path = std::path::PathBuf::from(&mount_dir).join(&previous.local_path);
                match std::fs::symlink_metadata(&previous_path) {
                    Ok(_) => {
                        tracing::warn!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            "同一 fileId 的旧本地路径仍存在，按复制/歧义处理，拒绝迁移成功基线"
                        );
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::warn!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            %error,
                            "无法证明旧本地路径已消失，拒绝迁移成功基线"
                        );
                        continue;
                    }
                }

                let migration = (|| -> AppResult<()> {
                    let transaction = conn.unchecked_transaction().map_err(|error| {
                        AppError::generic(format!("开始恢复云端移动基线事务失败：{error}"))
                    })?;
                    transaction
                        .execute(
                            "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                            rusqlite::params![file_id, rel],
                        )
                        .map_err(|error| {
                            AppError::generic(format!("清理移动前旧路径基线失败：{error}"))
                        })?;
                    repository::upsert(
                        &transaction,
                        &repository::SyncItem {
                            file_id: file_id.clone(),
                            local_path: rel.clone(),
                            parent_folder_id: cloud_file
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned()),
                            name: cloud_file.name.clone(),
                            is_folder: previous.is_folder,
                            size: cloud_file.size,
                            local_size: previous.local_size,
                            sha256: previous.sha256.clone(),
                            local_mtime: previous.local_mtime,
                            cloud_edited_time: cloud_file
                                .edited_time
                                .map(|time| time.timestamp_millis()),
                            last_sync_time: previous.last_sync_time,
                            status: previous.status,
                            error_message: previous.error_message.clone(),
                        },
                    )?;
                    transaction.commit().map_err(|error| {
                        AppError::generic(format!("提交恢复云端移动基线失败：{error}"))
                    })?;
                    Ok(())
                })();
                match migration {
                    Ok(()) => {
                        tracing::info!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            "已恢复响应丢失/进程中断后的云端移动基线"
                        );
                        self.cycle.request(CycleRequest::LOCAL_RESCAN);
                    }
                    Err(error) => {
                        tracing::warn!(old = %previous.local_path, new = %rel, file_id, %error, "恢复云端移动基线失败");
                    }
                }
                continue;
            }
            repository::upsert(
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
            )?;
        }
        drop(conn);
        Ok(())
    }

    fn reconcile_failed_and_purge_stale_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        cloud: &HashMap<String, DriveFile>,
    ) -> AppResult<FailedRecordReconciliation> {
        let conn = self.db.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始失败状态复核事务失败：{error}")))?;
        let items = repository::load_all(&transaction)?;
        let mut reconciliation = FailedRecordReconciliation::default();

        for item in items
            .iter()
            .filter(|item| item.status == repository::sync_status::FAILED)
        {
            let (local_entry, cloud_file) =
                match (local.get(&item.local_path), cloud.get(&item.local_path)) {
                    (None, None) => continue,
                    (Some(local_entry), Some(cloud_file)) => (local_entry, cloud_file),
                    _ => {
                        reconciliation.missing_side += 1;
                        if item.file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                            reconciliation.pending_id += 1;
                        }
                        continue;
                    }
                };
            if item.file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                reconciliation.pending_id += 1;
                continue;
            }
            if item.file_id != cloud_file.id {
                reconciliation.id_mismatch += 1;
                continue;
            }
            if local_entry.is_folder != cloud_file.is_folder() {
                reconciliation.type_conflict += 1;
                continue;
            }
            if local_entry.is_folder && cloud_file.is_folder() {
                let updated = transaction
                    .execute(
                        "UPDATE sync_items SET
                            parent_folder_id=?1, name=?2, is_folder=1, size=?3,
                            local_size=?4, sha256=NULL, local_mtime=?5, cloud_edited_time=?6,
                            last_sync_time=?7, status=?8, error_message=NULL
                         WHERE file_id=?9 AND local_path=?10 AND status=?11
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue
                               WHERE relative_path=?10 AND state NOT IN (?12, ?13)
                           )",
                        rusqlite::params![
                            cloud_file
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first()),
                            cloud_file.name,
                            cloud_file.size,
                            local_entry.size as i64,
                            local_entry.mtime,
                            cloud_file.edited_time.map(|time| time.timestamp_millis()),
                            chrono::Utc::now().timestamp_millis(),
                            repository::sync_status::SYNCED,
                            item.file_id,
                            item.local_path,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Completed),
                            i32::from(TransferState::Canceled),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("恢复目录失败状态失败：{error}")))?;
                reconciliation.healed += updated;
                if updated == 0 {
                    reconciliation.transfer_blocked += 1;
                }
                continue;
            }

            let cloud_edited_time = cloud_file.edited_time.map(|time| time.timestamp_millis());
            if item.is_folder {
                reconciliation.type_conflict += 1;
                continue;
            }
            let file_converged = !local_entry.is_placeholder
                && item.local_size == Some(local_entry.size as i64)
                && item.local_mtime == Some(local_entry.mtime)
                && item.size == cloud_file.size
                && item.cloud_edited_time.is_some()
                && item.cloud_edited_time == cloud_edited_time;
            if file_converged {
                let updated = transaction
                    .execute(
                        "UPDATE sync_items SET status=?1, error_message=NULL
                         WHERE file_id=?2 AND local_path=?3 AND status=?4
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue
                               WHERE relative_path=?3 AND state NOT IN (?5, ?6)
                           )",
                        rusqlite::params![
                            repository::sync_status::SYNCED,
                            item.file_id,
                            item.local_path,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Completed),
                            i32::from(TransferState::Canceled),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("恢复文件失败状态失败：{error}")))?;
                reconciliation.healed += updated;
                if updated == 0 {
                    reconciliation.transfer_blocked += 1;
                }
            } else {
                reconciliation.baseline_changed += 1;
            }
        }

        for item in items.iter().filter(|item| {
            !local.contains_key(&item.local_path) && !cloud.contains_key(&item.local_path)
        }) {
            let deleted = transaction
                .execute(
                    "DELETE FROM sync_items
                     WHERE local_path=?1
                       AND NOT EXISTS (
                           SELECT 1 FROM transfer_queue
                           WHERE relative_path=?1 AND state NOT IN (?2, ?3)
                       )",
                    rusqlite::params![
                        item.local_path,
                        i32::from(TransferState::Completed),
                        i32::from(TransferState::Canceled),
                    ],
                )
                .map_err(|error| AppError::generic(format!("清理残余基线失败：{error}")))?;
            reconciliation.purged += deleted;
            if deleted == 0 {
                reconciliation.transfer_blocked += 1;
                reconciliation.stale_transfer_blocked += 1;
            }
        }
        reconciliation.remaining_failed = transaction
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE status=?1",
                rusqlite::params![repository::sync_status::FAILED],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::generic(format!("统计剩余失败状态失败：{error}")))?
            as usize;
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交失败状态复核事务失败：{error}")))?;
        tracing::debug!(
            healed = reconciliation.healed,
            purged = reconciliation.purged,
            remaining_failed = reconciliation.remaining_failed,
            pending_id = reconciliation.pending_id,
            missing_side = reconciliation.missing_side,
            id_mismatch = reconciliation.id_mismatch,
            type_conflict = reconciliation.type_conflict,
            baseline_changed = reconciliation.baseline_changed,
            transfer_blocked = reconciliation.transfer_blocked,
            stale_transfer_blocked = reconciliation.stale_transfer_blocked,
            "失败状态复核完成"
        );
        Ok(reconciliation)
    }

    /// §2.8 改名检测：本地新文件若 xattr 含已知 fileId → 改名而非 upload+delete。
    /// 通过 xattr 匹配"本地新文件"与"缺失原路径的 DB 记录"，调用 update 同步改名到云端
    /// （先于内容同步），避免先删后传导致云端短暂不可用。
    fn detect_renames(&self, actions: &mut Vec<crate::sync::state::SyncAction>) -> AppResult<()> {
        use crate::mount::manager::XATTR_FILE_ID;
        use crate::sync::state::SyncActionType;
        let db = self.db.lock();
        // 先收集全体 DB 记录（按 fileId 索引）
        let db_by_id: std::collections::HashMap<String, crate::data::repository::SyncItem> =
            repository::load_all(&db)?
                .into_iter()
                .filter(|r| !r.file_id.is_empty())
                .map(|r| (r.file_id.clone(), r))
                .collect();
        drop(db);

        let path_to_id = self.path_to_id.lock().clone();
        let root_folder_id = self.root_folder_id.lock().clone();
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        let mut renamed_sources = std::collections::HashSet::new();
        let mut superseded_cloud_paths = std::collections::HashSet::new();
        for action in actions.iter_mut() {
            if action.action_type != SyncActionType::Upload || action.file_id.is_some() {
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
            // Resolve by immutable fileId, not only by the old DB path. This also converges after
            // a crash or uncertain response where Huawei already moved the file but local DB/cache
            // settlement did not finish.
            let Some((current_cloud_path, cloud_file)) =
                ct.iter().find(|(_, cloud_file)| cloud_file.id == fid)
            else {
                continue;
            };
            // 旧文件仍在本地 → 复制（非改名）：新文件应作为全新上传，
            // 不能复用旧 fileId 走 update/rename 路径（否则云端文件被移动/覆盖）。
            // 同时清除新文件上的旧 xattr fileId，避免下轮又被误判为改名。
            let old_abs = std::path::PathBuf::from(&mount_dir).join(&old_record.local_path);
            match std::fs::symlink_metadata(&old_abs) {
                Ok(_) => {
                    let _ = xattr::remove(&local_path, XATTR_FILE_ID);
                    tracing::info!(
                        old = %old_record.local_path,
                        new = action.relative_path.as_deref().unwrap_or("?"),
                        "复制检测（旧文件仍在本地），已清除新文件旧 xattr，按全新文件上传");
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(
                        old = %old_record.local_path,
                        new = action.relative_path.as_deref().unwrap_or("?"),
                        %error,
                        "无法证明旧路径已经消失，拒绝执行远端改名/移动");
                    continue;
                }
            }

            let new_relative_path = action.relative_path.clone().unwrap_or_default();
            let current_parent_path = current_cloud_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let new_parent_path = new_relative_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");

            action.file_id = Some(fid.clone());
            action.cloud_file = Some(cloud_file.clone());
            if current_parent_path == new_parent_path {
                action.parent_file_id = cloud_file
                    .parent_folder
                    .as_ref()
                    .and_then(|v| v.first().cloned());
                action.reason = Some(format!(
                    "同目录改名检测：{} → {}（fileId={}，先于内容同步）",
                    old_record.local_path, new_relative_path, fid,
                ));
            } else {
                action.action_type = SyncActionType::MoveInCloud;
                action.parent_file_id = if new_parent_path.is_empty() {
                    root_folder_id.clone()
                } else {
                    path_to_id.get(new_parent_path).cloned()
                };
                action.reason = Some(format!(
                    "跨目录移动检测：{} → {}（fileId={}，目标 parent={:?}）",
                    old_record.local_path, new_relative_path, fid, action.parent_file_id,
                ));
            }
            if current_cloud_path.as_str() != new_relative_path.as_str() {
                superseded_cloud_paths.insert((current_cloud_path.clone(), fid.clone()));
            }
            renamed_sources.insert((old_record.local_path.clone(), fid));
            tracing::info!(reason = action.reason.as_deref(), "检测到本地文件路径变化");
        }
        drop(ct);

        // The planner originally emits DeleteFromCloud(old) + Upload(new). Once xattr proves they
        // are the same file, the old delete and any deleting ancestor must disappear from this
        // cycle; otherwise either can race the rename/move and recycle the file being preserved.
        actions.retain(|action| {
            if action.action_type == SyncActionType::CreatePlaceholder {
                if let (Some(path), Some(file_id)) =
                    (action.relative_path.as_ref(), action.file_id.as_ref())
                {
                    if superseded_cloud_paths.contains(&(path.clone(), file_id.clone())) {
                        return false;
                    }
                }
            }
            if action.action_type != SyncActionType::DeleteFromCloud {
                return true;
            }
            let Some(old_path) = action.relative_path.as_ref() else {
                return true;
            };
            let Some(file_id) = action.file_id.as_ref() else {
                return true;
            };
            !renamed_sources.iter().any(|(source_path, source_file_id)| {
                // Exact source delete is the planner's original half of upload+delete. An ancestor
                // directory delete is also postponed: if the move is deferred, recycling that
                // directory would still recycle the source file. A successful move schedules an
                // immediate follow-up where the now-empty directory can be deleted safely.
                (source_path == old_path && source_file_id == file_id)
                    || source_path
                        .strip_prefix(old_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        });
        Ok(())
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

    async fn scan_local(&self) -> AppResult<HashMap<String, LocalFileEntry>> {
        match &self.mount {
            Some(m) => Ok(m
                .scan_local(&self.skip_patterns)
                .await?
                .into_iter()
                .map(|e| (e.relative_path.clone(), e))
                .collect()),
            None => Err(AppError::generic("同步挂载尚未初始化，拒绝按空本地树规划")),
        }
    }

    fn load_db_snapshot(&self) -> AppResult<HashMap<String, DbSnapshotEntry>> {
        let conn = self.db.lock();
        let mut snapshot = HashMap::new();
        for record in repository::load_all(&conn)? {
            let relative_path = record.local_path.clone();
            let entry = DbSnapshotEntry {
                file_id: record.file_id,
                local_mtime: record.local_mtime,
                local_size: record.local_size,
                cloud_edited_time: record.cloud_edited_time,
                status: record.status,
                is_folder: record.is_folder,
            };
            if snapshot.insert(relative_path.clone(), entry).is_some() {
                return Err(AppError::generic(format!(
                    "同步基线存在重复本地路径，拒绝继续规划：{relative_path}"
                )));
            }
        }
        Ok(snapshot)
    }

    /// 安全释放校验。
    pub fn can_safely_free_up(&self, rel_path: &str, file_id: &str) -> FreeUpCheckResult {
        if !self.cloud_tree_is_trusted() {
            return FreeUpCheckResult::NotSynced;
        }
        let tree = self.cloud_tree.lock();
        if tree.get(rel_path).map(|file| file.id.as_str()) != Some(file_id) {
            return FreeUpCheckResult::NotInCloud;
        }
        drop(tree);
        let conn = self.db.lock();
        if repository::list_all_transfers(&conn).is_ok_and(|tasks| {
            tasks.into_iter().any(|task| {
                task.relative_path.as_deref() == Some(rel_path)
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            TransferState::Completed
                                | TransferState::Failed
                                | TransferState::Canceled
                        )
                    })
            })
        }) {
            return FreeUpCheckResult::NotSynced;
        }
        if let Ok(Some(record)) = repository::find_by_file_id(&conn, file_id) {
            if record.local_path != rel_path || record.status != repository::sync_status::SYNCED {
                return FreeUpCheckResult::NotSynced;
            }
            let Some(mount) = &self.mount else {
                return FreeUpCheckResult::NotSynced;
            };
            let path = mount.mount_dir().join(&record.local_path);
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

        // A loaded checkpoint may be stale for planning but is still a structurally validated
        // baseline for Changes replay. `cloud_cursor` exists only after validated checkpoint
        // installation, so catch-up is allowed while destructive trust remains false.
        if !force_full {
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
                    let parents = file.parent_folder.as_ref().ok_or_else(|| {
                        AppError::generic(format!("Change {} 缺少 parentFolder", change.file_id()))
                    })?;
                    if parents.len() != 1 || parents[0].trim().is_empty() {
                        return Err(AppError::generic(format!(
                            "Change {} 的多父目录/空父目录语义不受支持",
                            change.file_id()
                        )));
                    }
                    let parent_id = &parents[0];
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
            match std::fs::symlink_metadata(&abs) {
                Ok(meta) => {
                    let size = meta.len();
                    // 占位符（0 字节 + xattr state=placeholder）→ 仍执行删除（无实际内容）
                    // 其余（size>0 或无 xattr）→ 本地文件实际存在，跳过删除
                    if size == 0 && crate::mount::manager::is_placeholder_file(&abs) {
                        continue;
                    }
                    tracing::info!(
                        rel = %rel,
                        size,
                        reason = a.reason.as_deref().unwrap_or("?"),
                        "DeleteFromCloud 防误删：本地文件实际存在，改为 Skip"
                    );
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason = Some(format!(
                        "防误删：本地文件实际存在（{} 字节），跳过 DeleteFromCloud",
                        size,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(rel = %rel, %error, "无法确认本地文件不存在，拒绝云端删除");
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason = Some(format!("本地路径访问异常，无法证明文件已删除：{error}"));
                }
            }
        }
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
            self.request_cycle_background("retry-replan");
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
