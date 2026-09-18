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

#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use crate::core::config::{AppConfig, SortField, SortOrder, DEFAULT_MOUNT_DIR};
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
        let json = to_json(&config)?;
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
        Ok(serde_json::to_string_pretty(&to_json(&config)?)?)
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
#[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
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

/// 磁盘 JSON 合同的镜像（camelCase 键）。与 IPC 的 `AppConfig`（snake_case）分离，
/// 两份合同互不泄漏；serde derive 替代历史的手写逐字段映射。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AppConfigJson {
    oauth_redirect_uri: String,
    oauth_callback_port: u16,
    mount_dir: String,
    mount_configured: bool,
    virtual_drive_enabled: bool,
    virtual_mount_dir: String,
    concurrency: u32,
    poll_interval_sec: u32,
    debounce_sec: u32,
    skip_patterns: Vec<String>,
    sort_field: SortField,
    sort_order: SortOrder,
    show_tray_icon: bool,
}

impl Default for AppConfigJson {
    fn default() -> Self {
        AppConfig::default().into()
    }
}

impl From<AppConfig> for AppConfigJson {
    fn from(c: AppConfig) -> Self {
        Self {
            oauth_redirect_uri: c.oauth_redirect_uri,
            oauth_callback_port: c.oauth_callback_port,
            mount_dir: c.mount_dir,
            mount_configured: c.mount_configured,
            virtual_drive_enabled: c.virtual_drive_enabled,
            virtual_mount_dir: c.virtual_mount_dir,
            concurrency: c.concurrency,
            poll_interval_sec: c.poll_interval_sec,
            debounce_sec: c.debounce_sec,
            skip_patterns: c.skip_patterns,
            sort_field: c.sort_field,
            sort_order: c.sort_order,
            show_tray_icon: c.show_tray_icon,
        }
    }
}

impl From<AppConfigJson> for AppConfig {
    fn from(j: AppConfigJson) -> Self {
        Self {
            oauth_redirect_uri: j.oauth_redirect_uri,
            oauth_callback_port: j.oauth_callback_port,
            mount_dir: j.mount_dir,
            mount_configured: j.mount_configured,
            virtual_drive_enabled: j.virtual_drive_enabled,
            virtual_mount_dir: j.virtual_mount_dir,
            concurrency: j.concurrency,
            poll_interval_sec: j.poll_interval_sec,
            debounce_sec: j.debounce_sec,
            skip_patterns: j.skip_patterns,
            sort_field: j.sort_field,
            sort_order: j.sort_order,
            show_tray_icon: j.show_tray_icon,
        }
    }
}

/// 序列化配置为磁盘 JSON。
fn to_json(c: &AppConfig) -> AppResult<Value> {
    serde_json::to_value(AppConfigJson::from(c.clone()))
        .map_err(|e| AppError::config(format!("配置序列化失败：{e}")))
}

/// 反序列化配置。含旧默认值迁移（30/30 → 10/3、未配置的旧默认 mount_dir 清空）。
/// 对齐 dart `_fromJson`。纯解析（不落盘）——返回 (config, dirty)，由调用方（load）决定是否 save。
/// 这样测试调用 from_json 不会污染真实 config.json。
fn from_json(json: &Value) -> (AppConfig, bool, bool) {
    let mut config: AppConfig = match serde_json::from_value::<AppConfigJson>(json.clone()) {
        Ok(parsed) => parsed.into(),
        Err(error) => {
            // 整体结构不兼容（如手写破坏了类型）时回退默认并标记回写，
            // 与逐字段容忍的旧实现同样保证 load 不失败。
            tracing::warn!(%error, "配置结构不兼容，回退为默认配置并回写");
            AppConfig::default()
        }
    };
    let default = AppConfig::default();

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

    // from_json 在非 Linux 平台会把 virtual 字段连同 mount_configured 一起安全降级清零，
    // roundtrip 合同仅在 Linux 成立，因此本测试只在 Linux 运行。
    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_drive_fields_roundtrip_through_json_contract() {
        let config = AppConfig {
            mount_dir: "/tmp/petallink-backing".to_string(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/tmp/PetalLinkDrive".to_string(),
            ..AppConfig::default()
        };

        let serialized = to_json(&config).unwrap();
        assert_eq!(serialized["virtualDriveEnabled"], true);
        assert_eq!(serialized["virtualMountDir"], "/tmp/PetalLinkDrive");

        let (decoded, dirty, reset_sync_state) = from_json(&serialized);
        assert!(decoded.virtual_drive_enabled);
        assert_eq!(decoded.mount_dir, "/tmp/petallink-backing");
        assert_eq!(decoded.virtual_mount_dir, "/tmp/PetalLinkDrive");
        assert!(!dirty);
        assert!(!reset_sync_state);
    }

    /// 磁盘 JSON 合同金样测试：手写映射时代的键集合与取值在
    /// serde 镜像结构下完全一致（平台无关字段）。
    #[test]
    fn golden_disk_config_contract_matches_legacy_mapping() {
        let config = AppConfig {
            oauth_redirect_uri: "http://127.0.0.1:9999/oauth/callback".to_string(),
            oauth_callback_port: 9999,
            mount_dir: "/tmp/petallink-backing".to_string(),
            mount_configured: true,
            concurrency: 8,
            poll_interval_sec: 120,
            debounce_sec: 5,
            skip_patterns: vec![".DS_Store".to_string(), ".tmp".to_string()],
            sort_field: SortField::ModifiedTime,
            sort_order: SortOrder::Descending,
            show_tray_icon: false,
            ..AppConfig::default()
        };

        let serialized = to_json(&config).unwrap();
        let object = serialized.as_object().unwrap();
        // 手写映射时代的键集合（顺序无关，且不得多出新键）
        let expected_keys = [
            "oauthRedirectUri",
            "oauthCallbackPort",
            "mountDir",
            "mountConfigured",
            "virtualDriveEnabled",
            "virtualMountDir",
            "concurrency",
            "pollIntervalSec",
            "debounceSec",
            "skipPatterns",
            "sortField",
            "sortOrder",
            "showTrayIcon",
        ];
        assert_eq!(object.len(), expected_keys.len());
        for key in expected_keys {
            assert!(object.contains_key(key), "磁盘合同缺少键 {key}");
        }
        assert_eq!(serialized["oauthCallbackPort"], 9999);
        assert_eq!(serialized["concurrency"], 8);
        assert_eq!(serialized["pollIntervalSec"], 120);
        assert_eq!(serialized["debounceSec"], 5);
        assert_eq!(serialized["sortField"], "modifiedTime");
        assert_eq!(serialized["sortOrder"], "descending");
        assert_eq!(serialized["showTrayIcon"], false);

        // 同一样本经 from_json 解析后平台无关字段保持一致。
        let (decoded, _, _) = from_json(&serialized);
        assert_eq!(decoded.oauth_callback_port, 9999);
        assert_eq!(decoded.concurrency, 8);
        assert_eq!(decoded.poll_interval_sec, 120);
        assert_eq!(decoded.debounce_sec, 5);
        assert_eq!(decoded.skip_patterns, vec![".DS_Store", ".tmp"]);
        assert_eq!(decoded.sort_field, SortField::ModifiedTime);
        assert_eq!(decoded.sort_order, SortOrder::Descending);
        assert!(!decoded.show_tray_icon);
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
}
