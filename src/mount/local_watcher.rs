//! 本地文件监听 —— FSEvents + 3 段式 debounce。
//!
//! 对齐 `legacy/lib/mount/local_watcher.dart`。
//!
//! 使用 notify crate（macOS 底层 FSEvents），递归监听。
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

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, Mutex};

use crate::mount::skip::should_skip;

/// 被通知的变更路径集合（相对路径）
pub type ChangeSet = Vec<String>;

/// warmup 窗口长度（秒）。仅需覆盖 FSEvents 历史回放（watcher 注册后立即涌入的
/// BFS 目录/占位符创建事件），2s 足够。之前 8s 会误吞用户启动后立即做的删除操作。
const WARMUP_SECS: u64 = 2;

/// 本地文件监视器。
pub struct LocalWatcher {
    /// 挂载目录
    mount_dir: PathBuf,
    /// 跳过模式
    skip_patterns: Vec<String>,
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
    pub fn new(mount_dir: &Path, skip_patterns: Vec<String>, debounce_secs: u32) -> Self {
        let (change_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            mount_dir: mount_dir.to_path_buf(),
            skip_patterns,
            debounce_secs,
            pending: Arc::new(Mutex::new(Vec::new())),
            timer_cancel: Arc::new(Mutex::new(None)),
            change_tx,
            watcher: Mutex::new(None),
            running: Arc::new(Mutex::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            stop_tx: Mutex::new(None),
            worker_handle: Mutex::new(None),
            warmup_handle: Mutex::new(None),
            timer_handle: Arc::new(Mutex::new(None)),
            lifecycle: Mutex::new(()),
        }
    }

    /// 订阅变更事件流。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ChangeSet> {
        self.change_tx.subscribe()
    }

    /// 启动 watcher（创建 FSEvents 监听）。
    /// **必须在 BFS 完成后才调用**。
    pub async fn start(&self) -> Result<(), notify::Error> {
        let _lifecycle = self.lifecycle.lock().await;
        if *self.running.lock().await {
            return Ok(());
        }

        let mount = self.mount_dir.clone();

        // 创建 notify watcher
        let (tx, rx) = mpsc::channel(256);
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
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
        {
            let mut running = self.running.lock().await;
            if *running {
                return;
            }
            *running = true;
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        *self.stop_tx.lock().await = Some(stop_tx);

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
        let skip_patterns = self.skip_patterns.clone();
        let debounce_secs = self.debounce_secs;
        let pending = self.pending.clone();
        let timer_cancel = self.timer_cancel.clone();
        let timer_handle = self.timer_handle.clone();
        let change_tx = self.change_tx.clone();
        let current_generation = self.generation.clone();
        let running = self.running.clone();
        let worker_handle = tokio::spawn(async move {
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
                            continue;
                        }
                        if current_generation.load(Ordering::Acquire) != generation {
                            break;
                        }
                        let paths = extract_relative_paths(&event, &mount, &skip_patterns);
                        if paths.is_empty() {
                            continue;
                        }
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
            if current_generation.load(Ordering::Acquire) == generation {
                *running.lock().await = false;
            }
        });
        *self.worker_handle.lock().await = Some(worker_handle);
    }

    /// 停止监视：释放 FSEvents 句柄（drop watcher），清空 pending。
    /// 释放系统监视器会关闭底层事件流，之后不再接收回调。
    /// 这确保引擎被替换/退出后，旧 watcher 不会继续向 detached 任务喂事件。
    pub async fn stop(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        // 关闭 FSEvents：drop watcher 即停止底层 stream
        if let Some(w) = self.watcher.lock().await.take() {
            drop(w);
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
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

/// 从 notify 事件中提取相对路径（跳过应排除的文件）。
fn extract_relative_paths(
    event: &Event,
    mount_dir: &Path,
    skip_patterns: &[String],
) -> Vec<String> {
    let mut paths = Vec::new();
    for p in &event.paths {
        // 提取相对于挂载目录的路径
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // 跳过应排除的文件
        if should_skip(&name, skip_patterns) {
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
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other => {
            tracing::debug!(kind = ?event.kind, paths = ?paths, "watcher: 接受事件");
            paths
        }
        _ => {
            tracing::debug!(kind = ?event.kind, "watcher: 忽略非变更事件");
            Vec::new()
        }
    }
}
