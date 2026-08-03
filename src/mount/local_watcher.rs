//! 本地文件监听 —— FSEvents / inotify + 3 段式 debounce。
//!
//! 对齐 `legacy/lib/mount/local_watcher.dart`。
//!
//! 使用 notify crate（macOS 底层 FSEvents、Linux 底层 inotify），递归监听。
//! - 3s debounce：时间内持续变化则重置计时器（对齐 dart 3s debounceSec）
//! - 跳过 .hwcloud_ 前缀 / .tmp 后缀文件
//! - **必须在 BFS 完成后才启动**（否则 _cloudTree 为空 → 误删本地文件）
//!
//! # FSEvents 历史回放防护（与 dart DirectoryWatcher 的关键差异）
//! macOS FSEvents 在新 watcher 注册时会**回放**「自进程启动以来」的历史事件——
//! 含本次 BFS / 首次 sync cycle 在本地建的几百个目录/占位符。这些非用户改动一旦
//! debounce 触发 sync cycle，planner 会把它们误判为「本地新建 → 重复上传」。
//! （dart 的 DirectoryWatcher 不回放历史，故 legacy 无此问题。）
//!
//! 防护：注册后设 `warming_up=true`，丢弃整个 warmup 窗口（> 1 个 debounce 周期）
//! 内的事件。窗口到期后转 `false`，开始正常监听用户改动。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, Flag};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, Mutex};

use crate::mount::skip::SkipMatcher;

/// 被通知的变更路径集合（相对路径）
pub type ChangeSet = Vec<String>;

/// warmup 窗口长度（秒）。覆盖 FSEvents 历史回放与扫描/监听切换间隙，
/// 窗口结束后会请求一次全量补偿扫描。
const WARMUP_SECS: u64 = 2;

/// 本地文件监视器。
pub struct LocalWatcher {
    /// 挂载目录
    mount_dir: PathBuf,
    /// 跨 watcher 生命周期共享的预编译跳过规则。
    skip_matcher: Arc<SkipMatcher>,
    /// debounce 定时器（tokio timer handle）
    debounce_secs: u32,
    /// 当前待冲刷的路径集合
    pending: Arc<Mutex<Vec<String>>>,
    /// 定时器取消句柄
    timer_cancel: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    /// 变更通知发送端（每次 flushed 发送一批相对路径）
    change_tx: tokio::sync::broadcast::Sender<ChangeSet>,
    /// notify watcher 句柄
    #[allow(dead_code)]
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// 是否正在运行
    running: Arc<Mutex<bool>>,
    /// 每次 start/stop 都推进 generation；旧 worker/timer 在发布前必须匹配当前代。
    generation: Arc<AtomicU64>,
    /// notify → async 通道发生饱和时置位；worker 在队列恢复前后各请求一次全量重扫。
    queue_overflowed: Arc<AtomicBool>,
    /// 取消当前实际 worker/warmup generation。
    stop_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    worker_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    warmup_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    timer_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    lifecycle: Mutex<()>,
}

impl LocalWatcher {
    /// 创建新监视器（未启动）。
    /// `on_change` 回调接收变更的相对路径集合。
    pub(crate) fn new(
        mount_dir: &Path,
        skip_matcher: Arc<SkipMatcher>,
        debounce_secs: u32,
    ) -> Self {
        let (change_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            mount_dir: mount_dir.to_path_buf(),
            skip_matcher,
            debounce_secs,
            pending: Arc::new(Mutex::new(Vec::new())),
            timer_cancel: Arc::new(Mutex::new(None)),
            change_tx,
            watcher: Mutex::new(None),
            running: Arc::new(Mutex::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            queue_overflowed: Arc::new(AtomicBool::new(false)),
            stop_tx: Mutex::new(None),
            worker_handle: Mutex::new(None),
            warmup_handle: Mutex::new(None),
            timer_handle: Arc::new(Mutex::new(None)),
            lifecycle: Mutex::new(()),
        }
    }

    /// 订阅文件变更 event stream。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ChangeSet> {
        self.change_tx.subscribe()
    }

