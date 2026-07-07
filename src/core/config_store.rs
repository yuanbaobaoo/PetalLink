//! 配置持久化（需求 F-CFG-02 / F-CFG-04）。
//!
//! 对齐 `legacy/lib/core/config/config_store.dart`。
//!
//! 存储位置：`<ApplicationSupport>/config.json`，不含 token（token 加密存 token.bin）。
//! 支持导入/导出 JSON（F-CFG-04：备份恢复配置）。

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::core::config::{
    AppConfig, SortField, SortOrder, DEFAULT_MOUNT_DIR, DEFAULT_REDIRECT_URI,
};
use crate::error::{AppError, AppResult};

/// 配置文件名
const CONFIG_FILE_NAME: &str = "config.json";

/// Application Support 目录下的 PetalLink 工作目录。
/// macOS: `~/Library/Application Support/io.github.yuanbaobaoo.PetalLink`
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
        let (config, dirty) = parse_config_raw(&raw)?;
        // 迁移改了值 → 落盘（仅 load 走此路径；from_json 纯解析不落盘，避免测试污染真实配置）
        if dirty {
            ConfigStore::save(&config)?;
        }
        Ok(config)
    }

    /// 保存配置（先校验）。
    /// 对齐 dart `ConfigStore.save()`。
    pub fn save(config: &AppConfig) -> AppResult<()> {
        config.validate()?;
        validate_configured_mount_dir_access(config)?;
        let path = config_file_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let json = to_json(config);
        let pretty = serde_json::to_string_pretty(&json)?;
        fs::write(&path, pretty)?;
        tracing::info!(mount = %config.mount_dir, "配置已保存");
        Ok(())
    }

    /// 导出配置为 JSON 字符串（F-CFG-04，不含 token）。
    pub fn export_to_json(config: &AppConfig) -> AppResult<String> {
        Ok(serde_json::to_string_pretty(&to_json(config))?)
    }

    /// 从 JSON 字符串导入配置（F-CFG-04），校验并持久化。
    pub fn import_from_json(json_str: &str) -> AppResult<AppConfig> {
        let (config, _dirty) = parse_config_raw(json_str)?;
        Self::save(&config)?;
        Ok(config)
    }
}

fn parse_config_raw(raw: &str) -> AppResult<(AppConfig, bool)> {
    let json: Value =
        serde_json::from_str(raw).map_err(|e| AppError::config(format!("配置解析失败：{e}")))?;
    let (config, dirty) = from_json(&json);
    config.validate()?;
    Ok((config, dirty))
}

fn validate_configured_mount_dir_access(config: &AppConfig) -> AppResult<()> {
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
    let probe = dir.join(format!(".petallink-write-test-{}", std::process::id()));
    fs::write(&probe, b"ok")
        .map_err(|e| AppError::config(format!("同步目录不可写：{}：{e}", dir.display())))?;
    fs::remove_file(&probe).map_err(|e| {
        AppError::config(format!(
            "同步目录写入探测清理失败：{}：{e}",
            probe.display()
        ))
    })?;
    Ok(())
}

/// 序列化配置为 JSON。对齐 dart `_toJson`。
fn to_json(c: &AppConfig) -> Value {
    json!({
        "oauthRedirectUri": c.oauth_redirect_uri,
        "oauthCallbackPort": c.oauth_callback_port,
        "mountDir": c.mount_dir,
        "mountConfigured": c.mount_configured,
        "concurrency": c.concurrency,
        "pollIntervalSec": c.poll_interval_sec,
        "debounceSec": c.debounce_sec,
        "skipPatterns": c.skip_patterns,
        // 排序字段序列化为枚举名（camelCase，对齐前端）
        "sortField": sort_field_to_str(c.sort_field),
        "sortOrder": sort_order_to_str(c.sort_order),
    })
}

/// 反序列化配置。含旧默认值迁移（30/30 → 10/3、未配置的旧默认 mount_dir 清空）。
/// 对齐 dart `_fromJson`。纯解析（不落盘）——返回 (config, dirty)，由调用方（load）决定是否 save。
/// 这样测试调用 from_json 不会污染真实 config.json。
fn from_json(json: &Value) -> (AppConfig, bool) {
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
    };

    let mut dirty = false;
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
    (config, dirty)
}

/// 排序字段字符串表示（与 dart 枚举 name 一致）
fn sort_field_to_str(f: SortField) -> &'static str {
    match f {
        SortField::Name => "name",
        SortField::Size => "size",
        SortField::ModifiedTime => "modifiedTime",
    }
}

