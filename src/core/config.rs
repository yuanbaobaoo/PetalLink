//! 应用配置模型（需求 F-CFG-03）。
//!
//! 对齐 `legacy/lib/core/config/app_config.dart`。所有可配置项集中在此，不含 token。
//! 持久化为 JSON 文件，见 [`crate::core::config_store`]。

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::constants::DEFAULT_CALLBACK_PORT;
use crate::error::{AppError, AppResult};

/// 同步状态展示排序字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum SortField {
    #[default]
    Name,
    Size,
    ModifiedTime,
}

/// 列表排序方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

/// 默认 OAuth 回调 URI（必须与 AGC 后台配置一致）
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:9999/oauth/callback";

/// 旧版默认挂载目录（仅供迁移引用：未配置但残留此值时清空）。
///
/// 新版不再设默认目录——用户未配置同步目录时 mount_dir 为空、不启动任何同步，
/// 避免误以为已默认到 ~/hwcloud-drive 而自动同步覆盖本地内容。
pub const DEFAULT_MOUNT_DIR: &str = "~/hwcloud-drive";

/// 默认跳过文件列表（通配符，名称匹配）
pub const DEFAULT_SKIP_PATTERNS: &[&str] = &[".DS_Store", ".tmp", "~$*", ".Trash"];

/// 应用配置（不可变值对象，修改通过 [`AppConfig::with`] 链式构造）。
///
/// 默认值对齐 dart：concurrency=6, pollIntervalSec=10, debounceSec=3。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct AppConfig {
    /// OAuth 回调 URI（必须与 AGC 后台一致）
    pub oauth_redirect_uri: String,
    /// OAuth 回调端口
    pub oauth_callback_port: u16,
    /// 同步引擎使用的本地物理目录（可能含 ~ 前缀）。
    ///
    /// Linux 上这是应用管理的隐藏 backing，不是用户选择或日常打开的目录；
    /// 用户选择的唯一目录保存在 [`Self::virtual_mount_dir`]。
    pub mount_dir: String,
    /// 用户是否已显式配置过挂载目录（首次同步引导用，F-MOUNT-13）。
    /// 区分"默认值"与"用户已确认"，避免未选目录就自动同步覆盖本地已有内容。
    pub mount_configured: bool,
    /// 是否启用 Linux FUSE 按需云盘。
    ///
    /// Linux 已配置目录时该值必须为 true；false 只用于“尚未配置”的初始态。
    pub virtual_drive_enabled: bool,
    /// 用户可见的 FUSE 挂载目录；`mount_dir` 仍作为物理 backing 目录。
    pub virtual_mount_dir: String,
    /// 并发传输数，范围 1-20（Q1 决策：默认 6）
    pub concurrency: u32,
    /// 云端定时刷新间隔（秒）。0 = 关闭自动刷新；开启时最小 60 秒。默认 900（15 分钟）。
    /// 每次到期全量 BFS 重拉云端树，使云端的新增/修改/删除自动同步到本地。
    pub poll_interval_sec: u32,
    /// 变更 debounce 时长，默认 3 秒（F-MOUNT-09）
    pub debounce_sec: u32,
    /// 跳过文件列表（通配符）
    pub skip_patterns: Vec<String>,
    /// 排序字段
    pub sort_field: SortField,
    /// 排序方向
    pub sort_order: SortOrder,
    /// 是否显示系统托盘图标。默认显示。
    /// 关闭后后台同步无托盘入口，此时关闭主窗口或退出应用会直接真退出。
    pub show_tray_icon: bool,
}

impl Default for AppConfig {
    /// 构造不自动启用同步目录的安全默认配置。
    fn default() -> Self {
        Self {
            oauth_redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            oauth_callback_port: DEFAULT_CALLBACK_PORT,
            mount_dir: String::new(),
            mount_configured: false,
            virtual_drive_enabled: false,
            virtual_mount_dir: String::new(),
            concurrency: 6,
            poll_interval_sec: 60,
            debounce_sec: 3,
            skip_patterns: DEFAULT_SKIP_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            sort_field: SortField::Name,
            sort_order: SortOrder::Ascending,
            show_tray_icon: true,
        }
    }
}

