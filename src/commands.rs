//! Tauri 命令层运行时与领域命令入口。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::auth::service::AuthService;
use crate::core::config_store::ConfigStore;
use crate::data::repository;
use crate::drive::changes_api::ChangesApi;
use crate::drive::client::DriveClient;
use crate::drive::download_api::DownloadApi;
use crate::drive::files_api::FilesApi;
use crate::drive::thumbnail_api::ThumbnailApi;
use crate::drive::upload_api::UploadApi;
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;
use crate::sync::engine::SyncEngine;
use crate::sync::executor::SyncExecutor;
use crate::sync::state::SyncGlobalState;
use crate::sync::status_aggregator::StatusAggregator;

/// 认证相关命令。
mod auth;
/// 配置读写命令。
mod config;
/// 云盘文件操作命令。
mod drive;
/// 目录递归同步命令。
mod folder_sync;
/// 本地空间释放与按需下载命令。
mod free_up;
/// 平台集成与应用维护命令。
mod platform;
/// 同步控制命令。
mod sync_control;
/// 同步状态查询命令。
mod sync_status;
/// 传输队列命令。
mod transfer;

pub use auth::*;
pub use config::*;
pub use drive::*;
pub use folder_sync::*;
pub use free_up::*;
pub use platform::*;
pub use sync_control::*;
pub use sync_status::*;
pub use transfer::*;

// 全局运行时。

/// 全局 AuthService 单例
pub static AUTH_SERVICE: Lazy<Arc<AuthService>> = Lazy::new(|| Arc::new(AuthService::new()));

/// 全局 DriveClient（首次访问时惰性创建）
pub static DRIVE_CLIENT: Lazy<Arc<DriveClient>> =
    Lazy::new(|| Arc::new(DriveClient::new(AUTH_SERVICE.clone())));

/// 全局 FilesApi
pub static FILES_API: Lazy<Arc<FilesApi>> =
    Lazy::new(|| Arc::new(FilesApi::new(DRIVE_CLIENT.clone())));

/// 全局 ChangesApi（增量变更接口）
pub static CHANGES_API: Lazy<Arc<ChangesApi>> =
    Lazy::new(|| Arc::new(ChangesApi::new(DRIVE_CLIENT.clone())));

/// 全局 DownloadApi
pub static DOWNLOAD_API: Lazy<Arc<DownloadApi>> =
    Lazy::new(|| Arc::new(DownloadApi::new(DRIVE_CLIENT.clone())));

/// 全局 UploadApi
pub static UPLOAD_API: Lazy<Arc<UploadApi>> =
    Lazy::new(|| Arc::new(UploadApi::new(DRIVE_CLIENT.clone())));

/// 全局 ThumbnailApi
pub static THUMBNAIL_API: Lazy<Arc<ThumbnailApi>> =
    Lazy::new(|| Arc::new(ThumbnailApi::new(DRIVE_CLIENT.clone())));

/// 全局 DB 连接（Arc 包裹，供 SyncEngine/SyncExecutor/命令层共享同一连接）
pub static DB: Lazy<Arc<Mutex<rusqlite::Connection>>> = Lazy::new(|| {
    let conn = crate::data::open().expect("打开数据库失败");
    Arc::new(Mutex::new(conn))
});

/// 跨同步引擎实例共享的进程级状态版本源。
static STATUS_AGGREGATOR: Lazy<Arc<StatusAggregator>> =
    Lazy::new(|| Arc::new(StatusAggregator::default()));

/// 全局 MountManager（启动时由 setup 注入路径）
static MOUNT_MANAGER: Mutex<Option<Arc<MountManager>>> = Mutex::new(None);

/// 设置 MountManager（lib.rs setup 调用）
pub fn set_mount_manager(m: Arc<MountManager>) {
    *MOUNT_MANAGER.lock() = Some(m);
}

/// 获取 MountManager
pub fn mount() -> AppResult<Arc<MountManager>> {
    MOUNT_MANAGER
        .lock()
        .clone()
        .ok_or_else(|| AppError::generic("同步引擎未启动（尚未配置同步目录）"))
}

/// 全局 SyncEngine（setup 或首次配置时注入；运行期共享同一实例）
static SYNC_ENGINE: Mutex<Option<Arc<SyncEngine>>> = Mutex::new(None);

/// 串行协调同步引擎安装与替换，防止两个生命周期交叠。
struct EngineOwnershipProtocol {
    gate: Mutex<()>,
    replacements: AtomicUsize,
}

