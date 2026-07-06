//! 同步引擎主循环 —— 核心编排（阶段 5 骨架，后续阶段逐步接入 mount/executor/cloud_tree 完成闭环）。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart`。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use rusqlite::{Connection, params};

use crate::error::AppResult;
use crate::sync::state::{FailedItem, FreeUpCheckResult, SyncGlobalState};
use crate::data::repository;
use crate::drive::files_api::FilesApi;
use crate::drive::download_api::DownloadApi;
use crate::drive::upload_api::UploadApi;
use crate::drive::models::DriveFile;
use crate::mount::manager::{LocalFileEntry, MountManager};
use crate::mount::local_watcher::LocalWatcher;
use crate::sync::planner::{DbSnapshotEntry, SyncPlanner, SyncSnapshot};
use crate::sync::executor::SyncExecutor;
use crate::sync::conflict::ConflictResolver;
use crate::sync::cloud_tree;

/// 增量同步安全网：连续走 N 次增量后强制一次全量 BFS，纠正改名/移动/新建文件的累积偏差。
/// 增量 merge 无法处理"已知 id 但 rel_path 变了"（改名/移动）和"全新文件"，需定期全量收敛。
/// 配合自动刷新间隔（默认 60s）：300 次 × 60s = 5 小时强制一次全量纠偏。
const INCREMENTAL_FORCED_FULL_THRESHOLD: u32 = 300;

pub struct SyncEngine {
    files_api: Arc<FilesApi>,
    changes_api: Arc<crate::drive::changes_api::ChangesApi>,
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
    syncing: Mutex<bool>,
    /// 手动同步（trigger_manual_sync）互斥锁：防重复点击触发并发 BFS。
    /// 与 syncing 区分：syncing 守护 run_sync_cycle；manual_syncing 守护整段手动同步。
    manual_syncing: Mutex<bool>,
    /// 目录递归同步（sync_folder_recursive）互斥锁。
    /// 独立于 syncing：folder sync 不被启动/常规 sync cycle 的 syncing 锁阻塞
    /// （启动 scan 可能耗时几十秒，不该挡住用户点目录同步）；run_sync_cycle 会检查本锁跳过，
    /// 避免 watcher cycle 与 folder sync 并发竞争本地文件/DB。
    folder_syncing: Mutex<bool>,
    cloud_tree: Mutex<HashMap<String, DriveFile>>,
    path_to_id: Mutex<HashMap<String, String>>,
    root_folder_id: Mutex<Option<String>>,
    recently_deleted_paths: Mutex<HashSet<String>>,
    state: Mutex<SyncGlobalState>,
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
    /// 连续增量刷新计数。达 INCREMENTAL_FORCED_FULL_THRESHOLD 后强制一次全量 BFS，
    /// 纠正增量无法处理的改名/移动/新建文件累积偏差。全量后归零。
    incremental_since_full: AtomicU32,
}

