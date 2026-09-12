//! Tauri 命令层运行时与领域命令入口。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tauri::AppHandle;
use tauri_specta::Event;

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
#[cfg(target_os = "linux")]
use crate::virtual_fs::{FuseMountSession, MutationCoordinator, PathLease, VirtualDriveConfig};

/// 认证相关命令。
mod auth;
/// 配置读写命令。
mod config;
/// 云盘文件操作命令。
pub(crate) mod drive;
/// 目录递归同步命令。
mod folder_sync;
/// 本地空间释放与按需下载命令。
mod free_up;
/// 拖拽导入命令。
mod import_files;
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
pub use import_files::*;
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

/// Linux 按需云盘的 FUSE 会话。必须先于 SyncEngine 停止，避免正在读取的请求失去下载服务。
#[cfg(target_os = "linux")]
static VIRTUAL_FS_SESSION: Mutex<Option<FuseMountSession>> = Mutex::new(None);

/// 最近一次 Linux 按需云盘启动错误，供文件管理器入口给出可操作反馈。
#[cfg(target_os = "linux")]
static VIRTUAL_FS_LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// 串行协调同步引擎安装与替换，防止两个生命周期交叠。
struct EngineOwnershipProtocol {
    gate: Mutex<()>,
    replacement_gate: tokio::sync::Mutex<()>,
    replacements: AtomicUsize,
}

impl EngineOwnershipProtocol {
    /// 创建尚无替换操作的生命周期协调器。
    const fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            replacement_gate: tokio::sync::Mutex::const_new(()),
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

/// 统计非终态传输任务数（Pending/Running/WaitingForNetwork/BackingOff/VerifyingRemote/RestartRequired）。
/// 用于「清空缓存」「退出登录」等高风险操作前的活动传输守卫；终态 Completed/Failed/Canceled 不计入。
pub(crate) fn active_transfer_count() -> AppResult<i64> {
    let conn = DB.lock();
    conn.query_row(
        "SELECT COUNT(*) FROM transfer_queue WHERE state NOT IN (?1, ?2, ?3)",
        rusqlite::params![
            repository::transfer_state::COMPLETED,
            repository::transfer_state::FAILED,
            repository::transfer_state::CANCELED
        ],
        |row| row.get(0),
    )
    .map_err(|e| AppError::generic(format!("查询活动传输任务失败：{e}")))
}

/// 返回当前真实挂载中的按需云盘入口；仅凭配置开启不算可用。
#[cfg(target_os = "linux")]
pub fn virtual_mountpoint() -> Option<std::path::PathBuf> {
    let mountpoint = VIRTUAL_FS_SESSION
        .lock()
        .as_ref()
        .map(|session| session.mountpoint().to_path_buf())?;
    match crate::core::config_store::is_active_petallink_mount(&mountpoint) {
        Ok(true) => Some(mountpoint),
        Ok(false) => {
            *VIRTUAL_FS_LAST_ERROR.lock() =
                Some("FUSE 会话已退出或挂载已断开，请重启 PetalLink".to_string());
            None
        }
        Err(error) => {
            *VIRTUAL_FS_LAST_ERROR.lock() = Some(error.to_string());
            None
        }
    }
}

/// 返回最近一次按需云盘启动失败原因。
#[cfg(target_os = "linux")]
pub fn virtual_mount_error() -> Option<String> {
    VIRTUAL_FS_LAST_ERROR.lock().clone()
}

/// 非 Linux 平台没有 FUSE 用户可见目录。
#[cfg(not(target_os = "linux"))]
pub fn virtual_mountpoint() -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn virtual_mount_error() -> Option<String> {
    None
}

/// 把同步引擎的路径活动门适配为 FUSE 句柄租约。
#[cfg(target_os = "linux")]
struct EnginePathLease {
    guards: Option<Vec<crate::sync::engine::ActivityGuard>>,
    engine: Arc<SyncEngine>,
    runtime: tokio::runtime::Handle,
    notify_local_change: bool,
}

/// FUSE 回调运行在 libfuse 工作线程，那里没有 Tokio reactor。路径租约可以在这些
/// 线程析构，因此通知必须先投递到应用运行时，再进入 SyncEngine 的后台周期调度器。
#[cfg(target_os = "linux")]
fn dispatch_virtual_drive_change(
    runtime: &tokio::runtime::Handle,
    engine: Arc<SyncEngine>,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        engine.notify_virtual_drive_change();
    })
}

