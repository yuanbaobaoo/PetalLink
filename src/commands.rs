//! Tauri 命令层 —— 前端通过 invoke 调用的全部后端命令。
//!
//! 对齐 Flutter 版全部 UI→后端调用，1:1 复刻。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::auth::models::{TokenPair, UserInfo};
use crate::auth::service::AuthService;
use crate::auth::token_store::TokenStore;
use crate::auth::user_info_api::UserInfoApi;
use crate::core::config::AppConfig;
use crate::core::config_store::ConfigStore;
use crate::data::repository;
use crate::drive::about_api::AboutApi;
use crate::drive::changes_api::ChangesApi;
use crate::drive::client::DriveClient;
use crate::drive::download_api::DownloadApi;
use crate::drive::files_api::FilesApi;
use crate::drive::models::{DriveAbout, DriveFile, FileListResult};
use crate::drive::thumbnail_api::ThumbnailApi;
use crate::drive::upload_api::UploadApi;
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;
use crate::sync::engine::SyncEngine;
use crate::sync::executor::SyncExecutor;
use crate::sync::state::{FreeUpCheckResult, SyncGlobalState};
use crate::sync::status_aggregator::{RuntimeStatus, StatusAggregator};

// ===== 全局单例 =====

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

/// Process-wide status revision source, shared across replacement SyncEngine instances.
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

struct EngineOwnershipProtocol {
    gate: Mutex<()>,
    replacements: AtomicUsize,
}

impl EngineOwnershipProtocol {
    const fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            replacements: AtomicUsize::new(0),
        }
    }

    fn install<R>(&self, install: impl FnOnce() -> AppResult<R>) -> AppResult<R> {
        let _gate = self.gate.lock();
        if self.replacements.load(Ordering::SeqCst) != 0 {
            return Err(AppError::generic("同步引擎正在替换，请稍后重试"));
        }
        install()
    }

    fn begin_replacement(&self) -> EngineReplacementGuard<'_> {
        let _gate = self.gate.lock();
        self.replacements.fetch_add(1, Ordering::SeqCst);
        EngineReplacementGuard { protocol: self }
    }

    fn finish_replacement(&self) {
        let _gate = self.gate.lock();
        self.replacements.fetch_sub(1, Ordering::SeqCst);
    }
}

struct EngineReplacementGuard<'a> {
    protocol: &'a EngineOwnershipProtocol,
}

impl Drop for EngineReplacementGuard<'_> {
    fn drop(&mut self) {
        self.protocol.finish_replacement();
    }
}

static ENGINE_OWNERSHIP: EngineOwnershipProtocol = EngineOwnershipProtocol::new();

/// 设置 SyncEngine。
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

/// 清空全局 SyncEngine / MountManager（清缓存 / 换目录重启前调用）。
///
/// **先停止旧引擎的 watcher**：detached watcher 任务持有 `Arc<SyncEngine>` 克隆，
/// 若不显式 shutdown，旧 watcher 会持续监听 FSEvents 并向过时的 cloud_tree 触发
/// sync cycle → 误判「本地新建」疯狂上传。shutdown_sync 释放 FSEvents + 置 shutdown
/// 标志，detached 任务下次循环退出。
/// 启动兜底：token 丢失但旧同步数据仍在时，清空所有同步状态。
///
/// 仅当 config mount_configured=true（确实有旧同步配置）时才清理；
/// 避免 token 瞬态不可用时误清正常运行的同步环境。
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
    );
    let _ = ConfigStore::save(&reset);
    tracing::info!("孤儿状态已清理（DB/cloudtree/syncstate/mount_configured）");
}

pub fn drop_runtime() {
    if let Some(eng) = SYNC_ENGINE.lock().take() {
        eng.shutdown_sync();
    }
    *MOUNT_MANAGER.lock() = None;
}

/// Replacement/cleanup path: publish cancellation, stop the actual watcher generation, and wait
/// for the old cycle owner (including an already-submitted TaskRunner settlement) before callers
/// clear shared DB/cache state or permit a replacement to start.
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

/// 清除账号相关同步缓存：DB 行 + 所有 syncstate_*/cloudtree_*。
///
/// 用于 fresh 登录（auth_login）成功后——token 可能是被删除后重新登录、或换账号，
/// 上一账号的 DB/cloudtree/syncstate 若残留，会让新会话的 planner 基于旧 fileId/旧云端树
/// 误同步。清掉后原地重启引擎，以新 token + 干净缓存重新 BFS。
///
/// 注意：本函数只清文件/DB 缓存，**不清 config 的挂载目录字段**。config 清理由
/// [`reset_account_config`] 配套完成（mount_dir 清空 + mount_configured=false）。
/// 不清 token（刚由 authorize 保存）。
/// 不删 DB 文件——auth_login 原地重启引擎复用同一 DB 连接，删文件会让新引擎写到
/// 已 unlink 的旧 fd（数据丢失）。清行即可让新引擎从空表起步。
fn clear_account_caches() {
    {
        let conn = DB.lock();
        let _ = repository::delete_all(&conn);
        let _ = repository::delete_all_transfers(&conn);
    }
    // 所有挂载目录的缓存文件（cloudtree/syncstate，含历史遗留临时目录 /var/folders/.../T/.tmpXXX）
    crate::core::cache_paths::clear_all_cache_files();
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

fn ensure_engine_started_owned(app: &AppHandle) -> AppResult<()> {
    if try_sync_engine().is_some() {
        return Ok(());
    }
    let config = ConfigStore::load()?;
    if !config.mount_configured {
        return Ok(()); // 未配置目录，不启动
    }

    let mount = mount()?;

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
        // 先订阅状态广播，再启动引擎——桥接任务与 start() 并发执行，
        // 保证启动期（BFS + 首次 sync cycle）的 is_indexing 等广播能转发到前端。
        // 之前在 start() 之后才订阅，启动期广播全部丢失 → 配置目录后状态条不显示
        // 「正在读取云端索引…」、刷新按钮不转圈、可重复点击触发并发 BFS。
        let mut rx = eng.state_receiver();
        let bridge_app = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            tracing::info!("状态桥接任务已启动，开始监听 sync_state");
            loop {
                match rx.recv().await {
                    Ok(state) => {
                        emit_sync_state(&bridge_app, &state);
                        // sync_state is already the complete authoritative snapshot. Transfer
                        // list refresh is emitted by TaskRunner's separate transfer_update sink.
                        crate::platform::tray::refresh_menu(&bridge_app);
                        if state.content_changed {
                            emit_folder_content_changed(&bridge_app);
                        }
                    }
                    // broadcast 滞后：旧 while let Ok 会在此静默退出，导致桥接死亡。
                    // 改为告警 + 继续，保证桥接长存。
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
        // 桥接任务 detached 持续运行，直到 engine 被释放
    });
    Ok(())
}

// ===== 通用响应结构 =====

#[derive(Debug, Clone, Serialize)]
pub struct AuthState {
    pub logged_in: bool,
    pub secret_configured: bool,
    pub callback_port: u16,
}

// ========================================================================
// 一、Auth 命令
// ========================================================================

#[tauri::command]
pub fn auth_check_secret() -> bool {
    crate::constants::client_id_configured() && crate::constants::client_secret_configured()
}

#[tauri::command]
pub async fn auth_restore() -> AppResult<AuthState> {
    let logged_in = AUTH_SERVICE.restore().await?;
    Ok(AuthState {
        logged_in,
        secret_configured: crate::constants::client_id_configured()
            && crate::constants::client_secret_configured(),
        callback_port: crate::constants::DEFAULT_CALLBACK_PORT,
    })
}

#[tauri::command]
pub async fn auth_login(app: AppHandle, port: u16) -> AppResult<TokenPair> {
    let token = AUTH_SERVICE.authorize(port).await?;
    // 重新登录一律视为「从头开始」，彻底清空上一账号的一切同步状态：
    //   ① DB 行（sync_items / transfer_queue）
    //   ② 缓存文件（所有挂载目录的 cloudtree_* / syncstate_*）
    //   ③ config 的挂载目录字段（mount_dir 清空、mount_configured=false）
    // 三者全清后，重新登录回到初始状态，强制用户重新选同步目录。
    // 不 relaunch——dev 模式跑裸二进制，relaunch 的 `open -n <bundle>` 拿到的是目录
    // 路径打不开，会导致进程退出却不重启。不清 token（刚保存）；其余设置（并发/
    // debounce/skip_patterns/排序/OAuth）保留。
    // 覆盖场景：① token 过期重登；② 换账号登录（旧目录/fileId/cloudtree 残留会污染新账号同步）。
    drop_runtime_async().await;
    clear_account_caches();
    let _ = reset_account_config();
    tracing::info!("登录成功，已彻底清空上一账号同步缓存与目录配置，等待用户重新配置");
    // mount_configured 已置 false → 不在此启动引擎，等用户在设置页选目录后由 config_save 启动
    // （保留 app 句柄以满足命令签名；此处不使用）
    let _ = &app;
    Ok(token)
}

/// 重置账号相关的 config 字段为初始态：mount_dir 清空、mount_configured=false。
/// 其余设置（并发/debounce/skip_patterns/排序/OAuth）保留。
/// 用于重新登录后强制用户从头配置同步目录。
/// 若已是初始态（mount_dir 空且 mount_configured=false）则跳过，避免重复写盘。
fn reset_account_config() -> AppResult<()> {
    let config = ConfigStore::load()?;
    if config.mount_dir.is_empty() && !config.mount_configured {
        return Ok(()); // 已是初始态，无需重置
    }
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
    );
    ConfigStore::save(&reset)
}