impl EngineOwnershipProtocol {
    /// 创建尚无替换操作的生命周期协调器。
    const fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            replacements: AtomicUsize::new(0),
        }
    }

    /// 在无替换进行时执行安装，否则返回“正在替换”错误。
    fn install<R>(&self, install: impl FnOnce() -> AppResult<R>) -> AppResult<R> {
        let _gate = self.gate.lock();
        if self.replacements.load(Ordering::SeqCst) != 0 {
            return Err(AppError::generic("同步引擎正在替换，请稍后重试"));
        }
        install()
    }

    /// 登记一次替换，并返回退出作用域时自动撤销登记的守卫。
    fn begin_replacement(&self) -> EngineReplacementGuard<'_> {
        let _gate = self.gate.lock();
        self.replacements.fetch_add(1, Ordering::SeqCst);
        EngineReplacementGuard { protocol: self }
    }

    /// 撤销一次已登记的替换操作。
    fn finish_replacement(&self) {
        let _gate = self.gate.lock();
        self.replacements.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 保证替换计数在提前返回或取消时也会回落。
struct EngineReplacementGuard<'a> {
    protocol: &'a EngineOwnershipProtocol,
}

impl Drop for EngineReplacementGuard<'_> {
    /// 离开替换作用域时释放生命周期门禁。
    fn drop(&mut self) {
        self.protocol.finish_replacement();
    }
}

/// 进程级同步引擎生命周期协调器。
static ENGINE_OWNERSHIP: EngineOwnershipProtocol = EngineOwnershipProtocol::new();

/// 发布当前同步引擎实例，供命令层共享；调用方须先完成任务状态接收器绑定。
pub fn set_sync_engine(e: Arc<SyncEngine>) {
    *SYNC_ENGINE.lock() = Some(e);
}

/// 获取 SyncEngine（未启动时报错）。
pub fn sync_engine() -> AppResult<Arc<SyncEngine>> {
    SYNC_ENGINE
        .lock()
        .clone()
        .ok_or_else(|| AppError::generic("同步引擎未启动（尚未配置同步目录）"))
}

/// 获取 SyncEngine（可能为 None，托盘等非关键路径用）。
pub fn try_sync_engine() -> Option<Arc<SyncEngine>> {
    SYNC_ENGINE.lock().clone()
}

/// 清理已配置但失去登录态的孤儿同步状态。
pub fn cleanup_orphan_state() {
    // 守卫：必须确实有旧同步配置才清
    let config = match ConfigStore::load() {
        Ok(c) => c,
        Err(_) => return,
    };
    if !config.mount_configured {
        return; // 无旧同步配置，无需清理
    }
    // DB 行（sync_items + transfer_queue）
    {
        let conn = DB.lock();
        let _ = repository::delete_all(&conn);
        let _ = repository::delete_all_transfers(&conn);
    }
    // 所有挂载目录的缓存文件
    crate::core::cache_paths::clear_all_cache_files();
    // config 挂载字段重置（其余设置保留）
    let reset = config.with(
        None,
        None,
        Some(String::new()),
        Some(false),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let _ = ConfigStore::save(&reset);
    tracing::info!("孤儿状态已清理（DB/cloudtree/syncstate/mount_configured）");
}

/// 同步释放全局运行时。
pub fn drop_runtime() {
    if let Some(eng) = SYNC_ENGINE.lock().take() {
        eng.shutdown_sync();
    }
    *MOUNT_MANAGER.lock() = None;
}

/// 异步释放运行时，并等待旧同步周期与已提交任务完成结算。
pub async fn drop_runtime_async() {
    let _replacement = ENGINE_OWNERSHIP.begin_replacement();
    let engine = {
        let _gate = ENGINE_OWNERSHIP.gate.lock();
        let engine = SYNC_ENGINE.lock().take();
        *MOUNT_MANAGER.lock() = None;
        engine
    };
    if let Some(engine) = engine {
        engine.shutdown().await;
    }
}

/// 重启 App：fork 独立 shell 子进程（sleep 后 open -n）+ 当前进程退出。
/// 对齐 legacy AppDelegate.relaunchApp（fork /bin/sh，子进程独立于本进程存活）。
pub fn relaunch(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    crate::platform::activation::mark_restarting();
    // .app bundle 路径（binary 上溯两级）
    let bundle = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_exe().unwrap_or_default());
    let path = bundle.to_string_lossy().to_string();
    let _ = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("sleep 0.5; open -n '{}'", path))
        .spawn();
    app.exit(0);
}

/// 若已配置同步目录且引擎未启动，则构造并启动 SyncEngine + 状态桥接。
/// setup 与 config_save（首次配置）共用。
pub fn ensure_engine_started(app: &AppHandle) -> AppResult<()> {
    ENGINE_OWNERSHIP.install(|| ensure_engine_started_owned(app))
}

