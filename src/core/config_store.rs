//! 配置持久化（需求 F-CFG-02 / F-CFG-04）。
//!
//! 对齐 `legacy/lib/core/config/config_store.dart`。
//!
//! 存储位置：`<ApplicationSupport>/config.json`，不含 token（token 加密存 token.bin）。
//! 支持导入/导出 JSON（F-CFG-04：备份恢复配置）。

use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::Command;

use serde_json::{json, Value};

use crate::core::config::{
    AppConfig, SortField, SortOrder, DEFAULT_MOUNT_DIR, DEFAULT_REDIRECT_URI,
};
use crate::error::{AppError, AppResult};

/// 配置文件名
const CONFIG_FILE_NAME: &str = "config.json";

/// Linux 按需云盘的持久 backing 根目录名。
///
/// 放在 XDG data/Application Support 下而不是 XDG cache 下，避免桌面清理工具删除
/// 尚未上传完成的本地写入。每次从未配置态选择目录都会分配新的 profile 子目录，
/// 因而退出账号后不会复用上一账号留下的 backing。
#[cfg(target_os = "linux")]
const LINUX_BACKING_ROOT_NAME: &str = "on-demand-backing";

/// 从旧传统同步配置迁移时留下的本地索引重建标记。
///
/// 旧工作目录本身绝不自动删除；标记只要求首次选择新按需目录前清空数据库同步行与
/// 派生缓存，避免旧路径基线在新空 backing 中被误判为本地删除。
#[cfg(target_os = "linux")]
const LINUX_SYNC_RESET_MARKER_NAME: &str = ".linux-on-demand-sync-reset-required";

/// 旧 traditional 配置的只写一次恢复副本，保留原目录位置供人工核对。
#[cfg(target_os = "linux")]
const LINUX_LEGACY_CONFIG_BACKUP_NAME: &str = "config.pre-linux-on-demand.json";

/// Application Support 目录下的 PetalLink 工作目录。
/// macOS 路径：`~/Library/Application Support/io.github.yuanbaobaoo.PetalLink`
/// 对齐 dart `getApplicationSupportDirectory()`。
pub fn support_dir() -> AppResult<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::config("无法获取 Application Support 目录".to_string()))?;
    // macOS data_dir() 已是 ~/Library/Application Support
    Ok(base.join(crate::constants::BUNDLE_IDENTIFIER))
}

/// 配置文件完整路径
pub fn config_file_path() -> AppResult<PathBuf> {
    Ok(support_dir()?.join(CONFIG_FILE_NAME))
}

/// 配置存储。负责序列化 / 反序列化 / 旧值迁移。
pub struct ConfigStore;

impl ConfigStore {
    /// 读取配置；文件不存在时返回默认配置，读取或解析失败时返回错误。
    /// 对齐 dart `ConfigStore.load()`。
    pub fn load() -> AppResult<AppConfig> {
        let path = config_file_path()?;
        if !path.exists() {
            tracing::info!("配置文件不存在，使用默认配置");
            return Ok(AppConfig::default());
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| AppError::config(format!("配置读取失败：{}：{e}", path.display())))?;
        let (config, dirty, reset_sync_state) = parse_config_raw(&raw)?;
        #[cfg(not(target_os = "linux"))]
        let _ = reset_sync_state;
        #[cfg(target_os = "linux")]
        if reset_sync_state {
            backup_legacy_linux_config(&raw)?;
            mark_linux_sync_reset_required()?;
        }
        // 迁移改了值 → 落盘（仅 load 走此路径；from_json 纯解析不落盘，避免测试污染真实配置）
        if dirty {
            ConfigStore::save(&config)?;
        }
        Ok(config)
    }

    /// 保存配置（先校验）。
    /// 对齐 dart `ConfigStore.save()`。
    pub fn save(config: &AppConfig) -> AppResult<()> {
        let config = normalize_for_save(config.clone())?;
        config.validate()?;
        let path = config_file_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let json = to_json(&config);
        let pretty = serde_json::to_string_pretty(&json)?;
        write_config_atomically(&path, pretty.as_bytes())?;
        tracing::info!(
            backing = %config.mount_dir,
            visible = %config.virtual_mount_dir,
            "配置已保存"
        );
        Ok(())
    }

    /// 导出配置为 JSON 字符串（F-CFG-04，不含 token）。
    pub fn export_to_json(config: &AppConfig) -> AppResult<String> {
        let config = normalize_for_save(config.clone())?;
        Ok(serde_json::to_string_pretty(&to_json(&config))?)
    }

    /// 从 JSON 字符串解析并校验配置（F-CFG-04），不直接改变当前运行时或配置文件。
    ///
    /// 真正提交必须统一经过 `commands::config_save`，由它负责停止旧引擎、卸载 FUSE、
    /// 校验目录能力和重启。这里若直接落盘，会让旧运行时继续操作旧目录。
    pub fn import_from_json(json_str: &str) -> AppResult<AppConfig> {
        let (config, _dirty, _reset_sync_state) = parse_config_raw(json_str)?;
        Ok(config)
    }

    /// 把来自 IPC 的 Linux 单目录配置规范化为“隐藏 backing + 可见 FUSE 目录”。
    ///
    /// 现有按需配置保留原 backing（避免静默切到空目录）；首次选择既接受新版
    /// `virtual_mount_dir`，也兼容旧引导暂时写入 `mount_dir` 的合同。
    pub(crate) fn normalize_for_save(config: AppConfig) -> AppResult<AppConfig> {
        normalize_for_save(config)
    }
}

