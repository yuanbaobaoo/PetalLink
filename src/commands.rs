//! Tauri 命令层 —— 前端通过 invoke 调用的全部后端命令。
//!
//! 对齐 Flutter 版全部 UI→后端调用，1:1 复刻。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use crate::sync::state::{FailedItem, FreeUpCheckResult, SyncGlobalState};
use crate::sync::engine::SyncEngine;
use crate::sync::executor::SyncExecutor;

// ===== 全局单例 =====

/// 全局 AuthService 单例
pub static AUTH_SERVICE: Lazy<Arc<AuthService>> = Lazy::new(|| Arc::new(AuthService::new()));

/// 全局 DriveClient（首次访问时惰性创建）
pub static DRIVE_CLIENT: Lazy<Arc<DriveClient>> =
    Lazy::new(|| Arc::new(DriveClient::new(AUTH_SERVICE.clone())));

/// 全局 FilesApi
pub static FILES_API: Lazy<Arc<FilesApi>> = Lazy::new(|| Arc::new(FilesApi::new(DRIVE_CLIENT.clone())));

/// 全局 ChangesApi（增量变更接口）
pub static CHANGES_API: Lazy<Arc<ChangesApi>> =
    Lazy::new(|| Arc::new(ChangesApi::new(DRIVE_CLIENT.clone())));

/// 全局 DownloadApi
pub static DOWNLOAD_API: Lazy<Arc<DownloadApi>> =
    Lazy::new(|| Arc::new(DownloadApi::new(DRIVE_CLIENT.clone())));

/// 全局 UploadApi
pub static UPLOAD_API: Lazy<Arc<UploadApi>> = Lazy::new(|| Arc::new(UploadApi::new(DRIVE_CLIENT.clone())));

/// 全局 ThumbnailApi
pub static THUMBNAIL_API: Lazy<Arc<ThumbnailApi>> =
    Lazy::new(|| Arc::new(ThumbnailApi::new(DRIVE_CLIENT.clone())));

/// 全局 DB 连接（Arc 包裹，供 SyncEngine/SyncExecutor/命令层共享同一连接）
pub static DB: Lazy<Arc<Mutex<rusqlite::Connection>>> = Lazy::new(|| {
    let conn = crate::data::open().expect("打开数据库失败");
    Arc::new(Mutex::new(conn))
});

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

/// 全局传输更新广播发送端（供非 executor 路径的同步进度回调触发前端刷新，如双端对齐）。
/// 由 ensure_engine_started 创建并 set。回调拿不到 AppHandle，但能通过此全局 tx 触发刷新。
static TRANSFER_UPDATE_TX: Mutex<Option<tokio::sync::broadcast::Sender<()>>> = Mutex::new(None);

