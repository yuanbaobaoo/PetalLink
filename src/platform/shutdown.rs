//! 关机/真退出 flush —— 对齐 `legacy AppDelegate.flushAndTerminate`。
//!
//! Tauri 2 的 `RunEvent::Exit` 在进程即将退出时触发（系统关机/注销/真退出均走此）。
//! 本模块在独立线程内 drop SyncEngine（释放 watcher FSEvents）+ 3.2s 硬兜底，
//! 防止 flush 卡死挂起关机。relaunch 场景跳过（缓存已清，flush 无意义）。
//!
//! # 索引完整性双保险
//! 退出时额外把云端树缓存标记为不完整（`complete=false`）。这样即使退出发生在
//! BFS 哨兵还没写下的窗口（startup→BFS 之间），下次 startup 也能检测到「未完成」
//! 并强制全量重跑 BFS，绝不拿残缺缓存去触发文件同步。纯本地文件操作，非阻塞。

use std::time::Duration;
use tauri::AppHandle;

/// 关机/真退出 flush（带 3.2s 超时兜底）。
pub fn flush_with_timeout(_handle: &AppHandle) {
    // relaunch 场景：跳过 flush（mark_restarting 已置位）
    if crate::platform::activation::is_restarting() {
        return;
    }
    // 系统关机/真退出：drop 引擎（释放 watcher）+ 等 DB autocommit 落盘
    // 独立线程跑，主线程阻塞 join + 超时兜底（RunEvent::Exit 在主线程）
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        // 索引完整性双保险：若索引可能在进行中，先把缓存标记为不完整
        // （下次 startup 必重跑 BFS，而非拿残缺缓存触发文件同步）
        crate::sync::cloud_tree::mark_cache_incomplete_if_exists();
        // take 全局引擎 → drop → LocalWatcher 释放 FSEvents 句柄
        // （DB 为 rusqlite autocommit，每次 execute 已落盘，无需额外 commit）
        crate::commands::drop_runtime();
        let _ = tx.send(());
    });
    // 3.2s 硬兜底：无论 flush 是否完成都放行退出
    let _ = rx.recv_timeout(Duration::from_millis(3200));
}