    /// 启动 watcher（创建平台推荐的递归文件监听）。
    /// **必须在 BFS 完成后才调用**。
    pub async fn start(&self) -> Result<(), notify::Error> {
        let _lifecycle = self.lifecycle.lock().await;
        if *self.running.lock().await {
            return Ok(());
        }

        let mount = self.mount_dir.clone();

        // 创建 notify watcher
        let (tx, rx) = mpsc::channel(256);
        let queue_overflowed = self.queue_overflowed.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                enqueue_notify_result(res, &tx, queue_overflowed.as_ref());
            },
            notify::Config::default(),
        )?;

        // 递归监听挂载目录
        watcher.watch(&mount, RecursiveMode::Recursive)?;

        *self.watcher.lock().await = Some(watcher);
        self.start_event_loop_for_receiver(rx, true).await;

        tracing::info!(dir = %self.mount_dir.display(), debounce = self.debounce_secs, "本地文件监视器已启动");
        Ok(())
    }

    /// 从事件接收端启动消抖工作器；独立入口便于确定性验证代际、预热和取消行为。
    pub(crate) async fn start_event_loop_for_receiver(
        &self,
        mut rx: mpsc::Receiver<Event>,
        warmup: bool,
    ) {
        // running 锁保证同一 watcher 只启动一个事件工作器。
        {
            let mut running = self.running.lock().await;
            if *running {
                return;
            }
            *running = true;
        }
        // generation 隔离 stop/start 前后的异步任务，旧代不能再发布事件。
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        *self.stop_tx.lock().await = Some(stop_tx);

        // 预热期间丢弃历史事件，结束后用空集合请求一次全量补偿扫描。
        let warming_up = Arc::new(AtomicBool::new(warmup));
        if warmup {
            let warming_up = warming_up.clone();
            let change_tx = self.change_tx.clone();
            let current_generation = self.generation.clone();
            let mut warmup_stop = stop_rx.clone();
            let warmup_handle = tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(WARMUP_SECS)) => {
                        if current_generation.load(Ordering::Acquire) == generation {
                            warming_up.store(false, Ordering::Release);
                            // 空变更集表示主动请求全量重扫，用于补偿扫描与监视启动间隙。
                            let _ = change_tx.send(Vec::new());
                        }
                    }
                    changed = warmup_stop.changed() => {
                        let _ = changed;
                    }
                }
            });
            *self.warmup_handle.lock().await = Some(warmup_handle);
        }

        let mount = self.mount_dir.clone();
        let skip_matcher = self.skip_matcher.clone();
        let debounce_secs = self.debounce_secs;
        let pending = self.pending.clone();
        let timer_cancel = self.timer_cancel.clone();
        let timer_handle = self.timer_handle.clone();
        let change_tx = self.change_tx.clone();
        let current_generation = self.generation.clone();
        let running = self.running.clone();
        let queue_overflowed = self.queue_overflowed.clone();
        // 主工作器负责收集路径并为每批变化重置消抖计时器。
        let worker_handle = tokio::spawn(async move {
            let mut overflow_announced = false;
            loop {
                tokio::select! {
                    changed = stop_rx.changed() => {
                        let _ = changed;
                        break;
                    }
                    event = rx.recv() => {
                        let Some(event) = event else { break; };
                        if warming_up.load(Ordering::Acquire) {
                            tracing::debug!(kind = ?event.kind, "watcher warmup: 丢弃历史事件");
                            // 预热结束本就会发布全量重扫；队列已排空时可结束本轮饱和状态。
                            if rx.is_empty() {
                                queue_overflowed.store(false, Ordering::Release);
                            }
                            continue;
                        }
                        if current_generation.load(Ordering::Acquire) != generation {
                            break;
                        }
                        let requires_rescan = event.need_rescan();
                        let overflow_active = queue_overflowed.load(Ordering::Acquire);
                        if requires_rescan || (overflow_active && !overflow_announced) {
                            tracing::warn!(
                                notify_rescan = requires_rescan,
                                queue_overflow = overflow_active,
                                "文件事件细节可能不完整，请求全量补偿扫描"
                            );
                            let _ = change_tx.send(Vec::new());
                            overflow_announced |= overflow_active;
                        }

                        // 队列从饱和恢复时再补一次尾部全量扫描，覆盖首次重扫之后仍被
                        // 丢弃的事件；后续新事件重新按正常 debounce 流程处理。
                        if rx.is_empty() && queue_overflowed.swap(false, Ordering::AcqRel) {
                            if overflow_announced {
                                tracing::warn!("文件事件队列已恢复，请求尾部全量补偿扫描");
                                let _ = change_tx.send(Vec::new());
                            }
                            overflow_announced = false;
                        }
                        if requires_rescan {
                            continue;
                        }
                        let paths = extract_relative_paths(&event, &mount, &skip_matcher);
                        if paths.is_empty() {
                            continue;
                        }
                        // 同一消抖窗口内按相对路径去重。
                        let mut guard = pending.lock().await;
                        if current_generation.load(Ordering::Acquire) != generation {
                            break;
                        }
                        for path in paths {
                            if !guard.contains(&path) {
                                guard.push(path);
                            }
                        }
                        drop(guard);

                        // 新事件取消旧计时器，确保安静窗口后才发布。
                        let mut cancel_guard = timer_cancel.lock().await;
                        if let Some(cancel) = cancel_guard.take() {
                            let _ = cancel.send(());
                        }
                        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                        *cancel_guard = Some(cancel_tx);
                        drop(cancel_guard);

                        let previous = timer_handle.lock().await.take();
                        if let Some(previous) = previous {
                            let _ = previous.await;
                        }

                        let pending = pending.clone();
                        let change_tx = change_tx.clone();
                        let current_generation = current_generation.clone();
                        // 计时器发布前再次核对代际，避免 stop 后迟到事件。
                        let handle = tokio::spawn(async move {
                            tokio::select! {
                                _ = cancel_rx => {}
                                _ = tokio::time::sleep(Duration::from_secs(debounce_secs as u64)) => {
                                    if current_generation.load(Ordering::Acquire) != generation {
                                        return;
                                    }
                                    let mut guard = pending.lock().await;
                                    if !guard.is_empty() {
                                        let paths = guard.drain(..).collect();
                                        drop(guard);
                                        let _ = change_tx.send(paths);
                                    }
                                }
                            }
                        });
                        *timer_handle.lock().await = Some(handle);
                    }
                }
            }
            // 只有当前代工作器可以清除 running 标记。
            if current_generation.load(Ordering::Acquire) == generation {
                *running.lock().await = false;
            }
        });
        *self.worker_handle.lock().await = Some(worker_handle);
    }

    /// 停止监视：释放平台 watcher 句柄并清空 pending。
    /// drop watcher 会关闭底层事件流，之后不再接收回调。
    /// 这确保引擎被替换/退出后，旧 watcher 不会继续向 detached 任务喂事件。
    pub async fn stop(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        // drop watcher 即停止平台底层事件流。
        if let Some(w) = self.watcher.lock().await.take() {
            drop(w);
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.queue_overflowed.store(false, Ordering::Release);
        if let Some(stop) = self.stop_tx.lock().await.take() {
            let _ = stop.send(true);
        }
        let worker_handle = self.worker_handle.lock().await.take();
        if let Some(worker_handle) = worker_handle {
            let _ = worker_handle.await;
        }
        let warmup_handle = self.warmup_handle.lock().await.take();
        if let Some(warmup_handle) = warmup_handle {
            let _ = warmup_handle.await;
        }
        *self.running.lock().await = false;
        if let Some(tx) = self.timer_cancel.lock().await.take() {
            let _ = tx.send(());
        }
        let timer_handle = self.timer_handle.lock().await.take();
        if let Some(timer_handle) = timer_handle {
            let _ = timer_handle.await;
        }
        self.pending.lock().await.clear();
        tracing::info!("本地文件监视器已停止");
    }
}