#[tauri::command]
pub async fn auth_cancel_login() -> AppResult<()> {
    AUTH_SERVICE.cancel_authorize().await;
    Ok(())
}

#[tauri::command]
pub async fn auth_logout() -> AppResult<()> {
    // 退出登录 = 彻底清空当前账号的一切同步状态（与 auth_login 重新登录一致）：
    //   ① 停引擎（含 watcher）
    //   ② 清 DB 行 + 所有 cloudtree_*/syncstate_* 文件
    //   ③ 重置 config 挂载字段（mount_dir 清空、mount_configured=false）
    //   ④ 清 token
    // 之前仅清 token 导致退出登录后 token.json 消失但其他缓存残留，
    // 随后换账号登录时仍带着旧目录/fileId/cloudtree 污染新会话。
    drop_runtime_async().await;
    clear_account_caches();
    let _ = reset_account_config();
    AUTH_SERVICE.logout().await
}

#[tauri::command]
pub async fn auth_get_user_info() -> AppResult<UserInfo> {
    let api = UserInfoApi::new(AUTH_SERVICE.clone());
    api.get().await
}

#[tauri::command]
pub async fn auth_is_logged_in() -> AppResult<bool> {
    use crate::auth::token_store::global_store;
    Ok(global_store().load()?.is_some())
}

// ========================================================================
// 二、Drive 文件操作命令
// ========================================================================

#[tauri::command]
pub async fn drive_list(
    parent_id: Option<String>,
    cursor: Option<String>,
    page_size: Option<u32>,
) -> AppResult<FileListResult> {
    FILES_API
        .list(
            parent_id.as_deref(),
            cursor.as_deref(),
            page_size.unwrap_or(100),
        )
        .await
}

#[tauri::command]
pub async fn drive_list_all(parent_id: Option<String>) -> AppResult<Vec<DriveFile>> {
    FILES_API.list_all(parent_id.as_deref()).await
}

#[tauri::command]
pub async fn drive_get_file(id: String) -> AppResult<DriveFile> {
    FILES_API.get(&id).await
}

#[tauri::command]
pub async fn drive_create_folder(name: String, parent_id: Option<String>) -> AppResult<DriveFile> {
    FILES_API.create_folder(&name, parent_id.as_deref()).await
}

