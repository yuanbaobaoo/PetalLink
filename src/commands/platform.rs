//! 平台与应用命令。

use std::path::{Component, Path, PathBuf};

use tauri::AppHandle;
use url::Url;

use crate::auth::token_store::TokenStore;
use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::ipc::VirtualDriveStatus;

use super::{drop_runtime_async, relaunch, DB};

/// 系统打开器的两种本地路径操作。
#[derive(Clone, Copy)]
enum LocalOpenOperation {
    /// 使用系统默认应用打开文件或目录。
    Open,
    /// 在系统文件管理器中显示目标。
    Reveal,
}

/// 解析允许交给系统浏览器的外部 URL。
///
/// IPC 只允许有主机名的 HTTP/HTTPS URL，拒绝 `file:`、`javascript:` 等可扩大
/// WebView 权限的协议；解析后的规范形式再作为一个参数直接交给系统打开器。
fn validate_external_url(raw_url: &str) -> AppResult<Url> {
    let url = Url::parse(raw_url)
        .map_err(|error| AppError::config(format!("外部链接格式无效：{error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::config("外部链接仅允许使用 http 或 https 协议"));
    }
    if url.host().is_none() {
        return Err(AppError::config("外部链接必须包含有效主机名"));
    }
    Ok(url)
}

/// 解析当前应交给用户和系统文件管理器的目录。
///
/// Linux 只在 FUSE 会话真实挂载后返回可见目录，绝不把私有 backing 回退暴露给用户。
/// 其他平台保持原有传统同步目录行为。
fn user_visible_root() -> AppResult<(PathBuf, bool)> {
    let config = crate::core::config_store::ConfigStore::load()?;
    if !config.mount_configured {
        return Err(AppError::config("尚未配置同步目录"));
    }
    #[cfg(target_os = "linux")]
    {
        if !config.virtual_drive_enabled {
            return Err(AppError::config(
                "Linux 配置尚未迁移为按需云盘，请重新选择云盘目录",
            ));
        }
        let mountpoint = super::virtual_mountpoint().ok_or_else(|| {
            let detail = super::virtual_mount_error()
                .map(|error| format!("：{error}"))
                .unwrap_or_default();
            AppError::generic(format!("按需云盘尚未挂载{detail}，请修复挂载问题后重试"))
        })?;
        Ok((mountpoint, true))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok((config.expanded_mount_dir(), false))
    }
}

/// 查询 Linux 按需云盘当前是否真实挂载。
///
/// 前端不能只看配置开关：引擎启动和首次索引是异步的，FUSE 也可能因系统能力问题
/// 挂载失败；此命令让“打开云盘”入口只在真正可用时出现。
#[tauri::command]
#[specta::specta]
pub fn virtual_drive_status() -> AppResult<VirtualDriveStatus> {
    let config = crate::core::config_store::ConfigStore::load()?;
    let enabled = config.virtual_drive_enabled;
    let mountpoint = if enabled {
        super::virtual_mountpoint()
    } else {
        None
    };
    let mounted = mountpoint.is_some();
    let error = if enabled && !mounted {
        super::virtual_mount_error()
    } else {
        None
    };
    Ok(VirtualDriveStatus {
        enabled,
        mounted,
        mount_dir: mountpoint.map(|path| path.to_string_lossy().into_owned()),
        error,
    })
}

/// 在阻塞线程中等待系统打开器退出，避免卡住 Tauri async runtime。
async fn run_local_opener(
    path: PathBuf,
    operation: LocalOpenOperation,
    error_context: &'static str,
) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || match operation {
        LocalOpenOperation::Open => crate::platform::opener::open_path(&path),
        LocalOpenOperation::Reveal => crate::platform::opener::reveal_path(&path),
    })
    .await
    .map_err(|error| AppError::generic(format!("{error_context}线程异常：{error}")))?
    .map(|_| true)
    .map_err(|error| AppError::generic(format!("{error_context}失败：{error}")))
}