impl SyncEngine {
    pub fn new(
        files_api: Arc<FilesApi>,
        changes_api: Arc<crate::drive::changes_api::ChangesApi>,
        download_api: Arc<DownloadApi>,
        upload_api: Arc<UploadApi>,
        db: Arc<Mutex<Connection>>,
        skip_patterns: Vec<String>,
        debounce_secs: u32,
        poll_interval_secs: u32,
    ) -> Self {
        let (state_tx, _) = broadcast::channel(256);
        Self {
            files_api, changes_api, download_api, upload_api, mount: None, db,
            planner: SyncPlanner,
            conflict: Arc::new(Mutex::new(ConflictResolver::new())),
            executor: None,
            syncing: Mutex::new(false),
            manual_syncing: Mutex::new(false),
            folder_syncing: Mutex::new(false),
            cloud_tree: Mutex::new(HashMap::new()),
            path_to_id: Mutex::new(HashMap::new()),
            root_folder_id: Mutex::new(None),
            recently_deleted_paths: Mutex::new(HashSet::new()),
            state: Mutex::new(SyncGlobalState::default()),
            running: Mutex::new(false),
            mount_dir: Mutex::new(None),
            skip_patterns, debounce_secs, poll_interval_secs, state_tx,
            is_first_time: Mutex::new(true),
            watcher: Mutex::new(None),
            shutdown: Mutex::new(false),
            incremental_since_full: AtomicU32::new(0),
        }
    }

    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        *self.mount_dir.lock() = Some(mount.mount_dir().to_string_lossy().to_string());
        self.mount = Some(mount);
    }

    pub fn set_executor(&mut self, executor: SyncExecutor) { self.executor = Some(executor); }
    pub fn state_receiver(&self) -> broadcast::Receiver<SyncGlobalState> { self.state_tx.subscribe() }
    pub fn current_state(&self) -> SyncGlobalState { self.state.lock().clone() }
    pub fn is_running(&self) -> bool { *self.running.lock() }

    /// 尝试获取 folder_syncing 锁（供 sync_folder_recursive 防并发用，独立于 syncing）。
    /// 已有目录同步进行中 → false；否则置 true 并返回 true。调用方负责在 finally 调 end_folder_sync。
    pub fn try_begin_folder_sync(&self) -> bool {
        let mut g = self.folder_syncing.lock();
        if *g {
            return false;
        }
        *g = true;
        true
    }
    /// 释放 folder_syncing 锁（与 try_begin_folder_sync 配对）。
    pub fn end_folder_sync(&self) {
        *self.folder_syncing.lock() = false;
    }

    /// 停止引擎：停 watcher（释放 FSEvents）+ 置 shutdown 标志（detached watcher 任务退出）。
    ///
    /// 必须在引擎被替换（换目录/换账号）或退出前调用。之前只 `drop_runtime()` 清全局指针，
    /// 但 detached watcher 任务持有 `Arc<SyncEngine>` 克隆，引擎永不被 drop → 旧 watcher
    /// 持续监听 FSEvents，向旧（cloud_tree 已过时的）引擎触发 sync cycle → 误判「本地新建」
    /// 疯狂上传。本方法确保旧 watcher 真正停止。
    pub async fn shutdown(&self) {
        self.shutdown_sync();
        // stop 内含 await（清 running/pending），async 路径用
        let watcher_opt = self.watcher.lock().clone();
        if let Some(watcher) = watcher_opt {
            watcher.stop().await;
        }
    }

    /// shutdown 的同步子集：仅置 shutdown 标志 + drop watcher 句柄（同步释放 FSEvents）。
    /// 供不能 await 的同步上下文（drop_runtime / shutdown.rs 线程）调用。
    /// drop RecommendedWatcher 会同步关闭底层 FSEvents stream → 不再有事件回调，
    /// detached watcher 任务下次循环见 shutdown 标志退出。
    pub fn shutdown_sync(&self) {
        *self.shutdown.lock() = true;
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
        self.recently_deleted_paths.lock().insert(rel.to_string());
    }
    /// 获取 cloud_tree 的可变锁（供 commands 层清理用）。
    pub fn cloud_tree_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, DriveFile>> {
        self.cloud_tree.lock()
    }
    /// 获取 path_to_id 的可变锁（供 commands 层清理用）。
    pub fn path_to_id_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, String>> {
        self.path_to_id.lock()
    }

    /// 启动引擎。
    pub async fn start(self: &Arc<Self>) -> AppResult<()> {
        // 启动前检查 shutdown 标志
        if *self.shutdown.lock() {
            tracing::info!("引擎已 shutdown，跳过启动");
            return Ok(());
        }
        *self.running.lock() = true;

        // 广播 is_running=true
        {
            let mut st = self.state.lock().clone();
            st.is_running = true;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();

        // 尝试缓存加载 → 回退全量 BFS
        self.load_or_refresh_cloud_tree(&mount_dir).await?;
        if *self.shutdown.lock() {
            tracing::info!("引擎在 BFS 后被 shutdown，跳过 startup-resume cycle");
            return Ok(());
        }

        // 全量 BFS 后尝试建立 changes 增量基线 cursor（失败静默，不影响启动）
        let _ = self.try_init_changes_cursor(&mount_dir).await;

        // 重置过期状态
        {
            let conn = self.db.lock();
            let _ = repository::reset_stale_statuses(&conn);
        }

        // ★ 恢复/清理所有中断的传输任务（kill 后重启用）
        self.recover_interrupted_transfers().await;

        // 首次全量 sync
        self.run_sync_cycle("startup-resume").await?;

        // BFS 后启动 watcher
        self.start_watcher().await;

        // 启动云端定时刷新任务（poll_interval_secs=0 时内部不启动）
        self.start_cloud_refresh_timer().await;

        // 启动完成，复位运行/索引状态
        {
            let mut st = self.state.lock().clone();
            st.is_running = false;
            // 启动全程（BFS + 首次 cycle）已结束，确保 is_indexing 也复位，
            // 防止 BFS 设的 true 因后续交错未被清，卡住状态条。
            st.is_indexing = false;
            st.sync_phase = None;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        Ok(())
    }

    /// 恢复/清理所有中断的传输任务（kill 后重启时调用）。
    /// - 下载：清理 .tmp → 标记 FAILED（planner 下轮重新创建下载任务）
    /// - 上传有断点（server_id+upload_id+本地文件在）：尝试续传，失败则标记 FAILED
    /// - 上传无断点：标记 FAILED（planner 下轮重新创建上传任务）
    /// - 删除：标记 FAILED
    async fn recover_interrupted_transfers(&self) {
        let all: Vec<repository::TransferTask> = {
            let conn = self.db.lock();
            repository::list_all_transfers(&conn).unwrap_or_default()
        };

        // 只关心 RUNNING / PENDING（kill 时未完成的任务）
        let interrupted: Vec<&repository::TransferTask> = all
            .iter()
            .filter(|t| {
                t.state == repository::transfer_state::RUNNING
                    || t.state == repository::transfer_state::PENDING
            })
            .collect();

        if interrupted.is_empty() {
            return;
        }

        let now_ms = || chrono::Utc::now().timestamp_millis();
        let mut resumed = 0u32;
        let mut failed_dl = 0u32;
        let mut failed_ul = 0u32;
        let mut failed_del = 0u32;

        for task in &interrupted {
            // --- DOWNLOAD（含 DOWNLOAD_UPDATE：执行路径相同，.tmp 清理/失败标记一致）---
            if task.direction == repository::transfer_direction::DOWNLOAD
                || task.direction == repository::transfer_direction::DOWNLOAD_UPDATE
            {
                // 清理孤儿 .tmp 文件（下载写 .tmp 再 rename，kill 后 .tmp 残留）
                if let Some(ref local_path) = task.local_path {
                    let tmp = format!("{}.tmp", local_path);
                    let _ = std::fs::remove_file(&tmp);
                }
                let conn = self.db.lock();
                let _ = conn.execute(
                    "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3 WHERE id=?4",
                    rusqlite::params![
                        repository::transfer_state::FAILED,
                        "进程中断，下载未完成",
                        now_ms(),
                        task.id
                    ],
                );
                drop(conn);
                failed_dl += 1;
                tracing::info!(name = %task.name, "中断下载已标记为失败，.tmp 已清理");
            }
            // --- UPLOAD ---
            else if task.direction == repository::transfer_direction::UPLOAD {
                // 有断点信息且本地文件还在 → 尝试续传
                if task.server_id.is_some()
                    && task.upload_id.is_some()
                    && task.local_path.is_some()
                {
                    let local_path = task.local_path.as_ref().unwrap();
                    let path = std::path::PathBuf::from(local_path);
                    if !path.exists() {
                        let conn = self.db.lock();
                        let _ = conn.execute(
                            "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3 WHERE id=?4",
                            rusqlite::params![
                                repository::transfer_state::FAILED,
                                "本地文件不存在，无法续传",
                                now_ms(),
                                task.id
                            ],
                        );
                        drop(conn);
                        failed_ul += 1;
                        continue;
                    }

                    let Some(ref exec) = self.executor else { break; };
                    let upload_api = exec.upload_api();
                    let session = crate::drive::upload_api::ResumeSession {
                        server_id: task.server_id.clone().unwrap_or_default(),
                        upload_id: task.upload_id.clone().unwrap_or_default(),
                        session_url: String::new(),
                        chunk_size: 0,
                    };

                    let db_clone = self.db.clone();
                    let task_id = task.id;
                    let on_resume: crate::drive::upload_api::ResumeProgressFn =
                        Box::new(move |_sid, _uid, offset| {
                            let conn = db_clone.lock();
                            let _ = conn.execute(
                                "UPDATE transfer_queue SET resume_offset=?1, transferred=?1 WHERE id=?2",
                                rusqlite::params![offset as i64, task_id],
                            );
                        });

                    tracing::info!(name = %task.name, offset = task.resume_offset, "尝试断点续传…");

                    match upload_api
                        .upload_resume(&path, None, Some(&session), None, Some(&on_resume))
                        .await
                    {
                        Ok(f) => {
                            let conn = self.db.lock();
                            let _ = conn.execute(
                                "UPDATE transfer_queue SET state=?1, finished_at=?2, transferred=total_size WHERE id=?3",
                                rusqlite::params![
                                    repository::transfer_state::COMPLETED,
                                    now_ms(),
                                    task_id
                                ],
                            );
                            drop(conn);
                            resumed += 1;
                            tracing::info!(name = %task.name, cloud_id = %f.id, "断点续传完成");
                        }
                        Err(e) => {
                            let conn = self.db.lock();
                            let _ = conn.execute(
                                "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3 WHERE id=?4",
                                rusqlite::params![
                                    repository::transfer_state::FAILED,
                                    format!("续传失败: {e}"),
                                    now_ms(),
                                    task_id
                                ],
                            );
                            drop(conn);
                            failed_ul += 1;
                            tracing::warn!(name = %task.name, error = %e, "断点续传失败");
                        }
                    }
                } else {
                    // 无断点信息 → 标记 FAILED，planner 下轮重建
                    let conn = self.db.lock();
                    let _ = conn.execute(
                        "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3 WHERE id=?4",
                        rusqlite::params![
                            repository::transfer_state::FAILED,
                            "进程中断，上传未完成",
                            now_ms(),
                            task.id
                        ],
                    );
                    drop(conn);
                    failed_ul += 1;
                    tracing::info!(name = %task.name, "中断上传（无断点）已标记为失败");
                }
            }
            // --- DELETE ---
            else if task.direction == repository::transfer_direction::DELETE {
                let conn = self.db.lock();
                let _ = conn.execute(
                    "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3 WHERE id=?4",
                    rusqlite::params![
                        repository::transfer_state::FAILED,
                        "进程中断",
                        now_ms(),
                        task.id
                    ],
                );
                drop(conn);
                failed_del += 1;
            }
        }

        if resumed > 0 || failed_dl > 0 || failed_ul > 0 || failed_del > 0 {
            tracing::info!(
                resumed,
                failed_download = failed_dl,
                failed_upload = failed_ul,
                failed_delete = failed_del,
                "中断传输恢复完成"
            );
        }
    }

    async fn load_or_refresh_cloud_tree(&self, mount_dir: &str) -> AppResult<()> {
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));
        let loaded = cloud_tree::load_persisted_cloud_tree(&abs_dir).map(|cache| {
            *self.cloud_tree.lock() = cache.tree;
            *self.path_to_id.lock() = cache.path_to_id;
            *self.root_folder_id.lock() = cache.root_folder_id;
        });
        if loaded.is_none() {
            *self.syncing.lock() = true;
            // 广播 is_indexing=true（对齐 dart BFS 期间的状态广播）
            {
                let mut st = self.state.lock().clone();
                st.is_indexing = true;
                st.sync_phase = Some("indexing-startup".to_string());
                *self.state.lock() = st.clone();
                let _ = self.state_tx.send(st);
            }
            let (tree, p2i, root) = cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, &abs_dir).await?;
            *self.cloud_tree.lock() = tree;
            *self.path_to_id.lock() = p2i;
            *self.root_folder_id.lock() = root;
            *self.syncing.lock() = false;
            {
                let mut st = self.state.lock().clone();
                st.is_indexing = false;
                st.sync_phase = None;
                *self.state.lock() = st.clone();
                let _ = self.state_tx.send(st);
            }
        }
        // ★ 清理无效墓碑：云端树里已不存在的 DELETED 记录可以真删了
        {
            let conn = self.db.lock();
            let ct = self.cloud_tree.lock();
            // 收集所有 DELETED 但云端已不存在的路径
            let to_purge: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT local_path FROM sync_items WHERE status=?1"
                ).unwrap();
                stmt.query_map(rusqlite::params![repository::sync_status::DELETED], |r| r.get::<_, String>(0))
                    .unwrap()
                    .filter_map(|r| r.ok())
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
        }
        Ok(())
    }

    async fn start_watcher(self: &Arc<Self>) {
        if let Some(ref m) = self.mount {
            let watcher = Arc::new(LocalWatcher::new(m.mount_dir(), self.skip_patterns.clone(), self.debounce_secs));
            if let Err(e) = watcher.start().await {
                tracing::error!("watcher启动失败: {e}");
            } else {
                let mut rx = watcher.subscribe();
                // 保活：存入字段，防止 Arc<LocalWatcher> 块结束即 drop → FSEvents 句柄释放
                *self.watcher.lock() = Some(watcher);
                // 持 Arc<SyncEngine> 共享实时 cloud_tree/syncing（不再用冻结快照克隆）
                let engine = self.clone();
                tokio::spawn(async move {
                    loop {
                        // 收到事件前先检查 shutdown（引擎被替换/退出 → 停止喂事件）
                        if *engine.shutdown.lock() {
                            tracing::info!("watcher 任务检测到 shutdown，退出循环");
                            break;
                        }
                        if rx.recv().await.is_err() {
                            // broadcast 发送端关闭（watcher 被 drop）→ 退出
                            break;
                        }
                        if *engine.shutdown.lock() {
                            tracing::info!("watcher 任务检测到 shutdown，退出循环");
                            break;
                        }
                        if let Err(e) = engine.run_sync_cycle("local-watcher").await {
                            tracing::warn!(error = %e, "watcher 触发的同步周期失败");
                        }
                    }
                });
            }
        }
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
        tracing::info!(interval_secs = engine.poll_interval_secs, "启动云端定时刷新任务");
        tokio::spawn(async move {
            loop {
                if *engine.shutdown.lock() {
                    tracing::info!("云端定时刷新任务检测到 shutdown，退出循环");
                    break;
                }
                tokio::time::sleep(Duration::from_secs(engine.poll_interval_secs as u64)).await;
                if *engine.shutdown.lock() {
                    tracing::info!("云端定时刷新任务检测到 shutdown，退出循环");
                    break;
                }
                engine.run_auto_cloud_refresh().await;
            }
        });
    }

    /// 执行一次同步周期。
    pub async fn run_sync_cycle(&self, triggered_by: &str) -> AppResult<()> {
        if *self.syncing.lock() || *self.folder_syncing.lock() { return Ok(()); }
        if self.is_indexing() && triggered_by != "startup-resume" && triggered_by != "retry-failed" {
            tracing::info!(triggered_by, "索引进行中，跳过同步周期");
            return Ok(());
        }
        *self.syncing.lock() = true;
        // ★ 周期开始 → 立即通知前端"同步中" + 设置 phase（auto-cloud-refresh / startup-resume
        //   由上层调用方已设好 phase，此处不覆盖；其余按 triggered_by 设对应 phase）
        {
            let mut st = self.state.lock().clone();
            st.is_running = true;
            if st.sync_phase.is_none() {
                st.sync_phase = match triggered_by {
                    "local-watcher" => Some("syncing-local".to_string()),
                    "manual-refresh" => Some("syncing-manual".to_string()),
                    "retry-failed" => Some("syncing-retry".to_string()),
                    _ => None, // auto-cloud-refresh / startup-resume 由上层设好
                };
            }
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }
        let result = self.run_sync_cycle_inner(triggered_by).await;
        *self.syncing.lock() = false;
        result
    }

    /// 当前是否处于索引中（cloud_tree BFS 重建）。读取 state 副本判断。
    fn is_indexing(&self) -> bool {
        self.state.lock().is_indexing
    }

    /// 设置当前同步阶段并广播（供前端状态条精确显示）。
    fn set_phase(&self, phase: &str) {
        let mut st = self.state.lock().clone();
        st.sync_phase = Some(phase.to_string());
        *self.state.lock() = st.clone();
        let _ = self.state_tx.send(st);
    }

    async fn run_sync_cycle_inner(&self, triggered_by: &str) -> AppResult<()> {
        let local = self.scan_local().await;
        let cloud = self.cloud_tree.lock().clone();
        let db = self.load_db_snapshot();

        // 诊断日志：统计三方数据
        let local_in_cloud_not_db: Vec<&str> = local.keys()
            .filter(|k| cloud.contains_key(*k) && !db.contains_key(*k))
            .map(|s| s.as_str()).collect();
        let in_cloud_db_not_local: Vec<&str> = cloud.keys()
            .filter(|k| db.contains_key(*k) && !local.contains_key(*k))
            .map(|s| s.as_str()).collect();
        if !local_in_cloud_not_db.is_empty() {
            tracing::debug!(count = local_in_cloud_not_db.len(), paths = ?local_in_cloud_not_db, "本地+云端有但DB无（reconcile 将补）");
        }
        if !in_cloud_db_not_local.is_empty() {
            tracing::info!(count = in_cloud_db_not_local.len(), paths = ?in_cloud_db_not_local, "云端+DB有但本地无（应生成 DeleteFromCloud）");
        }

        // #4 DB 自愈（对齐 dart _reconcileDbRecords）：
        // 本地有内容无 DB → 补 synced；本地占位符无 DB → 补 cloudOnly
        self.reconcile_db_records(&local, &db);

        let snapshot = SyncSnapshot { local, cloud, db, is_startup_resume: triggered_by == "startup-resume" };
        let mut actions = self.planner.plan(&snapshot);
        // §2.8 改名检测：在本地新文件上检查 xattr fileId，匹配 → 改名而非 upload+delete
        self.detect_renames(&mut actions);
        filter_anti_oscillation(&mut actions, &self.recently_deleted_paths.lock());
        fill_parent_file_ids(&mut actions, &self.path_to_id.lock());
        // 为"云端已删目录下有内容需救援"补建目录链（跳过用户主动删除的目录）
        add_rescue_folder_recreations(&mut actions, &snapshot, &self.recently_deleted_paths.lock());

        // #8 空操作短路（对齐 dart：无 action → 清零计数 + contentChanged=false → return）
        if actions.is_empty() {
            let mut st = self.state.lock().clone();
            st.uploading = 0;
            st.downloading = 0;
            st.editing = 0;
            st.content_changed = false;
            st.is_running = false;
            // 同步周期结束即非索引态：复位 is_indexing，防止此前某次 BFS/刷新
            // 的 is_indexing=true 因交错未被清，导致状态条永久卡在「正在读取云端索引」。
            st.is_indexing = false;
            st.sync_phase = None;
            st.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
            tracing::info!(triggered_by, "sync cycle: 无操作，短路返回");
            return Ok(());
        }

        tracing::info!(
            triggered_by,
            actions = actions.len(),
            "sync cycle: 开始执行动作"
        );

        let results = if let Some(ref exec) = self.executor {
            self.execute_actions_ordered(exec, &mut actions).await
        } else {
            Vec::new()
        };

        // 执行后回写：cloud_tree/path_to_id + DB（对齐 dart _updateDbFromResults + syncFolderRecursive）
        self.apply_results(&actions, &results);

        // #7 contentChanged 逻辑（对齐 dart：仅结构性操作成功才 true）
        let content_changed = actions.iter().zip(results.iter()).any(|(a, r)| {
            r.success && matches!(
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
        {
            let mut st = self.state.lock().clone();
            st.content_changed = content_changed;
            *self.state.lock() = st.clone();
        }

        // 广播状态更新
        self.update_and_push_state();

        tracing::info!(triggered_by, actions=actions.len(), content_changed, "sync cycle ok");
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
    ) -> Vec<crate::sync::state::ActionResult> {
        use crate::sync::state::{ActionResult, SyncActionType};
        let n = actions.len();
        let mut results: Vec<Option<ActionResult>> = (0..n).map(|_| None).collect();

        // 本地新建目录（CreateFolder 且无 cloud_file）下标，按深度升序
        let mut folder_idxs: Vec<usize> = (0..n)
            .filter(|&i| {
                actions[i].action_type == SyncActionType::CreateFolder && actions[i].cloud_file.is_none()
            })
            .collect();
        folder_idxs.sort_by_key(|&i| {
            actions[i].relative_path.as_deref().map(|p| p.matches('/').count()).unwrap_or(0)
        });

        // 阶段 1：顺序执行本地新建目录，成功后回填 path_to_id/cloud_tree
        for &i in &folder_idxs {
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
            if res.success {
                if let (Some(rel), Some(cf)) = (actions[i].relative_path.clone(), res.cloud_file.clone()) {
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
            let mut st = self.state.lock().clone();
            st.content_changed = true;
            // 不修改计数——update_and_push_state 在周期结束时更新准确数字
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        // 阶段 2：为其余动作重新填充 parent，再并发执行
        fill_parent_file_ids(actions, &self.path_to_id.lock());
        let other_idxs: Vec<usize> = (0..n).filter(|&i| results[i].is_none()).collect();
        let other_actions: Vec<crate::sync::state::SyncAction> =
            other_idxs.iter().map(|&i| actions[i].clone()).collect();
        let other_results = if other_actions.is_empty() {
            Vec::new()
        } else {
            exec.execute_all(&other_actions).await
        };
        for (k, &i) in other_idxs.iter().enumerate() {
            if let Some(r) = other_results.get(k) {
                results[i] = Some(r.clone());
            }
        }

        results
            .into_iter()
            .map(|r| {
                r.unwrap_or_else(|| ActionResult {
                    success: false,
                    error_message: Some("动作未执行".into()),
                    deferred: false,
                    cloud_file: None,
                })
            })
            .collect()
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
    pub fn apply_results(&self, actions: &[crate::sync::state::SyncAction], results: &[crate::sync::state::ActionResult]) {
        use crate::sync::state::SyncActionType;

        // 1. 防振荡维护 + cloud_tree/path_to_id 回写
        {
            let mut rdp = self.recently_deleted_paths.lock();
            let mut ct = self.cloud_tree.lock();
            let mut p2i = self.path_to_id.lock();
            for (action, result) in actions.iter().zip(results.iter()) {
                let Some(rel) = &action.relative_path else { continue };
                if result.success && action.action_type == SyncActionType::DeleteFromCloud {
                    // 云端已删 → 记入防振荡集，并从 cloud_tree/path_to_id 移除
                    rdp.insert(rel.clone());
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
            rdp.retain(|p| ct.contains_key(p));
        }

        // 2. 更新 DB（对齐 dart _updateDbFromResults：用执行结果回写 fileId/元数据/真实 mtime）
        let conn = self.db.lock();
        for (action, result) in actions.iter().zip(results.iter()) {
            let Some(rel) = &action.relative_path else { continue };

            // 删除/备份动作成功 → 清 DB 记录（按 local_path；file_id 可选，覆盖"双方都删清理"file_id=None 场景）
            // BackupBeforeCloudDelete：原文件改名走，原路径腾空 + 云端已删 → 同样清掉原 DB 记录，
            // 让下轮该路径「全缺席」无动作；副本是全新路径，下轮正常 Upload。
            if result.success && matches!(
                action.action_type,
                SyncActionType::DeleteFromCloud | SyncActionType::DeleteFromLocal | SyncActionType::BackupBeforeCloudDelete
            ) {
                let fid = action.file_id.as_deref().unwrap_or("");
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1 AND (?2='' OR file_id=?2)",
                    rusqlite::params![rel, fid],
                );
                continue;
            }

            let status = if !result.success && result.deferred {
                repository::sync_status::SYNCING // 延迟（稳定性/编辑中）→ syncing
            } else if !result.success {
                repository::sync_status::FAILED
            } else if action.action_type == SyncActionType::CreatePlaceholder {
                repository::sync_status::CLOUD_ONLY // 占位符 → cloudOnly（非 synced）
            } else if action.action_type == SyncActionType::CreateConflictCopy {
                repository::sync_status::CONFLICT
            } else {
                repository::sync_status::SYNCED // upload/download/createFolder → synced
            };

            // 云端元数据：成功时优先用 executor 返回的（新上传/建文件夹的 fileId 由此得到），
            // 否则用 action 携带的（download/placeholder 的 cloud_file）。
            let cloud_file = result.cloud_file.as_ref().or(action.cloud_file.as_ref());
            // fileId：成功 → cloud_file.id ?? action.fileId；失败/推迟 → action.fileId（新增场景为 None）
            let file_id = if result.success {
                cloud_file.map(|f| f.id.clone()).or(action.file_id.clone())
            } else {
                action.file_id.clone()
            };
            // 新增上传失败（fileId=null）→ 写入 pending: 占位项（status=FAILED），
            // 让 retry_failed 能找到它（否则重试按钮失效）。占位 fileId 前缀 pending: 不会与真实 fileId 冲突。
            // 其他失败（无 local_path，如缺少 fileId 的下载）仍跳过 DB 写入。
            let file_id = match file_id {
                Some(fid) => fid,
                None => {
                    if action.action_type == SyncActionType::Upload && action.local_path.is_some() {
                        format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel)
                    } else {
                        tracing::debug!(rel = %rel, status, "跳过 DB 写入（fileId=null 且非上传）");
                        continue;
                    }
                }
            };

            // 读取本地真实 mtime/size（对齐 dart _updateDbFromResults 从本地文件 stat）。
            // 写死 None 会导致 is_local_changed 恒 true（db.local_mtime.is_none()），每轮重传。
            let (local_mtime, local_size) = match &action.local_path {
                Some(p) => std::fs::metadata(p).ok().map(|m| {
                    let mt = m.modified().ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64);
                    (mt, Some(m.len() as i64))
                }).unwrap_or((None, None)),
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
            if result.success && !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                let _ = conn.execute(
                    "DELETE FROM sync_items WHERE local_path=?1 AND file_id LIKE ?2",
                    rusqlite::params![rel, format!("{}%", repository::PENDING_FILE_ID_PREFIX)],
                );
            }

            // upsert（对齐 dart insertOnConflictUpdate）
            let _ = repository::upsert(&conn, &repository::SyncItem {
                file_id,
                local_path: rel.clone(),
                parent_folder_id: action.parent_file_id.clone(),
                name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
                is_folder: matches!(action.action_type, SyncActionType::CreateFolder),
                size: cloud_file.map(|f| f.size).unwrap_or(0),
                local_size,
                sha256: None,
                local_mtime,
                cloud_edited_time: cloud_file.and_then(|f| f.edited_time.map(|t| t.timestamp_millis())),
                last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                status,
                // 成功时清空 error_message（Skip 收敛等场景 result 可能带 reason，但已同步不应残留错误）
                error_message: if result.success { None } else { result.error_message.clone() },
            });
        }
        drop(conn);
    }

    /// #4 DB 自愈（对齐 dart _reconcileDbRecords）：
    /// 本地有内容（非占位符）无 DB → upsert synced；
    /// 本地占位符无 DB → upsert cloudOnly。
    /// 防止孤儿记录导致 planner 误判。
    fn reconcile_db_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        db: &HashMap<String, DbSnapshotEntry>,
    ) {
        let conn = self.db.lock();
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
            let file_id = std::fs::metadata(&entry.absolute_path).ok()
                .and_then(|_| {
                    use crate::mount::manager::XATTR_FILE_ID;
                    xattr::get(&entry.absolute_path, XATTR_FILE_ID).ok().flatten()
                        .and_then(|b| String::from_utf8(b).ok())
                })
                .unwrap_or_default();
            if file_id.is_empty() {
                continue; // 无 fileId 无法 upsert（本地新增文件由 planner Upload 处理）
            }
            let _ = repository::upsert(&conn, &repository::SyncItem {
                file_id,
                local_path: rel.clone(),
                parent_folder_id: None,
                name: entry.relative_path.rsplit('/').next().unwrap_or(&entry.relative_path).to_string(),
                is_folder: entry.is_folder,
                size: 0,
                local_size: if entry.is_placeholder { None } else { Some(entry.size as i64) },
                sha256: None,
                local_mtime: Some(entry.mtime),
                cloud_edited_time: None,
                last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                status,
                error_message: None,
            });
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
            repository::load_all(&db).unwrap_or_default().into_iter()
                .filter(|r| !r.file_id.is_empty())
                .map(|r| (r.file_id.clone(), r))
                .collect();
        drop(db);

        let ct = self.cloud_tree.lock();
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default()
            .replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));
        for action in actions.iter_mut() {
            if action.action_type != crate::sync::state::SyncActionType::Upload || action.file_id.is_some() {
                continue;
            }
            let local_path = match &action.local_path { Some(p) => std::path::PathBuf::from(p), None => continue };
            let xattr_id = std::fs::metadata(&local_path).ok().and_then(|_| {
                xattr::get(&local_path, XATTR_FILE_ID).ok().flatten()
                    .and_then(|b| String::from_utf8(b).ok())
            });
            let Some(fid) = xattr_id else { continue };
            let Some(old_record) = db_by_id.get(&fid) else { continue };
            if Some(&old_record.local_path) == action.relative_path.as_ref() { continue; }
            if !ct.contains_key(&old_record.local_path) { continue; }
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
                action.parent_file_id = cloud_file.parent_folder.as_ref().and_then(|v| v.first().cloned());
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
    fn update_and_push_state(&self) {
        let conn = self.db.lock();
        let total: u64 = conn
            .query_row("SELECT COUNT(*) FROM sync_items", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64;
        let failed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE status=?1",
                params![repository::sync_status::FAILED],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;
        let conflict: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE status=?1",
                params![repository::sync_status::CONFLICT],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;
        let uploading: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transfer_queue WHERE state=?1 AND direction=?2",
                params![repository::transfer_state::RUNNING, repository::transfer_direction::UPLOAD],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;
        let downloading: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transfer_queue WHERE state=?1 AND direction IN (?2, ?3)",
                params![repository::transfer_state::RUNNING, repository::transfer_direction::DOWNLOAD, repository::transfer_direction::DOWNLOAD_UPDATE],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;
        let failed_items: Vec<FailedItem> = match conn.prepare(
            "SELECT local_path, error_message FROM sync_items WHERE status=?1 LIMIT 20",
        ) {
            Ok(mut stmt) => stmt
                .query_map(params![repository::sync_status::FAILED], |row| {
                    Ok(FailedItem {
                        relative_path: row.get::<_, String>(0)?,
                        error_message: row.get::<_, Option<String>>(1)?,
                    })
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        drop(conn);

        let mut st = self.state.lock().clone();
        st.total = total;
        st.completed = total - failed - conflict;
        st.failed = failed;
        st.conflict = conflict;
        st.uploading = uploading;
        st.downloading = downloading;
        st.failed_items = failed_items;
        st.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
        st.is_running = false; // 周期结束，重置运行状态
        st.is_indexing = false; // 周期结束即非索引态（防 BFS 的 true 因交错残留）
        st.sync_phase = None; // 周期结束回到空闲
        *self.state.lock() = st.clone();
        let _ = self.state_tx.send(st);
    }

    /// 实时重算传输队列的进行中计数并推送 SyncGlobalState（不重置 is_running）。
    ///
    /// 供 transfer_update 监听器调用：双端对齐/手动下载/传输进度等不经过 sync cycle 的场景，
    /// 入队/结算 RUNNING 传输后，需刷新状态条的 uploading/downloading 计数。
    /// 与 [`update_and_push_state`] 区别：后者只在周期结束调用且重置 is_running；
    /// 本方法保留 is_running/is_indexing 原值，仅刷新传输计数，避免误清「同步中」。
    pub fn push_live_transfer_state(&self) {
        let (uploading, downloading) = {
            let conn = self.db.lock();
            let uploading: u64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transfer_queue WHERE state=?1 AND direction=?2",
                    params![repository::transfer_state::RUNNING, repository::transfer_direction::UPLOAD],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0) as u64;
            let downloading: u64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transfer_queue WHERE state=?1 AND direction IN (?2, ?3)",
                    params![repository::transfer_state::RUNNING, repository::transfer_direction::DOWNLOAD, repository::transfer_direction::DOWNLOAD_UPDATE],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0) as u64;
            (uploading, downloading)
        };
        let mut st = self.state.lock().clone();
        st.uploading = uploading;
        st.downloading = downloading;
        *self.state.lock() = st.clone();
        let _ = self.state_tx.send(st);
    }

    async fn scan_local(&self) -> HashMap<String, LocalFileEntry> {
        match &self.mount {
            Some(m) => m.scan_local(&self.skip_patterns).await.unwrap_or_default().into_iter()
                .map(|e| (e.relative_path.clone(), e)).collect(),
            None => HashMap::new(),
        }
    }

    fn load_db_snapshot(&self) -> HashMap<String, DbSnapshotEntry> {
        let conn = self.db.lock();
        repository::load_all(&conn).unwrap_or_default().into_iter()
            .map(|r| (r.local_path.clone(), DbSnapshotEntry { file_id: r.file_id, local_mtime: r.local_mtime, local_size: r.local_size, cloud_edited_time: r.cloud_edited_time, status: r.status, is_folder: r.is_folder }))
            .collect()
    }

    /// 安全释放校验。
    pub fn can_safely_free_up(&self, rel_path: &str, file_id: &str) -> FreeUpCheckResult {
        let tree = self.cloud_tree.lock();
        if !tree.is_empty() && !tree.contains_key(rel_path) { return FreeUpCheckResult::NotInCloud; }
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
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
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
        // 防重复点击：manual_syncing 锁覆盖整段手动同步（BFS + cycle），
        // 避免重复点击触发并发 BFS（前端按钮也 disable，此处兜底）。
        {
            let mut guard = self.manual_syncing.lock();
            if *guard {
                tracing::info!("trigger_manual_sync: 已在手动同步中，跳过");
                return Ok(());
            }
            *guard = true;
        }
        let result = self.trigger_manual_sync_impl().await;
        *self.manual_syncing.lock() = false;
        result
    }

    /// 手动同步实现：刷新云端树 + 同步周期（持有 manual_syncing 锁时调用）。
    async fn trigger_manual_sync_impl(&self) -> AppResult<()> {
        // 对齐 dart triggerManualSync：先刷新云端树获取最新状态，再跑同步周期
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));

        // 广播 is_indexing=true（对齐 dart _refreshCloudTree 的 isIndexing 广播）。
        // trigger_manual_sync 直接调 refresh_cloud_tree，需在此补 is_indexing 广播，
        // 否则手动刷新期间前端全程收到 idle 态、状态条显示「同步完成」。
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = true;
            st.sync_phase = Some("indexing-manual".to_string());
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }
        // 无论成功失败都要复位 is_indexing，避免 BFS 出错后状态条卡在索引态
        let refresh_result = cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, &abs_dir).await;
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = false;
            st.sync_phase = None;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }
        let (tree, p2i, root) = refresh_result?;
        *self.cloud_tree.lock() = tree;
        *self.path_to_id.lock() = p2i;
        *self.root_folder_id.lock() = root;
        self.run_sync_cycle("manual-refresh").await?;
        // 强制 contentChanged=true（用户主动刷新就是要看新内容）
        let mut state = self.state.lock().clone();
        state.content_changed = true;
        let _ = self.state_tx.send(state);
        Ok(())
    }

    /// 自动定时刷新云端树（定时轮询专用，静默、容错）。
    /// 对齐 `trigger_manual_sync_impl` 的核心流程（刷新 + 置换 + cycle），差异：
    /// - 复用 `manual_syncing` 互斥锁，与手动刷新互斥，避免并发 BFS；
    /// - 不强制 `content_changed = true`（静默刷新，真实变化由 planner 的
    ///   `folder_content_changed` 事件驱动前端刷新，避免每 15 分钟无谓全量重拉 UI）；
    /// - 失败仅 `warn` 不传播，后台任务不应因单次失败终止循环。
    async fn run_auto_cloud_refresh(self: &Arc<Self>) {
        // 索引中（含手动刷新/启动 BFS/其他自动刷新的 BFS 阶段）→ 跳过本次，等下次定时
        if self.is_indexing() {
            tracing::info!("自动云端刷新：索引进行中，跳过本次");
            return;
        }
        // 与手动刷新互斥：若手动刷新进行中，跳过本次自动刷新
        {
            let mut guard = self.manual_syncing.lock();
            if *guard {
                tracing::info!("自动云端刷新：手动同步进行中，跳过本次");
                return;
            }
            *guard = true;
        }
        let result = self.run_auto_cloud_refresh_impl().await;
        *self.manual_syncing.lock() = false;
        if let Err(e) = result {
            tracing::warn!(error = %e, "自动云端刷新失败（忽略，下次定时重试）");
        }
    }

    /// 自动云端刷新实现：有 cursor 走增量 changes，无/失效走全量 BFS（持有 manual_syncing 锁时调用）。
    async fn run_auto_cloud_refresh_impl(&self) -> AppResult<()> {
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));

        // 广播 is_indexing=true（phase 由 try_incremental_or_full_refresh 内部按增量/全量设）
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = true;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        // 尝试增量（有持久化 cursor）；失败/无 cursor 回退全量 BFS
        let refresh_result = self.try_incremental_or_full_refresh(&abs_dir).await;

        // 无论成败复位 is_indexing + phase
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = false;
            st.sync_phase = None;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        refresh_result?;
        // 增量 merge 完成后，cycle 阶段显示"同步云端变更"（try_incremental 设了 querying，
        // 此处 cycle 前覆盖为 syncing；run_sync_cycle 对 auto-cloud-refresh 不覆盖 phase）
        self.set_phase("syncing-auto-incremental");
        self.run_sync_cycle("auto-cloud-refresh").await?;
        Ok(())
    }

    /// 增量优先：有 cursor → changes API merge；失败/无 cursor → 全量 BFS。
    async fn try_incremental_or_full_refresh(&self, abs_dir: &str) -> AppResult<()> {
        let cursor_path = crate::core::cache_paths::changes_cursor_file(abs_dir)?;
        let saved_cursor = std::fs::read_to_string(&cursor_path)
            .ok()
            .filter(|s| !s.trim().is_empty());

        // 安全网：连续增量达阈值 → 强制全量，纠正改名/移动/新建文件的累积偏差
        let consecutive = self.incremental_since_full.load(Ordering::Relaxed);
        let force_full = consecutive >= INCREMENTAL_FORCED_FULL_THRESHOLD;
        if force_full {
            tracing::info!(consecutive, threshold = INCREMENTAL_FORCED_FULL_THRESHOLD, "连续增量达阈值，强制全量 BFS 纠偏");
        }

        if !force_full {
            if let Some(ref cursor) = saved_cursor {
                // 增量路径：先查询云端变更，再 merge（phase 分两步：querying → syncing）
                self.set_phase("querying-changes");
                match self.changes_api.list_all_changes(Some(cursor)).await {
                    Ok((changes, new_cursor)) => {
                        tracing::info!(count = changes.len(), "增量 changes 拉取成功，merge 进 cloud_tree");
                        self.merge_changes_into_cloud_tree(&changes);
                        // 更新 cursor（new_cursor 为 None 表示已追平，保留旧 cursor 即可）
                        let to_write = new_cursor.as_deref().unwrap_or(cursor);
                        let _ = std::fs::write(&cursor_path, to_write);
                        // 计数 +1（下次达阈值会强制全量）
                        self.incremental_since_full.fetch_add(1, Ordering::Relaxed);
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "增量 changes 失败，回退全量 BFS 并清 cursor");
                        let _ = std::fs::remove_file(&cursor_path);
                    }
                }
            }
        }

        // 全量 BFS 路径（无 cursor / 增量失败 / 强制纠偏）
        self.set_phase("indexing-auto-full");
        let (tree, p2i, root) = cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, abs_dir).await?;
        *self.cloud_tree.lock() = tree;
        *self.path_to_id.lock() = p2i;
        *self.root_folder_id.lock() = root;
        // 全量成功后：清旧 cursor + 计数归零 + 重建增量基线（否则本会话后续一直走全量）
        let _ = std::fs::remove_file(&cursor_path);
        self.incremental_since_full.store(0, Ordering::Relaxed);
        self.try_init_changes_cursor_abs(abs_dir).await;
        Ok(())
    }

    /// 全量 BFS 后尝试取 changes 首页游标建立增量基线。失败静默。
    /// 已有 cursor 则不重复初始化；不支持时退化为「首次自动刷新走全量」，不报错。
    async fn try_init_changes_cursor(&self, mount_dir: &str) {
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));
        self.try_init_changes_cursor_abs(&abs_dir).await;
    }

    /// 同 try_init_changes_cursor，但直接用 abs_dir（供 try_incremental_or_full_refresh 复用）。
    async fn try_init_changes_cursor_abs(&self, abs_dir: &str) {
        let cursor_path = match crate::core::cache_paths::changes_cursor_file(abs_dir) {
            Ok(p) => p,
            Err(_) => return,
        };
        // 已有 cursor 则不重复初始化
        if cursor_path.exists() { return; }
        // 取初始游标：华为 /changes 强制要求 cursor，初始 cursor 必须先调 getStartCursor 获取。
        // 失败（如接口不可用）静默，首次自动刷新会因无 cursor 走全量回退，不报错。
        match self.changes_api.get_start_cursor().await {
            Ok(c) => {
                let _ = std::fs::write(&cursor_path, &c);
                tracing::info!(cursor = %c, "已建立 changes 增量基线 cursor");
            }
            Err(e) => tracing::debug!(error = %e, "取 changes startCursor 失败（忽略，首次自动刷新会全量回退）"),
        }
    }


    /// 把增量 changes merge 进内存 cloud_tree + path_to_id（按 fileId 反查 rel_path 增删改）。
    ///
    /// 已知局限（靠安全网定期全量纠偏，见 INCREMENTAL_FORCED_FULL_THRESHOLD）：
    /// - 新建文件（无已知 rel_path）：跳过，等全量兜底
    /// - 改名/移动（已知 id 但 rel_path 已变）：会按旧 rel_path 更新，与真实路径不一致，
    ///   靠定期强制全量收敛
    fn merge_changes_into_cloud_tree(&self, changes: &[crate::drive::changes_api::Change]) {
        use crate::drive::changes_api::ChangeKind;
        // 先读 path_to_id 建 fileId→rel_path 反查表，读完即释放锁（缩小持锁范围）
        let id_to_path: std::collections::HashMap<String, String> = {
            let p2i = self.path_to_id.lock();
            p2i.iter().map(|(p, id)| (id.clone(), p.clone())).collect()
        };
        let mut tree = self.cloud_tree.lock();
        let mut p2i = self.path_to_id.lock();
        let mut hit = 0u32;
        let mut skip = 0u32;
        for c in changes {
            match c.kind {
                ChangeKind::Removed => {
                    if let Some(rel) = id_to_path.get(&c.file.id) {
                        tree.remove(rel);
                        p2i.remove(rel);
                        hit += 1;
                    } else {
                        skip += 1;
                    }
                }
                ChangeKind::Modified => {
                    // 已知路径：更新 cloud_tree（id 不变，path_to_id 无需改）；未知路径：跳过
                    if let Some(rel) = id_to_path.get(&c.file.id) {
                        tree.insert(rel.clone(), c.file.clone());
                        hit += 1;
                    } else {
                        skip += 1;
                    }
                }
            }
        }
        tracing::debug!(total = changes.len(), hit, skip, "增量 merge 完成（hit=已知路径更新，skip=未知路径跳过）");
    }
    pub async fn retry_failed(&self) -> AppResult<()> {
        // 块作用域确保 MutexGuard 在 await 前释放
        {
            let conn = self.db.lock();
            // 对齐 dart：重置为 synced(0) 而非 cloudOnly(1)，让 diff 重新评估
            let _ = conn.execute("UPDATE sync_items SET status=?1 WHERE status=?2", rusqlite::params![repository::sync_status::SYNCED, repository::sync_status::FAILED]);
        }
        self.run_sync_cycle("retry-failed").await
    }
}