/// 设置传输更新广播发送端。
pub fn set_transfer_update_tx(tx: tokio::sync::broadcast::Sender<()>) {
    *TRANSFER_UPDATE_TX.lock() = Some(tx);
}

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
        None, None, Some(String::new()), Some(false), None, None, None, None, None, None,
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
    // 存入全局，供双端对齐等非 executor 路径的进度回调触发前端刷新
    set_transfer_update_tx(transfer_update_tx);
    // 传输进度实时推送监听器
    {
        let app_for_transfer = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match transfer_update_rx.recv().await {
                    Ok(()) => {
                        // emit_transfer_update 内部已统一刷新传输面板 + 状态条计数
                        emit_transfer_update(&app_for_transfer);
                        // 同步刷新托盘菜单的「正在传输」段（入队/进度/结算）
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
        config.skip_patterns.clone(),
        config.debounce_sec,
        config.poll_interval_sec,
    );
    engine.set_mount(mount.clone());
    engine.set_executor(executor);

    let engine = Arc::new(engine);
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
                        // 注意：此处不再调 emit_transfer_update。
                        // emit_transfer_update 内部会调 push_live_transfer_state（广播 sync_state），
                        // 而本桥接监听 sync_state → 会形成无限循环（emit_transfer_update →
                        // push_live_transfer_state → sync_state 广播 → 本桥接 → emit_transfer_update …），
                        // 导致 CPU 满载 + UI 卡死。sync_state 已携带传输计数，无需反向触发。
                        // 同步刷新托盘菜单的「正在传输」段（refresh_menu 不触发 sync_state 广播，安全）
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
        secret_configured: crate::constants::client_id_configured() && crate::constants::client_secret_configured(),
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
    drop_runtime();
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
        None, None, Some(String::new()), Some(false), None, None, None, None, None, None,
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
    drop_runtime();
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
        .list(parent_id.as_deref(), cursor.as_deref(), page_size.unwrap_or(100))
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
    if sync_engine().ok().map(|e| e.current_state().is_indexing).unwrap_or(false) {
        return Err(AppError::generic("正在读取云端索引，请稍后再试".to_string()));
    }
    // ★ 查询 DB 中是否有本地同步记录（用于同步删除本地文件）
    let local_info: Option<(String, bool)> = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)
            .ok()
            .flatten()
            .map(|r| (r.local_path.clone(), r.is_folder))
    };
    match FILES_API.delete(&id).await {
        Ok(()) => {
            tracing::info!(file_id = %id, "删除云端文件成功");
            if let Some((local_path, is_folder)) = local_info {
                if let Ok(m) = mount() {
                    let abs_path = m.mount_dir().join(&local_path);
                    if abs_path.exists() && !is_folder {
                        // ★ 文件：删除本地副本 + 标记 DB 为 DELETED（tombstone 防云端重建）
                        if let Err(e) = m.delete_local(&abs_path).await {
                            tracing::warn!(path = %local_path, error = %e, "删除本地文件失败（云端已删除）");
                        } else {
                            tracing::info!(path = %local_path, "已同步删除本地文件");
                        }
                        let conn = DB.lock();
                        let _ = conn.execute(
                            "UPDATE sync_items SET status=?1 WHERE file_id=?2",
                            rusqlite::params![repository::sync_status::DELETED, id],
                        );
                        if let Some(eng) = try_sync_engine() {
                            eng.cloud_tree_remove(&local_path);
                            eng.path_to_id_remove(&local_path);
                            eng.add_recently_deleted(&local_path);
                        }
                    } else if is_folder {
                        // ★ 目录：删除本地目录 + 标记 DB 为 DELETED
                        if let Err(e) = tokio::fs::remove_dir_all(&abs_path).await {
                            tracing::warn!(path = %local_path, error = %e, "删除本地目录失败");
                        } else {
                            tracing::info!(path = %local_path, "已同步删除本地目录");
                        }
                        let conn = DB.lock();
                        let _ = conn.execute(
                            "UPDATE sync_items SET status=?1 WHERE local_path=?2 OR local_path LIKE ?3",
                            rusqlite::params![repository::sync_status::DELETED, local_path, format!("{local_path}/%")],
                        );
                        if let Some(eng) = try_sync_engine() {
                            let mut ct = eng.cloud_tree_lock();
                            let mut p2i = eng.path_to_id_lock();
                            ct.retain(|k, _| k != &local_path && !k.starts_with(&format!("{local_path}/")));
                            p2i.retain(|k, _| k != &local_path && !k.starts_with(&format!("{local_path}/")));
                            drop(ct);
                            drop(p2i);
                            eng.add_recently_deleted(&local_path);
                            tracing::info!(path = %local_path, "目录已从云端+本地删除，缓存已清理");
                        }
                    }
                }
            }
            // ★ 通知前端刷新目录树
            emit_folder_content_changed(&app);
            Ok(())
        }
        Err(e) => { tracing::warn!(file_id = %id, error = %e, "删除云端文件失败"); Err(e) }
    }
}

#[tauri::command]
pub async fn drive_rename_file(id: String, new_name: String) -> AppResult<DriveFile> {
    // 索引中拒绝：同 drive_delete_file，避免与重建中的 cloud_tree 冲突。
    if sync_engine().ok().map(|e| e.current_state().is_indexing).unwrap_or(false) {
        return Err(AppError::generic("正在读取云端索引，请稍后再试".to_string()));
    }
    match FILES_API.update(&id, Some(&new_name), None, None).await {
        Ok(f) => { tracing::info!(file_id = %id, new_name = %new_name, "重命名成功"); Ok(f) }
        Err(e) => { tracing::warn!(file_id = %id, new_name = %new_name, error = %e, "重命名失败"); Err(e) }
    }
}