#[tauri::command]
pub async fn drive_delete_file(app: AppHandle, id: String) -> AppResult<()> {
    // 索引中（云端树 BFS 重建）：删除会与索引并发改云端，且 cloud_tree 不完整
    // 无法正确反映删除后的状态 → 拒绝，等索引完成。
    ensure_not_indexing()?;
    // ★ 查询 DB 中是否有本地同步记录（用于同步删除本地文件）
    let local_info: Option<(String, bool)> = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?.map(|r| (r.local_path.clone(), r.is_folder))
    };
    if let Err(write_error) = FILES_API.delete(&id).await {
        match FILES_API.verify_deleted(&id).await {
            Ok(true) => tracing::info!(file_id = %id, "删除响应丢失，但 fileId 核验确认已回收"),
            Ok(false) => return Err(write_error),
            Err(verification_error) => {
                return Err(crate::error::AppError::generic(format!(
                    "删除结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        }
    }

    tracing::info!(file_id = %id, "删除云端文件已核验");
    if let Some((local_path, is_folder)) = local_info {
        let mount = mount()?;
        let absolute_path =
            crate::core::paths::safe_join_under(mount.mount_dir(), &local_path, false)?;
        if absolute_path.exists() {
            if is_folder {
                tokio::fs::remove_dir_all(&absolute_path)
                    .await
                    .map_err(|error| {
                        crate::error::AppError::generic(format!(
                            "云端已回收，但本地目录删除失败：{error}"
                        ))
                    })?;
            } else {
                mount.delete_local(&absolute_path).await.map_err(|error| {
                    crate::error::AppError::generic(format!(
                        "云端已回收，但本地文件删除失败：{error}"
                    ))
                })?;
            }
        }

        {
            let conn = DB.lock();
            if is_folder {
                let prefix = format!("{local_path}/");
                conn.execute(
                    "UPDATE sync_items SET status=?1
                     WHERE local_path=?2 OR substr(local_path, 1, length(?3))=?3",
                    rusqlite::params![repository::sync_status::DELETED, local_path, prefix],
                )
            } else {
                conn.execute(
                    "UPDATE sync_items SET status=?1 WHERE file_id=?2",
                    rusqlite::params![repository::sync_status::DELETED, id],
                )
            }
            .map_err(|error| {
                crate::error::AppError::generic(format!("结算删除基线失败：{error}"))
            })?;
        }

        if let Some(engine) = try_sync_engine() {
            if is_folder {
                let prefix = format!("{local_path}/");
                engine
                    .cloud_tree_lock()
                    .retain(|path, _| path != &local_path && !path.starts_with(&prefix));
                engine
                    .path_to_id_lock()
                    .retain(|path, _| path != &local_path && !path.starts_with(&prefix));
            } else {
                engine.cloud_tree_remove(&local_path);
                engine.path_to_id_remove(&local_path);
            }
            engine.add_recently_deleted(&local_path);
        }
    }
    emit_folder_content_changed(&app);
    Ok(())
}

async fn settle_verified_remote_path_change(
    file_id: &str,
    new_relative_path: &str,
    verified: &DriveFile,
) -> AppResult<()> {
    crate::core::paths::validate_relative_path(new_relative_path, false)?;
    let old_record = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, file_id)?
    };
    let Some(old_record) = old_record else {
        return Ok(());
    };
    let old_relative_path = old_record.local_path.clone();
    if old_relative_path == new_relative_path {
        return Ok(());
    }

    let mount = mount()?;
    let old_absolute =
        crate::core::paths::safe_join_under(mount.mount_dir(), &old_relative_path, false)?;
    let new_absolute =
        crate::core::paths::safe_join_under(mount.mount_dir(), new_relative_path, false)?;
    if let Some(parent) = new_absolute.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::generic(format!("创建目标父目录失败：{error}")))?;
    }
    if old_absolute.exists() {
        if new_absolute.exists() {
            return Err(AppError::generic("目标本地路径已存在，拒绝覆盖"));
        }
        tokio::fs::rename(&old_absolute, &new_absolute)
            .await
            .map_err(|error| AppError::generic(format!("同步本地路径变更失败：{error}")))?;
    } else if new_absolute.exists() {
        let target_id = xattr::get(&new_absolute, crate::mount::manager::XATTR_FILE_ID)
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        if target_id.as_deref() != Some(file_id) {
            return Err(AppError::generic("目标路径已存在且无法证明是同一云端文件"));
        }
    }

    let affected = {
        let conn = DB.lock();
        let prefix = format!("{old_relative_path}/");
        repository::load_all(&conn)?
            .into_iter()
            .filter(|record| {
                record.local_path == old_relative_path || record.local_path.starts_with(&prefix)
            })
            .collect::<Vec<_>>()
    };
    {
        let conn = DB.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始路径结算事务失败：{error}")))?;
        for record in &affected {
            transaction
                .execute(
                    "DELETE FROM sync_items WHERE file_id=?1 AND local_path=?2",
                    rusqlite::params![record.file_id, record.local_path],
                )
                .map_err(|error| AppError::generic(format!("删除旧路径基线失败：{error}")))?;
        }
        for mut record in affected {
            let suffix = record
                .local_path
                .strip_prefix(&old_relative_path)
                .unwrap_or_default();
            record.local_path = format!("{new_relative_path}{suffix}");
            if record.file_id == file_id {
                record.name = verified.name.clone();
                record.parent_folder_id = verified
                    .parent_folder
                    .as_ref()
                    .and_then(|parents| parents.first().cloned());
                record.cloud_edited_time = verified
                    .edited_time
                    .map(|edited_time| edited_time.timestamp_millis());
            }
            repository::upsert(&transaction, &record)?;
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交路径结算事务失败：{error}")))?;
    }

    if let Some(engine) = try_sync_engine() {
        let prefix = format!("{old_relative_path}/");
        let mut cloud = engine.cloud_tree_lock();
        let mut path_to_id = engine.path_to_id_lock();
        let stale_paths: Vec<String> = cloud
            .keys()
            .filter(|path| *path == &old_relative_path || path.starts_with(&prefix))
            .cloned()
            .collect();
        let mut moved = Vec::with_capacity(stale_paths.len());
        for old_path in stale_paths {
            if let Some(file) = cloud.remove(&old_path) {
                path_to_id.remove(&old_path);
                let suffix = old_path
                    .strip_prefix(&old_relative_path)
                    .unwrap_or_default();
                moved.push((format!("{new_relative_path}{suffix}"), file));
            }
        }
        for (path, mut file) in moved {
            if file.id == file_id {
                file = verified.clone();
            }
            path_to_id.insert(path.clone(), file.id.clone());
            cloud.insert(path, file);
        }
        drop(path_to_id);
        drop(cloud);
        engine.add_recently_deleted(&old_relative_path);
    }
    Ok(())
}

#[tauri::command]
pub async fn drive_rename_file(id: String, new_name: String) -> AppResult<DriveFile> {
    // 索引中拒绝：同 drive_delete_file，避免与重建中的 cloud_tree 冲突。
    ensure_not_indexing()?;
    crate::core::paths::validate_path_segment(&new_name)?;
    let old_relative_path = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?.map(|record| record.local_path)
    };
    let file = match FILES_API.rename_file(&id, &new_name).await {
        Ok(file) => file,
        Err(write_error) => match FILES_API.get(&id).await {
            Ok(file) if file.id == id && file.name == new_name => file,
            Ok(_) => return Err(write_error),
            Err(verification_error) => {
                return Err(AppError::generic(format!(
                    "重命名结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        },
    };
    if let Some(old_relative_path) = old_relative_path {
        let new_relative_path = std::path::Path::new(&old_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join(&new_name))
            .unwrap_or_else(|| std::path::PathBuf::from(&new_name));
        settle_verified_remote_path_change(&id, &new_relative_path.to_string_lossy(), &file)
            .await?;
    }
    tracing::info!(file_id = %id, new_name = %new_name, "重命名已核验并结算");
    Ok(file)
}

#[tauri::command]
pub async fn drive_move_file(id: String, new_parent_folder: String) -> AppResult<DriveFile> {
    // 索引中拒绝：移动改 parentFolder，与重建中的 path_to_id/cloud_tree 冲突。
    ensure_not_indexing()?;
    let old_relative_path = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?.map(|record| record.local_path)
    };
    let file = match FILES_API
        .update(&id, None, Some(&new_parent_folder), None)
        .await
    {
        Ok(file) => file,
        Err(write_error) => match FILES_API.get(&id).await {
            Ok(file)
                if file.id == id
                    && file.parent_folder.as_deref().is_some_and(|parents| {
                        parents.len() == 1 && parents[0] == new_parent_folder
                    }) =>
            {
                file
            }
            Ok(_) => return Err(write_error),
            Err(verification_error) => {
                return Err(AppError::generic(format!(
                    "移动结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        },
    };
    if let Some(old_relative_path) = old_relative_path {
        let target_parent_path = if let Some(engine) = try_sync_engine() {
            engine
                .path_to_id_lock()
                .iter()
                .find_map(|(path, file_id)| (file_id == &new_parent_folder).then_some(path.clone()))
        } else {
            None
        }
        .or_else(|| {
            let conn = DB.lock();
            repository::find_by_file_id(&conn, &new_parent_folder)
                .ok()
                .flatten()
                .map(|record| record.local_path)
        })
        .ok_or_else(|| AppError::generic("无法解析目标云端目录的本地路径"))?;
        let name = std::path::Path::new(&old_relative_path)
            .file_name()
            .ok_or_else(|| AppError::generic("移动源路径缺少文件名"))?;
        let new_relative_path = if target_parent_path.is_empty() {
            std::path::PathBuf::from(name)
        } else {
            std::path::Path::new(&target_parent_path).join(name)
        };
        settle_verified_remote_path_change(&id, &new_relative_path.to_string_lossy(), &file)
            .await?;
    }
    tracing::info!(file_id = %id, target_folder = %new_parent_folder, "移动已核验并结算");
    Ok(file)
}

#[tauri::command]
pub async fn drive_search(
    keyword: String,
    parent_id: Option<String>,
    page_size: Option<u32>,
) -> AppResult<FileListResult> {
    FILES_API
        .search(&keyword, parent_id.as_deref(), page_size.unwrap_or(100))
        .await
}

#[tauri::command]
pub async fn drive_get_thumbnail(file_id: String) -> AppResult<Vec<u8>> {
    THUMBNAIL_API.get(&file_id).await
}

#[tauri::command]
pub async fn drive_get_about() -> AppResult<DriveAbout> {
    AboutApi::new(DRIVE_CLIENT.clone()).get().await
}

#[tauri::command]
pub async fn drive_download_file(file_id: String, dest_path: String) -> AppResult<()> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    let m = mount()?;
    let dest = std::path::PathBuf::from(&dest_path);
    let rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &dest)?;
    let dest = crate::core::paths::safe_join_under(m.mount_dir(), &rel, false)?;
    let cloud = FILES_API.get(&file_id).await?;
    let is_update = dest.is_file();
    let operation = if is_update {
        crate::sync::transfer_state::TransferOperation::DownloadUpdate
    } else {
        crate::sync::transfer_state::TransferOperation::Download
    };
    let result = engine
        .task_runner()?
        .enqueue_and_run(repository::TransferTask {
            id: 0,
            direction: if is_update {
                repository::transfer_direction::DOWNLOAD_UPDATE
            } else {
                repository::transfer_direction::DOWNLOAD
            },
            file_id: Some(file_id),
            local_path: Some(dest.to_string_lossy().into_owned()),
            name: cloud.name,
            total_size: cloud.size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(rel),
            parent_file_id: cloud
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned()),
            operation: Some(i32::from(operation)),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: cloud.edited_time.map(|time| time.timestamp_millis()),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        })
        .await?;
    if result.outcome.disposition == crate::sync::task_runner::TaskDisposition::Completed {
        Ok(())
    } else {
        Err(AppError::generic(format!(
            "下载已进入恢复队列：{:?}",
            result.outcome.disposition
        )))
    }
}

#[tauri::command]
pub async fn drive_upload_file(
    local_path: String,
    parent_id: Option<String>,
) -> AppResult<DriveFile> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    let m = mount()?;
    let path = std::path::PathBuf::from(&local_path);
    let rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &path)?;
    let path = crate::core::paths::safe_join_under(m.mount_dir(), &rel, false)?;
    let parent_id = parent_id.filter(|id| !id.trim().is_empty());
    let metadata = std::fs::metadata(&path)
        .map_err(|error| AppError::generic(format!("读取上传源失败：{error}")))?;
    let source_mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    let result = engine
        .task_runner()?
        .enqueue_and_run(repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::UPLOAD,
            file_id: None,
            local_path: Some(path.to_string_lossy().into_owned()),
            name: rel.rsplit('/').next().unwrap_or(&rel).to_string(),
            total_size: metadata.len() as i64,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(rel),
            parent_file_id: parent_id,
            operation: Some(i32::from(
                crate::sync::transfer_state::TransferOperation::Create,
            )),
            source_mtime,
            source_size: Some(metadata.len() as i64),
            expected_cloud_edited_time: None,
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        })
        .await?;
    if result.outcome.disposition != crate::sync::task_runner::TaskDisposition::Completed {
        return Err(AppError::generic(format!(
            "上传已进入恢复队列：{:?}",
            result.outcome.disposition
        )));
    }
    result
        .outcome
        .cloud_file
        .ok_or_else(|| AppError::generic("上传完成但缺少云端文件结果"))
}

// ========================================================================
// 三、Sync 同步命令
// ========================================================================

/// 全量刷新云端树 + 同步周期（走 SyncEngine）。
#[tauri::command]
pub async fn sync_manual_refresh(_app: AppHandle) -> AppResult<()> {
    let e = sync_engine()?;
    e.trigger_manual_sync().await
}

/// 查询文件本地同步状态（供前端删除确认用）。
/// 返回 "folder" | "synced" | "placeholder" | "not_synced"
#[tauri::command]
pub fn sync_check_file_local_status(file_id: String) -> AppResult<String> {
    let conn = DB.lock();
    let record = repository::find_by_file_id(&conn, &file_id).ok().flatten();
    let Some(record) = record else {
        return Ok("not_synced".to_string());
    };
    if record.is_folder {
        return Ok("folder".to_string());
    }
    // 检查本地文件是否存在且有真实内容（非占位符）
    if let Ok(m) = mount() {
        let abs_path = m.mount_dir().join(&record.local_path);
        if abs_path.exists() {
            if let Ok(meta) = std::fs::metadata(&abs_path) {
                if meta.len() > 0 {
                    // 进一步确认不是占位符（占位符 0 字节已过滤，但检查 xattr 更严谨）
                    let is_placeholder = xattr::get(&abs_path, crate::mount::manager::XATTR_STATE)
                        .ok()
                        .flatten()
                        .map(|b| {
                            String::from_utf8_lossy(&b) == crate::mount::manager::STATE_PLACEHOLDER
                        })
                        .unwrap_or(false);
                    if !is_placeholder {
                        return Ok("synced".to_string());
                    }
                }
            }
            return Ok("placeholder".to_string());
        }
    }
    Ok("not_synced".to_string())
}

/// 批量查询文件同步状态（供前端文件列表状态列展示用）。
/// 接受文件 ID 列表，返回 fileId → "folder" | "synced" | "placeholder" | "not_synced" 映射。
/// 未挂载同步目录时回退到仅 DB 状态判断。
#[tauri::command]
pub fn sync_batch_file_status(file_ids: Vec<String>) -> AppResult<HashMap<String, String>> {
    let conn = DB.lock();
    let mount_opt = mount().ok();
    let mut result: HashMap<String, String> = HashMap::with_capacity(file_ids.len());

    for file_id in &file_ids {
        let status = match repository::find_by_file_id(&conn, file_id).ok().flatten() {
            None => "not_synced",
            Some(record) => {
                if record.is_folder {
                    "folder"
                } else if let Some(ref m) = mount_opt {
                    let abs_path = m.mount_dir().join(&record.local_path);
                    if abs_path.exists() {
                        if let Ok(meta) = std::fs::metadata(&abs_path) {
                            if meta.len() > 0 {
                                // 进一步确认不是占位符（占位符 0 字节已过滤，但 xattr 更严谨）
                                let is_placeholder =
                                    xattr::get(&abs_path, crate::mount::manager::XATTR_STATE)
                                        .ok()
                                        .flatten()
                                        .map(|b| {
                                            String::from_utf8_lossy(&b)
                                                == crate::mount::manager::STATE_PLACEHOLDER
                                        })
                                        .unwrap_or(false);
                                if !is_placeholder {
                                    "synced"
                                } else {
                                    "placeholder"
                                }
                            } else {
                                "placeholder"
                            }
                        } else {
                            "placeholder"
                        }
                    } else {
                        "not_synced"
                    }
                } else {
                    // 未配置挂载目录：仅从 DB 状态判定
                    if record.status == repository::sync_status::SYNCED {
                        "synced"
                    } else {
                        "not_synced"
                    }
                }
            }
        };
        result.insert(file_id.clone(), status.to_string());
    }

    Ok(result)
}

#[tauri::command]
pub async fn sync_check_safe_free_up(rel_path: String, file_id: String) -> AppResult<String> {
    // 引擎已启动 → 用 cloud_tree + DB 精确校验
    if let Some(e) = try_sync_engine() {
        return Ok(match e.can_safely_free_up(&rel_path, &file_id) {
            FreeUpCheckResult::Safe => "safe",
            FreeUpCheckResult::NotInCloud => "not_in_cloud",
            FreeUpCheckResult::NotSynced => "not_synced",
        }
        .to_string());
    }
    // Without the engine there is no trusted cloud checkpoint or activity gate. Fail closed.
    let _ = (rel_path, file_id);
    Ok("not_synced".to_string())
}

#[tauri::command]
pub async fn sync_free_up_space(
    file_id: String,
    rel_path: String,
    local_path: String,
    _name: String,
    size: i64,
) -> AppResult<()> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    let m = mount()?;
    let frontend_rel = crate::core::paths::relative_path_from_mount(
        m.mount_dir(),
        &std::path::PathBuf::from(&local_path),
    )?;
    if frontend_rel != rel_path {
        return Err(AppError::config(format!(
            "释放空间路径不一致：rel_path={rel_path}, local_path={local_path}"
        )));
    }
    let lp = crate::core::paths::safe_join_under(m.mount_dir(), &rel_path, false)?;

    if size < 0 || !engine.cloud_tree_is_trusted() {
        return Err(AppError::generic("云端索引尚未追平，拒绝释放本地唯一副本"));
    }

    let metadata_snapshot = std::fs::symlink_metadata(&lp)
        .map_err(|error| AppError::generic(format!("读取待释放文件失败：{error}")))?;
    if !metadata_snapshot.file_type().is_file() || crate::mount::manager::is_placeholder_file(&lp) {
        return Err(AppError::generic("待释放目标不是已下载的普通文件"));
    }
    let source_mtime = metadata_snapshot
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .ok_or_else(|| AppError::generic("无法读取待释放文件修改时间"))?;
    let source_size = metadata_snapshot.len() as i64;
    if source_size != size {
        return Err(AppError::generic("待释放文件大小已变化，请刷新后重试"));
    }

    let baseline = {
        let conn = DB.lock();
        let active = repository::list_all_transfers(&conn)?
            .into_iter()
            .any(|task| {
                (task.relative_path.as_deref() == Some(rel_path.as_str())
                    || task.file_id.as_deref() == Some(file_id.as_str()))
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            crate::sync::transfer_state::TransferState::Completed
                                | crate::sync::transfer_state::TransferState::Failed
                                | crate::sync::transfer_state::TransferState::Canceled
                        )
                    })
            });
        if active {
            return Err(AppError::generic("该文件存在活动传输任务，暂不能释放空间"));
        }
        repository::find_by_file_id(&conn, &file_id)?
            .filter(|record| record.local_path == rel_path)
            .ok_or_else(|| AppError::generic("找不到与路径匹配的成功同步基线"))?
    };
    if baseline.status != repository::sync_status::SYNCED
        || baseline.local_mtime != Some(source_mtime)
        || baseline.local_size != Some(source_size)
        || baseline.size != size
    {
        return Err(AppError::generic(
            "本地内容与最后成功同步基线不一致，拒绝释放",
        ));
    }
    {
        let cloud = engine.cloud_tree_lock();
        if cloud.get(&rel_path).map(|file| file.id.as_str()) != Some(file_id.as_str()) {
            return Err(AppError::generic("可信云树中不存在同一 fileId"));
        }
    }

    // Remote verification is intentionally between two local/DB checks. If the network call is
    // slow, any local edit or new transfer intent invalidates the lease before unlink.
    let remote = FILES_API.get(&file_id).await?;
    if remote.id != file_id || remote.size != size || FILES_API.verify_deleted(&file_id).await? {
        return Err(AppError::generic("远端副本不存在、已回收或大小不一致"));
    }

    let current_metadata = std::fs::symlink_metadata(&lp)
        .map_err(|error| AppError::generic(format!("释放前复核本地文件失败：{error}")))?;
    let current_mtime = current_metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    if !current_metadata.file_type().is_file()
        || current_metadata.len() as i64 != source_size
        || current_mtime != Some(source_mtime)
    {
        return Err(AppError::generic("远端核验期间本地文件已变化，拒绝删除"));
    }
    {
        let conn = DB.lock();
        let active = repository::list_all_transfers(&conn)?
            .into_iter()
            .any(|task| {
                (task.relative_path.as_deref() == Some(rel_path.as_str())
                    || task.file_id.as_deref() == Some(file_id.as_str()))
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            crate::sync::transfer_state::TransferState::Completed
                                | crate::sync::transfer_state::TransferState::Failed
                                | crate::sync::transfer_state::TransferState::Canceled
                        )
                    })
            });
        let current = repository::find_by_file_id(&conn, &file_id)?;
        if active
            || current.as_ref().is_none_or(|record| {
                record.local_path != baseline.local_path
                    || record.status != baseline.status
                    || record.local_mtime != baseline.local_mtime
                    || record.local_size != baseline.local_size
                    || record.cloud_edited_time != baseline.cloud_edited_time
            })
        {
            return Err(AppError::generic("释放租约已失效，请刷新后重试"));
        }
    }

    m.delete_local(&lp).await?;

    // 创建占位符
    m.create_placeholder_if_needed(&rel_path, &file_id, size)
        .await?;

    // 更新 DB
    let conn = DB.lock();
    let changed = conn
        .execute(
            "UPDATE sync_items
             SET status=?1, local_size=0, error_message=NULL
             WHERE file_id=?2 AND local_path=?3 AND status=?4
               AND local_mtime=?5 AND local_size=?6",
            rusqlite::params![
                repository::sync_status::CLOUD_ONLY,
                file_id,
                rel_path,
                repository::sync_status::SYNCED,
                source_mtime,
                source_size,
            ],
        )
        .map_err(|error| AppError::generic(format!("提交释放空间基线失败：{error}")))?;
    if changed != 1 {
        return Err(AppError::generic(
            "释放空间后基线发生并发变化，请立即重新同步",
        ));
    }

    Ok(())
}