/// Linux 的应用管理 backing 根目录。
#[cfg(target_os = "linux")]
pub(crate) fn linux_managed_backing_root() -> AppResult<PathBuf> {
    Ok(support_dir()?.join(LINUX_BACKING_ROOT_NAME))
}

/// 为一次新的目录配置分配不会复用旧账号数据的持久 backing。
#[cfg(target_os = "linux")]
fn allocate_linux_managed_backing() -> AppResult<PathBuf> {
    Ok(linux_managed_backing_root()?.join(format!(
        "profile-{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )))
}

/// 保存入口的跨平台规范化。非 Linux 保持传统同步合同不变。
fn normalize_for_save(mut config: AppConfig) -> AppResult<AppConfig> {
    #[cfg(target_os = "linux")]
    {
        if !config.mount_configured {
            config.mount_dir.clear();
            config.virtual_drive_enabled = false;
            config.virtual_mount_dir.clear();
            return Ok(config);
        }

        // 已有 FUSE 配置（包括旧版本使用的自定义 backing）必须保持 backing 不变，
        // 否则新空目录配合旧 DB 基线可能把“本地缺失”规划成云端删除。
        if !config.virtual_mount_dir.trim().is_empty() && !config.mount_dir.trim().is_empty() {
            config.virtual_drive_enabled = true;
            return Ok(config);
        }

        // 新前端把唯一选择写入 virtual_mount_dir；兼容旧首次引导把它写在 mount_dir。
        let visible_mount = if config.virtual_mount_dir.trim().is_empty() {
            config.mount_dir.clone()
        } else {
            config.virtual_mount_dir.clone()
        };
        config.mount_dir = allocate_linux_managed_backing()?
            .into_os_string()
            .into_string()
            .map_err(|_| AppError::config("应用数据目录不是有效 UTF-8 路径".to_string()))?;
        config.virtual_drive_enabled = true;
        config.virtual_mount_dir = visible_mount;
    }
    Ok(config)
}

/// 是否存在“首次配置前必须丢弃旧同步索引”的 Linux 迁移标记。
#[cfg(target_os = "linux")]
pub(crate) fn linux_sync_reset_required() -> AppResult<bool> {
    Ok(support_dir()?.join(LINUX_SYNC_RESET_MARKER_NAME).is_file())
}

/// 旧同步索引已清空后提交迁移标记。
#[cfg(target_os = "linux")]
pub(crate) fn finish_linux_sync_reset() -> AppResult<()> {
    let marker = support_dir()?.join(LINUX_SYNC_RESET_MARKER_NAME);
    match fs::remove_file(&marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::config(format!(
            "提交 Linux 按需云盘迁移状态失败：{}：{error}",
            marker.display()
        ))),
    }
}

/// “清空应用缓存”专用：只移除 PetalLink 自己分配的 managed backing 根。
///
/// 旧版自定义 backing/传统同步目录不在该根下，绝不会被此函数删除。
#[cfg(target_os = "linux")]
pub(crate) fn clear_linux_managed_backings() -> AppResult<()> {
    let root = linux_managed_backing_root()?;
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(&root)?;
    } else {
        fs::remove_dir_all(&root)?;
    }
    Ok(())
}

/// 显式“清空应用缓存”时移除迁移标记与旧配置恢复副本。
#[cfg(target_os = "linux")]
pub(crate) fn clear_linux_migration_receipts() {
    let Ok(support) = support_dir() else { return };
    let _ = fs::remove_file(support.join(LINUX_SYNC_RESET_MARKER_NAME));
    let _ = fs::remove_file(support.join(LINUX_LEGACY_CONFIG_BACKUP_NAME));
}

#[cfg(target_os = "linux")]
fn mark_linux_sync_reset_required() -> AppResult<()> {
    let support = support_dir()?;
    fs::create_dir_all(&support)?;
    let marker = support.join(LINUX_SYNC_RESET_MARKER_NAME);
    if !marker.exists() {
        fs::write(
            &marker,
            b"Legacy local sync indexes must be rebuilt before configuring Linux FUSE.\n",
        )
        .map_err(|error| {
            AppError::config(format!(
                "记录 Linux 按需云盘迁移状态失败：{}：{error}",
                marker.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn backup_legacy_linux_config(raw: &str) -> AppResult<()> {
    let support = support_dir()?;
    fs::create_dir_all(&support)?;
    let backup = support.join(LINUX_LEGACY_CONFIG_BACKUP_NAME);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
    {
        Ok(mut file) => {
            file.write_all(raw.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(AppError::config(format!(
            "备份旧 Linux 同步配置失败：{}：{error}",
            backup.display()
        ))),
    }
}

/// 同目录临时文件完整落盘后再原子替换配置，避免崩溃留下截断 JSON。
fn write_config_atomically(path: &std::path::Path, contents: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::config("配置文件缺少父目录".to_string()))?;
    let mut last_collision = None;
    for _ in 0..16 {
        let temporary = parent.join(format!(
            ".{CONFIG_FILE_NAME}.tmp-{:016x}",
            rand::random::<u64>()
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let committed = (|| -> std::io::Result<()> {
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if let Err(error) = committed {
            let _ = std::fs::remove_file(&temporary);
            return Err(error.into());
        }
        return Ok(());
    }
    Err(last_collision
        .map(AppError::from)
        .unwrap_or_else(|| AppError::config("无法分配配置临时文件".to_string())))
}

/// 解析、迁移并校验配置文本，同时返回是否需要回写。
fn parse_config_raw(raw: &str) -> AppResult<(AppConfig, bool, bool)> {
    let json: Value =
        serde_json::from_str(raw).map_err(|e| AppError::config(format!("配置解析失败：{e}")))?;
    let (config, dirty, reset_sync_state) = from_json(&json);
    config.validate()?;
    Ok((config, dirty, reset_sync_state))
}

/// Linux 上确认同步目录可创建、可写且支持 PetalLink 所需扩展属性。
///
/// 该探测在选择新目录和安装同步引擎前执行，不挂在普通配置保存上，避免用户只修改
/// 托盘等设置时产生文件系统副作用。macOS 保持既有目录行为，由原生同步路径处理。
#[cfg(target_os = "linux")]
pub(crate) fn validate_configured_mount_dir_access(config: &AppConfig) -> AppResult<()> {
    if !config.mount_configured {
        return Ok(());
    }
    let dir = config.expanded_mount_dir();
    if dir.exists() && !dir.is_dir() {
        return Err(AppError::config(format!(
            "同步目录不是文件夹：{}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::config(format!("同步目录创建失败：{}：{e}", dir.display())))?;
    let probe = dir.join(format!(
        "{}xattr-probe-{}-{:016x}",
        crate::constants::INTERNAL_FILE_PREFIX,
        std::process::id(),
        rand::random::<u64>()
    ));
    fs::write(&probe, b"ok")
        .map_err(|e| AppError::config(format!("同步目录不可写：{}：{e}", dir.display())))?;

    const PROBE_KEY: &str = "com.hwcloud.capabilityProbe";
    const PROBE_VALUE: &[u8] = b"petallink";
    let capability_result = (|| {
        crate::platform::xattr::set(&probe, PROBE_KEY, PROBE_VALUE).map_err(|e| {
            AppError::config(format!(
                "同步目录不支持扩展属性，无法安全保存占位文件状态：{}：{e}",
                dir.display()
            ))
        })?;
        let actual = crate::platform::xattr::get(&probe, PROBE_KEY).map_err(|e| {
            AppError::config(format!("同步目录扩展属性不可读：{}：{e}", dir.display()))
        })?;
        if actual.as_deref() != Some(PROBE_VALUE) {
            return Err(AppError::config(format!(
                "同步目录扩展属性校验失败：{}",
                dir.display()
            )));
        }
        crate::platform::xattr::remove(&probe, PROBE_KEY).map_err(|e| {
            AppError::config(format!("同步目录扩展属性不可删除：{}：{e}", dir.display()))
        })
    })();

    let cleanup_result = fs::remove_file(&probe).map_err(|e| {
        AppError::config(format!(
            "同步目录写入探测清理失败：{}：{e}",
            probe.display()
        ))
    });
    capability_result.and(cleanup_result)
}

/// 非 Linux 平台暂不改变稳定版本的挂载目录准入语义。
#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_configured_mount_dir_access(_config: &AppConfig) -> AppResult<()> {
    Ok(())
}

/// Linux 上确认 FUSE 挂载点可安全创建、当前用户可写、为空且尚未被挂载。
///
/// 这是显式能力探测，不会从普通 `load`/`save` 路径调用。初版按需云盘仍使用
/// `mount_dir` 作为 writable+xattr backing，由 [`validate_configured_mount_dir_access`]
/// 独立探测。
#[cfg(target_os = "linux")]
pub(crate) fn validate_virtual_mount_dir_access(config: &AppConfig) -> AppResult<()> {
    if !config.virtual_drive_enabled {
        return Ok(());
    }
    config.validate()?;
    validate_fuse_runtime()?;

    let dir = config.expanded_virtual_mount_dir();
    // 必须先查 mountinfo、再碰挂载点 inode。FUSE daemon 异常退出后，
    // exists/is_dir/canonicalize 都可能直接返回 ENOTCONN，若顺序反过来就永远走不到
    // stale mount 恢复逻辑。
    if is_current_mountpoint(&dir)? && !recover_disconnected_petallink_mount(&dir)? {
        return Err(AppError::config(format!(
            "按需云盘挂载点已被其他文件系统占用：{}",
            dir.display()
        )));
    }
    if dir.exists() && !dir.is_dir() {
        return Err(AppError::config(format!(
            "按需云盘挂载点不是文件夹：{}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::config(format!("按需云盘挂载点创建失败：{}：{e}", dir.display())))?;
    let canonical_dir = fs::canonicalize(&dir).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点无法解析为安全路径：{}：{e}",
            dir.display()
        ))
    })?;
    let canonical_backing = fs::canonicalize(config.expanded_mount_dir()).map_err(|e| {
        AppError::config(format!(
            "物理 backing 目录不存在或无法访问：{}：{e}",
            config.expanded_mount_dir().display()
        ))
    })?;

    validate_resolved_virtual_path(&canonical_backing, &canonical_dir)?;
    // 配置路径可能经过符号链接。解析后的真实路径再查一次，仍只允许恢复身份明确、
    // 当前用户持有且已断开的 PetalLink 挂载。
    if canonical_dir != dir
        && is_current_mountpoint(&canonical_dir)?
        && !recover_disconnected_petallink_mount(&canonical_dir)?
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点已被其他文件系统占用：{}",
            canonical_dir.display()
        )));
    }

    let mut entries = fs::read_dir(&canonical_dir).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点不可读取：{}：{e}",
            canonical_dir.display()
        ))
    })?;
    if entries
        .next()
        .transpose()
        .map_err(|e| {
            AppError::config(format!(
                "按需云盘挂载点读取失败：{}：{e}",
                canonical_dir.display()
            ))
        })?
        .is_some()
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点必须为空：{}",
            canonical_dir.display()
        )));
    }

    let probe = canonical_dir.join(format!(
        "{}fuse-write-probe-{}-{:016x}",
        crate::constants::INTERNAL_FILE_PREFIX,
        std::process::id(),
        rand::random::<u64>()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| {
            AppError::config(format!(
                "当前用户不可写按需云盘挂载点：{}：{e}",
                canonical_dir.display()
            ))
        })?;
    fs::remove_file(&probe).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点写入探测清理失败：{}：{e}",
            probe.display()
        ))
    })
}