impl AppConfig {
    /// 校验配置合法性（范围、非空等）。非法则返回 [`AppError::Config`]。
    /// 对齐 dart `AppConfig.validate()`。
    pub fn validate(&self) -> AppResult<()> {
        if self.oauth_callback_port < 1 {
            return Err(AppError::config(format!(
                "回调端口越界：{}",
                self.oauth_callback_port
            )));
        }
        if !(1..=20).contains(&self.concurrency) {
            return Err(AppError::config(format!(
                "并发数必须在 1-20 之间：{}",
                self.concurrency
            )));
        }
        // 云端定时刷新间隔：0 = 关闭；开启时最小 60 秒（防止误设过小拖垮大网盘）
        if self.poll_interval_sec != 0 && self.poll_interval_sec < 60 {
            return Err(AppError::config(format!(
                "云端刷新间隔必须为 0（关闭）或 ≥ 60 秒：{}",
                self.poll_interval_sec
            )));
        }
        if self.debounce_sec < 1 {
            return Err(AppError::config("debounce 时长必须 ≥ 1 秒".to_string()));
        }
        if self.mount_configured {
            let expanded = self.expanded_mount_dir();
            #[cfg(target_os = "linux")]
            validate_safe_internal_directory(&self.mount_dir, &expanded, "按需云盘本地缓存目录")?;
            #[cfg(not(target_os = "linux"))]
            validate_safe_user_directory(&self.mount_dir, &expanded, "同步目录")?;
        }
        #[cfg(target_os = "linux")]
        if self.mount_configured && !self.virtual_drive_enabled {
            return Err(AppError::config(
                "Linux 仅支持按需云盘；已配置目录时必须启用 FUSE 云盘".to_string(),
            ));
        }
        #[cfg(not(target_os = "linux"))]
        if self.virtual_drive_enabled {
            return Err(AppError::config(
                "按需云盘当前仅支持 Linux；本平台请使用传统同步目录".to_string(),
            ));
        }
        if self.virtual_drive_enabled {
            if !self.mount_configured {
                return Err(AppError::config(
                    "启用按需云盘前必须先配置物理 backing 目录".to_string(),
                ));
            }

            let backing = self.expanded_mount_dir();
            let virtual_mount = self.expanded_virtual_mount_dir();
            validate_safe_user_directory(
                &self.virtual_mount_dir,
                &virtual_mount,
                "按需云盘挂载目录",
            )?;
            if paths_overlap(&backing, &virtual_mount) {
                return Err(AppError::config(format!(
                    "按需云盘挂载目录必须与物理 backing 目录不同且互不包含：{} ↔ {}",
                    backing.display(),
                    virtual_mount.display()
                )));
            }
        }
        Ok(())
    }

    /// 展开 ~ 为真实 home 路径。
    /// 对齐 dart `AppConfig.expandedMountDir`。
    pub fn expanded_mount_dir(&self) -> PathBuf {
        expand_home_path(&self.mount_dir)
    }

    /// 展开按需云盘挂载目录中的 `~` 为真实 home 路径。
    pub fn expanded_virtual_mount_dir(&self) -> PathBuf {
        expand_home_path(&self.virtual_mount_dir)
    }

    /// 链式构造：返回带修改的新配置（不可变值对象）。
    #[allow(clippy::too_many_arguments)]
    pub fn with(
        &self,
        oauth_redirect_uri: Option<String>,
        oauth_callback_port: Option<u16>,
        mount_dir: Option<String>,
        mount_configured: Option<bool>,
        virtual_drive_enabled: Option<bool>,
        virtual_mount_dir: Option<String>,
        concurrency: Option<u32>,
        poll_interval_sec: Option<u32>,
        debounce_sec: Option<u32>,
        skip_patterns: Option<Vec<String>>,
        sort_field: Option<SortField>,
        sort_order: Option<SortOrder>,
        show_tray_icon: Option<bool>,
    ) -> Self {
        Self {
            oauth_redirect_uri: oauth_redirect_uri
                .unwrap_or_else(|| self.oauth_redirect_uri.clone()),
            oauth_callback_port: oauth_callback_port.unwrap_or(self.oauth_callback_port),
            mount_dir: mount_dir.unwrap_or_else(|| self.mount_dir.clone()),
            mount_configured: mount_configured.unwrap_or(self.mount_configured),
            virtual_drive_enabled: virtual_drive_enabled.unwrap_or(self.virtual_drive_enabled),
            virtual_mount_dir: virtual_mount_dir.unwrap_or_else(|| self.virtual_mount_dir.clone()),
            concurrency: concurrency.unwrap_or(self.concurrency),
            poll_interval_sec: poll_interval_sec.unwrap_or(self.poll_interval_sec),
            debounce_sec: debounce_sec.unwrap_or(self.debounce_sec),
            skip_patterns: skip_patterns.unwrap_or_else(|| self.skip_patterns.clone()),
            sort_field: sort_field.unwrap_or(self.sort_field),
            sort_order: sort_order.unwrap_or(self.sort_order),
            show_tray_icon: show_tray_icon.unwrap_or(self.show_tray_icon),
        }
    }
}

/// 展开配置路径中唯一支持的 `~/` 形式。
fn expand_home_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        return Path::new(&home).join(rest);
    }
    PathBuf::from(raw)
}