/// 把 notify 同步回调无阻塞地转交给 async worker。
///
/// `notify` 用 `Flag::Rescan` 表示 inotify/FSEvents 已丢事件；错误也转换为同一
/// 哨兵。若有界通道已满，则只置位饱和状态，由正在排空队列的 worker 发起补偿扫描。
fn enqueue_notify_result(
    result: Result<Event, notify::Error>,
    tx: &mpsc::Sender<Event>,
    queue_overflowed: &AtomicBool,
) {
    let event = match result {
        Ok(event) => event,
        Err(error) => {
            tracing::error!(%error, "文件监视器报告错误，请求全量补偿扫描");
            Event::new(EventKind::Other).set_flag(Flag::Rescan)
        }
    };

    if event.need_rescan() {
        tracing::warn!("文件监视器要求全量重扫");
    }
    // Linux inotify 会为目录遍历产生大量 OPEN/ACCESS/CLOSE_NOWRITE。它们不是本地
    // 内容变更，若先塞进 256 通道再由 worker 丢弃，会制造假 overflow 和无谓全量扫描。
    // Rescan 永远优先保留；Access 中只有 CLOSE_WRITE 能证明一次写入已经完成。
    if !event.need_rescan()
        && matches!(event.kind, EventKind::Access(access) if access != AccessKind::Close(AccessMode::Write))
    {
        tracing::trace!(kind = ?event.kind, "watcher: 入队前丢弃非变更 Access 事件");
        return;
    }
    match tx.try_send(event) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            if !queue_overflowed.swap(true, Ordering::AcqRel) {
                tracing::warn!("文件事件通道已满，合并为全量补偿扫描");
            }
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::debug!("文件监视器工作器已关闭，忽略迟到事件");
        }
    }
}