/// 提前给出可操作的 FUSE 依赖错误，避免保存配置并重启后才静默挂载失败。
#[cfg(all(target_os = "linux", not(test)))]
fn validate_fuse_runtime() -> AppResult<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/fuse")
        .map_err(|error| {
            AppError::config(format!(
                "当前用户无法访问 /dev/fuse：{error}。请安装并启用 fuse3，重新登录后再试"
            ))
        })?;

    let candidates: Vec<PathBuf> = std::env::var_os("FUSERMOUNT_PATH")
        .map(PathBuf::from)
        .map(|helper| vec![helper])
        .unwrap_or_else(|| {
            [
                "fusermount3",
                "fusermount",
                "/bin/fusermount3",
                "/bin/fusermount",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect()
        });
    if candidates
        .iter()
        .any(|helper| Command::new(helper).arg("-h").output().is_ok())
    {
        return Ok(());
    }
    Err(AppError::config(
        "未找到 fusermount3；请安装发行版的 fuse3 软件包".to_string(),
    ))
}

/// 单元测试不依赖宿主是否把 `/dev/fuse` 映射进测试容器；真实挂载由 ignored smoke 覆盖。
#[cfg(all(target_os = "linux", test))]
fn validate_fuse_runtime() -> AppResult<()> {
    Ok(())
}

