//! 配置命令。

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::config::AppConfig;
use crate::core::config_store::ConfigStore;
use crate::error::AppResult;
use crate::mount::manager::MountManager;

use super::{drop_runtime_async, install_runtime_with_mount, relaunch, replace_runtime_with_mount};

/// 串行化完整配置保存事务，避免两个请求基于同一旧快照交错替换运行时。
static CONFIG_SAVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 判断需要重建同步引擎才能生效的运行参数是否变化。
fn runtime_settings_changed(old: &AppConfig, new: &AppConfig) -> bool {
    old.concurrency != new.concurrency
        || old.poll_interval_sec != new.poll_interval_sec
        || old.debounce_sec != new.debounce_sec
        || old.skip_patterns != new.skip_patterns
}

/// 读取并校验当前持久化配置。
#[tauri::command]
#[specta::specta]
pub fn config_load() -> AppResult<AppConfig> {
    ConfigStore::load()
}

/// 切换托盘图标显示：持久化到配置并立即生效（对齐开机自启开关的即时生效模式）。
#[tauri::command]
#[specta::specta]
pub fn tray_set_visible(app: AppHandle, visible: bool) -> AppResult<()> {
    let mut config = ConfigStore::load()?;
    config.show_tray_icon = visible;
    ConfigStore::save(&config)?;
    crate::platform::tray::set_tray_visible(&app, visible);
    Ok(())
}

/// 保存配置；挂载目录变化时停止旧运行时、清理缓存并重启，运行参数变化时原位重建引擎。
#[tauri::command]
#[specta::specta]
pub async fn config_save(app: AppHandle, config: AppConfig) -> AppResult<()> {
    let _save_transaction = CONFIG_SAVE_LOCK.lock().await;
    let old = ConfigStore::load()?;
    let old_configured = old.mount_configured;
    let old_abs = old.expanded_mount_dir();
    let new_abs = config.expanded_mount_dir();
    let dir_changed = old_configured && config.mount_configured && old_abs != new_abs;
    let runtime_changed = runtime_settings_changed(&old, &config);

    ConfigStore::save(&config)?;
    // 保存/导入路径也可能改托盘可见性，与运行时状态对齐
    crate::platform::tray::set_tray_visible(&app, config.show_tray_icon);

    // 切换或取消挂载目录
    if old_configured && (!config.mount_configured || dir_changed) {
        drop_runtime_async().await;
        crate::core::cache_paths::clear_cache_files(&old_abs.to_string_lossy());
        crate::core::cache_paths::clear_cache_files(&new_abs.to_string_lossy());
        tracing::info!("挂载目录变更，relaunch");
        relaunch(&app);
        return Ok(());
    }

    // 首次配置并启动引擎
    if !old_configured && config.mount_configured {
        let mount = Arc::new(MountManager::new(&new_abs));
        install_runtime_with_mount(&app, mount)?;
        return Ok(());
    }

    // 运行参数属于引擎生命周期快照；安全关闭旧周期后原目录重建，不破坏可信 checkpoint。
    if old_configured && config.mount_configured && runtime_changed {
        let mount = Arc::new(MountManager::new(&new_abs));
        replace_runtime_with_mount(&app, mount).await?;
        return Ok(());
    }

    // 更新挂载管理器；若此前启动失败导致引擎缺失，同一入口会同时完成恢复安装。
    let mount = Arc::new(MountManager::new(&new_abs));
    install_runtime_with_mount(&app, mount)?;
    Ok(())
}

/// 将当前配置序列化为可导入的 JSON 文本。
#[tauri::command]
#[specta::specta]
pub fn config_export_json() -> AppResult<String> {
    let config = ConfigStore::load()?;
    ConfigStore::export_to_json(&config)
}

/// 解析并校验 JSON 配置，但不在此入口直接覆盖当前配置文件。
#[tauri::command]
#[specta::specta]
pub fn config_import_json(json_str: String) -> AppResult<AppConfig> {
    ConfigStore::import_from_json(&json_str)
}

/// 覆盖配置保存时引擎重建边界的核心合同。
#[cfg(test)]
mod tests {
    use super::runtime_settings_changed;
    use crate::core::config::AppConfig;

    /// skipPatterns 变化必须重建引擎，使预编译 matcher 立即生效。
    #[test]
    fn skip_pattern_change_requires_runtime_rebuild() {
        let old = AppConfig::default();
        let mut new = old.clone();
        new.skip_patterns.push("*.cache".to_string());

        assert!(runtime_settings_changed(&old, &new));
    }

    /// 纯展示设置变化不得中断正在运行的同步引擎。
    #[test]
    fn tray_visibility_change_does_not_require_runtime_rebuild() {
        let old = AppConfig::default();
        let mut new = old.clone();
        new.show_tray_icon = !old.show_tray_icon;

        assert!(!runtime_settings_changed(&old, &new));
    }
}