/// 从 notify 事件中提取相对路径（跳过应排除的文件）。
fn extract_relative_paths(
    event: &Event,
    mount_dir: &Path,
    skip_matcher: &SkipMatcher,
) -> Vec<String> {
    let mut paths = Vec::new();
    for p in &event.paths {
        // 提取相对于挂载目录的路径
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // 跳过应排除的文件
        if skip_matcher.should_skip(&name) {
            tracing::debug!(path = %p.display(), "watcher: 跳过排除文件");
            continue;
        }
        if let Ok(rel) = p.strip_prefix(mount_dir) {
            paths.push(rel.to_string_lossy().to_string());
        } else {
            tracing::debug!(path = %p.display(), mount = %mount_dir.display(), "watcher: 路径不在挂载目录下，跳过");
        }
    }
    // 仅关注文件/目录变更事件（创建/修改/删除/其他）。
    // EventKind::Other 也需包含：Finder 粘贴/复制等操作在 macOS 上可能产生 Other 事件。
    match event.kind {
        EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Access(AccessKind::Close(AccessMode::Write))
        | EventKind::Other => {
            tracing::debug!(kind = ?event.kind, paths = ?paths, "watcher: 接受事件");
            paths
        }
        _ => {
            tracing::debug!(kind = ?event.kind, "watcher: 忽略非变更事件");
            Vec::new()
        }
    }
}