/// 非 Linux 平台不启用 FUSE 能力探测，保持原有配置行为。
#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_virtual_mount_dir_access(_config: &AppConfig) -> AppResult<()> {
    Ok(())
}

/// 显式探测初版按需云盘所需的 backing 与 FUSE 挂载点能力。
pub(crate) fn validate_virtual_drive_capabilities(config: &AppConfig) -> AppResult<()> {
    validate_configured_mount_dir_access(config)?;
    validate_virtual_mount_dir_access(config)
}

/// 使用解析后的真实路径再次防御符号链接绕过配置层的路径隔离。
#[cfg(target_os = "linux")]
fn validate_resolved_virtual_path(
    canonical_backing: &Path,
    canonical_virtual: &Path,
) -> AppResult<()> {
    if canonical_backing == canonical_virtual
        || canonical_backing.starts_with(canonical_virtual)
        || canonical_virtual.starts_with(canonical_backing)
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点必须与物理 backing 目录不同且互不包含：{} ↔ {}",
            canonical_backing.display(),
            canonical_virtual.display()
        )));
    }
    if canonical_virtual == Path::new("/") {
        return Err(AppError::config(
            "不能把系统根目录作为按需云盘挂载目录".to_string(),
        ));
    }
    if let Some(home) = dirs::home_dir() {
        let resolved_home = fs::canonicalize(&home).unwrap_or(home);
        if canonical_virtual == resolved_home {
            return Err(AppError::config(
                "不能把用户 Home 目录作为按需云盘挂载目录".to_string(),
            ));
        }
    }
    if let Some(data_dir) = dirs::data_dir() {
        let resolved_data = fs::canonicalize(&data_dir).unwrap_or(data_dir);
        if canonical_virtual.starts_with(resolved_data) {
            return Err(AppError::config(
                "不能把 Application Support 目录作为按需云盘挂载目录".to_string(),
            ));
        }
    }
    Ok(())
}

/// 判断路径是否正好是当前 mount namespace 中的挂载点。
#[cfg(target_os = "linux")]
fn is_current_mountpoint(path: &Path) -> AppResult<bool> {
    Ok(current_mount_record(path)?.is_some())
}

/// `/proc/self/mountinfo` 中与目标路径匹配的最小安全身份。
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MountRecord {
    mount_id: u64,
    fs_type: String,
    source: String,
    super_options: String,
}

/// 查找目标路径的挂载记录，并解析 ` - ` 分隔后的文件系统身份。
#[cfg(target_os = "linux")]
fn current_mount_record(path: &Path) -> AppResult<Option<MountRecord>> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|e| AppError::config(format!("无法读取当前挂载信息：{e}")))?;
    Ok(parse_mount_record(&mountinfo, path))
}

/// 核对目标是否仍是当前 mount namespace 中可访问的 PetalLink FUSE 挂载。
#[cfg(target_os = "linux")]
pub(crate) fn is_active_petallink_mount(path: &Path) -> AppResult<bool> {
    let Some(record) = current_mount_record(path)? else {
        return Ok(false);
    };
    if record.fs_type != "fuse.petallink"
        || (record.source != "PetalLink" && record.source != "petallink")
    {
        return Ok(false);
    }
    match fs::read_dir(path) {
        Ok(_) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ENOTCONN) => Ok(false),
        Err(error) => Err(AppError::config(format!(
            "PetalLink 挂载点当前不可访问：{}：{error}",
            path.display()
        ))),
    }
}

