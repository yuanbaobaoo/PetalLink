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
    pending: Mutex<Vec<String>>,
    /// 定时器取消句柄
    timer_cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// 变更通知发送端（每次 flushed 发送一批相对路径）
    change_tx: tokio::sync::broadcast::Sender<ChangeSet>,
    /// notify watcher 句柄
    #[allow(dead_code)]
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// 是否正在运行
    running: Mutex<bool>,
}

impl LocalWatcher {
    /// 创建新监视器（未启动）。
    /// `on_change` 回调接收变更的相对路径集合。
    pub fn new(
        mount_dir: &Path,
        skip_patterns: Vec<String>,
        debounce_secs: u32,
    ) -> Self {
        let (change_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            mount_dir: mount_dir.to_path_buf(),
            skip_patterns,
            debounce_secs,
            pending: Mutex::new(Vec::new()),
            timer_cancel: Mutex::new(None),
            change_tx,
            watcher: Mutex::new(None),
            running: Mutex::new(false),
        }
    }

    /// 订阅变更事件流。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ChangeSet> {
        self.change_tx.subscribe()
    }

    /// 启动 watcher（创建 FSEvents 监听）。
    /// **必须在 BFS 完成后才调用**。
    pub async fn start(&self) -> Result<(), notify::Error> {
        if *self.running.lock().await {
            return Ok(());
        }

        let mount = self.mount_dir.clone();
        let skip_patterns = self.skip_patterns.clone();
        let debounce_secs = self.debounce_secs;
        let change_tx = self.change_tx.clone();

        // 共享的待冲刷路径集合（watcher 回调 + flush timer 之间共享）
        let pending: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // warmup 标志：注册后 WARMUP_SECS 内丢弃所有事件（FSEvents 历史回放防护）
        let warming_up: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

        // 定时器取消通道（用于重置计时器）
        let timer_cancel: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
            Arc::new(Mutex::new(None));

        // 创建 notify watcher
        let (tx, mut rx) = mpsc::channel(256);
        let warming_up_clone = warming_up.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // warmup 期间的事件视为 FSEvents 历史回放（含 BFS/首次 cycle 建的
                    // 目录/占位符），直接丢弃。此处是 fsevents 线程，用 try_lock 非阻塞。
                    // 锁竞争（极少）时不丢，保守放行。
                    if let Ok(g) = warming_up_clone.try_lock() {
                        if *g {
                            tracing::debug!(kind = ?event.kind, paths = ?event.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(), "warmup: 丢弃事件");
                            return;
                        }
                    }
                    let _ = tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        )?;

        // 递归监听挂载目录
        watcher.watch(&mount, RecursiveMode::Recursive)?;

        *self.watcher.lock().await = Some(watcher);
        *self.running.lock().await = true;

        let mount_clone = mount.clone();

        // warmup 计时器：WARMUP_SECS 后关闭 warming_up，开始正常监听
        let warming_up_timer = warming_up.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(WARMUP_SECS)).await;
            *warming_up_timer.lock().await = false;
        });

        // 后台任务：消费 notify 事件 → 维护 pending + 重置 timer → 到期后冲刷
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = rx.recv() => {
                        let Some(event) = event else {
                            // channel closed → 停止
                            break;
                        };
                        let paths = extract_relative_paths(&event, &mount_clone, &skip_patterns);
                        if paths.is_empty() {
                            continue;
                        }
                        tracing::debug!(
                            kind = ?event.kind,
                            paths = ?paths,
                            "watcher: 检测到本地文件变更"
                        );
                        // 追加到 pending
                        let mut guard = pending.lock().await;
                        for p in paths {
                            if !guard.contains(&p) {
                                guard.push(p);
                            }
                        }
                        drop(guard);

                        // 重置 debounce 计时器
                        let mut cancel_guard = timer_cancel.lock().await;
                        // 取消旧定时器
                        if let Some(tx) = cancel_guard.take() {
                            let _ = tx.send(());
                        }
                        // 新建定时器
                        let (new_tx, new_rx) = tokio::sync::oneshot::channel();
                        *cancel_guard = Some(new_tx);
                        drop(cancel_guard);

                        let pending_clone = pending.clone();
                        let change_tx_clone = change_tx.clone();
                        tokio::spawn(async move {
                            tokio::select! {
                                _ = new_rx => {
                                    // 定时器被取消（新事件到达），不冲刷
                                }
                                _ = tokio::time::sleep(Duration::from_secs(debounce_secs as u64)) => {
                                    // 定时器到期 → 冲刷
                                    let mut guard = pending_clone.lock().await;
                                    if !guard.is_empty() {
                                        let paths: Vec<String> = guard.drain(..).collect();
                                        drop(guard);
                                        let _ = change_tx_clone.send(paths);
                                    }
                                }
                            }
                        });
                    }
                }
            }
        });

        tracing::info!(dir = %self.mount_dir.display(), debounce = debounce_secs, "本地文件监视器已启动");
        Ok(())
    }

    /// 停止监视：释放 FSEvents 句柄（drop watcher），清空 pending。
    /// drop RecommendedWatcher 会关闭底层 FSEvents stream，之后不再有事件回调。
    /// 这确保引擎被替换/退出后，旧 watcher 不会继续向 detached 任务喂事件。
    pub async fn stop(&self) {
        // 关闭 FSEvents：drop watcher 即停止底层 stream
        if let Some(w) = self.watcher.lock().await.take() {
            drop(w);
        }
        *self.running.lock().await = false;
        self.pending.lock().await.clear();
        if let Some(tx) = self.timer_cancel.lock().await.take() {
            let _ = tx.send(());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_extract_paths_skips_internal() {
        let dir = tempdir().unwrap().keep();
        let event = Event::new(EventKind::Create(notify::event::CreateKind::File))
            .add_path(dir.join("normal.txt"))
            .add_path(dir.join(".hwcloud_cache.json"))
            .add_path(dir.join("temp.tmp"))
            .clone();
        let paths = extract_relative_paths(&event, &dir, &[]);
        // 仅 normal.txt 应被保留
        assert!(paths.contains(&"normal.txt".to_string()));
        assert!(!paths.iter().any(|p| p.contains(".hwcloud")));
        assert!(!paths.iter().any(|p| p.contains(".tmp")));
    }

    #[test]
    fn test_extract_paths_empty_on_access() {
        let dir = tempdir().unwrap().keep();
        let event = Event::new(EventKind::Access(notify::event::AccessKind::Read))
            .add_path(dir.join("file.txt"))
            .clone();
        let paths = extract_relative_paths(&event, &dir, &[]);
        // Access 事件应被忽略（不触发同步）
        assert!(paths.is_empty());
    }
}