/// 纯配置层的目录安全校验，不访问文件系统，因而可安全用于普通配置加载。
fn validate_safe_user_directory(raw: &str, expanded: &Path, label: &str) -> AppResult<()> {
    validate_safe_internal_directory(raw, expanded, label)?;
    if let Some(data_dir) = dirs::data_dir() {
        if expanded.starts_with(&data_dir) {
            return Err(AppError::config(format!(
                "不能把 Application Support 目录作为{label}"
            )));
        }
    }
    Ok(())
}

/// 校验应用管理的内部目录。
///
/// 与用户可见目录的校验相比，它有意允许 XDG data/Application Support：
/// Linux 的持久 backing 正应放在那里，不能使用可能被系统自动清理的 cache 目录。
fn validate_safe_internal_directory(raw: &str, expanded: &Path, label: &str) -> AppResult<()> {
    if raw.trim().is_empty() {
        return Err(AppError::config(format!("{label}不能为空")));
    }
    if !expanded.is_absolute() {
        return Err(AppError::config(format!("{label}必须是绝对路径：{raw}")));
    }
    if expanded
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(AppError::config(format!(
            "{label}不能包含 . 或 .. 路径段：{raw}"
        )));
    }
    if expanded == Path::new("/") {
        return Err(AppError::config(format!("不能把系统根目录作为{label}")));
    }
    if let Some(home) = dirs::home_dir() {
        if expanded == home {
            return Err(AppError::config(format!("不能把用户 Home 目录作为{label}")));
        }
    }
    Ok(())
}

/// 两个已校验的绝对路径相同或任一方包含另一方时视为重叠。
fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_virtual_config() -> AppConfig {
        AppConfig {
            mount_dir: "/tmp/petallink-config-test/backing".to_string(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/tmp/petallink-config-test/drive".to_string(),
            ..AppConfig::default()
        }
    }

    #[test]
    fn virtual_drive_defaults_are_backward_compatible() {
        let config = AppConfig::default();
        assert!(!config.virtual_drive_enabled);
        assert!(config.virtual_mount_dir.is_empty());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn virtual_drive_requires_a_configured_backing_directory() {
        let config = AppConfig {
            virtual_drive_enabled: true,
            virtual_mount_dir: "/tmp/PetalLinkDrive".to_string(),
            ..AppConfig::default()
        };
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("backing"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn configured_linux_directory_cannot_fall_back_to_traditional_sync() {
        let config = AppConfig {
            mount_dir: "/tmp/petallink-config-test/traditional".to_string(),
            mount_configured: true,
            virtual_drive_enabled: false,
            ..AppConfig::default()
        };
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("仅支持按需云盘"), "{error}");
    }

    #[test]
    fn virtual_mount_directory_must_be_safe_and_absolute() {
        for invalid in ["", "relative/path", "/", "/tmp/../PetalLinkDrive"] {
            let config = AppConfig {
                virtual_mount_dir: invalid.to_string(),
                ..valid_virtual_config()
            };
            assert!(
                config.validate().is_err(),
                "unsafe virtual mount unexpectedly accepted: {invalid}"
            );
        }

        if let Some(home) = dirs::home_dir() {
            let config = AppConfig {
                virtual_mount_dir: home.to_string_lossy().into_owned(),
                ..valid_virtual_config()
            };
            assert!(config.validate().is_err());
        }
        if let Some(data_dir) = dirs::data_dir() {
            let config = AppConfig {
                virtual_mount_dir: data_dir
                    .join("PetalLinkDrive")
                    .to_string_lossy()
                    .into_owned(),
                ..valid_virtual_config()
            };
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn backing_and_virtual_mount_must_not_overlap() {
        for virtual_mount in [
            "/tmp/petallink-config-test/backing",
            "/tmp/petallink-config-test/backing/visible",
            "/tmp/petallink-config-test",
        ] {
            let config = AppConfig {
                virtual_mount_dir: virtual_mount.to_string(),
                ..valid_virtual_config()
            };
            assert!(
                config.validate().is_err(),
                "overlapping path unexpectedly accepted: {virtual_mount}"
            );
        }
        assert!(valid_virtual_config().validate().is_ok());
    }

    #[test]
    fn with_can_update_virtual_drive_fields_without_mutating_source() {
        let source = AppConfig::default();
        let updated = source.with(
            None,
            None,
            Some("/tmp/petallink-backing".to_string()),
            Some(true),
            Some(true),
            Some("/tmp/PetalLinkDrive".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(updated.virtual_drive_enabled);
        assert_eq!(updated.virtual_mount_dir, "/tmp/PetalLinkDrive");
        assert!(!source.virtual_drive_enabled);
        assert!(source.virtual_mount_dir.is_empty());
    }
}