#[cfg(target_os = "linux")]
fn parse_mount_record(mountinfo: &str, path: &Path) -> Option<MountRecord> {
    mountinfo.lines().find_map(|line| {
        let (left, right) = line.split_once(" - ")?;
        let mut left_fields = left.split_whitespace();
        let mount_id = left_fields.next()?.parse().ok()?;
        let mountpoint = left_fields.nth(3)?;
        if decode_mountinfo_path(mountpoint) != path {
            return None;
        }
        let mut fields = right.split_whitespace();
        Some(MountRecord {
            mount_id,
            fs_type: fields.next()?.to_string(),
            source: fields.next()?.to_string(),
            super_options: fields.next().unwrap_or_default().to_string(),
        })
    })
}

#[cfg(target_os = "linux")]
fn is_petallink_mount_record(record: &MountRecord) -> bool {
    record.fs_type == "fuse.petallink"
        && (record.source == "PetalLink" || record.source == "petallink")
}

#[cfg(target_os = "linux")]
fn mount_owner_uid(record: &MountRecord) -> Option<u32> {
    record
        .super_options
        .split(',')
        .find_map(|option| option.strip_prefix("user_id="))
        .and_then(|uid| uid.parse::<u32>().ok())
}

/// 只清理由当前用户持有、身份明确且已经断开的 PetalLink FUSE 挂载。
///
/// 活跃挂载、其他 FUSE 类型或所有权不匹配一律不碰，避免把宽泛的自动清理变成
/// 任意卸载能力。崩溃后的典型访问错误是 `ENOTCONN`。
#[cfg(target_os = "linux")]
fn recover_disconnected_petallink_mount(path: &Path) -> AppResult<bool> {
    cleanup_verified_petallink_mount(path, true)
}

/// 正常关闭时的兜底卸载。
///
/// 调用方只可传入当前进程已经持有的 [`crate::virtual_fs::FuseMountSession`] 挂载点。
/// 即便如此仍重新核对 mount id、`fuse.petallink` 身份和当前 uid；路径若已被别的
/// 文件系统替换则拒绝操作。
#[cfg(target_os = "linux")]
pub(crate) fn detach_owned_petallink_mount(path: &Path) -> AppResult<bool> {
    cleanup_verified_petallink_mount(path, false)
}