#[tauri::command]
pub async fn drive_move_file(id: String, new_parent_folder: String) -> AppResult<DriveFile> {
    // 索引中拒绝：移动改 parentFolder，与重建中的 path_to_id/cloud_tree 冲突。
    if sync_engine().ok().map(|e| e.current_state().is_indexing).unwrap_or(false) {
        return Err(AppError::generic("正在读取云端索引，请稍后再试".to_string()));
    }
    match FILES_API.update(&id, None, Some(&new_parent_folder), None).await {
        Ok(f) => { tracing::info!(file_id = %id, target_folder = %new_parent_folder, "移动成功"); Ok(f) }
        Err(e) => { tracing::warn!(file_id = %id, target_folder = %new_parent_folder, error = %e, "移动失败"); Err(e) }
    }
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
    let dest = std::path::PathBuf::from(&dest_path);
    let progress: crate::drive::download_api::ProgressFn = Box::new(|_, _| {});
    DOWNLOAD_API.download(&file_id, &dest, Some(&progress)).await
}

#[tauri::command]
pub async fn drive_upload_file(
    local_path: String,
    parent_id: Option<String>,
) -> AppResult<DriveFile> {
    let path = std::path::PathBuf::from(&local_path);
    let progress: crate::drive::upload_api::ProgressFn = Box::new(|_| {});
    UPLOAD_API.upload(&path, parent_id.as_deref(), Some(&progress), None).await
}

// ========================================================================
// 三、Sync 同步命令
// ========================================================================

/// 全量刷新云端树 + 同步周期（走 SyncEngine）。
#[tauri::command]
pub async fn sync_manual_refresh(app: AppHandle) -> AppResult<()> {
    let e = sync_engine()?;
    // 直接向前端广播 is_indexing=true（不依赖状态桥接任务，保证手动刷新期间
    // 状态条立刻显示「正在读取云端索引…」）。trigger_manual_sync 内部仍会经桥接
    // 广播一次，二者并存无害；此处直接 emit 是为绕开桥接可能的滞后/死亡。
    {
        let mut st = e.current_state();
        st.is_indexing = true;
        emit_sync_state(&app, &st);
    }
    let result = e.trigger_manual_sync().await;
    // 无论成败都复位 is_indexing（避免状态条卡在索引态）
    {
        let mut st = e.current_state();
        st.is_indexing = false;
        emit_sync_state(&app, &st);
    }
    result
}

/// 查询文件本地同步状态（供前端删除确认用）。
/// 返回 "folder" | "synced" | "placeholder" | "not_synced"
#[tauri::command]
pub fn sync_check_file_local_status(file_id: String) -> AppResult<String> {
    let conn = DB.lock();
    let record = repository::find_by_file_id(&conn, &file_id)
        .ok()
        .flatten();
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
                    .map(|b| String::from_utf8_lossy(&b) == crate::mount::manager::STATE_PLACEHOLDER)
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
        let status = match repository::find_by_file_id(&conn, file_id)
            .ok()
            .flatten()
        {
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
        }.to_string());
    }
    // 引擎未启动：DB 兜底
    let conn = DB.lock();
    if let Ok(Some(record)) = repository::find_by_file_id(&conn, &file_id) {
        let local_path = std::path::Path::new(&record.local_path);
        if local_path.exists() {
            if let Ok(meta) = std::fs::metadata(local_path) {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64);
                if record.local_mtime != mtime || record.local_size != Some(meta.len() as i64) {
                    return Ok("not_synced".to_string());
                }
            }
        }
        Ok("safe".to_string())
    } else {
        Ok("not_synced".to_string())
    }
}

#[tauri::command]
pub async fn sync_free_up_space(
    file_id: String,
    rel_path: String,
    local_path: String,
    _name: String,
    size: i64,
) -> AppResult<()> {
    let m = mount()?;
    let lp = std::path::PathBuf::from(&local_path);

    // 删除本地文件
    if lp.exists() {
        m.delete_local(&lp).await?;
    }

    // 创建占位符
    m.create_placeholder_if_needed(&rel_path, &file_id, size).await?;

    // 更新 DB
    let conn = DB.lock();
    let _ = conn.execute(
        "UPDATE sync_items SET status = ?1, local_size = 0, error_message = NULL WHERE file_id = ?2",
        rusqlite::params![repository::sync_status::CLOUD_ONLY, file_id],
    );

    Ok(())
}