fn sort_order_to_str(o: SortOrder) -> &'static str {
    match o {
        SortOrder::Ascending => "ascending",
        SortOrder::Descending => "descending",
    }
}

fn parse_sort_field(v: Option<&Value>) -> SortField {
    match v.and_then(Value::as_str) {
        Some("name") => SortField::Name,
        Some("size") => SortField::Size,
        Some("modifiedTime") => SortField::ModifiedTime,
        _ => SortField::Name,
    }
}

fn parse_sort_order(v: Option<&Value>) -> SortOrder {
    match v.and_then(Value::as_str) {
        Some("ascending") => SortOrder::Ascending,
        Some("descending") => SortOrder::Descending,
        _ => SortOrder::Ascending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_migration_legacy_small_poll_to_new_default() {
        // 旧配置 poll=30 debounce=30 应迁移为新默认 60/3
        let json = json!({
            "pollIntervalSec": 30,
            "debounceSec": 30,
            "concurrency": 6,
        });
        let config = from_json(&json).0;
        assert_eq!(config.poll_interval_sec, 60);
        assert_eq!(config.debounce_sec, 3);
    }

    #[test]
    fn test_migration_legacy_small_poll_variants() {
        // 旧版秒级小值（10/45）一律迁移到 60；0（关闭）与 ≥60 的值保留
        for &old_poll in &[10u32, 30, 45] {
            let json = json!({ "pollIntervalSec": old_poll });
            let config = from_json(&json).0;
            assert_eq!(config.poll_interval_sec, 60, "poll={old_poll} 应迁移到 60");
        }
        // 0 = 关闭，保留
        assert_eq!(
            from_json(&json!({ "pollIntervalSec": 0 }))
                .0
                .poll_interval_sec,
            0
        );
        // ≥60 保留
        assert_eq!(
            from_json(&json!({ "pollIntervalSec": 60 }))
                .0
                .poll_interval_sec,
            60
        );
        assert_eq!(
            from_json(&json!({ "pollIntervalSec": 600 }))
                .0
                .poll_interval_sec,
            600
        );
    }

    #[test]
    fn test_no_migration_when_valid() {
        // 合法的新版值不迁移
        let json = json!({
            "pollIntervalSec": 120,
            "debounceSec": 5,
        });
        let config = from_json(&json).0;
        assert_eq!(config.poll_interval_sec, 120);
        assert_eq!(config.debounce_sec, 5);
    }

    #[test]
    fn test_migration_clears_legacy_default_mount_when_unconfigured() {
        // 旧版：未配置但 mount_dir 残留旧默认 ~/hwcloud-drive → 清空
        let json = json!({
            "mountDir": "~/hwcloud-drive",
            "mountConfigured": false,
        });
        let config = from_json(&json).0;
        assert_eq!(config.mount_dir, "");
        assert!(!config.mount_configured);
    }

    #[test]
    fn test_migration_keeps_explicitly_configured_mount() {
        // 用户显式配置过（mount_configured=true）的 ~/hwcloud-drive 保留
        let json = json!({
            "mountDir": "~/hwcloud-drive",
            "mountConfigured": true,
        });
        let config = from_json(&json).0;
        assert_eq!(config.mount_dir, "~/hwcloud-drive");
        assert!(config.mount_configured);
    }

    #[test]
    fn test_roundtrip_serialization() {
        let config = AppConfig {
            mount_configured: true,
            concurrency: 10,
            ..AppConfig::default()
        };
        let json = to_json(&config);
        let restored = from_json(&json).0;
        assert!(restored.mount_configured);
        assert_eq!(restored.concurrency, 10);
    }

    #[test]
    fn test_export_import_roundtrip() {
        // export/import 应能还原（用临时目录避免污染真实配置）
        let _td = tempdir();
        let config = AppConfig {
            concurrency: 8,
            mount_configured: true,
            ..AppConfig::default()
        };
        let exported = ConfigStore::export_to_json(&config).unwrap();
        assert!(exported.contains("\"concurrency\": 8"));
    }

    #[test]
    fn test_sort_field_roundtrip() {
        let json = json!({ "sortField": "size", "sortOrder": "descending" });
        let config = from_json(&json).0;
        assert_eq!(config.sort_field, SortField::Size);
        assert_eq!(config.sort_order, SortOrder::Descending);
    }

    #[test]
    fn test_parse_config_raw_rejects_invalid_json() {
        let err = parse_config_raw("{ invalid json").unwrap_err();
        assert!(matches!(err, AppError::Config { .. }));
    }
}