#[cfg(target_os = "linux")]
fn cleanup_verified_petallink_mount(path: &Path, require_disconnected: bool) -> AppResult<bool> {
    let Some(record) = current_mount_record(path)? else {
        return Ok(false);
    };
    if !is_petallink_mount_record(&record) {
        return Ok(false);
    }
    if require_disconnected {
        match fs::read_dir(path) {
            Err(error) if error.raw_os_error() == Some(libc::ENOTCONN) => {}
            Ok(_) => return Ok(false),
            Err(error) => {
                return Err(AppError::config(format!(
                    "无法确认 PetalLink 挂载是否已经断开：{}：{error}",
                    path.display()
                )));
            }
        }
    }

    let current_uid = unsafe { libc::geteuid() };
    if mount_owner_uid(&record) != Some(current_uid) {
        return Err(AppError::config(format!(
            "检测到 PetalLink 挂载，但其所有者不是当前用户，拒绝自动卸载：{}",
            path.display()
        )));
    }

    let mut last_error = None;
    let configured_helper = std::env::var_os("FUSERMOUNT_PATH").map(PathBuf::from);
    let candidates: Vec<PathBuf> =
        configured_helper
            .map(|helper| vec![helper])
            .unwrap_or_else(|| {
                [
                    "fusermount3",
                    "fusermount",
                    "/bin/fusermount3",
                    "/bin/fusermount",
                ]
                .into_iter()
                .map(PathBuf::from)
                .collect()
            });
    for helper in candidates {
        // 防止检查后路径被换挂：mount id 和完整 PetalLink 身份必须仍与首次快照一致。
        let Some(current) = current_mount_record(path)? else {
            return Ok(true);
        };
        if current.mount_id != record.mount_id
            || !is_petallink_mount_record(&current)
            || mount_owner_uid(&current) != Some(current_uid)
        {
            return Err(AppError::config(format!(
                "挂载点身份在自动卸载前发生变化，拒绝继续操作：{}",
                path.display()
            )));
        }

        match Command::new(&helper)
            .args(["-u", "-z", "--"])
            .arg(path)
            .output()
        {
            Ok(output) if output.status.success() => {
                for _ in 0..10 {
                    match current_mount_record(path)? {
                        None => {
                            tracing::warn!(
                                mountpoint = %path.display(),
                                helper = %helper.display(),
                                disconnected_only = require_disconnected,
                                "已清理 PetalLink FUSE 挂载"
                            );
                            return Ok(true);
                        }
                        Some(current) if current.mount_id == record.mount_id => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Some(_) => {
                            return Err(AppError::config(format!(
                                "卸载期间挂载点被另一个文件系统替换：{}",
                                path.display()
                            )));
                        }
                    }
                }
                last_error = Some("卸载命令成功返回，但挂载记录仍然存在".to_string());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = Some(format!(
                    "{} 退出状态 {}：{}",
                    helper.display(),
                    output.status,
                    stderr.trim()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                last_error = Some(format!("无法执行 {}：{error}", helper.display()));
            }
        }
    }
    Err(AppError::config(format!(
        "无法清理 PetalLink 挂载 {}：{}",
        path.display(),
        last_error.unwrap_or_else(|| "未找到 fusermount3，请安装 fuse3".to_string())
    )))
}

/// `/proc/*/mountinfo` 使用反斜线加三位八进制数转义空格、制表符、换行和反斜线。
#[cfg(target_os = "linux")]
fn decode_mountinfo_path(encoded: &str) -> PathBuf {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..=index + 3].iter().all(u8::is_ascii_digit)
            && bytes[index + 1..=index + 3].iter().all(|b| *b < b'8')
        {
            let value = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    PathBuf::from(OsString::from_vec(decoded))
}

/// 序列化配置为 JSON。对齐 dart `_toJson`。
fn to_json(c: &AppConfig) -> Value {
    json!({
        "oauthRedirectUri": c.oauth_redirect_uri,
        "oauthCallbackPort": c.oauth_callback_port,
        "mountDir": c.mount_dir,
        "mountConfigured": c.mount_configured,
        "virtualDriveEnabled": c.virtual_drive_enabled,
        "virtualMountDir": c.virtual_mount_dir,
        "concurrency": c.concurrency,
        "pollIntervalSec": c.poll_interval_sec,
        "debounceSec": c.debounce_sec,
        "skipPatterns": c.skip_patterns,
        // 排序字段序列化为枚举名（camelCase，对齐前端）
        "sortField": sort_field_to_str(c.sort_field),
        "sortOrder": sort_order_to_str(c.sort_order),
        "showTrayIcon": c.show_tray_icon,
    })
}

/// 反序列化配置。含旧默认值迁移（30/30 → 10/3、未配置的旧默认 mount_dir 清空）。
/// 对齐 dart `_fromJson`。纯解析（不落盘）——返回 (config, dirty)，由调用方（load）决定是否 save。
/// 这样测试调用 from_json 不会污染真实 config.json。
fn from_json(json: &Value) -> (AppConfig, bool, bool) {
    let default = AppConfig::default();
    let mut config = AppConfig {
        oauth_redirect_uri: json
            .get("oauthRedirectUri")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| DEFAULT_REDIRECT_URI.to_string()),
        oauth_callback_port: json
            .get("oauthCallbackPort")
            .and_then(Value::as_u64)
            .map(|v| v as u16)
            .unwrap_or(default.oauth_callback_port),
        mount_dir: json
            .get("mountDir")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| default.mount_dir.clone()),
        mount_configured: json
            .get("mountConfigured")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        virtual_drive_enabled: json
            .get("virtualDriveEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        virtual_mount_dir: json
            .get("virtualMountDir")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_default(),
        concurrency: json
            .get("concurrency")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(default.concurrency),
        poll_interval_sec: json
            .get("pollIntervalSec")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(default.poll_interval_sec),
        debounce_sec: json
            .get("debounceSec")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(default.debounce_sec),
        skip_patterns: json
            .get("skipPatterns")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| default.skip_patterns.clone()),
        sort_field: parse_sort_field(json.get("sortField")),
        sort_order: parse_sort_order(json.get("sortOrder")),
        // 旧配置文件无此键 → 默认显示托盘图标
        show_tray_icon: json
            .get("showTrayIcon")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    };

    // 旧配置缺少按需云盘字段时写入安全默认值，使迁移后的导出合同保持完整。
    let mut dirty = json
        .get("virtualDriveEnabled")
        .and_then(Value::as_bool)
        .is_none()
        || json
            .get("virtualMountDir")
            .and_then(Value::as_str)
            .is_none();
    // 自动升级旧默认值：
    // - poll_interval_sec：新版校验要求 0 或 ≥60。旧版可能存的是秒级小值（如 10/30），
    //   这些值在「定时全量刷新」语义下过激进，统一迁移到新默认 900；0（关闭）与 ≥60 的值保留。
    // - debounce_sec：旧版 hardcoded 30 → 新默认 3。
    if (config.poll_interval_sec != 0 && config.poll_interval_sec < 60) || config.debounce_sec == 30
    {
        if config.poll_interval_sec != 0 && config.poll_interval_sec < 60 {
            config.poll_interval_sec = default.poll_interval_sec;
        }
        if config.debounce_sec == 30 {
            config.debounce_sec = default.debounce_sec;
        }
        dirty = true;
    }
    // 迁移：旧版默认 mount_dir="~/hwcloud-drive" 但用户从未配置（mount_configured=false）
    // → 清空。新版不再设默认目录，未配置时 mount_dir 应为空、不启动同步。
    // 仅清"未配置 + 恰为旧默认值"的情形；用户显式配置过（mount_configured=true）的保留。
    if !config.mount_configured && config.mount_dir == DEFAULT_MOUNT_DIR {
        config.mount_dir = String::new();
        dirty = true;
    }
    // Linux 不再启动传统同步目录。旧 traditional 配置没有第二个可见挂载点，也不能
    // 安全地把可能含未上传内容的旧目录静默搬到另一个文件系统。因此保留旧目录原样，
    // 把应用退回待配置态，并要求首次新配置前丢弃旧 DB/缓存基线。
    #[cfg(target_os = "linux")]
    let mut reset_sync_state = false;
    #[cfg(target_os = "linux")]
    if config.mount_configured
        && !config.virtual_drive_enabled
        && !config.virtual_mount_dir.trim().is_empty()
    {
        // 旧 UI 允许关闭 FUSE、但仍保留完整双目录信息。Linux 现在只有按需模式，
        // 恢复开关即可，无需替换 backing 或清理同步基线。
        config.virtual_drive_enabled = true;
        dirty = true;
    } else if config.mount_configured && !config.virtual_drive_enabled {
        config.mount_configured = false;
        config.mount_dir.clear();
        config.virtual_drive_enabled = false;
        config.virtual_mount_dir.clear();
        dirty = true;
        reset_sync_state = true;
    }
    #[cfg(target_os = "linux")]
    if !config.mount_configured
        && (!config.mount_dir.is_empty()
            || config.virtual_drive_enabled
            || !config.virtual_mount_dir.is_empty())
    {
        config.mount_dir.clear();
        config.virtual_drive_enabled = false;
        config.virtual_mount_dir.clear();
        dirty = true;
    }

    // FUSE 按需云盘目前只在 Linux 装配。跨平台导入 Linux 配置时安全降级为传统模式，
    // 避免 macOS 因保留 virtual 开关而使所有“打开本地项”命令不可用。
    #[cfg(not(target_os = "linux"))]
    if config.virtual_drive_enabled || !config.virtual_mount_dir.is_empty() {
        config.mount_configured = false;
        config.mount_dir.clear();
        config.virtual_drive_enabled = false;
        config.virtual_mount_dir.clear();
        dirty = true;
    }
    #[cfg(not(target_os = "linux"))]
    let reset_sync_state = false;
    (config, dirty, reset_sync_state)
}

