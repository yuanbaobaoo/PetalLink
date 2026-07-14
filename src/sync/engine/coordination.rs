use super::{AppError, AppResult, SyncEngine};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CycleRequest(u32);

impl CycleRequest {
    pub(super) const LOCAL_RESCAN: Self = Self(1 << 0);
    pub(super) const CLOUD_INCREMENTAL: Self = Self(1 << 1);
    pub(super) const CLOUD_FULL: Self = Self(1 << 2);
    pub(super) const ONLINE_RECOVERY: Self = Self(1 << 3);
    pub(super) const STARTUP: Self = Self(1 << 4);
    pub(super) const RETRY: Self = Self(1 << 5);
    /// 仅重新规划一个 RestartRequired 任务，不接受所有失败的同步项进行重试。
    pub(super) const REPLAN: Self = Self(1 << 6);

    pub(super) fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(super) fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for CycleRequest {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// 每个引擎的所有扫描/恢复工作来源都由唯一 owner 协调。请求在等待所有权之前记录，
/// 因而周期执行期间（或其释放窗口）到达的边沿会保持为 sticky request，
/// 并由当前 owner 或下一个等待者消费。
#[derive(Default)]
pub(super) struct CycleCoordinator {
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
    pub(super) fn request(&self, request: CycleRequest) -> u64 {
        let mut state = self.state.lock();
        state.requested = state.requested.wrapping_add(1).max(1);
        state.pending |= request.0;
        state.requested
    }

    pub(super) async fn lock_owner(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.owner.lock().await
    }

    pub(super) fn take_pending_with_sequence(&self) -> (CycleRequest, u64) {
        let mut state = self.state.lock();
        let request = CycleRequest(state.pending);
        state.pending = 0;
        (request, state.requested)
    }

    pub(super) fn restore(&self, request: CycleRequest) {
        self.state.lock().pending |= request.0;
    }

    pub(super) fn complete(&self, through: u64, error: Option<&AppError>) {
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

    pub(super) fn result_if_completed(&self, sequence: u64) -> Option<AppResult<()>> {
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

    pub(super) fn is_idle(&self) -> bool {
        self.state.lock().pending == 0 && self.owner.try_lock().is_ok()
    }

    pub(super) fn has_pending(&self) -> bool {
        self.state.lock().pending != 0
    }

    pub(super) fn has_uncompleted_request(&self) -> bool {
        let state = self.state.lock();
        state.requested > state.completed
    }
}

#[derive(Default)]
struct ActivityState {
    accepting: bool,
    count: usize,
    active_paths: HashMap<String, usize>,
    exclusive_paths: HashSet<String>,
}

pub(super) struct ActivityTracker {
    state: Mutex<ActivityState>,
    idle: tokio::sync::Notify,
}

impl Default for ActivityTracker {
    fn default() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                accepting: true,
                count: 0,
                active_paths: HashMap::new(),
                exclusive_paths: HashSet::new(),
            }),
            idle: tokio::sync::Notify::new(),
        }
    }
}

impl ActivityTracker {
    pub(super) fn begin(self: &Arc<Self>, relative_path: Option<&str>) -> AppResult<ActivityGuard> {
        let mut state = self.state.lock();
        if !state.accepting {
            return Err(AppError::generic("同步引擎已停止，拒绝新传输活动"));
        }
        let relative_path = relative_path.map(str::to_string);
        if let Some(path) = relative_path.as_deref() {
            if state
                .exclusive_paths
                .iter()
                .any(|leased| sync_paths_overlap(leased, path))
            {
                return Err(AppError::generic("该路径正在执行破坏性操作，请稍后重试"));
            }
            *state.active_paths.entry(path.to_string()).or_default() += 1;
        }
        state.count += 1;
        Ok(ActivityGuard {
            tracker: self.clone(),
            kind: ActivityKind::Shared(relative_path),
        })
    }

    pub(super) fn begin_exclusive(
        self: &Arc<Self>,
        relative_path: &str,
    ) -> AppResult<ActivityGuard> {
        let mut state = self.state.lock();
        if !state.accepting {
            return Err(AppError::generic("同步引擎已停止，拒绝破坏性操作"));
        }
        if state
            .active_paths
            .keys()
            .any(|active| sync_paths_overlap(active, relative_path))
            || state
                .exclusive_paths
                .iter()
                .any(|leased| sync_paths_overlap(leased, relative_path))
        {
            return Err(AppError::generic("该路径或其子树存在活动任务，请稍后重试"));
        }
        state.exclusive_paths.insert(relative_path.to_string());
        state.count += 1;
        Ok(ActivityGuard {
            tracker: self.clone(),
            kind: ActivityKind::Exclusive(relative_path.to_string()),
        })
    }

    /// 关闭活动门禁以拒绝新活动；已经登记的活动仍可继续结算并释放 guard。
    pub(super) fn close(&self) {
        self.state.lock().accepting = false;
    }

    /// 等待关闭屏障之前已经登记的全部活动释放。
    pub(super) async fn wait_idle(&self) {
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
    kind: ActivityKind,
}

enum ActivityKind {
    Shared(Option<String>),
    Exclusive(String),
}

fn sync_paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(crate) struct FolderSyncGuard {
    pub(super) engine: Arc<SyncEngine>,
}

impl Drop for FolderSyncGuard {
    fn drop(&mut self) {
        self.engine.end_folder_sync();
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut state = self.tracker.state.lock();
        match &self.kind {
            ActivityKind::Shared(Some(path)) => {
                if let Some(count) = state.active_paths.get_mut(path) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        state.active_paths.remove(path);
                    }
                }
            }
            ActivityKind::Exclusive(path) => {
                state.exclusive_paths.remove(path);
            }
            ActivityKind::Shared(None) => {}
        }
        state.count = state.count.saturating_sub(1);
        if state.count == 0 {
            self.tracker.idle.notify_waiters();
        }
    }
}

pub(super) struct TaskRunnerActivityGate(pub(super) Arc<ActivityTracker>);

impl crate::sync::task_runner::TaskActivityGate for TaskRunnerActivityGate {
    fn begin(&self, relative_path: Option<&str>) -> AppResult<Box<dyn Send>> {
        Ok(Box::new(self.0.begin(relative_path)?))
    }
}

pub(super) async fn network_listener_loop<L, R>(
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
    // 调用方先创建订阅，再读取当前网络 level。此处对 level 做一次收敛，确保 listener
    // 启动前已经发送的 Online 边沿不会成为继续推进所必需的条件。
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

/// 长期存活的 watcher receiver。广播 Lagged 表示路径级细节已经丢失，因此唯一安全的
/// 收敛动作是请求一次合并后的完整重扫；它不同于 Closed，不能终止 listener，
/// 因为后续文件系统批次仍是权威触发信号。
pub(super) async fn watcher_listener_loop<R>(
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