#[tauri::command]
pub async fn sync_download_on_demand(
    _app: AppHandle,
    file_id: String,
    dest_path: String,
) -> AppResult<bool> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    // 全局索引读取中：禁止按需下载（同 sync_folder_recursive，cloud_tree 构建中）
    if engine.current_state().is_indexing {
        return Err(AppError::generic(
            "正在读取云端索引，请稍后再试".to_string(),
        ));
    }
    let m = mount()?;
    let frontend_dest = PathBuf::from(&dest_path);
    let frontend_rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &frontend_dest)?;
    let record = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &file_id).ok().flatten()
    };
    let dest_rel = match record.as_ref().map(|record| record.local_path.clone()) {
        Some(rel) => {
            crate::core::paths::validate_relative_path(&rel, false)?;
            if rel != frontend_rel {
                return Err(AppError::config(format!(
                    "下载路径不一致：file_id={file_id}, rel_path={rel}, dest_path={dest_path}"
                )));
            }
            rel
        }
        None => frontend_rel,
    };
    let dest = crate::core::paths::safe_join_under(m.mount_dir(), &dest_rel, false)?;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // 查云端真实元数据（editedTime + size）：
    // - editedTime 必须写真实值，写 None 会让 is_cloud_changed 永远判"云端已变"
    //   → watcher 触发的 cycle 重复下载 → 同步循环（恶性 bug 根因）
    // - size 用于传输队列进度条显示（避免 0/0 显示 0%）
    // 云端查询失败时回退查库既有 editedTime；size 缺失则 0（进度条不显示）
    let cloud_file = FILES_API.get(&file_id).await.ok();
    let cloud_edited_time = cloud_file
        .as_ref()
        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis()))
        .or_else(|| record.as_ref().and_then(|record| record.cloud_edited_time));
    let cloud_size = cloud_file
        .as_ref()
        .map(|file| file.size)
        .or_else(|| record.as_ref().map(|record| record.size))
        .unwrap_or(0);
    let is_update = std::fs::metadata(&dest)
        .ok()
        .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
    let operation = if is_update {
        crate::sync::transfer_state::TransferOperation::DownloadUpdate
    } else {
        crate::sync::transfer_state::TransferOperation::Download
    };
    let direction = if is_update {
        repository::transfer_direction::DOWNLOAD_UPDATE
    } else {
        repository::transfer_direction::DOWNLOAD
    };
    let task = repository::TransferTask {
        id: 0,
        direction,
        file_id: Some(file_id),
        local_path: Some(dest.to_string_lossy().into_owned()),
        name,
        total_size: cloud_size,
        transferred: 0,
        state: i32::from(crate::sync::transfer_state::TransferState::Pending),
        error_message: None,
        created_at: chrono::Utc::now().timestamp_millis(),
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(dest_rel),
        parent_file_id: cloud_file
            .as_ref()
            .and_then(|file| file.parent_folder.as_ref())
            .and_then(|parents| parents.first().cloned()),
        operation: Some(i32::from(operation)),
        source_mtime: None,
        source_size: None,
        expected_cloud_edited_time: cloud_edited_time,
        attempt_count: 0,
        next_retry_at: None,
        error_kind: None,
        remote_result_file_id: None,
        state_revision: 0,
    };
    let result = engine.task_runner()?.enqueue_and_run(task).await?;
    match result.outcome.disposition {
        crate::sync::task_runner::TaskDisposition::Completed => Ok(true),
        disposition => Err(AppError::generic(format!(
            "下载已进入恢复队列：{disposition:?}"
        ))),
    }
}

