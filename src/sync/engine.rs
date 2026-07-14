//! 同步引擎主循环 —— 核心编排（阶段 5 骨架，后续阶段逐步接入 mount/executor/cloud_tree 完成闭环）。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart`。

use parking_lot::Mutex;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// 过滤、补全并去重同步动作。
mod action_filters;
/// 读取和提交云端树缓存。
mod cache;
/// 协调同步周期所有权与活动关闭屏障。
mod coordination;
/// 执行单次同步周期的阶段编排。
mod cycle;
/// 管理引擎启动、监听器和关闭流程。
mod lifecycle;
/// 串行发布带版本号的运行时状态。
mod publication;
/// 对齐本地快照、数据库记录与云端事实。
mod reconciliation;
/// 将动作结果结算到缓存与数据库。
mod results;
/// 调度可恢复周期错误的退避重试。
mod retry;

pub(crate) use coordination::{ActivityGuard, FolderSyncGuard};
use coordination::{ActivityTracker, CycleCoordinator};

use crate::drive::download_api::DownloadApi;
use crate::drive::files_api::FilesApi;
use crate::drive::models::DriveFile;
use crate::drive::upload_api::UploadApi;
use crate::error::{AppError, AppResult};
use crate::mount::local_watcher::LocalWatcher;
use crate::mount::manager::MountManager;
use crate::sync::conflict::ConflictResolver;
use crate::sync::executor::SyncExecutor;
use crate::sync::planner::SyncPlanner;
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::StatusAggregator;
use crate::sync::task_runner::TaskRunner;

#[async_trait::async_trait]
/// 提供同步启动游标，便于生产实现与测试替身共用。
trait StartCursorSource: Send + Sync {
    /// 请求当前可用的云端变更起始游标。
    async fn get_start_cursor(&self) -> AppResult<String>;
}

#[async_trait::async_trait]
impl StartCursorSource for crate::drive::changes_api::ChangesApi {
    /// 通过 Changes API 获取同步起始游标。
    async fn get_start_cursor(&self) -> AppResult<String> {
        crate::drive::changes_api::ChangesApi::get_start_cursor(self).await
    }
}

/// 增量同步安全网：连续走 N 次增量后强制一次全量 BFS，纠正改名/移动/新建文件的累积偏差。
/// 增量 merge 无法处理"已知 id 但 rel_path 变了"（改名/移动）和"全新文件"，需定期全量收敛。
/// 配合自动刷新间隔（默认 60s）：300 次 × 60s = 5 小时强制一次全量纠偏。
const INCREMENTAL_FORCED_FULL_THRESHOLD: u32 = 300;
/// 可恢复周期错误的最大重试延迟秒数。
const RECOVERABLE_CYCLE_RETRY_MAX_SECS: u64 = 32;

/// 按连续失败次数计算有上限的指数退避。
fn recoverable_cycle_retry_delay(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(5);
    Duration::from_secs((1_u64 << exponent).min(RECOVERABLE_CYCLE_RETRY_MAX_SECS))
}

/// 在所有退出路径上复位生命周期门禁。
struct ResetFlag<'a> {
    flag: &'a Mutex<bool>,
}

impl<'a> ResetFlag<'a> {
    /// 绑定需在作用域结束时复位的布尔门禁。
    fn new(flag: &'a Mutex<bool>) -> Self {
        Self { flag }
    }
}

impl Drop for ResetFlag<'_> {
    /// 无论作用域如何退出都释放生命周期门禁。
    fn drop(&mut self) {
        *self.flag.lock() = false;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// 汇总失败记录的对账结果。
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

/// 持有同步依赖、缓存与生命周期状态的核心引擎。
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
    /// 仅当当前云树来自完整、崩溃一致的 checkpoint 时为 `true`。
    /// 失败或局部刷新可保留旧树展示，但会撤销破坏性操作信任。
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
