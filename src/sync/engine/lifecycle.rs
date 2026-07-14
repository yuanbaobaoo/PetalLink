//! 同步引擎的构造、依赖注入、启动、监听与关闭。

use std::sync::atomic::Ordering;

use super::coordination::{network_listener_loop, watcher_listener_loop, TaskRunnerActivityGate};
use super::*;

impl SyncEngine {
    #[allow(clippy::too_many_arguments)]
    /// 创建尚未绑定挂载目录与执行器的同步引擎。
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

    /// 绑定挂载管理器并记录当前挂载目录。
    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        *self.mount_dir.lock() = Some(mount.mount_dir().to_string_lossy().to_string());
        self.mount = Some(mount);
    }

    /// 绑定执行器，并共享 TaskRunner 与活动门。
    pub fn set_executor(&mut self, mut executor: SyncExecutor) {
        let activity_gate = Arc::new(TaskRunnerActivityGate(self.activity.clone()));
        executor.set_action_activity_gate(activity_gate.clone());
        self.task_runner = executor.task_runner().ok();
        if let Some(task_runner) = &self.task_runner {
            task_runner.set_activity_gate(activity_gate);
        }
        self.executor = Some(executor);
    }

    /// 替换引擎的实时网络可用性判定器。
    pub fn set_online_check(&mut self, online_check: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.online_check = online_check;
    }

    /// 设置周期启动观察器，用于记录真实触发来源。
    pub fn set_cycle_observer(&mut self, cycle_observer: Arc<dyn Fn(&'static str) + Send + Sync>) {
        self.cycle_observer = cycle_observer;
    }

    /// 读取当前网络可用性快照。
    pub(super) fn is_online(&self) -> bool {
        (self.online_check)()
    }

    /// 登记不绑定具体路径的外部引擎活动。
    pub(crate) fn begin_external_activity(&self) -> AppResult<ActivityGuard> {
        self.activity.begin(None)
    }

    /// 校验路径后尝试获取排他子树活动租约。
    pub(crate) fn begin_exclusive_path_activity(
        &self,
        relative_path: &str,
    ) -> AppResult<ActivityGuard> {
        crate::core::paths::validate_relative_path(relative_path, false)?;
        self.activity.begin_exclusive(relative_path)
    }

    /// 返回已绑定的持久传输调度器。
    pub(crate) fn task_runner(&self) -> AppResult<Arc<TaskRunner>> {
        self.task_runner
            .clone()
            .ok_or_else(|| AppError::generic("TaskRunner 未初始化"))
    }

    /// 返回当前引擎使用的统一文件跳过规则。
    pub(crate) fn skip_patterns(&self) -> &[String] {
        &self.skip_patterns
    }

    /// 判断引擎是否已进入运行生命周期。
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

    /// 尝试获取会自动释放的目录同步 guard。
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

    /// 关闭流程的同步子集：仅设置关闭标志并释放监视器句柄。
    /// 供无法等待异步任务的运行时销毁或关闭线程调用；句柄释放后不再接收文件事件。
    pub fn shutdown_sync(&self) {
        self.shutdown_sync_with_contention_hook(|| {});
    }

    /// 在发布屏障内置位 shutdown，并可观察已确认的锁竞争。
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
        // 取出并释放 watcher，同步关闭底层 FSEvents stream。
        let taken = self.watcher.lock().take();
        drop(taken);
        tracing::info!("SyncEngine shutdown_sync（shutdown 标志置位、FSEvents 释放）");
    }

    /// 先订阅网络转换，再完成启动收敛并装配运行期监听任务。
    pub async fn start(self: &Arc<Self>) -> AppResult<()> {
        // 先订阅再读取当前网络 level，启动收敛结束后才运行 receiver，
        // 避免已缓冲边沿与首个云端快照竞争。
        let network_transitions = crate::core::net_guard::subscribe();
        self.start_with_network_receiver(network_transitions, true)
            .await
    }

    /// 在已建立网络订阅的前提下完成启动收敛与后台任务装配。
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

        // 网络探测会派生异步任务，必须在 Tokio 运行时内启动，不能放进同步初始化闭包。
        if start_probe {
            crate::core::net_guard::start_probe_task();
            crate::core::net_guard::init_sleep_handling();
        }

        // 启动恢复、云端刷新与首次本地收敛共用后续触发器的唯一 owner；
        // 此前到达的请求保持待处理，并合并进启动周期或唯一跟进周期。
        let startup_deferred = match self.run_sync_cycle("startup-resume").await {
            Ok(()) => false,
            Err(error) if !self.is_online() || Self::is_recoverable_cycle_error(&error) => {
                tracing::warn!(%error, "启动同步暂不可执行，保留 STARTUP 等待后台恢复");
                true
            }
            Err(error) => return Err(error),
        };
        self.ensure_cycle_active()?;

        // 在启用任何可入队新周期的来源前先发布启动空闲状态；
        // 已创建 receiver 会保留缓冲的网络边沿，并在此后处理。
        self.update_runtime_and_broadcast(|runtime| {
            runtime.is_running = false;
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        })?;
        self.started.store(true, Ordering::Release);

        self.start_network_listener(network_transitions);
        self.ensure_cycle_active()?;

        // 云端树扫描完成后再启动本地监视器。
        self.start_watcher().await;
        self.ensure_cycle_active()?;

        // 启动云端定时刷新任务（poll_interval_secs=0 时内部不启动）
        self.start_cloud_refresh_timer().await;
        self.start_backoff_scheduler();

        // 网络仍为 Online 时也可能发生短暂云端失败，因此不能依赖转换边沿唤醒。
        // 显式消费保留的 STARTUP 请求，由后台 worker 执行有界退避。
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
    pub(super) async fn recover_interrupted_transfers(
        &self,
    ) -> crate::sync::task_runner::StartupRecoverySummary {
        let Some(task_runner) = &self.task_runner else {
            tracing::warn!("TaskRunner 未初始化，跳过中断任务恢复");
            return crate::sync::task_runner::StartupRecoverySummary::default();
        };
        match task_runner.recover_startup().await {
            Ok(summary) => {
                tracing::info!(
                    completed = summary.completed,
                    waiting_network = summary.waiting_network,
                    verifying_remote = summary.verifying_remote,
                    failed = summary.failed,
                    "中断传输已通过统一 TaskRunner 恢复"
                );
                summary
            }
            Err(error) => {
                tracing::warn!(%error, "统一中断任务恢复失败");
                crate::sync::task_runner::StartupRecoverySummary::default()
            }
        }
    }
    /// 启动本地文件监听器，并把变更合并为重扫请求。
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

    /// 启动网络转换监听，在恢复在线时请求收敛。
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

    /// 启动持久传输退避 deadline 调度器。
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
    /// 关闭检查模式：循环开始与休眠唤醒后各检查一次，置位即退出。
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
}