/// folder sync 进度事件 payload
#[derive(Clone, Serialize)]
pub struct FolderSyncProgress {
    pub done: usize,
    pub total: usize,
}

/// 递归同步云端目录子树到本地（下载缺失 + 上传本地独有 + 建目录），对齐 dart syncFolderRecursive。
/// folder_id：云端目录 id；rel_path：该目录相对挂载根的路径（定位本地 dest + cloud_tree 全路径）。
#[tauri::command]
pub async fn sync_folder_recursive(
    app: AppHandle,
    folder_id: String,
    rel_path: String,
) -> AppResult<i64> {
    let eng = sync_engine()?;
    let activity = eng.begin_external_activity()?;
    // 全局索引读取中（云端树 BFS 重建中）：选择目录同步会基于不完整的 cloud_tree/path_to_id，
    // 且与全局 BFS 并发拉取易冲突 → 拒绝，等索引完成。
    if eng.current_state().is_indexing {
        return Err(AppError::generic(
            "正在读取云端索引，请稍后再试".to_string(),
        ));
    }
    let Some(folder_guard) = eng.try_begin_folder_sync_guard() else {
        return Err(AppError::generic(
            "已有同步周期或目录同步正在运行，本次请求未开始",
        ));
    };
    // 后台异步执行：立即返回，不阻塞前端。传输项实时进传输队列（菜单栏/弹窗可见）。
    // spawn 内 finally 释放 folder_syncing 锁 + 广播 contentChanged（前端目录刷新）。
    let eng_clone = eng.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let _activity = activity;
        let _folder = folder_guard;
        let result =
            sync_folder_recursive_impl(&app_clone, &eng_clone, &folder_id, &rel_path).await;
        // 完成后广播 contentChanged 让前端目录刷新
        if let Err(error) = eng_clone.update_runtime_and_broadcast(|runtime| {
            runtime.content_changed = true;
            runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
        }) {
            tracing::warn!(%error, "目录同步完成后重算全局状态失败");
        }
        match &result {
            Ok(done) => tracing::info!(done, rel = %rel_path, "sync_folder_recursive（后台）完成"),
            Err(e) => {
                tracing::warn!(error = %e, rel = %rel_path, "sync_folder_recursive（后台）失败")
            }
        }
    });
    Ok(0)
}

async fn sync_folder_recursive_impl(
    app: &AppHandle,
    eng: &SyncEngine,
    folder_id: &str,
    rel_path: &str,
) -> AppResult<i64> {
    let m = mount()?;
    let task_runner = eng.task_runner()?;
    crate::core::paths::validate_relative_path(rel_path, true)?;
    let dest_dir = crate::core::paths::safe_join_under(m.mount_dir(), rel_path, true)?;
    tracing::info!(folder_id, rel = %rel_path, "sync_folder_recursive: 开始递归同步");

    // 1. BFS 子树：listAll 递归
    let mut cloud_files: Vec<(String, DriveFile)> = Vec::new(); // (subrel, DriveFile)
    let mut cloud_folders: Vec<String> = Vec::new();
    let mut folder_rel_to_id: HashMap<String, String> = HashMap::new();
    folder_rel_to_id.insert(String::new(), folder_id.to_string());
    let mut queue: Vec<(String, String)> = vec![(folder_id.to_string(), String::new())];
    while !queue.is_empty() {
        let (id, path) = queue.remove(0);
        eng.ensure_cycle_active()?;
        let _operation = eng.begin_external_activity()?;
        let children = FILES_API.list_all(Some(id.as_str())).await?;
        for f in children {
            if f.name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                continue;
            }
            crate::core::paths::validate_path_segment(&f.name)?;
            let subrel = if path.is_empty() {
                f.name.clone()
            } else {
                format!("{path}/{}", f.name)
            };
            if f.is_folder() {
                cloud_folders.push(subrel.clone());
                folder_rel_to_id.insert(subrel.clone(), f.id.clone());
                queue.push((f.id.clone(), subrel));
            } else {
                cloud_files.push((subrel, f));
            }
        }
    }
    tracing::info!(
        files = cloud_files.len(),
        folders = cloud_folders.len(),
        "sync_folder_recursive: 云端子树"
    );

    // 2. 本地目录
    tokio::fs::create_dir_all(&dest_dir).await.ok();
    for sub in &cloud_folders {
        let path = crate::core::paths::safe_join_under(&dest_dir, sub, false)?;
        let _ = tokio::fs::create_dir_all(path).await;
    }

    // 3. 扫描本地真实文件（spawn_blocking，排除 .tmp/.hwcloud_/0 字节占位）
    let dest_dir_clone = dest_dir.clone();
    let local_files: HashMap<String, PathBuf> = tokio::task::spawn_blocking(move || {
        let mut out: HashMap<String, PathBuf> = HashMap::new();
        let _ = scan_dir_for_real_files(&dest_dir_clone, &dest_dir_clone, &mut out);
        out
    })
    .await
    .unwrap_or_default();

    // 4. to_download（云端有、本地无或仅占位）+ to_upload（本地有、云端无）
    let cloud_map: HashMap<String, DriveFile> = cloud_files
        .iter()
        .map(|(r, f)| (r.clone(), f.clone()))
        .collect();
    let to_download: Vec<(String, DriveFile)> = cloud_files
        .into_iter()
        .filter(|(r, _)| !local_files.contains_key(r))
        .collect();
    let to_upload: Vec<(String, PathBuf)> = local_files
        .into_iter()
        .filter(|(r, _)| !cloud_map.contains_key(r))
        .collect();
    let total = to_download.len() + to_upload.len();
    tracing::info!(
        download = to_download.len(),
        upload = to_upload.len(),
        "sync_folder_recursive: 任务"
    );

    let mut done: i64 = 0;

    // ★ 5. 为本地独有文件补建缺失的云端父目录链（解决目录结构被压平的问题）。
    //    场景：用户往同步目录粘贴了 b/c/d/a.txt，云端只知道 A 文件夹，不知道 b、c、d。
    //    若直接上传 a.txt，parent_subrel 查不到 b/c/d 的云端 ID → 回退到 A 的 ID →
    //    所有文件都被平铺到 A 下。必须在文件上传前先建好目录链。
    {
        // 收集所有需要上传的文件的祖先目录路径
        let mut missing_dirs: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for (subrel, _) in &to_upload {
            let parts: Vec<&str> = subrel.split('/').collect();
            // parts 的最后一段是文件名，之前的是目录层级
            for i in 1..parts.len() {
                let ancestor = parts[..i].join("/");
                if !cloud_folders.contains(&ancestor) && !folder_rel_to_id.contains_key(&ancestor) {
                    missing_dirs.insert(ancestor);
                }
            }
        }
        // 按深度升序创建（父目录先建，子目录才能找到父 ID）
        let mut sorted_dirs: Vec<String> = missing_dirs.into_iter().collect();
        sorted_dirs.sort_by_key(|p| p.matches('/').count());
        for dir_rel in &sorted_dirs {
            let dir_name = dir_rel.rsplit('/').next().unwrap_or(dir_rel);
            let parent_rel = parent_subrel_of(dir_rel);
            let parent_id = if parent_rel.is_empty() {
                Some(folder_id)
            } else {
                folder_rel_to_id.get(&parent_rel).map(|s| s.as_str())
            };
            let create_result = {
                eng.ensure_cycle_active()?;
                let _operation = eng.begin_external_activity()?;
                FILES_API.create_folder(dir_name, parent_id).await
            };
            match create_result {
                Ok(f) => {
                    folder_rel_to_id.insert(dir_rel.clone(), f.id.clone());
                    // 本地也建目录（确保后续 scan 能看到）
                    let _ = tokio::fs::create_dir_all(dest_dir.join(dir_rel)).await;
                    tracing::info!(dir = %dir_rel, cloud_id = %f.id, "sync_folder_recursive: 已补建云端父目录");
                }
                Err(e) => {
                    // 409/400 时查同名已存在目录，存在则视为成功
                    if matches!(e.drive_status(), Some(400 | 409)) {
                        if let Some(pid) = parent_id {
                            eng.ensure_cycle_active()?;
                            let _operation = eng.begin_external_activity()?;
                            if let Ok(list) = FILES_API.list_all(Some(pid)).await {
                                if let Some(existing) =
                                    list.iter().find(|c| c.is_folder() && c.name == dir_name)
                                {
                                    folder_rel_to_id.insert(dir_rel.clone(), existing.id.clone());
                                    let _ = tokio::fs::create_dir_all(dest_dir.join(dir_rel)).await;
                                    tracing::info!(dir = %dir_rel, cloud_id = %existing.id, "sync_folder_recursive: 父目录已存在（409容错）");
                                    continue;
                                }
                            }
                        }
                    }
                    tracing::warn!(dir = %dir_rel, error = %e, "sync_folder_recursive: 补建云端父目录失败，其内文件将继续上传（可能平铺）");
                }
            }
        }
    }

    // 6. 下载
    for (subrel, f) in &to_download {
        eng.ensure_cycle_active()?;
        let dest = crate::core::paths::safe_join_under(&dest_dir, subrel, false)?;
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        let is_update = std::fs::metadata(&dest)
            .ok()
            .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
        let task = repository::TransferTask {
            id: 0,
            direction: if is_update {
                repository::transfer_direction::DOWNLOAD_UPDATE
            } else {
                repository::transfer_direction::DOWNLOAD
            },
            file_id: Some(f.id.clone()),
            local_path: Some(dest.to_string_lossy().into_owned()),
            name: f.name.clone(),
            total_size: f.size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(full_rel.clone()),
            parent_file_id: f
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned()),
            operation: Some(i32::from(if is_update {
                crate::sync::transfer_state::TransferOperation::DownloadUpdate
            } else {
                crate::sync::transfer_state::TransferOperation::Download
            })),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: f.edited_time.map(|time| time.timestamp_millis()),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };
        match task_runner.enqueue_and_run(task).await {
            Ok(result)
                if result.outcome.disposition
                    == crate::sync::task_runner::TaskDisposition::Completed => {}
            Ok(result) => tracing::warn!(
                subrel = %subrel,
                disposition = ?result.outcome.disposition,
                "sync_folder_recursive: 下载进入恢复队列"
            ),
            Err(error) => tracing::warn!(
                subrel = %subrel,
                %error,
                "sync_folder_recursive: 下载失败"
            ),
        }
        done += 1;
        let _ = app.emit(
            "folder_sync_progress",
            FolderSyncProgress {
                done: done as usize,
                total,
            },
        );
    }
    // 7. 上传
    for (subrel, local_path) in &to_upload {
        eng.ensure_cycle_active()?;
        let parent_subrel = parent_subrel_of(subrel);
        let parent_id = folder_rel_to_id
            .get(&parent_subrel)
            .map(|s| s.as_str())
            .unwrap_or(folder_id);
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        let file_size = local_path.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let source_mtime = local_path
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let task = repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::UPLOAD,
            file_id: None,
            local_path: Some(local_path.to_string_lossy().into_owned()),
            name: subrel.rsplit('/').next().unwrap_or(subrel).to_string(),
            total_size: file_size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(full_rel.clone()),
            parent_file_id: Some(parent_id.to_string()),
            operation: Some(i32::from(
                crate::sync::transfer_state::TransferOperation::Create,
            )),
            source_mtime,
            source_size: Some(file_size),
            expected_cloud_edited_time: None,
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };
        match task_runner.enqueue_and_run(task).await {
            Ok(result)
                if result.outcome.disposition
                    == crate::sync::task_runner::TaskDisposition::Completed =>
            {
                if let Some(uploaded) = result.outcome.cloud_file {
                    eng.cloud_tree_insert(full_rel.clone(), uploaded.clone());
                    eng.path_to_id_insert(full_rel.clone(), uploaded.id);
                }
            }
            Ok(result) => tracing::warn!(
                subrel = %subrel,
                disposition = ?result.outcome.disposition,
                "sync_folder_recursive: 上传进入恢复队列"
            ),
            Err(error) => tracing::warn!(
                subrel = %subrel,
                %error,
                "sync_folder_recursive: 上传失败"
            ),
        }
        done += 1;
        let _ = app.emit(
            "folder_sync_progress",
            FolderSyncProgress {
                done: done as usize,
                total,
            },
        );
    }

    tracing::info!(done, total, "sync_folder_recursive: 完成");
    Ok(done)
}

