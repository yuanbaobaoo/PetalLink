//! 应用配置模型（需求 F-CFG-03）。
//!
//! 对齐 `legacy/lib/core/config/app_config.dart`。所有可配置项集中在此，不含 token。
//! 持久化为 JSON 文件，见 [`crate::core::config_store`]。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::constants::DEFAULT_CALLBACK_PORT;
use crate::error::{AppError, AppResult};

/// 同步状态展示排序字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum SortField {
    #[default]
    Name,
    Size,
    ModifiedTime,
}

/// 列表排序方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// OAuth 回调 URI（必须与 AGC 后台一致）
    pub oauth_redirect_uri: String,
    /// OAuth 回调端口
    pub oauth_callback_port: u16,
    /// 本地挂载目录（可能含 ~ 前缀）
    pub mount_dir: String,
    /// 用户是否已显式配置过挂载目录（首次同步引导用，F-MOUNT-13）。
    /// 区分"默认值"与"用户已确认"，避免未选目录就自动同步覆盖本地已有内容。
    pub mount_configured: bool,
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            oauth_redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            oauth_callback_port: DEFAULT_CALLBACK_PORT,
            mount_dir: String::new(),
            mount_configured: false,
            concurrency: 6,
            poll_interval_sec: 60,
            debounce_sec: 3,
            skip_patterns: DEFAULT_SKIP_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            sort_field: SortField::Name,
            sort_order: SortOrder::Ascending,
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
            if self.mount_dir.trim().is_empty() {
                return Err(AppError::config("同步目录不能为空".to_string()));
            }
            let expanded = self.expanded_mount_dir();
            if !expanded.is_absolute() {
                return Err(AppError::config(format!(
                    "同步目录必须是绝对路径：{}",
                    self.mount_dir
                )));
            }
            if expanded == Path::new("/") {
                return Err(AppError::config("不能把系统根目录作为同步目录".to_string()));
            }
            if let Some(home) = dirs::home_dir() {
                if expanded == home {
                    return Err(AppError::config(
                        "不能把用户 Home 目录作为同步目录".to_string(),
                    ));
                }
            }
            if let Some(data_dir) = dirs::data_dir() {
                if expanded.starts_with(&data_dir) {
                    return Err(AppError::config(
                        "不能把 Application Support 目录作为同步目录".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// 展开 ~ 为真实 home 路径。
    /// 对齐 dart `AppConfig.expandedMountDir`。
    pub fn expanded_mount_dir(&self) -> PathBuf {
        if let Some(rest) = self.mount_dir.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            return Path::new(&home).join(rest);
        }
        PathBuf::from(&self.mount_dir)
    }

    /// 链式构造：返回带修改的新配置（不可变值对象）。
    #[allow(clippy::too_many_arguments)]
    pub fn with(
        &self,
        oauth_redirect_uri: Option<String>,
        oauth_callback_port: Option<u16>,
        mount_dir: Option<String>,
        mount_configured: Option<bool>,
        concurrency: Option<u32>,
        poll_interval_sec: Option<u32>,
        debounce_sec: Option<u32>,
        skip_patterns: Option<Vec<String>>,
        sort_field: Option<SortField>,
        sort_order: Option<SortOrder>,
    ) -> Self {
        Self {
            oauth_redirect_uri: oauth_redirect_uri
                .unwrap_or_else(|| self.oauth_redirect_uri.clone()),
            oauth_callback_port: oauth_callback_port.unwrap_or(self.oauth_callback_port),
            mount_dir: mount_dir.unwrap_or_else(|| self.mount_dir.clone()),
            mount_configured: mount_configured.unwrap_or(self.mount_configured),
            concurrency: concurrency.unwrap_or(self.concurrency),
            poll_interval_sec: poll_interval_sec.unwrap_or(self.poll_interval_sec),
            debounce_sec: debounce_sec.unwrap_or(self.debounce_sec),
            skip_patterns: skip_patterns.unwrap_or_else(|| self.skip_patterns.clone()),
            sort_field: sort_field.unwrap_or(self.sort_field),
            sort_order: sort_order.unwrap_or(self.sort_order),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let c = AppConfig::default();
        assert_eq!(c.concurrency, 6);
        assert_eq!(c.debounce_sec, 3);
        assert_eq!(c.poll_interval_sec, 60);
        assert!(!c.mount_configured);
        assert_eq!(c.mount_dir, "");
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_concurrency_range() {
        let c = AppConfig {
            concurrency: 0,
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
        let c = AppConfig {
            concurrency: 21,
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
        let c = AppConfig {
            concurrency: 6,
            ..AppConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_validate_poll_interval_range() {
        // 0 = 关闭，合法
        let c = AppConfig {
            poll_interval_sec: 0,
            ..AppConfig::default()
        };
        assert!(c.validate().is_ok());
        // 开启但 < 60 非法
        let c = AppConfig {
            poll_interval_sec: 30,
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
        // 60 秒是开启下界，合法
        let c = AppConfig {
            poll_interval_sec: 60,
            ..AppConfig::default()
        };
        assert!(c.validate().is_ok());
        // 默认值合法
        let c = AppConfig {
            poll_interval_sec: 900,
            ..AppConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_expanded_mount_dir() {
        // 显式构造含 ~ 的 mount_dir 测试展开（默认 mount_dir 已为空）
        let c = AppConfig {
            mount_dir: "~/hwcloud-drive".to_string(),
            ..AppConfig::default()
        };
        let expanded = c.expanded_mount_dir();
        // 应展开 ~ 为 $HOME
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        assert_eq!(expanded, Path::new(&home).join("hwcloud-drive"));
    }

    #[test]
    fn test_expanded_mount_dir_absolute() {
        let c = AppConfig {
            mount_dir: "/Users/test/mydrive".to_string(),
            ..AppConfig::default()
        };
        assert_eq!(c.expanded_mount_dir(), PathBuf::from("/Users/test/mydrive"));
    }

    #[test]
    fn test_with_chain() {
        let c = AppConfig::default();
        let c2 = c.with(
            None,
            None,
            None,
            Some(true),
            Some(10),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(c2.mount_configured);
        assert_eq!(c2.concurrency, 10);
        // 原对象不变
        assert!(!c.mount_configured);
    }

    #[test]
    fn test_validate_rejects_empty_configured_mount_dir() {
        let c = AppConfig {
            mount_configured: true,
            mount_dir: String::new(),
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_relative_configured_mount_dir() {
        let c = AppConfig {
            mount_configured: true,
            mount_dir: "relative/path".to_string(),
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_root_mount_dir() {
        let c = AppConfig {
            mount_configured: true,
            mount_dir: "/".to_string(),
            ..AppConfig::default()
        };
        assert!(c.validate().is_err());
    }
}