#[cfg(target_os = "linux")]
impl Drop for EnginePathLease {
    fn drop(&mut self) {
        // 必须先释放路径租约，再请求重扫；否则新周期会立刻被本租约挡回。
        drop(self.guards.take());
        if self.notify_local_change {
            // Handle::spawn 可以从非 Tokio 的 FUSE 工作线程调用；任务真正执行时已
            // 进入应用 runtime，SyncEngine 内部的 tokio::spawn 因而有 reactor。
            drop(dispatch_virtual_drive_change(
                &self.runtime,
                self.engine.clone(),
            ));
        }
    }
}

/// FUSE 与同步执行器共用的路径级共享/排他协调器。
#[cfg(target_os = "linux")]
struct EngineMutationCoordinator {
    engine: Arc<SyncEngine>,
    backing_root: std::path::PathBuf,
    runtime: tokio::runtime::Handle,
}

#[cfg(target_os = "linux")]
impl EngineMutationCoordinator {
    fn new(
        engine: Arc<SyncEngine>,
        backing_root: std::path::PathBuf,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            engine,
            backing_root,
            runtime,
        }
    }

    async fn acquire(
        &self,
        paths: Vec<std::path::PathBuf>,
        exclusive: bool,
    ) -> std::io::Result<Box<dyn PathLease>> {
        if paths.is_empty() {
            return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
        }
        let paths = paths
            .into_iter()
            .map(|path| {
                path.to_str()
                    .map(str::to_string)
                    .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EILSEQ))
            })
            .collect::<std::io::Result<Vec<_>>>()?;

        loop {
            if self.engine.is_shutting_down() {
                return Err(std::io::Error::from_raw_os_error(libc::EINTR));
            }
            let mut guards = Vec::with_capacity(paths.len());
            let mut busy = false;
            for path in &paths {
                let acquired = if exclusive {
                    self.engine.begin_exclusive_path_activity(path)
                } else {
                    self.engine.begin_path_activity(path)
                };
                match acquired {
                    Ok(guard) => guards.push(guard),
                    Err(_) => {
                        busy = true;
                        break;
                    }
                }
            }
            if !busy {
                return Ok(Box::new(EnginePathLease {
                    guards: Some(guards),
                    engine: self.engine.clone(),
                    runtime: self.runtime.clone(),
                    notify_local_change: exclusive,
                }));
            }
            // 部分取得的租约先全部释放；短暂退让后按相同顺序重试，避免 ABBA。
            drop(guards);
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// 在 backing rename 前核验 DB/云端目标碰撞，并把稳定身份写在源 inode 上。
    fn prepare_rename_identity(
        &self,
        source: &std::path::Path,
        destination: &std::path::Path,
        is_directory: bool,
    ) -> std::io::Result<()> {
        let (Some(source), Some(destination)) = (source.to_str(), destination.to_str()) else {
            return Err(std::io::Error::from_raw_os_error(libc::EILSEQ));
        };
        let (source_record, destination_record) = {
            let conn = DB.lock();
            let records = repository::load_all(&conn).map_err(|error| {
                tracing::warn!(%error, source, "读取 rename 身份失败");
                std::io::Error::from_raw_os_error(libc::EIO)
            })?;
            let source_records = records
                .iter()
                .filter(|record| record.local_path == source)
                .collect::<Vec<_>>();
            let destination_records = records
                .iter()
                .filter(|record| record.local_path == destination)
                .collect::<Vec<_>>();
            if source_records.len() > 1 || destination_records.len() > 1 {
                tracing::warn!(
                    source,
                    destination,
                    source_records = source_records.len(),
                    destination_records = destination_records.len(),
                    "rename 路径对应多条同步基线，拒绝猜测身份"
                );
                return Err(std::io::Error::from_raw_os_error(libc::EIO));
            }
            (
                source_records.first().map(|record| (*record).clone()),
                destination_records.first().map(|record| (*record).clone()),
            )
        };
        if source_record
            .as_ref()
            .is_some_and(|record| record.is_folder != is_directory)
        {
            return Err(std::io::Error::from_raw_os_error(libc::EIO));
        }

        let source_path = crate::core::paths::safe_join_under(&self.backing_root, source, false)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let destination_path =
            crate::core::paths::safe_join_under(&self.backing_root, destination, false)
                .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let source_xattr =
            crate::platform::xattr::get(&source_path, crate::mount::manager::XATTR_FILE_ID)?;
        let source_cloud_id = self.engine.path_to_id_lock().get(source).cloned();
        let source_identity = source_record
            .as_ref()
            .map(|record| record.file_id.clone())
            .or(source_cloud_id)
            .or_else(|| {
                source_xattr
                    .as_ref()
                    .and_then(|owner| String::from_utf8(owner.clone()).ok())
            });
        if let (Some(record), Some(owner)) = (source_identity.as_deref(), source_xattr.as_deref()) {
            if owner != record.as_bytes() {
                return Err(std::io::Error::from_raw_os_error(libc::EIO));
            }
        }

        let destination_cloud_id = self.engine.path_to_id_lock().get(destination).cloned();
        let destination_identity = destination_record
            .as_ref()
            .map(|record| record.file_id.clone())
            .or(destination_cloud_id);
        if let Some(destination_identity) = destination_identity.as_deref() {
            match source_identity.as_deref() {
                Some(source_identity) if source_identity == destination_identity => {}
                Some(_) => return Err(std::io::Error::from_raw_os_error(libc::EEXIST)),
                None => {
                    // 无云端身份的临时普通文件覆盖一个真实本地目标是编辑器原子保存；
                    // 目标在本地已消失却仍被 DB/云索引占用时则拒绝碰撞。
                    let destination_is_local_file = std::fs::symlink_metadata(&destination_path)
                        .is_ok_and(|metadata| metadata.is_file());
                    if is_directory || !destination_is_local_file {
                        return Err(std::io::Error::from_raw_os_error(libc::EEXIST));
                    }
                }
            }
        }

        if source_xattr.is_none() {
            if let Some(source_identity) = source_identity {
                crate::platform::xattr::set(
                    &source_path,
                    crate::mount::manager::XATTR_FILE_ID,
                    source_identity.as_bytes(),
                )?;
                let written = crate::platform::xattr::get(
                    &source_path,
                    crate::mount::manager::XATTR_FILE_ID,
                )?;
                if written.as_deref() != Some(source_identity.as_bytes()) {
                    return Err(std::io::Error::from_raw_os_error(libc::EIO));
                }
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl MutationCoordinator for EngineMutationCoordinator {
    async fn acquire_shared(
        &self,
        paths: Vec<std::path::PathBuf>,
    ) -> std::io::Result<Box<dyn PathLease>> {
        self.acquire(paths, false).await
    }

    async fn acquire_exclusive(
        &self,
        paths: Vec<std::path::PathBuf>,
    ) -> std::io::Result<Box<dyn PathLease>> {
        self.acquire(paths, true).await
    }

    async fn prepare_rename(
        &self,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        is_directory: bool,
    ) -> std::io::Result<()> {
        self.prepare_rename_identity(&source, &destination, is_directory)
    }

    async fn rename_observed(
        &self,
        _source: std::path::PathBuf,
        _destination: std::path::PathBuf,
        _is_directory: bool,
    ) -> std::io::Result<()> {
        // backing rename 成功后，exclusive EnginePathLease 析构会在释放路径门禁后统一
        // 请求一次本地重扫。这里不重复通知，避免同一个 rename 排入两个周期。
        Ok(())
    }
}

/// 为已经装配好任务队列的引擎创建 Linux FUSE 会话。
#[cfg(target_os = "linux")]
fn create_virtual_fs_session(
    config: &crate::core::config::AppConfig,
    engine: Arc<SyncEngine>,
    mount: Arc<MountManager>,
) -> AppResult<Option<FuseMountSession>> {
    if !config.virtual_drive_enabled {
        return Err(AppError::config(
            "Linux 仅支持按需云盘，拒绝以传统同步模式启动".to_string(),
        ));
    }
    crate::core::config_store::validate_virtual_drive_capabilities(config)?;
    let hydrator = Arc::new(crate::sync::hydration::HydrationService::new(
        engine.clone(),
        mount,
        DB.clone(),
        FILES_API.clone(),
    ));
    let backing_root = config.expanded_mount_dir();
    let runtime = tauri::async_runtime::handle().inner().clone();
    let coordinator = Arc::new(EngineMutationCoordinator::new(
        engine,
        backing_root.clone(),
        runtime.clone(),
    ));
    let session = crate::virtual_fs::mount_virtual_drive(VirtualDriveConfig::new(
        backing_root,
        config.expanded_virtual_mount_dir(),
        runtime,
        hydrator,
        coordinator,
    ))
    .map_err(|error| AppError::generic(format!("按需云盘挂载失败：{error}")))?;
    tracing::info!(
        mountpoint = %session.mountpoint().display(),
        backing = %config.expanded_mount_dir().display(),
        "Linux 按需云盘已挂载"
    );
    Ok(Some(session))
}

/// 仅在目标引擎仍是当前实例时发布 FUSE 会话，防止启动与配置替换交错。
#[cfg(target_os = "linux")]
fn install_virtual_fs_session(
    config: &crate::core::config::AppConfig,
    engine: Arc<SyncEngine>,
    mount: Arc<MountManager>,
) -> AppResult<()> {
    ENGINE_OWNERSHIP.install(|| {
        let is_current = SYNC_ENGINE
            .lock()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &engine));
        if !is_current {
            return Err(AppError::generic("同步引擎已被替换，取消按需云盘挂载"));
        }
        if VIRTUAL_FS_SESSION.lock().is_some() {
            return Ok(());
        }
        let session = create_virtual_fs_session(config, engine, mount)?;
        *VIRTUAL_FS_SESSION.lock() = session;
        *VIRTUAL_FS_LAST_ERROR.lock() = None;
        Ok(())
    })
}

/// 取出并优雅卸载 FUSE 会话。调用时不持有全局锁，避免卸载等待期间阻塞状态查询。
#[cfg(target_os = "linux")]
fn stop_virtual_fs() {
    let session = VIRTUAL_FS_SESSION.lock().take();
    if let Some(mut session) = session {
        let mountpoint = session.mountpoint().to_path_buf();
        match session.unmount() {
            Ok(()) => tracing::info!(mountpoint = %mountpoint.display(), "Linux 按需云盘已卸载"),
            Err(error) => {
                tracing::error!(mountpoint = %mountpoint.display(), %error, "Linux 按需云盘正常卸载失败，尝试安全 detach");
                match crate::core::config_store::detach_owned_petallink_mount(&mountpoint) {
                    Ok(true) => {
                        tracing::warn!(mountpoint = %mountpoint.display(), "已安全 detach 当前 PetalLink FUSE 挂载")
                    }
                    Ok(false) => tracing::debug!(
                        mountpoint = %mountpoint.display(),
                        "卸载失败后挂载记录已消失，或目标已不再是 PetalLink"
                    ),
                    Err(detach_error) => tracing::error!(
                        mountpoint = %mountpoint.display(),
                        error = %detach_error,
                        "PetalLink FUSE 安全 detach 失败"
                    ),
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn stop_virtual_fs() {}

/// 异步生命周期中把可能等待 FUSE worker 退出的卸载放到阻塞线程池。
async fn stop_virtual_fs_async() {
    if let Err(error) = tokio::task::spawn_blocking(stop_virtual_fs).await {
        tracing::error!(%error, "等待按需云盘卸载线程失败");
    }
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
        Some(false),
        Some(String::new()),
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
    let _replacement = ENGINE_OWNERSHIP.begin_replacement();
    let engine = {
        let _gate = ENGINE_OWNERSHIP.gate.lock();
        let engine = SYNC_ENGINE.lock().take();
        *MOUNT_MANAGER.lock() = None;
        engine
    };
    // 先关闭新的传输准入，再卸载可能仍在接收 read/write 的 FUSE 入口。
    if let Some(engine) = &engine {
        engine.shutdown_sync();
    }
    stop_virtual_fs();
}

/// 异步释放运行时，并等待旧同步周期与已提交任务完成结算。
pub async fn drop_runtime_async() {
    let _replacement_operation = ENGINE_OWNERSHIP.replacement_gate.lock().await;
    let _replacement = ENGINE_OWNERSHIP.begin_replacement();
    let engine = {
        let _gate = ENGINE_OWNERSHIP.gate.lock();
        let engine = SYNC_ENGINE.lock().take();
        *MOUNT_MANAGER.lock() = None;
        engine
    };
    // 先阻止新 IPC/FUSE 请求进入 TaskRunner；已经登记的传输仍可完成并持久化结算。
    if let Some(engine) = &engine {
        engine.shutdown_sync();
    }
    stop_virtual_fs_async().await;
    if let Some(engine) = engine {
        engine.shutdown().await;
    }
}

/// 在同一替换事务中关闭旧引擎，并以指定挂载目录安装新引擎。
pub(crate) async fn replace_runtime_with_mount(
    app: &AppHandle,
    mount: Arc<MountManager>,
) -> AppResult<()> {
    mount.ensure_mount_dir()?;
    let _replacement_operation = ENGINE_OWNERSHIP.replacement_gate.lock().await;
    let _replacement = ENGINE_OWNERSHIP.begin_replacement();
    let old_engine = {
        let _gate = ENGINE_OWNERSHIP.gate.lock();
        let engine = SYNC_ENGINE.lock().take();
        *MOUNT_MANAGER.lock() = None;
        engine
    };
    // FUSE 仍可能持有引擎的路径租约；先关闭新准入并卸载入口，再等待旧任务结算。
    if let Some(engine) = &old_engine {
        engine.shutdown_sync();
    }
    stop_virtual_fs_async().await;
    if let Some(engine) = old_engine {
        engine.shutdown().await;
    }
    {
        let _gate = ENGINE_OWNERSHIP.gate.lock();
        *MOUNT_MANAGER.lock() = Some(mount);
        ensure_engine_started_owned(app)
    }
}

/// 在同一所有权门禁内安装挂载管理器，并按持久化配置补齐缺失的同步引擎。
pub(crate) fn install_runtime_with_mount(
    app: &AppHandle,
    mount: Arc<MountManager>,
) -> AppResult<()> {
    mount.ensure_mount_dir()?;
    ENGINE_OWNERSHIP.install(|| {
        *MOUNT_MANAGER.lock() = Some(mount);
        ensure_engine_started_owned(app)
    })
}

/// 收束状态后通过 Tauri 的平台安全路径重启。
///
/// Tauri 会在 Linux 解析 AppImage 原始路径，并在 macOS 按 `Info.plist`
/// 解析 bundle 内的真实可执行文件；统一走核心实现可保持更新前后的平台语义。
pub fn relaunch(app: &AppHandle) {
    crate::platform::activation::mark_restarting();
    app.request_restart();
}

/// 若已配置同步目录且引擎未启动，则构造并启动 SyncEngine + 状态桥接。
/// setup 与 config_save（首次配置）共用。
pub fn ensure_engine_started(app: &AppHandle) -> AppResult<()> {
    let result = ENGINE_OWNERSHIP.install(|| ensure_engine_started_owned(app));
    #[cfg(target_os = "linux")]
    if let Err(error) = &result {
        // 配置能力探测、MountManager 或 TaskRunner 若在异步启动前失败，也必须进入
        // 挂载状态快照；否则前端只能永久显示“正在启动”且没有失败原因。
        *VIRTUAL_FS_LAST_ERROR.lock() = Some(error.to_string());
        emit_virtual_drive_status(app);
    }
    result
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
    #[cfg(target_os = "linux")]
    crate::core::config_store::validate_virtual_drive_capabilities(&config)?;
    #[cfg(not(target_os = "linux"))]
    {
        crate::core::config_store::validate_configured_mount_dir_access(&config)?;
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
    // 注入 AppHandle：上传结算为终态失败时 emit upload_failed → 前端弹 toast
    executor.set_app_handle(app.clone());
    let task_runner = executor.initialize_task_runner()?;
    // 传输进度实时推送监听器
    {
        let app_for_transfer = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match transfer_update_rx.recv().await {
                    Ok(()) => {
                        let _ = crate::ipc::TransferUpdateEvent.emit(&app_for_transfer);
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
    #[cfg(target_os = "linux")]
    let virtual_config = config.clone();
    #[cfg(target_os = "linux")]
    let virtual_mount = mount.clone();
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
            #[cfg(target_os = "linux")]
            if virtual_config.virtual_drive_enabled {
                *VIRTUAL_FS_LAST_ERROR.lock() =
                    Some(format!("同步引擎启动失败，按需云盘未挂载：{e}"));
                emit_virtual_drive_status(&app_handle);
            }
        } else {
            tracing::info!("SyncEngine 启动完成 ✓");
            #[cfg(target_os = "linux")]
            if virtual_config.virtual_drive_enabled {
                if let Err(error) =
                    install_virtual_fs_session(&virtual_config, eng.clone(), virtual_mount.clone())
                {
                    let message = error.to_string();
                    *VIRTUAL_FS_LAST_ERROR.lock() = Some(message.clone());
                    tracing::error!(
                        error = %message,
                        "按需云盘启动失败；私有 backing 不会作为用户目录暴露"
                    );
                }
                emit_virtual_drive_status(&app_handle);
            }
        }
        // 桥接任务独立运行，直到引擎释放。
    });
    Ok(())
}

/// 推送同步状态到前端（Tauri event）
pub fn emit_sync_state(app: &AppHandle, state: &SyncGlobalState) {
    let _ = crate::ipc::SyncStateEvent(state.clone()).emit(app);
}

/// 推送目录内容变更通知（触发前端 folderChildren 刷新）
pub fn emit_folder_content_changed(app: &AppHandle) {
    let _ = crate::ipc::FolderContentChangedEvent.emit(app);
}

/// 覆盖同步引擎安装与替换互斥边界的核心合同。
#[cfg(test)]
mod engine_ownership_tests {
    use super::EngineOwnershipProtocol;

    /// 活动替换事务存在时，普通安装必须被拒绝。
    #[test]
    fn active_replacement_blocks_installation() {
        let protocol = EngineOwnershipProtocol::new();
        let replacement = protocol.begin_replacement();

        assert!(protocol.install(|| Ok(())).is_err());

        drop(replacement);
        assert!(protocol.install(|| Ok(())).is_ok());
    }

    /// 异步替换门必须串行覆盖完整的关闭与重新安装区间。
    #[tokio::test]
    async fn replacement_operations_are_serialized() {
        let protocol = EngineOwnershipProtocol::new();
        let first_operation = protocol.replacement_gate.lock().await;

        assert!(protocol.replacement_gate.try_lock().is_err());

        drop(first_operation);
        assert!(protocol.replacement_gate.try_lock().is_ok());
    }
}

/// 推送 Linux 按需云盘的真实挂载状态。
#[cfg(target_os = "linux")]
pub fn emit_virtual_drive_status(app: &AppHandle) {
    match platform::virtual_drive_status() {
        Ok(status) => {
            let _ = crate::ipc::VirtualDriveStatusEvent(status).emit(app);
        }
        Err(error) => tracing::warn!(%error, "读取按需云盘状态失败，未发送状态事件"),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::EnginePathLease;

    #[cfg(target_os = "linux")]
    #[test]
    fn fuse_lease_drop_dispatches_sync_notification_from_non_tokio_thread() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        use parking_lot::Mutex;
        use rusqlite::Connection;

        let runtime = tokio::runtime::Runtime::new().expect("创建测试 runtime 失败");
        let auth = Arc::new(crate::auth::service::AuthService::new());
        let client = Arc::new(crate::drive::client::DriveClient::new(auth));
        let files_api = Arc::new(crate::drive::files_api::FilesApi::new(client.clone()));
        let changes_api = Arc::new(crate::drive::changes_api::ChangesApi::new(client));
        let online_checks = Arc::new(AtomicUsize::new(0));
        let observed_checks = online_checks.clone();
        let mut engine = crate::sync::engine::SyncEngine::new(
            files_api,
            changes_api,
            Arc::new(Mutex::new(
                Connection::open_in_memory().expect("创建内存数据库失败"),
            )),
            Arc::new(crate::sync::status_aggregator::StatusAggregator::default()),
            Vec::new(),
            0,
            0,
        );
        // 保持周期离线，使测试不访问数据库或网络；只验证通知已进入 runtime。
        engine.set_online_check(Arc::new(move || {
            observed_checks.fetch_add(1, Ordering::SeqCst);
            false
        }));
        let lease = EnginePathLease {
            guards: Some(Vec::new()),
            engine: Arc::new(engine),
            runtime: runtime.handle().clone(),
            notify_local_change: true,
        };

        // fuser 的 release 回调正是在这种没有 Tokio reactor 的工作线程上析构句柄。
        std::thread::spawn(move || drop(lease))
            .join()
            .expect("FUSE 工作线程析构租约不应 panic");

        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while online_checks.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("同步通知应在应用 runtime 中得到执行");
        });
    }
}