/// 递归扫描目录，收集"真实内容文件"（subrel → PathBuf）：跳过 .tmp、.hwcloud_ 前缀、占位符（需 xattr 验证）。
fn scan_dir_for_real_files(
    base: &Path,
    current: &Path,
    out: &mut HashMap<String, PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            scan_dir_for_real_files(base, &path, out)?;
        } else if ft.is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tmp") || name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                continue;
            }
            let meta = entry.metadata()?;
            // 跳过占位符（xattr state=placeholder），0 字节用户文件（如空配置）不是占位符
            if meta.len() == 0 && crate::mount::manager::is_placeholder_file(&path) {
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            out.insert(rel, path);
        }
    }
    Ok(())
}

/// 取 subrel 的父 subrel（最后一个 / 之前；无 / 则空串）。
fn parent_subrel_of(subrel: &str) -> String {
    match subrel.rfind('/') {
        Some(i) => subrel[..i].to_string(),
        None => String::new(),
    }
}

#[tauri::command]
pub async fn sync_retry_failed() -> AppResult<()> {
    let e = sync_engine()?;
    e.retry_failed().await
}

#[tauri::command]
pub async fn sync_state() -> AppResult<SyncGlobalState> {
    // 引擎已启动 → 以当前 runtime 重新聚合并广播完整快照。
    if let Some(e) = try_sync_engine() {
        return e.recompute_and_broadcast_state();
    }
    // 引擎未启动：复用进程级 revision source 从 DB 生成完整兜底快照。
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let conn = DB.lock();
    STATUS_AGGREGATOR.snapshot(&conn, RuntimeStatus::default())
}

// ========================================================================
// 四、Config 命令
// ========================================================================

#[tauri::command]
pub fn config_load() -> AppResult<AppConfig> {
    ConfigStore::load()
}

#[tauri::command]
pub async fn config_save(app: AppHandle, config: AppConfig) -> AppResult<()> {
    let old = ConfigStore::load().ok();
    let old_configured = old.as_ref().map(|c| c.mount_configured).unwrap_or(false);
    let old_abs = old.as_ref().map(|c| c.expanded_mount_dir());
    let new_abs = config.expanded_mount_dir();
    let dir_changed =
        old_configured && config.mount_configured && old_abs.as_ref() != Some(&new_abs);

    ConfigStore::save(&config)?;

    // 换目录 / 取消配置：清缓存 + relaunch（setup 按新 config 重建引擎+watcher）
    if old_configured && (!config.mount_configured || dir_changed) {
        drop_runtime_async().await;
        if let Some(old_abs) = old_abs {
            crate::core::cache_paths::clear_cache_files(&old_abs.to_string_lossy());
        }
        crate::core::cache_paths::clear_cache_files(&new_abs.to_string_lossy());
        tracing::info!("挂载目录变更，relaunch");
        relaunch(&app);
        return Ok(());
    }

    // 首次配置：原地构造 MountManager + 启动引擎
    if !old_configured && config.mount_configured {
        let m = Arc::new(MountManager::new(&new_abs));
        m.ensure_mount_dir()?;
        set_mount_manager(m);
        ensure_engine_started(&app)?;
        return Ok(());
    }

    // 未变：仅更新 MountManager
    let m = Arc::new(MountManager::new(&new_abs));
    m.ensure_mount_dir()?;
    set_mount_manager(m);
    Ok(())
}

#[tauri::command]
pub fn config_export_json() -> AppResult<String> {
    let config = ConfigStore::load()?;
    ConfigStore::export_to_json(&config)
}

#[tauri::command]
pub fn config_import_json(json_str: String) -> AppResult<AppConfig> {
    ConfigStore::import_from_json(&json_str)
}

// ========================================================================
// 五、Transfer 传输命令
// ========================================================================

#[tauri::command]
pub fn transfer_list_all() -> AppResult<Vec<repository::TransferTask>> {
    let conn = DB.lock();
    repository::list_all_transfers(&conn)
}

/// 检查是否有进行中的传输任务（PENDING / RUNNING）。
/// 供更新流程使用：重启前等待所有传输完成。
#[tauri::command]
pub fn transfer_has_active() -> AppResult<bool> {
    let conn = DB.lock();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transfer_queue WHERE state IN (?1, ?2)",
            rusqlite::params![
                repository::transfer_state::PENDING,
                repository::transfer_state::RUNNING
            ],
            |row| row.get(0),
        )
        .map_err(|e| AppError::generic(format!("查询传输状态失败：{e}")))?;
    Ok(count > 0)
}