/// 排序字段字符串表示（与 dart 枚举 name 一致）
fn sort_field_to_str(f: SortField) -> &'static str {
    match f {
        SortField::Name => "name",
        SortField::Size => "size",
        SortField::ModifiedTime => "modifiedTime",
    }
}

/// 将排序方向映射为持久化字符串。
fn sort_order_to_str(o: SortOrder) -> &'static str {
    match o {
        SortOrder::Ascending => "ascending",
        SortOrder::Descending => "descending",
    }
}

/// 解析排序字段，未知值回退为按名称排序。
fn parse_sort_field(v: Option<&Value>) -> SortField {
    match v.and_then(Value::as_str) {
        Some("name") => SortField::Name,
        Some("size") => SortField::Size,
        Some("modifiedTime") => SortField::ModifiedTime,
        _ => SortField::Name,
    }
}

/// 解析排序方向，未知值回退为升序。
fn parse_sort_order(v: Option<&Value>) -> SortOrder {
    match v.and_then(Value::as_str) {
        Some("ascending") => SortOrder::Ascending,
        Some("descending") => SortOrder::Descending,
        _ => SortOrder::Ascending,
    }
}

#[cfg(test)]
/// 配置兼容性合同测试。
mod tests {
    use super::*;

    /// 旧配置文件无 showTrayIcon 键 → 解析默认 true；显式 false 保留。
    #[test]
    fn show_tray_icon_defaults_true_when_key_missing() {
        let (config, _, _) = from_json(&json!({}));
        assert!(config.show_tray_icon);

        let (config, _, _) = from_json(&json!({ "showTrayIcon": false }));
        assert!(!config.show_tray_icon);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn old_traditional_config_is_gated_before_linux_fuse_migration() {
        let (config, dirty, reset_sync_state) = from_json(&json!({
            "mountDir": "/tmp/petallink-backing",
            "mountConfigured": true
        }));

        assert!(!config.mount_configured);
        assert!(config.mount_dir.is_empty());
        assert!(!config.virtual_drive_enabled);
        assert!(config.virtual_mount_dir.is_empty());
        assert!(dirty);
        assert!(reset_sync_state);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn old_config_remains_a_traditional_drive_off_linux() {
        let (config, dirty, reset_sync_state) = from_json(&json!({
            "mountDir": "/tmp/petallink-backing",
            "mountConfigured": true
        }));

        assert!(config.mount_configured);
        assert!(!config.virtual_drive_enabled);
        assert!(config.virtual_mount_dir.is_empty());
        assert!(dirty);
        assert!(!reset_sync_state);
    }

    #[test]
    fn virtual_drive_fields_roundtrip_through_json_contract() {
        let config = AppConfig {
            mount_dir: "/tmp/petallink-backing".to_string(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/tmp/PetalLinkDrive".to_string(),
            ..AppConfig::default()
        };

        let serialized = to_json(&config);
        assert_eq!(serialized["virtualDriveEnabled"], true);
        assert_eq!(serialized["virtualMountDir"], "/tmp/PetalLinkDrive");

        let (decoded, dirty, reset_sync_state) = from_json(&serialized);
        assert!(decoded.virtual_drive_enabled);
        assert_eq!(decoded.mount_dir, "/tmp/petallink-backing");
        assert_eq!(decoded.virtual_mount_dir, "/tmp/PetalLinkDrive");
        assert!(!dirty);
        assert!(!reset_sync_state);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn first_linux_directory_selection_gets_unique_persistent_managed_backing() {
        let temp = tempfile::tempdir().unwrap();
        let visible = temp.path().join("visible");
        let selected = AppConfig {
            mount_dir: String::new(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: visible.to_string_lossy().into_owned(),
            ..AppConfig::default()
        };

        let normalized = normalize_for_save(selected).unwrap();
        assert!(normalized.virtual_drive_enabled);
        assert_eq!(normalized.expanded_virtual_mount_dir(), visible);
        assert!(normalized
            .expanded_mount_dir()
            .starts_with(linux_managed_backing_root().unwrap()));
        assert_ne!(
            normalize_for_save(AppConfig {
                virtual_drive_enabled: true,
                virtual_mount_dir: temp.path().join("visible-2").to_string_lossy().into_owned(),
                mount_configured: true,
                ..AppConfig::default()
            })
            .unwrap()
            .mount_dir,
            normalized.mount_dir,
            "新配置必须隔离旧账号 backing"
        );
        assert!(normalized.validate().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn existing_linux_virtual_config_preserves_legacy_backing() {
        let config = AppConfig {
            mount_dir: "/tmp/legacy-petallink-backing".to_string(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/tmp/PetalLinkDrive".to_string(),
            ..AppConfig::default()
        };

        let normalized = normalize_for_save(config.clone()).unwrap();
        assert_eq!(normalized.mount_dir, config.mount_dir);
        assert_eq!(normalized.virtual_mount_dir, config.virtual_mount_dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_creates_empty_writable_directories() {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig {
            mount_dir: temp.path().join("backing").to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: temp.path().join("drive").to_string_lossy().into_owned(),
            ..AppConfig::default()
        };

        validate_virtual_drive_capabilities(&config).unwrap();
        assert!(config.expanded_mount_dir().is_dir());
        assert!(config.expanded_virtual_mount_dir().is_dir());
        assert_eq!(
            fs::read_dir(config.expanded_virtual_mount_dir())
                .unwrap()
                .count(),
            0
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_rejects_nonempty_mountpoint() {
        let temp = tempfile::tempdir().unwrap();
        let backing = temp.path().join("backing");
        let virtual_mount = temp.path().join("drive");
        fs::create_dir_all(&virtual_mount).unwrap();
        fs::write(virtual_mount.join("existing.txt"), b"keep me").unwrap();
        let config = AppConfig {
            mount_dir: backing.to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: virtual_mount.to_string_lossy().into_owned(),
            ..AppConfig::default()
        };

        let error = validate_virtual_drive_capabilities(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("必须为空"), "{error}");
        assert_eq!(
            fs::read(virtual_mount.join("existing.txt")).unwrap(),
            b"keep me"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_rejects_existing_mount() {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig {
            mount_dir: temp.path().join("backing").to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/proc".to_string(),
            ..AppConfig::default()
        };

        let error = validate_virtual_drive_capabilities(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("已被其他文件系统占用"), "{error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parsing_virtual_config_has_no_filesystem_side_effects() {
        let temp = tempfile::tempdir().unwrap();
        let backing = temp.path().join("missing-backing");
        let virtual_mount = temp.path().join("missing-drive");
        let raw = json!({
            "mountDir": backing,
            "mountConfigured": true,
            "virtualDriveEnabled": true,
            "virtualMountDir": virtual_mount,
        })
        .to_string();

        let (config, _, _) = parse_config_raw(&raw).unwrap();
        assert!(config.virtual_drive_enabled);
        assert!(!config.expanded_mount_dir().exists());
        assert!(!config.expanded_virtual_mount_dir().exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn importing_config_is_a_pure_parse_until_config_save() {
        let temp = tempfile::tempdir().unwrap();
        let backing = temp.path().join("import-backing");
        let virtual_mount = temp.path().join("import-drive");
        let raw = json!({
            "mountDir": backing,
            "mountConfigured": true,
            "virtualDriveEnabled": true,
            "virtualMountDir": virtual_mount,
        })
        .to_string();

        let imported = ConfigStore::import_from_json(&raw).unwrap();

        assert!(imported.virtual_drive_enabled);
        assert!(!imported.expanded_mount_dir().exists());
        assert!(!imported.expanded_virtual_mount_dir().exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_decoder_handles_escaped_paths() {
        assert_eq!(
            decode_mountinfo_path("/tmp/PetalLink\\040Drive\\134cache"),
            PathBuf::from("/tmp/PetalLink Drive\\cache")
        );
        let record = parse_mount_record(
            "42 31 0:77 / /tmp/PetalLink\\040Drive rw,nosuid,nodev - \
             fuse.petallink PetalLink rw,user_id=1000,group_id=1000\n",
            Path::new("/tmp/PetalLink Drive"),
        )
        .unwrap();
        assert_eq!(record.mount_id, 42);
        assert_eq!(record.fs_type, "fuse.petallink");
        assert_eq!(record.source, "PetalLink");
        assert_eq!(record.super_options, "rw,user_id=1000,group_id=1000");
        assert!(is_petallink_mount_record(&record));
        assert_eq!(mount_owner_uid(&record), Some(1000));
        assert!(is_current_mountpoint(Path::new("/")).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_cleanup_identity_rejects_other_filesystems_and_owners() {
        let other_fuse = parse_mount_record(
            "51 31 0:88 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.sshfs remote rw,user_id=1000,group_id=1000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(!is_petallink_mount_record(&other_fuse));

        let wrong_source = parse_mount_record(
            "52 31 0:89 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.petallink NotPetalLink rw,user_id=1000,group_id=1000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(!is_petallink_mount_record(&wrong_source));

        let other_owner = parse_mount_record(
            "53 31 0:90 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.petallink PetalLink rw,user_id=2000,group_id=2000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(is_petallink_mount_record(&other_owner));
        assert_eq!(mount_owner_uid(&other_owner), Some(2000));

        assert!(
            parse_mount_record(
                "54 31 0:91 / /mnt/huawei_cloud-old rw - \
                 fuse.petallink PetalLink rw,user_id=1000\n",
                Path::new("/mnt/huawei_cloud"),
            )
            .is_none(),
            "挂载点必须精确匹配，不能按路径前缀误判"
        );
    }
}