/// 把前端相对路径解析为挂载根内已存在的普通文件或目录。
///
/// `safe_join_under` 先拒绝绝对路径与 `..`；随后逐级拒绝符号链接，并对规范路径
/// 再做一次根目录边界检查，防止挂载目录中的链接把系统打开器导向根目录之外。
fn resolve_existing_mount_path(mount_dir: &Path, rel_path: &str) -> AppResult<PathBuf> {
    let target = crate::core::paths::safe_join_under(mount_dir, rel_path, false)?;
    let root_metadata = std::fs::metadata(mount_dir)
        .map_err(|error| AppError::generic(format!("读取同步目录失败：{error}")))?;
    if !root_metadata.is_dir() {
        return Err(AppError::config("配置的同步路径不是目录"));
    }
    let canonical_root = mount_dir
        .canonicalize()
        .map_err(|error| AppError::generic(format!("解析同步目录失败：{error}")))?;

    let mut current = mount_dir.to_path_buf();
    for component in Path::new(rel_path).components() {
        let Component::Normal(segment) = component else {
            return Err(AppError::config("本地相对路径包含不安全分量"));
        };
        current.push(segment);
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::generic("本地文件或目录尚不存在")
            } else {
                AppError::generic(format!("读取本地路径失败：{error}"))
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::config("本地路径包含符号链接，拒绝打开"));
        }
    }

    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|error| AppError::generic(format!("读取本地目标失败：{error}")))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(AppError::generic("本地目标不是普通文件或目录"));
    }
    let canonical_target = target
        .canonicalize()
        .map_err(|error| AppError::generic(format!("解析本地目标失败：{error}")))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(AppError::config("本地路径解析到同步目录之外，拒绝打开"));
    }
    Ok(canonical_target)
}

/// 确保内容打开操作不会把尚未下载的占位文件交给系统默认应用。
fn ensure_local_item_openable(path: &Path) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| AppError::generic(format!("读取本地目标失败：{error}")))?;
    if metadata.is_dir() {
        return Ok(());
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AppError::generic("本地目标不是可打开的普通文件"));
    }
    let state = crate::platform::xattr::get(path, crate::mount::manager::XATTR_STATE)
        .map_err(|error| AppError::generic(format!("读取本地同步状态失败：{error}")))?;
    match state.as_deref() {
        Some(value) if value == crate::mount::manager::STATE_PLACEHOLDER.as_bytes() => {
            Err(AppError::generic("文件尚未同步到本地，请先同步后再打开"))
        }
        Some(value) if value == crate::mount::manager::STATE_DOWNLOADED.as_bytes() => Ok(()),
        Some(_) => Err(AppError::generic("本地文件同步状态标记无效，拒绝打开")),
        // 本地新增或上传完成的真实文件可能只有 fileId、没有 state。
        None => Ok(()),
    }
}

/// 在系统文件管理器中打开路径。
#[tauri::command]
#[specta::specta]
pub async fn open_in_finder() -> AppResult<bool> {
    let (path, _) = user_visible_root()?;
    run_local_opener(path, LocalOpenOperation::Open, "打开系统文件管理器").await
}

/// 使用系统默认应用打开同步目录内的本地文件或目录。
#[tauri::command]
#[specta::specta]
pub async fn open_local_item(rel_path: String) -> AppResult<bool> {
    let (root, is_virtual) = user_visible_root()?;
    let path = resolve_existing_mount_path(&root, &rel_path)?;
    if !is_virtual {
        ensure_local_item_openable(&path)?;
    }
    run_local_opener(path, LocalOpenOperation::Open, "打开本地项").await
}

/// 在系统文件管理器中显示同步目录内的本地文件或目录。
///
/// 此操作允许显示占位文件，但不会把占位文件作为内容交给默认应用。
#[tauri::command]
#[specta::specta]
pub async fn reveal_local_item(rel_path: String) -> AppResult<bool> {
    let (root, _) = user_visible_root()?;
    let path = resolve_existing_mount_path(&root, &rel_path)?;
    run_local_opener(path, LocalOpenOperation::Reveal, "在文件管理器中显示本地项").await
}

/// 使用系统默认浏览器打开合法的 HTTP/HTTPS 外部链接。
#[tauri::command]
#[specta::specta]
pub async fn open_external_url(url: String) -> AppResult<bool> {
    let url = validate_external_url(&url)?;
    tokio::task::spawn_blocking(move || crate::platform::opener::open(url.as_str()))
        .await
        .map_err(|error| AppError::generic(format!("打开系统浏览器线程异常：{error}")))?
        .map(|_| true)
        .map_err(|error| AppError::generic(format!("打开系统浏览器失败：{error}")))
}