#[tauri::command]
pub async fn sync_download_on_demand(app: AppHandle, file_id: String, dest_path: String) -> AppResult<bool> {
    // 全局索引读取中：禁止按需下载（同 sync_folder_recursive，cloud_tree 构建中）
    if let Some(e) = try_sync_engine() {
        if e.current_state().is_indexing {
            return Err(AppError::generic("正在读取云端索引，请稍后再试".to_string()));
        }
    }
    let dest = PathBuf::from(&dest_path);
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // 查云端真实元数据（editedTime + size）：
    // - editedTime 必须写真实值，写 None 会让 is_cloud_changed 永远判"云端已变"
    //   → watcher 触发的 cycle 重复下载 → 同步循环（恶性 bug 根因）
    // - size 用于传输队列进度条显示（避免 0/0 显示 0%）
    // 云端查询失败时回退查库既有 editedTime；size 缺失则 0（进度条不显示）
    let cloud_file = FILES_API.get(&file_id).await.ok();
    let cloud_edited_time = cloud_file
        .as_ref()
        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis()))
        .or_else(|| {
            let conn = DB.lock();
            repository::find_by_file_id(&conn, &file_id)
                .ok()
                .flatten()
                .and_then(|r| r.cloud_edited_time)
        });
    let cloud_size = cloud_file.as_ref().map(|f| f.size).unwrap_or(0);
    // 入队传输记录（按需下载也纳入传输队列 + 状态条「同步中」）
    let task_id = {
        let conn = DB.lock();
        repository::insert_transfer(&conn, &repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::DOWNLOAD,
            file_id: Some(file_id.clone()),
            local_path: Some(dest_path.clone()),
            name: name.clone(),
            total_size: cloud_size, // 云端真实大小（用于进度条）；查询失败则 0
            transferred: 0,
            state: repository::transfer_state::RUNNING,
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
        }).unwrap_or(0)
    };
    emit_transfer_update(&app); // 入队即通知：传输面板 + 状态条变「同步中」
    let result = download_to_dest(&file_id, &dest, &name, cloud_size, cloud_edited_time).await;
    // 结算传输记录（成功 transferred=total_size，失败保持）
    {
        let conn = DB.lock();
        let (state, transferred_sql) = if result.is_ok() {
            (repository::transfer_state::COMPLETED, "transferred = total_size")
        } else {
            (repository::transfer_state::FAILED, "transferred = transferred")
        };
        let sql = format!(
            "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, {transferred_sql} WHERE id=?4"
        );
        let _ = conn.execute(
            &sql,
            rusqlite::params![state,
                result.as_ref().err().map(|e| e.to_string()).as_deref(),
                chrono::Utc::now().timestamp_millis(),
                task_id],
        );
    }
    emit_transfer_update(&app); // 结算后通知：传输面板 + 状态条刷新
    match result {
        Ok(()) => Ok(true),
        Err(e) => {
            // 清理 .tmp 残留
            let tmp = PathBuf::from(format!("{}.tmp", dest.display()));
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 下载单个文件到 dest（folder sync 与 sync_download_on_demand 共用）：
/// 备份被修改的占位（改名保留）→ 删未修改占位 → 下载 → mark_downloaded → upsert DB。
///
/// `cloud_edited_time`：云端 editedTime（毫秒）。必须传入真实值——写 None 会让
/// `is_cloud_changed` 判定云端已变 → 下一轮重复下载刚下好的文件。folder sync 从
/// DriveFile 传；downloadOnDemand 无 DriveFile 时查库保留既有值。
async fn download_to_dest(
    file_id: &str,
    dest: &Path,
    name: &str,
    size: i64,
    cloud_edited_time: Option<i64>,
) -> AppResult<()> {
    let m = mount()?;
    // 相对挂载根的路径：DB/cloud_tree/scan_local 全用相对路径，绝对路径会与
    // 占位时写入的相对路径记录共存（主键 file_id+local_path），形成孤儿记录
    // 导致 planner 每轮误判（288 振荡根因）。
    let rel = dest
        .strip_prefix(m.mount_dir())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| dest.to_string_lossy().to_string());
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    // 占位被用户修改过 → 改名保留；未修改/非占位 → None
    let _ = m.backup_modified_placeholder_if_needed(dest).await?;
    // 未修改占位（仍存在、0 字节）或残留 → 删除，让下载干净写入
    if dest.exists() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    let progress: crate::drive::download_api::ProgressFn = Box::new(|_, _| {});
    DOWNLOAD_API.download(file_id, dest, Some(&progress)).await?;
    // mark_downloaded（xattr state=downloaded + 清灰标）
    let _ = m.mark_downloaded(dest).await;
    // 补写 fileId xattr：删占位再下载产生新 inode，占位时的 fileId xattr 丢失，
    // reconcile 无法自愈。对齐 dart downloadOnDemand 原地覆盖保留 fileId 的语义。
    let _ = m.set_file_id_xattr(dest, file_id).await;
    // upsert DB（status=synced，相对路径，覆盖旧 cloudOnly 记录）
    let conn = DB.lock();
    let local_size = std::fs::metadata(dest).ok().map(|m| m.len() as i64);
    let local_mtime = std::fs::metadata(dest).ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let _ = repository::upsert(&conn, &repository::SyncItem {
        file_id: file_id.to_string(),
        local_path: rel,
        parent_folder_id: None,
        name: name.to_string(),
        is_folder: false,
        size,
        local_size,
        sha256: None,
        local_mtime,
        cloud_edited_time,
        last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
        status: repository::sync_status::SYNCED,
        error_message: None,
    });
    Ok(())
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
pub async fn sync_folder_recursive(app: AppHandle, folder_id: String, rel_path: String) -> AppResult<i64> {
    let eng = sync_engine()?;
    // 全局索引读取中（云端树 BFS 重建中）：选择目录同步会基于不完整的 cloud_tree/path_to_id，
    // 且与全局 BFS 并发拉取易冲突 → 拒绝，等索引完成。
    if eng.current_state().is_indexing {
        return Err(AppError::generic("正在读取云端索引，请稍后再试".to_string()));
    }
    if !eng.try_begin_folder_sync() {
        tracing::info!("sync_folder_recursive: 已有目录同步进行中，跳过");
        return Ok(0);
    }
    // 后台异步执行：立即返回，不阻塞前端。传输项实时进传输队列（菜单栏/弹窗可见）。
    // spawn 内 finally 释放 folder_syncing 锁 + 广播 contentChanged（前端目录刷新）。
    let eng_clone = eng.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = sync_folder_recursive_impl(&app_clone, &eng_clone, &folder_id, &rel_path).await;
        // 无论成功失败都释放锁（防 panic 泄漏 → 放在 spawn 任务收尾，确定性释放）
        eng_clone.end_folder_sync();
        // 完成后广播 contentChanged 让前端目录刷新
        {
            let mut st = eng_clone.current_state();
            st.content_changed = true;
            st.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            emit_sync_state(&app_clone, &st);
        }
        match &result {
            Ok(done) => tracing::info!(done, rel = %rel_path, "sync_folder_recursive（后台）完成"),
            Err(e) => tracing::warn!(error = %e, rel = %rel_path, "sync_folder_recursive（后台）失败"),
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
    let dest_dir = m.mount_dir().join(rel_path);
    tracing::info!(folder_id, rel = %rel_path, "sync_folder_recursive: 开始递归同步");

    // 1. BFS 子树：listAll 递归
    let mut cloud_files: Vec<(String, DriveFile)> = Vec::new(); // (subrel, DriveFile)
    let mut cloud_folders: Vec<String> = Vec::new();
    let mut folder_rel_to_id: HashMap<String, String> = HashMap::new();
    folder_rel_to_id.insert(String::new(), folder_id.to_string());
    let mut queue: Vec<(String, String)> = vec![(folder_id.to_string(), String::new())];
    while !queue.is_empty() {
        let (id, path) = queue.remove(0);
        let children = FILES_API.list_all(Some(id.as_str())).await?;
        for f in children {
            if f.name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                continue;
            }
            let subrel = if path.is_empty() { f.name.clone() } else { format!("{path}/{}", f.name) };
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
        let _ = tokio::fs::create_dir_all(dest_dir.join(sub)).await;
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
    let cloud_map: HashMap<String, DriveFile> =
        cloud_files.iter().map(|(r, f)| (r.clone(), f.clone())).collect();
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
        let mut missing_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
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
            match FILES_API.create_folder(dir_name, parent_id).await {
                Ok(f) => {
                    folder_rel_to_id.insert(dir_rel.clone(), f.id.clone());
                    // 本地也建目录（确保后续 scan 能看到）
                    let _ = tokio::fs::create_dir_all(dest_dir.join(dir_rel)).await;
                    tracing::info!(dir = %dir_rel, cloud_id = %f.id, "sync_folder_recursive: 已补建云端父目录");
                }
                Err(e) => {
                    // 409/400 时查同名已存在目录，存在则视为成功
                    let msg = e.to_string();
                    if msg.contains("400") || msg.contains("409") {
                        if let Some(pid) = parent_id {
                            if let Ok(list) = FILES_API.list_all(Some(pid)).await {
                                if let Some(existing) = list.iter().find(|c| c.is_folder() && c.name == dir_name) {
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
        let dest = dest_dir.join(subrel);
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        // 入队传输记录（双端对齐要求所有文件操作出现在传输队列）
        let task_id = {
            let conn = DB.lock();
            repository::insert_transfer(&conn, &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::DOWNLOAD,
                file_id: Some(f.id.clone()),
                local_path: Some(dest.to_string_lossy().to_string()),
                name: full_rel.clone(),
                total_size: f.size,
                transferred: 0,
                state: repository::transfer_state::RUNNING,
                error_message: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
            }).unwrap_or(0)
        };
        // 入队即通知：前端传输面板 + 托盘菜单立即显示新下载项
        emit_transfer_update(app);
        crate::platform::tray::refresh_menu(app);
        let download_result = download_to_dest(
            &f.id, &dest, &f.name, f.size,
            f.edited_time.map(|t| t.timestamp_millis()),
        ).await;
        // 结算传输记录（成功时 transferred=total_size，失败时保持原值）
        {
            let conn = DB.lock();
            let (state, transferred_sql) = if download_result.is_ok() {
                (repository::transfer_state::COMPLETED, "transferred = total_size")
            } else {
                (repository::transfer_state::FAILED, "transferred = transferred")
            };
            let sql = format!(
                "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, {transferred_sql} WHERE id=?4"
            );
            let _ = conn.execute(
                &sql,
                rusqlite::params![state,
                    download_result.as_ref().err().map(|e| e.to_string()).as_deref(),
                    chrono::Utc::now().timestamp_millis(),
                    task_id],
            );
        }
        // 结算后通知：传输面板 + 托盘菜单刷新（状态变化/消失）
        emit_transfer_update(app);
        crate::platform::tray::refresh_menu(app);
        if let Err(e) = download_result {
            tracing::warn!(subrel = %subrel, error = %e, "sync_folder_recursive: 下载失败");
        }
        done += 1;
        let _ = app.emit("folder_sync_progress", FolderSyncProgress { done: done as usize, total });
    }
    // 7. 上传
    for (subrel, local_path) in &to_upload {
        let parent_subrel = parent_subrel_of(subrel);
        let parent_id = folder_rel_to_id.get(&parent_subrel).map(|s| s.as_str()).unwrap_or(folder_id);
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        let file_size = local_path.metadata().map(|m| m.len() as i64).unwrap_or(0);
        // 入队传输记录
        let task_id = {
            let conn = DB.lock();
            repository::insert_transfer(&conn, &repository::TransferTask {
                id: 0,
                direction: repository::transfer_direction::UPLOAD,
                file_id: None,
                local_path: Some(local_path.to_string_lossy().to_string()),
                name: full_rel.clone(),
                total_size: file_size,
                transferred: 0,
                state: repository::transfer_state::RUNNING,
                error_message: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
            }).unwrap_or(0)
        };
        // 入队即通知：前端传输面板 + 托盘菜单立即显示新上传项
        emit_transfer_update(app);
        crate::platform::tray::refresh_menu(app);
        // 进度回调：节流写 transferred + 通知刷新（按 task_id 更新，500ms 节流）
        let prog_db = DB.clone();
        let prog_task_id = task_id;
        let prog_throttle = std::sync::atomic::AtomicI64::new(0);
        let on_progress: crate::drive::upload_api::ProgressFn = Box::new(move |ratio: f64| {
            emit_transfer_progress(&prog_db, prog_task_id, ratio, &prog_throttle);
        });
        let upload_result = UPLOAD_API.upload(local_path, Some(parent_id), Some(&on_progress), None).await;
        // 结算传输记录（成功时 transferred=total_size，失败时保持原值）
        {
            let conn = DB.lock();
            let (state, transferred_sql) = if upload_result.is_ok() {
                (repository::transfer_state::COMPLETED, "transferred = total_size")
            } else {
                (repository::transfer_state::FAILED, "transferred = transferred")
            };
            let sql = format!(
                "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, {transferred_sql} WHERE id=?4"
            );
            let _ = conn.execute(
                &sql,
                rusqlite::params![state,
                    upload_result.as_ref().err().map(|e| e.to_string()).as_deref(),
                    chrono::Utc::now().timestamp_millis(),
                    task_id],
            );
        }
        // 结算后通知：传输面板 + 托盘菜单刷新（状态变化/消失）
        emit_transfer_update(app);
        crate::platform::tray::refresh_menu(app);
        match upload_result {
            Ok(uploaded) => {
                eng.cloud_tree_insert(full_rel.clone(), uploaded.clone());
                eng.path_to_id_insert(full_rel.clone(), uploaded.id.clone());
                let conn = DB.lock();
                let local_size = local_path.metadata().map(|m| m.len() as i64).ok();
                let local_mtime = local_path
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64);
                // DB local_path 用相对路径 full_rel（与 cloud_tree/path_to_id 一致），
                // 绝对路径会与占位记录共存成孤儿（288 振荡根因）。
                let _ = repository::upsert(&conn, &repository::SyncItem {
                    file_id: uploaded.id,
                    local_path: full_rel,
                    parent_folder_id: Some(parent_id.to_string()),
                    name: uploaded.name,
                    is_folder: false,
                    size: uploaded.size,
                    local_size,
                    sha256: None,
                    local_mtime,
                    cloud_edited_time: uploaded.edited_time.map(|t| t.timestamp_millis()),
                    last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                    status: repository::sync_status::SYNCED,
                    error_message: None,
                });
            }
            Err(e) => tracing::warn!(subrel = %subrel, error = %e, "sync_folder_recursive: 上传失败"),
        }
        done += 1;
        let _ = app.emit("folder_sync_progress", FolderSyncProgress { done: done as usize, total });
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
            let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();
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
    // 引擎已启动 → 返回引擎聚合状态（含实时计数）
    if let Some(e) = try_sync_engine() {
        return Ok(e.current_state());
    }
    // 引擎未启动：DB 兜底
    let conn = DB.lock();
    let total: u64 = conn
        .query_row("SELECT COUNT(*) FROM sync_items", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64;
    let failed: u64 = conn
        .query_row("SELECT COUNT(*) FROM sync_items WHERE status = 4", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64;
    let conflict: u64 = conn
        .query_row("SELECT COUNT(*) FROM sync_items WHERE status = 5", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as u64;

    // 失败项详情（最多 20 条，供 SyncStatusBar 失败项弹窗）
    let failed_items: Vec<FailedItem> = {
        let mut stmt = conn
            .prepare("SELECT local_path, error_message FROM sync_items WHERE status = 4 LIMIT 20")
            .map_err(|e| AppError::generic(format!("查询失败项失败：{e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FailedItem {
                    relative_path: row.get::<_, String>(0)?,
                    error_message: row.get::<_, Option<String>>(1)?,
                })
            })
            .map_err(|e| AppError::generic(format!("查询失败项失败：{e}")))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    Ok(SyncGlobalState {
        total,
        completed: total - failed - conflict,
        failed,
        failed_items,
        conflict,
        last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
        ..Default::default()
    })
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
    let dir_changed = old_configured && config.mount_configured && old_abs.as_ref() != Some(&new_abs);

    ConfigStore::save(&config)?;

    // 换目录 / 取消配置：清缓存 + relaunch（setup 按新 config 重建引擎+watcher）
    if old_configured && (!config.mount_configured || dir_changed) {
        if let Some(old_abs) = old_abs {
            crate::core::cache_paths::clear_cache_files(&old_abs.to_string_lossy());
        }
        crate::core::cache_paths::clear_cache_files(&new_abs.to_string_lossy());
        drop_runtime();
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

#[tauri::command]
pub fn transfer_clear_completed() -> AppResult<()> {
    let conn = DB.lock();
    conn.execute(
        "DELETE FROM transfer_queue WHERE state = ?1",
        rusqlite::params![repository::transfer_state::COMPLETED],
    )
    .map_err(|e| AppError::generic(format!("清除已完成传输失败：{e}")))?;
    Ok(())
}

#[tauri::command]
pub fn transfer_clear_failed() -> AppResult<()> {
    let conn = DB.lock();
    conn.execute(
        "DELETE FROM transfer_queue WHERE state = ?1",
        rusqlite::params![repository::transfer_state::FAILED],
    )
    .map_err(|e| AppError::generic(format!("清除失败传输失败：{e}")))?;
    Ok(())
}

#[tauri::command]
pub fn transfer_clear_finished() -> AppResult<()> {
    let conn = DB.lock();
    conn.execute(
        "DELETE FROM transfer_queue WHERE state IN (?1, ?2)",
        rusqlite::params![
            repository::transfer_state::COMPLETED,
            repository::transfer_state::FAILED
        ],
    )
    .map_err(|e| AppError::generic(format!("清除已结束传输失败：{e}")))?;
    Ok(())
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
    drop_runtime();
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
    files.sort(); // 按文件名升序（日期 oldest-first）
    let mut out = String::new();
    for f in files {
        if let Ok(content) = std::fs::read_to_string(&f) {
            use std::fmt::Write;
            let _ = writeln!(out, "===== {} =====", f.display());
            out.push_str(&content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    if out.is_empty() {
        return Err(AppError::generic("日志目录为空，无可导出内容"));
    }
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

/// 节流写传输进度并触发前端刷新（供双端对齐等非 executor 路径的同步进度回调用）。
///
/// - `db`：传输队列 DB 连接（全局）
/// - `task_id`：transfer_queue 行 id
/// - `ratio`：进度比例（0.0-1.0）
/// - `last_emit_ms`：节流状态（AtomicI64，500ms 节流）
///
/// 与 executor::emit_throttled_progress 同构，但按 task_id 更新（双端对齐串行执行）。
fn emit_transfer_progress(
    db: &Arc<Mutex<rusqlite::Connection>>,
    task_id: i64,
    ratio: f64,
    last_emit_ms: &std::sync::atomic::AtomicI64,
) {
    use std::sync::atomic::Ordering;
    const PROGRESS_THROTTLE_MS: i64 = 500;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let last = last_emit_ms.load(Ordering::Relaxed);
    if last != 0 && now - last < PROGRESS_THROTTLE_MS {
        return;
    }
    last_emit_ms.store(now, Ordering::Relaxed);
    // 写 transferred = ratio * total_size（查 total_size 防越界）
    {
        let conn = db.lock();
        let total: i64 = conn
            .query_row(
                "SELECT total_size FROM transfer_queue WHERE id=?1",
                rusqlite::params![task_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if total > 0 {
            let transferred = (ratio * total as f64) as i64;
            let _ = conn.execute(
                "UPDATE transfer_queue SET transferred=?1 WHERE id=?2",
                rusqlite::params![transferred, task_id],
            );
        }
    }
    // 通过全局 tx 触发前端 + 托盘菜单刷新（回调拿不到 AppHandle，故走全局 tx）
    if let Some(tx) = TRANSFER_UPDATE_TX.lock().as_ref() {
        let _ = tx.send(());
    }
}

/// 推送传输队列变更通知
pub fn emit_transfer_update(app: &AppHandle) {
    let _ = app.emit("transfer_update", ());
    // 同时刷新状态条 uploading/downloading 计数：重算 transfer_queue RUNNING 数并推送 sync_state。
    // 统一在此刷新，覆盖所有传输场景（自动同步 executor、双端对齐、手动下载），
    // 无需每个调用点单独处理。
    if let Some(e) = try_sync_engine() {
        e.push_live_transfer_state();
    }
}