fn clear_transfer_history_and_snapshot(
    conn: &rusqlite::Connection,
    aggregator: &StatusAggregator,
    include_completed: bool,
    include_failed: bool,
) -> AppResult<SyncGlobalState> {
    conn.execute(
        "DELETE FROM transfer_queue
         WHERE (?1=1 AND state=?2) OR (?3=1 AND state=?4)",
        rusqlite::params![
            include_completed as i32,
            i32::from(crate::sync::transfer_state::TransferState::Completed),
            include_failed as i32,
            i32::from(crate::sync::transfer_state::TransferState::Failed),
        ],
    )
    .map_err(|error| AppError::generic(format!("清除传输历史失败：{error}")))?;
    aggregator.snapshot(conn, RuntimeStatus::default())
}

#[tauri::command]
pub fn transfer_clear_completed(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(true, false)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, true, false)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub fn transfer_clear_failed(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(false, true)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, false, true)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub fn transfer_clear_finished(app: AppHandle) -> AppResult<()> {
    if let Some(engine) = try_sync_engine() {
        engine.clear_transfer_history_and_broadcast(true, true)?;
        return Ok(());
    }
    let _publish_guard = STATUS_AGGREGATOR.lock_publication();
    let snapshot = {
        let conn = DB.lock();
        clear_transfer_history_and_snapshot(&conn, &STATUS_AGGREGATOR, true, true)?
    };
    emit_sync_state(&app, &snapshot);
    Ok(())
}

/// 接受一个持久化传输任务重试；上传和下载均由统一 TaskRunner 在后台执行。
#[tauri::command]
pub async fn transfer_retry(task_id: i64) -> AppResult<()> {
    let engine = sync_engine()?;
    engine.retry_transfer(task_id).await
}

// ========================================================================
// 六、SyncItem 状态命令（FileTile 用）
// ========================================================================

#[tauri::command]
pub fn sync_items_by_folder(folder_local_path: String) -> AppResult<Vec<repository::SyncItem>> {
    let conn = DB.lock();
    let mut stmt = conn
        .prepare("SELECT * FROM sync_items WHERE local_path LIKE ?1")
        .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
    let pattern = format!("{}%", folder_local_path);
    let rows = stmt
        .query_map(rusqlite::params![pattern], repository::SyncItem::from_row)
        .map_err(|e| AppError::generic(format!("查询失败：{e}")))?;
    let mut items = Vec::new();
    for item in rows.flatten() {
        items.push(item);
    }
    Ok(items)
}

// ========================================================================
// 七、Platform 平台命令
// ========================================================================

#[tauri::command]
pub async fn open_in_finder(path: String) -> AppResult<bool> {
    #[cfg(target_os = "macos")]
    {
        let result = std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map(|_| true)
            .map_err(|e| AppError::generic(format!("打开 Finder 失败：{e}")))?;
        Ok(result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(false)
    }
}

#[tauri::command]
pub fn launch_at_login_is_enabled() -> bool {
    crate::platform::launch_at_login::is_enabled()
}

#[tauri::command]
pub fn launch_at_login_set_enabled(enabled: bool) -> bool {
    match crate::platform::launch_at_login::set_enabled(enabled) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(error = %e, "设置开机自启失败");
            false
        }
    }
}

#[tauri::command]
pub async fn app_clear_cache(app: AppHandle) -> AppResult<()> {
    // 停引擎 + 释放 mount（relaunch 后 setup 会重建）
    drop_runtime_async().await;
    // 登出
    let _ = crate::auth::token_store::global_store().clear();
    // 删 DB：先清行（兜底，文件删不掉时至少数据没了），再删文件（彻底）。
    // DB 连接是进程级 Lazy 不释放，但 macOS 允许删除已打开的文件（unlink），
    // relaunch 后新进程会重建 fresh petal_link.db。
    {
        let conn = DB.lock();
        let _ = repository::delete_all(&conn);
        let _ = repository::delete_all_transfers(&conn);
    }
    if let Ok(p) = crate::data::db_file_path() {
        let _ = std::fs::remove_file(&p);
    }
    // 删所有挂载目录的缓存文件（含历史遗留的旧目录 syncstate_*/cloudtree_*，
    // 如临时目录 /var/folders/.../T/.tmpXXX 留下的）
    crate::core::cache_paths::clear_all_cache_files();
    // 删配置文件（回到首次启动状态）
    if let Ok(p) = crate::core::config_store::config_file_path() {
        let _ = std::fs::remove_file(&p);
    }
    tracing::info!("缓存已清空，准备重启");
    // 标记跳过关机 flush，再重启
    relaunch(&app);
    Ok(())
}

/// 读取最近日志快照（newest-first，供设置页日志查看）。
/// 暴露 core::logging::LOG_BUFFER 环形缓冲。
#[tauri::command]
pub fn logs_list() -> AppResult<Vec<crate::core::logging::LogRecord>> {
    Ok(crate::core::logging::snapshot())
}

/// 导出完整日志到指定路径（拼接 logs 目录下所有滚动日志文件，oldest-first）。
/// 数据源是 tracing_appender 的滚动文件 sink（完整持久日志），非仅缓冲的最近 1000 条。
#[tauri::command]
pub fn logs_export(path: String) -> AppResult<()> {
    let dir = crate::core::logging::log_dir()?;
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    // 只处理 PetalLink.log 开头的文件（过滤 .DS_Store 等非日志文件）
    files.retain(|f| {
        f.file_name()
            .map(|n| n.to_string_lossy().starts_with("PetalLink.log"))
            .unwrap_or(false)
    });
    files.sort(); // 按文件名升序（日期 oldest-first）

    // ★ 诊断：记录实际读到的文件，定位"只有当天"根因
    tracing::info!(
        dir = %dir.display(),
        count = files.len(),
        files = ?files.iter()
            .map(|f| f.file_name().unwrap_or_default().to_string_lossy().to_string())
            .collect::<Vec<_>>(),
        "logs_export 开始导出"
    );

    let mut out = String::new();
    for f in &files {
        // 用 read + from_utf8_lossy 容忍非 UTF-8（不丢内容），替代 read_to_string 的静默跳过
        let content = match std::fs::read(f) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                tracing::warn!(file = %f.display(), error = %e, "日志文件读取失败，跳过");
                continue;
            }
        };
        use std::fmt::Write;
        let _ = writeln!(out, "===== {} =====", f.display());
        out.push_str(&content);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.is_empty() {
        return Err(AppError::generic("日志目录为空，无可导出内容"));
    }
    tracing::info!(
        out_bytes = out.len(),
        file_count = files.len(),
        "logs_export 完成"
    );
    std::fs::write(&path, out)?;
    Ok(())
}

/// 清空日志环形缓冲（供日志查看页「清空」按钮真正清后端，而非仅清本地视图）。
#[tauri::command]
pub fn logs_clear() -> AppResult<()> {
    crate::core::logging::clear();
    Ok(())
}

/// 获取应用版本号（编译期从 Cargo.toml 注入，保持与打包版本一致）。
#[tauri::command]
pub fn app_get_version() -> String {
    crate::constants::APP_VERSION.to_string()
}

/// 推送同步状态到前端（Tauri event）
pub fn emit_sync_state(app: &AppHandle, state: &SyncGlobalState) {
    let _ = app.emit("sync_state", state);
}

/// 推送目录内容变更通知（触发前端 folderChildren 刷新）
pub fn emit_folder_content_changed(app: &AppHandle) {
    let _ = app.emit("folder_content_changed", ());
}