/// 检查开机自启。
#[tauri::command]
#[specta::specta]
pub fn launch_at_login_is_enabled() -> bool {
    crate::platform::launch_at_login::is_enabled()
}

/// 设置开机自启。
#[tauri::command]
#[specta::specta]
pub fn launch_at_login_set_enabled(enabled: bool) -> bool {
    match crate::platform::launch_at_login::set_enabled(enabled) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(error = %e, "设置开机自启失败");
            false
        }
    }
}

/// 查询托盘图标当前实际可见性（运行时真实状态，而非配置文件里的目标值）。
#[tauri::command]
#[specta::specta]
pub fn tray_is_visible() -> bool {
    crate::platform::activation::is_tray_icon_visible()
}

/// 当前发行渠道是否提供可验证的自动更新产物。
///
/// macOS 沿用已有的签名更新渠道。Linux 必须同时满足：
/// - 构建时显式设置 `PETALLINK_UPDATE_CHANNEL=appimage`；
/// - 当前确实由 AppImage runtime 启动；
/// - AppImage 所在目录允许当前用户原子替换该文件。
///
/// 因此 AUR/系统包和普通源码构建即使人为设置 `APPIMAGE` 也不会误走应用内更新，
/// 放在只读目录中的 AppImage 则交由用户或包管理器更新。
#[tauri::command]
#[specta::specta]
pub fn updater_is_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        linux_appimage_updater_is_supported(
            option_env!("PETALLINK_UPDATE_CHANNEL"),
            std::env::var_os("APPIMAGE").as_deref().map(Path::new),
            std::env::var_os("APPDIR").as_deref().map(Path::new),
            std::env::current_exe().ok().as_deref(),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// 纯路径部分的 Linux AppImage runtime 判定。
///
/// 调用方传入已规范化的 `appdir` 与 `current_exe`；把这部分独立出来可以不修改
/// 进程环境就覆盖渠道、相对路径和伪造 runtime 等边界条件。
#[cfg(target_os = "linux")]
fn linux_appimage_runtime_shape_is_valid(
    update_channel: Option<&str>,
    appimage: Option<&Path>,
    appdir: Option<&Path>,
    current_exe: Option<&Path>,
) -> bool {
    if update_channel != Some("appimage") {
        return false;
    }
    let (Some(appimage), Some(appdir), Some(current_exe)) = (appimage, appdir, current_exe) else {
        return false;
    };
    appimage.is_absolute()
        && appdir.is_absolute()
        && current_exe.is_absolute()
        && current_exe != appdir
        && current_exe.starts_with(appdir)
}

/// 按当前进程的有效用户身份检查 Linux 路径访问能力。
#[cfg(target_os = "linux")]
fn linux_has_effective_access(path: &Path, mode: libc::c_int) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` 是有结尾 NUL 且内部无 NUL 的 CString，调用期间保持有效。
    unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) == 0 }
}

/// 探测 AppImage 所在目录是否真的允许创建并删除替换用的同目录临时文件。
///
/// 不能尝试以写模式打开正在运行的 AppImage：Linux 会返回 `ETXTBSY`，而 Tauri
/// updater 实际采用的是“重命名旧文件，再在原路径写入新文件”，并不需要写开旧文件。
#[cfg(target_os = "linux")]
fn linux_parent_allows_atomic_replace(parent: &Path) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};

    if !linux_has_effective_access(parent, libc::W_OK | libc::X_OK) {
        return false;
    }

    static NEXT_PROBE_ID: AtomicU64 = AtomicU64::new(0);
    for _ in 0..3 {
        let probe_id = NEXT_PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let probe_path = parent.join(format!(
            ".petallink-update-probe-{}-{probe_id}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe_path)
        {
            Ok(file) => {
                drop(file);
                return std::fs::remove_file(probe_path).is_ok();
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return false,
        }
    }
    false
}

/// 检查当前 Linux 进程是否处于可安全自更新的官方 AppImage 渠道。
#[cfg(target_os = "linux")]
fn linux_appimage_updater_is_supported(
    update_channel: Option<&str>,
    appimage: Option<&Path>,
    appdir: Option<&Path>,
    current_exe: Option<&Path>,
) -> bool {
    let (Some(appimage), Some(appdir), Some(current_exe)) = (appimage, appdir, current_exe) else {
        return false;
    };
    if !appimage.is_absolute() || !appdir.is_absolute() || !current_exe.is_absolute() {
        return false;
    }

    // APPIMAGE 本身必须是普通文件（不接受符号链接），避免更新器替换意外目标。
    let Ok(appimage_metadata) = std::fs::symlink_metadata(appimage) else {
        return false;
    };
    if !appimage_metadata.file_type().is_file() {
        return false;
    }
    let Some(parent) = appimage.parent() else {
        return false;
    };
    let Ok(parent_metadata) = std::fs::metadata(parent) else {
        return false;
    };
    if !parent_metadata.is_dir() {
        return false;
    }

    // APPDIR 与可执行文件必须真实存在，且规范化后可执行文件位于 APPDIR 中。
    let Ok(canonical_appdir) = appdir.canonicalize() else {
        return false;
    };
    let Ok(canonical_exe) = current_exe.canonicalize() else {
        return false;
    };
    if !canonical_appdir.is_dir()
        || !canonical_exe.is_file()
        || !linux_appimage_runtime_shape_is_valid(
            update_channel,
            Some(appimage),
            Some(&canonical_appdir),
            Some(&canonical_exe),
        )
    {
        return false;
    }

    // Tauri updater 会先把旧 AppImage 重命名到同一文件系统，再写回原路径。
    // 因此只要求父目录能完成同目录创建/删除，不写开正在运行的 AppImage。
    linux_parent_allows_atomic_replace(parent)
}

/// 收束同步运行时并以前台模式重启应用。
#[tauri::command]
#[specta::specta]
pub async fn app_relaunch(app: AppHandle) -> AppResult<()> {
    drop_runtime_async().await;
    relaunch(&app);
    Ok(())
}

/// 清空应用缓存。
#[tauri::command]
#[specta::specta]
pub async fn app_clear_cache(app: AppHandle) -> AppResult<()> {
    // 守卫：有非终态传输任务时禁止清空，避免删掉 VerifyingRemote 任务的 remote_result_file_id
    // 等免重传凭证，导致已上传文件无法核验、被迫全量重传。
    if super::active_transfer_count()? > 0 {
        return Err(AppError::generic(
            "有文件正在上传/下载，请等待传输完成后再清空缓存",
        ));
    }
    // 停止运行时
    drop_runtime_async().await;
    // 清除登录状态
    let _ = crate::auth::token_store::global_store().clear();
    // 清空数据库
    {
        let conn = DB.lock();
        let _ = repository::delete_all(&conn);
        let _ = repository::delete_all_transfers(&conn);
    }
    if let Ok(p) = crate::data::db_file_path() {
        let _ = std::fs::remove_file(&p);
    }
    // 清除同步缓存
    crate::core::cache_paths::clear_all_cache_files();
    #[cfg(target_os = "linux")]
    {
        // 只清理由本版本在 Application Support 下分配的 backing；旧自定义目录不碰。
        let _ = crate::core::config_store::clear_linux_managed_backings();
        crate::core::config_store::clear_linux_migration_receipts();
    }
    // 删除配置
    if let Ok(p) = crate::core::config_store::config_file_path() {
        let _ = std::fs::remove_file(&p);
    }
    tracing::info!("缓存已清空，准备重启");
    // 重启应用
    relaunch(&app);
    Ok(())
}

/// 读取最近日志。
#[tauri::command]
#[specta::specta]
pub fn logs_list() -> AppResult<Vec<crate::core::logging::LogRecord>> {
    Ok(crate::core::logging::snapshot())
}

/// 导出完整日志。
#[tauri::command]
#[specta::specta]
pub fn logs_export(path: String) -> AppResult<()> {
    let dir = crate::core::logging::log_dir()?;
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    // 只处理应用日志
    files.retain(|f| {
        f.file_name()
            .map(|n| n.to_string_lossy().starts_with("PetalLink.log"))
            .unwrap_or(false)
    });
    files.sort(); // 按日期升序

    // 记录导出文件
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
        // 容忍非 UTF-8 日志
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

/// 清空内存日志缓冲区；磁盘滚动日志由保留策略单独管理。
#[tauri::command]
#[specta::specta]
pub fn logs_clear() -> AppResult<()> {
    crate::core::logging::clear();
    Ok(())
}

/// 获取应用版本。
#[tauri::command]
#[specta::specta]
pub fn app_get_version() -> String {
    crate::constants::APP_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::{ensure_local_item_openable, resolve_existing_mount_path, validate_external_url};
    #[cfg(target_os = "linux")]
    use super::{linux_appimage_runtime_shape_is_valid, linux_appimage_updater_is_supported};

    #[test]
    fn external_url_accepts_valid_http_and_https_urls() {
        for raw_url in [
            "https://github.com/yuanbaobaoo/PetalLink",
            "http://127.0.0.1:8080/status?full=true#summary",
        ] {
            let url = validate_external_url(raw_url).expect("合法 HTTP(S) 链接应通过");
            assert!(matches!(url.scheme(), "http" | "https"));
            assert!(url.host().is_some());
        }
    }

    #[test]
    fn external_url_rejects_non_web_schemes_and_invalid_urls() {
        for raw_url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "not a url",
            "https://",
        ] {
            assert!(
                validate_external_url(raw_url).is_err(),
                "不安全或畸形链接不应通过：{raw_url}"
            );
        }
    }

    #[test]
    fn existing_file_under_mount_resolves_to_canonical_path() {
        let root = tempfile::tempdir().expect("创建测试目录失败");
        let nested = root.path().join("docs");
        std::fs::create_dir(&nested).expect("创建子目录失败");
        let file = nested.join("report.txt");
        std::fs::write(&file, b"content").expect("创建测试文件失败");

        let resolved =
            resolve_existing_mount_path(root.path(), "docs/report.txt").expect("安全路径应通过");
        assert_eq!(resolved, file.canonicalize().expect("规范化测试文件失败"));
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        let root = tempfile::tempdir().expect("创建测试目录失败");
        for rel_path in [
            "../outside.txt",
            "docs/../../outside.txt",
            "/tmp/outside.txt",
        ] {
            assert!(
                resolve_existing_mount_path(root.path(), rel_path).is_err(),
                "应拒绝不安全路径：{rel_path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_path_component_is_rejected_even_when_target_exists() {
        let root = tempfile::tempdir().expect("创建挂载测试目录失败");
        let outside = tempfile::tempdir().expect("创建外部测试目录失败");
        std::fs::write(outside.path().join("secret.txt"), b"secret").expect("创建外部测试文件失败");
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked"))
            .expect("创建符号链接失败");

        let error = resolve_existing_mount_path(root.path(), "linked/secret.txt")
            .expect_err("符号链接不得越过挂载边界");
        assert!(error.to_string().contains("符号链接"));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn placeholder_file_cannot_be_opened_as_content() {
        let file = tempfile::NamedTempFile::new().expect("创建占位测试文件失败");
        crate::platform::xattr::set(
            file.path(),
            crate::mount::manager::XATTR_STATE,
            crate::mount::manager::STATE_PLACEHOLDER.as_bytes(),
        )
        .expect("写入占位状态失败");

        let error = ensure_local_item_openable(file.path()).expect_err("占位文件不得作为内容打开");
        assert!(error.to_string().contains("尚未同步"));
    }

    #[test]
    fn ordinary_local_file_and_directory_are_openable() {
        let root = tempfile::tempdir().expect("创建测试目录失败");
        let file = root.path().join("local.txt");
        std::fs::write(&file, b"local").expect("创建本地测试文件失败");

        ensure_local_item_openable(&file).expect("普通本地文件应可打开");
        ensure_local_item_openable(root.path()).expect("本地目录应可打开");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_updater_shape_requires_official_appimage_runtime() {
        let appimage = std::path::Path::new("/home/user/PetalLink.AppImage");
        let appdir = std::path::Path::new("/tmp/.mount_PetalLink");
        let executable = std::path::Path::new("/tmp/.mount_PetalLink/usr/bin/PetalLink");

        assert!(linux_appimage_runtime_shape_is_valid(
            Some("appimage"),
            Some(appimage),
            Some(appdir),
            Some(executable),
        ));
        assert!(!linux_appimage_runtime_shape_is_valid(
            None,
            Some(appimage),
            Some(appdir),
            Some(executable),
        ));
        assert!(!linux_appimage_runtime_shape_is_valid(
            Some("aur"),
            Some(appimage),
            Some(appdir),
            Some(executable),
        ));
        assert!(!linux_appimage_runtime_shape_is_valid(
            Some("appimage"),
            Some(std::path::Path::new("PetalLink.AppImage")),
            Some(appdir),
            Some(executable),
        ));
        assert!(!linux_appimage_runtime_shape_is_valid(
            Some("appimage"),
            Some(appimage),
            Some(appdir),
            Some(std::path::Path::new("/usr/bin/PetalLink")),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_updater_accepts_writable_regular_appimage() {
        let root = tempfile::tempdir().expect("创建 AppImage 测试目录失败");
        let appimage = root.path().join("PetalLink.AppImage");
        std::fs::write(&appimage, b"appimage").expect("创建 AppImage 测试文件失败");
        let appdir = root.path().join(".mount_PetalLink");
        let executable = appdir.join("usr/bin/PetalLink");
        std::fs::create_dir_all(executable.parent().expect("可执行文件应有父目录"))
            .expect("创建 AppImage runtime 目录失败");
        std::fs::write(&executable, b"binary").expect("创建 runtime 可执行文件失败");

        assert!(linux_appimage_updater_is_supported(
            Some("appimage"),
            Some(&appimage),
            Some(&appdir),
            Some(&executable),
        ));
        assert!(!linux_appimage_updater_is_supported(
            None,
            Some(&appimage),
            Some(&appdir),
            Some(&executable),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_updater_rejects_symlink_but_accepts_read_only_file_in_writable_parent() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = tempfile::tempdir().expect("创建 AppImage 测试目录失败");
        let appimage = root.path().join("PetalLink.AppImage");
        std::fs::write(&appimage, b"appimage").expect("创建 AppImage 测试文件失败");
        let linked_appimage = root.path().join("PetalLink-link.AppImage");
        symlink(&appimage, &linked_appimage).expect("创建 AppImage 符号链接失败");
        let appdir = root.path().join(".mount_PetalLink");
        let executable = appdir.join("usr/bin/PetalLink");
        std::fs::create_dir_all(executable.parent().expect("可执行文件应有父目录"))
            .expect("创建 AppImage runtime 目录失败");
        std::fs::write(&executable, b"binary").expect("创建 runtime 可执行文件失败");

        assert!(!linux_appimage_updater_is_supported(
            Some("appimage"),
            Some(&linked_appimage),
            Some(&appdir),
            Some(&executable),
        ));

        let mut permissions = std::fs::metadata(&appimage)
            .expect("读取 AppImage 权限失败")
            .permissions();
        permissions.set_mode(0o444);
        std::fs::set_permissions(&appimage, permissions).expect("设置只读权限失败");
        assert!(linux_appimage_updater_is_supported(
            Some("appimage"),
            Some(&appimage),
            Some(&appdir),
            Some(&executable),
        ));

        let mut parent_permissions = std::fs::metadata(root.path())
            .expect("读取 AppImage 父目录权限失败")
            .permissions();
        parent_permissions.set_mode(0o555);
        std::fs::set_permissions(root.path(), parent_permissions)
            .expect("设置 AppImage 父目录只读失败");
        assert!(!linux_appimage_updater_is_supported(
            Some("appimage"),
            Some(&appimage),
            Some(&appdir),
            Some(&executable),
        ));

        let mut parent_permissions = std::fs::metadata(root.path())
            .expect("重新读取 AppImage 父目录权限失败")
            .permissions();
        parent_permissions.set_mode(0o755);
        std::fs::set_permissions(root.path(), parent_permissions)
            .expect("恢复 AppImage 父目录权限失败");
    }
}