#[cfg(test)]
/// notify 重扫标记与有界通道饱和合同测试。
mod tests {
    use super::{enqueue_notify_result, extract_relative_paths, LocalWatcher};
    use crate::mount::skip::SkipMatcher;
    use notify::event::{AccessKind, AccessMode, Flag};
    use notify::{Event, EventKind};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// 无路径的 Rescan 事件必须发布空集合，不能被路径提取逻辑静默丢弃。
    #[tokio::test]
    async fn rescan_flag_requests_full_scan() {
        let directory = tempfile::tempdir().expect("创建 watcher 测试目录失败");
        let watcher = LocalWatcher::new(directory.path(), Arc::new(SkipMatcher::new(&[])), 1);
        let mut changes = watcher.subscribe();
        let (tx, rx) = mpsc::channel(4);
        watcher.start_event_loop_for_receiver(rx, false).await;

        tx.send(Event::new(EventKind::Other).set_flag(Flag::Rescan))
            .await
            .expect("发送 Rescan 事件失败");
        let batch = tokio::time::timeout(Duration::from_secs(1), changes.recv())
            .await
            .expect("未收到全量重扫请求")
            .expect("重扫广播已关闭");
        assert!(batch.is_empty());

        drop(tx);
        watcher.stop().await;
    }

    /// 通道饱和时回调必须立即返回并置位补偿标记，不能 blocking_send 卡住 notify 线程。
    #[test]
    fn full_channel_marks_overflow_without_blocking() {
        let (tx, _rx) = mpsc::channel(1);
        tx.try_send(Event::new(EventKind::Other))
            .expect("预填 watcher 通道失败");
        let queue_overflowed = AtomicBool::new(false);

        enqueue_notify_result(Ok(Event::new(EventKind::Other)), &tx, &queue_overflowed);

        assert!(queue_overflowed.load(Ordering::Acquire));
    }

    /// 目录遍历产生的 OPEN/READ 洪水必须在 notify 回调侧丢弃，不能占用有界通道，
    /// 更不能把一次纯读取误报为 queue overflow。
    #[test]
    fn access_open_and_read_flood_never_enters_channel_or_marks_overflow() {
        let (tx, mut rx) = mpsc::channel(1);
        let queue_overflowed = AtomicBool::new(false);

        for _ in 0..1024 {
            enqueue_notify_result(
                Ok(Event::new(EventKind::Access(AccessKind::Open(
                    AccessMode::Read,
                )))),
                &tx,
                &queue_overflowed,
            );
            enqueue_notify_result(
                Ok(Event::new(EventKind::Access(AccessKind::Read))),
                &tx,
                &queue_overflowed,
            );
        }

        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(!queue_overflowed.load(Ordering::Acquire));
    }

    /// CLOSE_WRITE 是一次真实写入完成信号，既要进入通道，也要被路径提取接受。
    #[test]
    fn access_close_write_is_enqueued_and_extracted_as_change() {
        let directory = tempfile::tempdir().expect("创建 watcher 测试目录失败");
        let changed = directory.path().join("changed.txt");
        let event =
            Event::new(EventKind::Access(AccessKind::Close(AccessMode::Write))).add_path(changed);
        let (tx, mut rx) = mpsc::channel(1);
        let queue_overflowed = AtomicBool::new(false);

        enqueue_notify_result(Ok(event), &tx, &queue_overflowed);
        let received = rx.try_recv().expect("CLOSE_WRITE 应进入 watcher 通道");
        assert_eq!(
            extract_relative_paths(&received, directory.path(), &SkipMatcher::new(&[]),),
            vec!["changed.txt".to_string()]
        );
        assert!(!queue_overflowed.load(Ordering::Acquire));
    }

    /// 即使底层把重扫哨兵附在 Access 事件上，Rescan 也必须优先于访问过滤。
    #[test]
    fn rescan_flag_on_access_event_is_never_filtered() {
        let (tx, mut rx) = mpsc::channel(1);
        let queue_overflowed = AtomicBool::new(false);
        let event = Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
            .set_flag(Flag::Rescan);

        enqueue_notify_result(Ok(event), &tx, &queue_overflowed);

        assert!(rx
            .try_recv()
            .expect("Rescan Access 事件应进入 watcher 通道")
            .need_rescan());
        assert!(!queue_overflowed.load(Ordering::Acquire));
    }
}