/// 防振荡过滤。
fn filter_anti_oscillation(actions: &mut Vec<crate::sync::state::SyncAction>, rdp: &HashSet<String>) {
    use crate::sync::state::SyncActionType;
    actions.retain(|a| {
        let rel = match &a.relative_path { Some(p) => p, None => return true };
        !rdp.contains(rel) || matches!(a.action_type, SyncActionType::DeleteFromCloud)
    });
}

/// 填充 parent_file_id。
fn fill_parent_file_ids(actions: &mut [crate::sync::state::SyncAction], p2i: &HashMap<String, String>) {
    for a in actions {
        if a.parent_file_id.is_some() || a.relative_path.is_none() { continue; }
        let rel = a.relative_path.as_ref().unwrap();
        if let Some(pos) = rel.rfind('/') {
            if let Some(pid) = p2i.get(&rel[..pos]) { a.parent_file_id = Some(pid.clone()); }
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
    recently_deleted: &std::collections::HashSet<String>,
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
    let existing: std::collections::HashSet<String> =
        actions.iter().filter_map(|a| a.relative_path.clone()).collect();

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
                && !recently_deleted.contains(&ancestor);
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
        let Some(entry) = snapshot.local.get(&rel) else { continue };
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

#[cfg(test)]
mod tests {
    //! apply_results 回归测试：覆盖本地新增/删除后 DB 与 cloud_tree 的回写。
    use super::SyncEngine;
    use crate::auth::service::AuthService;
    use crate::data::repository;
    use crate::drive::client::DriveClient;
    use crate::drive::download_api::DownloadApi;
    use crate::drive::files_api::FilesApi;
    use crate::drive::models::DriveFile;
    use crate::drive::upload_api::UploadApi;
    use crate::sync::state::{ActionResult, SyncAction, SyncActionType};
    use tempfile::tempdir;

    /// 构造测试用引擎：in-memory SQLite（含 sync_items 表）+ 桩 API。
    /// apply_results 不触网，故 API Arc 仅需可构造。
    fn build_engine() -> (SyncEngine, std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sync_items (
                file_id TEXT NOT NULL, local_path TEXT NOT NULL, parent_folder_id TEXT,
                name TEXT NOT NULL, is_folder INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
                local_size INTEGER, sha256 TEXT, local_mtime INTEGER, cloud_edited_time INTEGER,
                last_sync_time INTEGER, status INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                PRIMARY KEY (file_id, local_path)
            );",
        )
        .unwrap();
        let db = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        let auth = std::sync::Arc::new(AuthService::new());
        let client = std::sync::Arc::new(DriveClient::new(auth));
        let files_api = std::sync::Arc::new(FilesApi::new(client.clone()));
        let changes_api = std::sync::Arc::new(crate::drive::changes_api::ChangesApi::new(client.clone()));
        let download_api = std::sync::Arc::new(DownloadApi::new(client.clone()));
        let upload_api = std::sync::Arc::new(UploadApi::new(client));
        let eng = SyncEngine::new(
            files_api,
            changes_api,
            download_api,
            upload_api,
            db.clone(),
            vec![],
            3,
            0,
        );
        (eng, db)
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
        assert!(row.2.is_some(), "local_mtime 必须写入真实值（None 会致每轮重传）");
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
            DriveFile { id: "c-old".into(), ..Default::default() },
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
        assert_eq!(actions[0].parent_file_id.as_deref(), Some("folder-id-1"),
            "NEWDIR/file.txt 的 parent 应指向新建目录 NEWDIR 的 id");
        assert_eq!(actions[1].parent_file_id.as_deref(), Some("folder-id-2"),
            "NEWDIR/sub/deep.txt 的 parent 应指向新建子目录 NEWDIR/sub 的 id");
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
            size: 0, mtime: 1000, is_folder: true, is_placeholder: false,
        };
        let mk_file = |rel: &str, size: u64, mtime: i64| LocalFileEntry {
            absolute_path: PathBuf::from(format!("/mount/{rel}")),
            relative_path: rel.into(),
            size, mtime, is_folder: false, is_placeholder: false,
        };
        let mk_db_folder = |fid: &str| DbSnapshotEntry {
            file_id: fid.into(), local_mtime: None, local_size: None,
            cloud_edited_time: Some(1000), status: 0, is_folder: true,
        };
        let mk_db_file = |fid: &str, mtime: i64, size: i64| DbSnapshotEntry {
            file_id: fid.into(), local_mtime: Some(mtime), local_size: Some(size),
            cloud_edited_time: Some(1000), status: 0, is_folder: false,
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
            local, cloud: std::collections::HashMap::new(), db, is_startup_resume: false,
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

        super::add_rescue_folder_recreations(&mut actions, &snapshot, &std::collections::HashSet::new());

        let has_create = |rel: &str| actions.iter().any(|a|
            a.action_type == SyncActionType::CreateFolder && a.relative_path.as_deref() == Some(rel));
        assert!(has_create("B"), "应为被删祖先目录 B 补 CreateFolder");
        assert!(has_create("B/sub"), "应为被删祖先目录 B/sub 补 CreateFolder");
        assert!(actions.iter().any(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete),
            "原备份动作应保留");
    }

    /// 反例：祖先目录仍在云端（非被删）→ 不应补 CreateFolder。
    #[test]
    fn test_add_rescue_folder_recreations_skips_when_folder_on_cloud() {
        use crate::mount::manager::LocalFileEntry;
        use crate::sync::planner::{DbSnapshotEntry, SyncSnapshot};
        use crate::drive::models::{DriveFile, FileCategory};
        use std::path::PathBuf;

        let mut local = std::collections::HashMap::new();
        local.insert("A".into(), LocalFileEntry {
            absolute_path: PathBuf::from("/mount/A"), relative_path: "A".into(),
            size: 0, mtime: 1000, is_folder: true, is_placeholder: false,
        });
        local.insert("A/f.txt".into(), LocalFileEntry {
            absolute_path: PathBuf::from("/mount/A/f.txt"), relative_path: "A/f.txt".into(),
            size: 200, mtime: 9000, is_folder: false, is_placeholder: false,
        });
        // A 仍在云端
        let mut cloud = std::collections::HashMap::new();
        cloud.insert("A".into(), DriveFile {
            id: "fa".into(), name: "A".into(), category: FileCategory::Folder,
            ..Default::default()
        });
        let mut db = std::collections::HashMap::new();
        db.insert("A".into(), DbSnapshotEntry {
            file_id: "fa".into(), local_mtime: None, local_size: None,
            cloud_edited_time: Some(1000), status: 0, is_folder: true,
        });
        db.insert("A/f.txt".into(), DbSnapshotEntry {
            file_id: "fid".into(), local_mtime: Some(1000), local_size: Some(100),
            cloud_edited_time: Some(1000), status: 0, is_folder: false,
        });
        let snapshot = SyncSnapshot { local, cloud, db, is_startup_resume: false };
        let mut actions = vec![SyncAction {
            action_type: SyncActionType::BackupBeforeCloudDelete,
            relative_path: Some("A/f.txt".into()),
            file_id: Some("fid".into()),
            parent_file_id: None,
            local_path: Some("/mount/A/f.txt".into()),
            cloud_file: None,
            reason: None,
        }];

        super::add_rescue_folder_recreations(&mut actions, &snapshot, &std::collections::HashSet::new());

        assert!(!actions.iter().any(|a|
            a.action_type == SyncActionType::CreateFolder && a.relative_path.as_deref() == Some("A")),
            "A 仍在云端 → 不应补建");
        assert_eq!(actions.len(), 1, "仅保留原备份动作");
    }
}