/// 在生命周期门禁内按持久化配置安装并异步启动同步引擎。
fn ensure_engine_started_owned(app: &AppHandle) -> AppResult<()> {
    if try_sync_engine().is_some() {
        return Ok(());
    }
    let config = ConfigStore::load()?;
    if !config.mount_configured {
        return Ok(()); // 未配置目录，不启动
    }

    let mount = mount()?;
    let recovered_free_up = {
        let conn = DB.lock();
        mount.recover_interrupted_free_up(&conn)?
    };
    if recovered_free_up > 0 {
        tracing::warn!(count = recovered_free_up, "启动前已收敛中断的释放空间操作");
    }

    // 构造 SyncExecutor
    let mut executor = SyncExecutor::new(
        config.concurrency,
        FILES_API.clone(),
        DOWNLOAD_API.clone(),
        UPLOAD_API.clone(),
    );
    executor.set_mount(mount.clone());
    executor.set_conflict(Arc::new(std::sync::Mutex::new(
        crate::sync::conflict::ConflictResolver::new(),
    )));
    executor.set_stability(Arc::new(tokio::sync::Mutex::new(
        crate::sync::stability::StabilityChecker::new(),
    )));
    executor.set_db(DB.clone());
    // 传输进度实时推送通道：每次传输结算时触发前端刷新
    let (transfer_update_tx, mut transfer_update_rx) = tokio::sync::broadcast::channel::<()>(64);
    executor.set_transfer_update_tx(transfer_update_tx.clone());
    // 注入 AppHandle：上传失败时直接 emit upload_failed → 前端弹 toast
    executor.set_app_handle(app.clone());
    let task_runner = executor.initialize_task_runner()?;
    // 传输进度实时推送监听器
    {
        let app_for_transfer = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match transfer_update_rx.recv().await {
                    Ok(()) => {
                        use tauri::Emitter;
                        let _ = app_for_transfer.emit("transfer_update", ());
                        crate::platform::tray::refresh_menu(&app_for_transfer);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });
    }

    // 构造 SyncEngine（set_mount/set_executor 必须在 Arc::new 之前）
    let mut engine = SyncEngine::new(
        FILES_API.clone(),
        CHANGES_API.clone(),
        DOWNLOAD_API.clone(),
        UPLOAD_API.clone(),
        DB.clone(),
        STATUS_AGGREGATOR.clone(),
        config.skip_patterns.clone(),
        config.debounce_sec,
        config.poll_interval_sec,
    );
    engine.set_mount(mount.clone());
    engine.set_executor(executor);

    let engine = Arc::new(engine);
    engine.bind_task_runner_state_sink(&task_runner);
    set_sync_engine(engine.clone());

    // 异步启动引擎（start 内含 BFS + watcher，不阻塞调用方）
    let app_handle = app.clone();
    let eng = engine.clone();
    tracing::info!("SyncEngine 异步启动中...");
    tauri::async_runtime::spawn(async move {
        // 先订阅状态广播，保证启动期状态也能转发到前端。
        let mut rx = eng.state_receiver();
        let bridge_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!("状态桥接任务已启动，开始监听 sync_state");
            loop {
                match rx.recv().await {
                    Ok(state) => {
                        emit_sync_state(&bridge_app, &state);
                        // 同步状态是完整权威快照，传输列表由独立事件刷新。
                        crate::platform::tray::refresh_menu(&bridge_app);
                        if state.content_changed {
                            emit_folder_content_changed(&bridge_app);
                        }
                    }
                    // 广播滞后时告警并继续监听。
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "状态桥接: broadcast 滞后，跳过并继续");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("状态桥接: 通道关闭，退出");
                        break;
                    }
                }
            }
        });
        // 启动引擎（BFS + 首次 sync cycle + watcher）；桥接已订阅，启动期广播会被转发
        if let Err(e) = eng.start().await {
            tracing::error!(error = %e, "SyncEngine 启动失败");
        } else {
            tracing::info!("SyncEngine 启动完成 ✓");
        }
        // 桥接任务独立运行，直到引擎释放。
    });
    Ok(())
}

/// 推送同步状态到前端（Tauri event）
pub fn emit_sync_state(app: &AppHandle, state: &SyncGlobalState) {
    let _ = app.emit("sync_state", state);
}

/// 推送目录内容变更通知（触发前端 folderChildren 刷新）
pub fn emit_folder_content_changed(app: &AppHandle) {
    let _ = app.emit("folder_content_changed", ());
}
