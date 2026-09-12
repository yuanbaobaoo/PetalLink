//! 关机/真退出 flush —— 对齐 `legacy AppDelegate.flushAndTerminate`。
//!
//! Tauri 2 的 `RunEvent::Exit` 在进程即将退出时触发（系统关机/注销/真退出均走此）。
//! 本模块在独立线程内 drop SyncEngine（释放平台 watcher）+ 3.2s 硬兜底，
//! 防止 flush 卡死挂起关机。relaunch 场景跳过（缓存已清，flush 无意义）。
//!
//! # 索引完整性双保险
//! 仅当同步引擎当前正在索引（is_indexing=true）时才把云端树缓存标记为不完整。
//! 正常退出（索引已完成）时保留完整缓存，下次启动秒级加载，不再重复全量 BFS。
//! 这确保：① BFS 中途强退 → 下次重跑（哨兵 + 本双保险）；② 正常退出 → 缓存复用。

use std::time::Duration;
use tauri::AppHandle;

/// Linux SIGTERM/SIGINT 不一定经过 Tauri 的窗口退出请求。把它们转换成 `app.exit`，
/// 让统一的 `RunEvent::Exit` flush 有机会先卸载 FUSE，再终止进程。
#[cfg(target_os = "linux")]
pub fn install_signal_handler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(%error, "注册 SIGTERM 退出处理失败");
                return;
            }
        };
        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(%error, "注册 SIGINT 退出处理失败");
                return;
            }
        };
        let signal_name = tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
        };
        tracing::info!(signal = signal_name, "收到终止信号，开始卸载按需云盘并退出");
        crate::platform::activation::mark_real_quit();
        app.exit(0);
    });
}

#[cfg(not(target_os = "linux"))]
pub fn install_signal_handler(_app: AppHandle) {}

/// 关机/真退出 flush（带 3.2s 超时兜底）。
pub fn flush_with_timeout(_handle: &AppHandle) {
    // relaunch 场景：跳过 flush（mark_restarting 已置位）
    if crate::platform::activation::is_restarting() {
        return;
    }
    #[cfg(target_os = "linux")]
    let fuse_mountpoint = crate::core::config_store::ConfigStore::load()
        .ok()
        .filter(|config| config.mount_configured && config.virtual_drive_enabled)
        .map(|config| config.expanded_virtual_mount_dir());

    // 系统关机/真退出：drop 引擎（释放 watcher）+ 等 DB autocommit 落盘
    // 独立线程跑，主线程阻塞 join + 超时兜底（RunEvent::Exit 在主线程）
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        // 索引完整性双保险：仅当 BFS 正在跑时才把缓存标记为不完整。
        // 正常退出（索引已完成）保留完整缓存，下次启动无需重跑 BFS。
        if let Some(engine) = crate::commands::try_sync_engine() {
            if engine.current_state().is_indexing {
                crate::sync::cloud_tree::mark_cache_incomplete_if_exists();
            }
        }
        // take 全局引擎 → drop → LocalWatcher 释放平台监听句柄
        // （DB 为 rusqlite autocommit，每次 execute 已落盘，无需额外 commit）
        crate::commands::drop_runtime();
        let _ = tx.send(());
    });
    // 3.2s 硬兜底：无论 flush 是否完成都放行退出
    if rx.recv_timeout(Duration::from_millis(3200)).is_err() {
        tracing::error!("退出清理超过 3.2 秒，执行 PetalLink FUSE 定向 detach 兜底");
        #[cfg(target_os = "linux")]
        if let Some(mountpoint) = fuse_mountpoint {
            match crate::core::config_store::detach_owned_petallink_mount(&mountpoint) {
                Ok(true) => tracing::warn!(
                    mountpoint = %mountpoint.display(),
                    "退出超时后已 detach PetalLink FUSE"
                ),
                Ok(false) => {}
                Err(error) => tracing::error!(
                    mountpoint = %mountpoint.display(),
                    %error,
                    "退出超时后的 PetalLink FUSE detach 失败"
                ),
            }
        }
    }
}