/// 索引守卫：cloud_tree BFS 重建中时拒绝文件操作（基于不完整数据会误判）。
///
/// 替代散布在 drive_delete/rename/move、sync_download_on_demand、sync_folder_recursive 等
/// 处的重复 `is_indexing` 检查 + 返回错误模式。
fn ensure_not_indexing() -> AppResult<()> {
    if sync_engine()
        .ok()
        .map(|e| e.current_state().is_indexing)
        .unwrap_or(false)
    {
        return Err(AppError::generic(
            "正在读取云端索引，请稍后再试".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod status_command_tests {
    use super::{
        clear_transfer_history_and_snapshot, drop_runtime_async, set_sync_engine, try_sync_engine,
        EngineOwnershipProtocol, CHANGES_API, DOWNLOAD_API, ENGINE_OWNERSHIP, FILES_API,
        UPLOAD_API,
    };
    use crate::data::repository::{self, SyncItem};
    use crate::drive::models::DriveFile;
    use crate::mount::manager::MountManager;
    use crate::sync::engine::SyncEngine;
    use crate::sync::executor::SyncExecutor;
    use crate::sync::status_aggregator::{RuntimeStatus, StatusAggregator};
    use crate::sync::task_runner::{
        TaskDisposition, TaskExecutionError, TaskExecutionOutcome, TaskProgressReporter,
        TaskRunner, TransferOperations,
    };
    use crate::sync::transfer_state::{TransferOperation, TransferState};

    static GLOBAL_RUNTIME_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct GlobalRuntimeTestCleanup {
        release_backend: std::sync::Arc<tokio::sync::Notify>,
    }

    impl Drop for GlobalRuntimeTestCleanup {
        fn drop(&mut self) {
            self.release_backend.notify_waiters();
            if let Some(engine) = super::SYNC_ENGINE.lock().take() {
                engine.shutdown_sync();
            }
            *super::MOUNT_MANAGER.lock() = None;
        }
    }

    struct SubmittedAmbiguousBackend {
        calls: std::sync::Mutex<Vec<i64>>,
        submitted: tokio::sync::Notify,
        release_response: std::sync::Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl TransferOperations for SubmittedAmbiguousBackend {
        async fn execute(
            &self,
            task: &repository::TransferTask,
            _progress: &TaskProgressReporter,
        ) -> Result<TaskExecutionOutcome, TaskExecutionError> {
            self.calls.lock().unwrap().push(task.id);
            self.submitted.notify_one();
            self.release_response.notified().await;
            Ok(TaskExecutionOutcome {
                cloud_file: Some(DriveFile {
                    id: format!("ambiguous-remote-{}", task.id),
                    name: task.name.clone(),
                    size: task.total_size,
                    ..Default::default()
                }),
                disposition: TaskDisposition::VerifyingRemote,
            })
        }
    }

    #[test]
    fn no_engine_history_clear_recomputes_complete_snapshot() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&conn).unwrap();
        repository::upsert(
            &conn,
            &SyncItem {
                file_id: "failed-sync".into(),
                local_path: "failed.txt".into(),
                parent_folder_id: None,
                name: "failed.txt".into(),
                is_folder: false,
                size: 1,
                local_size: Some(1),
                sha256: None,
                local_mtime: Some(1),
                cloud_edited_time: Some(1),
                last_sync_time: Some(1),
                status: repository::sync_status::FAILED,
                error_message: Some("sync failure".into()),
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transfer_queue
             (direction, name, total_size, transferred, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                repository::transfer_direction::UPLOAD,
                "failed.txt",
                1,
                0,
                i32::from(TransferState::Failed),
                1,
            ],
        )
        .unwrap();
        let aggregator = StatusAggregator::default();
        let before = aggregator
            .snapshot(&conn, RuntimeStatus::default())
            .unwrap();

        let after = clear_transfer_history_and_snapshot(&conn, &aggregator, false, true).unwrap();

        assert!(after.revision > before.revision);
        assert_eq!(before.transfer_failed, 1);
        assert_eq!(after.transfer_failed, 0);
        assert_eq!(after.failed, 1);
        assert_eq!(after.failed_items.len(), 1);
    }

    #[test]
    fn concurrent_replacement_rejects_install_until_old_owner_quiesces() {
        let protocol = std::sync::Arc::new(EngineOwnershipProtocol::new());
        let replacement_started = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release_replacement = std::sync::Arc::new(std::sync::Barrier::new(2));
        let replacement = {
            let protocol = protocol.clone();
            let replacement_started = replacement_started.clone();
            let release_replacement = release_replacement.clone();
            std::thread::spawn(move || {
                let _replacement = protocol.begin_replacement();
                replacement_started.wait();
                release_replacement.wait();
            })
        };
        replacement_started.wait();

        let live_installs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let blocked = {
            let live_installs = live_installs.clone();
            protocol.install(|| {
                live_installs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
        };
        assert!(blocked.is_err());
        assert_eq!(live_installs.load(std::sync::atomic::Ordering::SeqCst), 0);

        release_replacement.wait();
        replacement.join().unwrap();
        protocol
            .install(|| {
                live_installs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        assert_eq!(live_installs.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn global_replacement_waits_for_submitted_ambiguous_write_settlement() {
        const FAILURE_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

        let _serial = GLOBAL_RUNTIME_TEST_LOCK.lock().await;
        drop_runtime_async().await;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("ambiguous-replacement.txt");
        std::fs::write(&source, b"submitted payload").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let source_mtime = metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&conn).unwrap();
        let db = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        let release_response = std::sync::Arc::new(tokio::sync::Notify::new());
        let backend = std::sync::Arc::new(SubmittedAmbiguousBackend {
            calls: std::sync::Mutex::new(Vec::new()),
            submitted: tokio::sync::Notify::new(),
            release_response: release_response.clone(),
        });
        let _cleanup = GlobalRuntimeTestCleanup {
            release_backend: release_response,
        };
        let mount = std::sync::Arc::new(MountManager::new(root.path()));
        let runner = std::sync::Arc::new(TaskRunner::new(
            db.clone(),
            root.path().to_path_buf(),
            backend.clone(),
            std::sync::Arc::new(|| true),
            std::sync::Arc::new(|| Ok(())),
            None,
        ));
        let mut executor = SyncExecutor::new(
            1,
            FILES_API.clone(),
            DOWNLOAD_API.clone(),
            UPLOAD_API.clone(),
        );
        executor.set_mount(mount.clone());
        executor.set_db(db.clone());
        executor.set_task_runner_for_test(runner.clone());
        let mut old_engine = SyncEngine::new(
            FILES_API.clone(),
            CHANGES_API.clone(),
            DOWNLOAD_API.clone(),
            UPLOAD_API.clone(),
            db.clone(),
            std::sync::Arc::new(StatusAggregator::default()),
            Vec::new(),
            0,
            0,
        );
        old_engine.set_mount(mount);
        old_engine.set_executor(executor);
        let old_engine = std::sync::Arc::new(old_engine);
        old_engine.bind_task_runner_state_sink(&runner);
        let bound_runner = old_engine.task_runner().unwrap();
        assert!(std::sync::Arc::ptr_eq(&runner, &bound_runner));

        ENGINE_OWNERSHIP
            .install(|| {
                set_sync_engine(old_engine.clone());
                Ok(())
            })
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &try_sync_engine().unwrap(),
            &old_engine
        ));

        let transfer = tokio::spawn(async move {
            bound_runner
                .enqueue_and_run(repository::TransferTask {
                    id: 0,
                    direction: repository::transfer_direction::UPLOAD,
                    file_id: None,
                    local_path: Some(source.to_string_lossy().into_owned()),
                    name: "ambiguous-replacement.txt".into(),
                    total_size: metadata.len() as i64,
                    transferred: 0,
                    state: TransferState::Pending.into(),
                    error_message: None,
                    created_at: 1,
                    finished_at: None,
                    server_id: None,
                    upload_id: None,
                    resume_offset: 0,
                    session_url: None,
                    relative_path: Some("ambiguous-replacement.txt".into()),
                    parent_file_id: None,
                    operation: Some(TransferOperation::Create.into()),
                    source_mtime: Some(source_mtime),
                    source_size: Some(metadata.len() as i64),
                    expected_cloud_edited_time: None,
                    attempt_count: 0,
                    next_retry_at: None,
                    error_kind: None,
                    remote_result_file_id: None,
                    state_revision: 0,
                })
                .await
        });
        tokio::time::timeout(FAILURE_BOUND, backend.submitted.notified())
            .await
            .expect("backend submission must reach the ambiguity barrier");

        let running = repository::list_all_transfers(&db.lock()).unwrap();
        assert_eq!(running.len(), 1);
        let task_id = running[0].id;
        assert_eq!(running[0].state_kind().unwrap(), TransferState::Running);
        assert_eq!(&*backend.calls.lock().unwrap(), &[task_id]);

        let mut replacement = tokio::spawn(async { drop_runtime_async().await });
        tokio::time::timeout(FAILURE_BOUND, async {
            while try_sync_engine().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop_runtime_async must remove the old global slot");
        assert!(
            !replacement.is_finished(),
            "replacement must wait while the submitted write is unsettled"
        );

        let successor_conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&successor_conn).unwrap();
        let successor = std::sync::Arc::new(SyncEngine::new(
            FILES_API.clone(),
            CHANGES_API.clone(),
            DOWNLOAD_API.clone(),
            UPLOAD_API.clone(),
            std::sync::Arc::new(parking_lot::Mutex::new(successor_conn)),
            std::sync::Arc::new(StatusAggregator::default()),
            Vec::new(),
            0,
            0,
        ));
        let blocked_install_ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let blocked = {
            let blocked_install_ran = blocked_install_ran.clone();
            let successor = successor.clone();
            ENGINE_OWNERSHIP.install(|| {
                blocked_install_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                set_sync_engine(successor);
                Ok(())
            })
        };
        assert!(blocked.unwrap_err().to_string().contains("正在替换"));
        assert!(!blocked_install_ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(try_sync_engine().is_none());

        backend.release_response.notify_one();
        tokio::time::timeout(FAILURE_BOUND, &mut replacement)
            .await
            .expect("replacement must finish after backend settlement")
            .unwrap();

        let settled = repository::get_transfer_by_id(&db.lock(), task_id)
            .unwrap()
            .unwrap();
        assert_eq!(settled.id, task_id);
        assert_eq!(
            settled.state_kind().unwrap(),
            TransferState::VerifyingRemote
        );
        assert_eq!(
            settled.remote_result_file_id.as_deref(),
            Some(format!("ambiguous-remote-{task_id}").as_str())
        );
        let outcome = tokio::time::timeout(FAILURE_BOUND, transfer)
            .await
            .expect("transfer future must converge")
            .unwrap()
            .unwrap();
        assert_eq!(outcome.task_id, task_id);
        assert_eq!(
            outcome.outcome.disposition,
            TaskDisposition::VerifyingRemote
        );
        assert_eq!(&*backend.calls.lock().unwrap(), &[task_id]);

        ENGINE_OWNERSHIP
            .install(|| {
                set_sync_engine(successor.clone());
                Ok(())
            })
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &try_sync_engine().unwrap(),
            &successor
        ));
        drop_runtime_async().await;
        assert!(try_sync_engine().is_none());
    }
}
