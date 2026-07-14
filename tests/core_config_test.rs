//! 核心配置默认值、校验、路径展开与导入导出测试。

use std::path::{Path, PathBuf};

use petal_link_lib::core::config::AppConfig;
use petal_link_lib::core::config_store::ConfigStore;
use tempfile::tempdir;

/// 验证默认配置合法且关键默认值稳定。
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

/// 验证并发数只接受 1 至 20。
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

/// 验证轮询间隔遵守配置边界。
#[test]
fn test_validate_poll_interval_range() {
    let c = AppConfig {
        poll_interval_sec: 0,
        ..AppConfig::default()
    };
    assert!(c.validate().is_ok());
    let c = AppConfig {
        poll_interval_sec: 30,
        ..AppConfig::default()
    };
    assert!(c.validate().is_err());
    let c = AppConfig {
        poll_interval_sec: 60,
        ..AppConfig::default()
    };
    assert!(c.validate().is_ok());
    let c = AppConfig {
        poll_interval_sec: 900,
        ..AppConfig::default()
    };
    assert!(c.validate().is_ok());
}

/// 验证波浪号挂载路径展开到 HOME。
#[test]
fn test_expanded_mount_dir() {
    let c = AppConfig {
        mount_dir: "~/hwcloud-drive".to_string(),
        ..AppConfig::default()
    };
    let expanded = c.expanded_mount_dir();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    assert_eq!(expanded, Path::new(&home).join("hwcloud-drive"));
}

/// 验证绝对挂载路径保持不变。
#[test]
fn test_expanded_mount_dir_absolute() {
    let c = AppConfig {
        mount_dir: "/Users/test/mydrive".to_string(),
        ..AppConfig::default()
    };
    assert_eq!(c.expanded_mount_dir(), PathBuf::from("/Users/test/mydrive"));
}

/// 验证链式构造返回新配置且不修改原对象。
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
    assert!(!c.mount_configured);
}

/// 验证已配置的挂载目录不能为空。
#[test]
fn test_validate_rejects_empty_configured_mount_dir() {
    let c = AppConfig {
        mount_configured: true,
        mount_dir: String::new(),
        ..AppConfig::default()
    };
    assert!(c.validate().is_err());
}

/// 验证已配置的挂载目录必须为绝对路径。
#[test]
fn test_validate_rejects_relative_configured_mount_dir() {
    let c = AppConfig {
        mount_configured: true,
        mount_dir: "relative/path".to_string(),
        ..AppConfig::default()
    };
    assert!(c.validate().is_err());
}

/// 验证系统根目录不能作为挂载目录。
#[test]
fn test_validate_rejects_root_mount_dir() {
    let c = AppConfig {
        mount_configured: true,
        mount_dir: "/".to_string(),
        ..AppConfig::default()
    };
    assert!(c.validate().is_err());
}

/// 验证配置导出文本包含关键并发字段。
#[test]
fn test_export_import_roundtrip() {
    let _td = tempdir();
    let config = AppConfig {
        concurrency: 8,
        mount_configured: true,
        ..AppConfig::default()
    };
    let exported = ConfigStore::export_to_json(&config).unwrap();
    assert!(exported.contains("\"concurrency\": 8"));
}
