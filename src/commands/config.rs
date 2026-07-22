//! 配置命令。

use std::sync::Arc;

use tauri::AppHandle;

use crate::core::config::AppConfig;
use crate::core::config_store::ConfigStore;
use crate::error::AppResult;
use crate::mount::manager::MountManager;

use super::{drop_runtime_async, ensure_engine_started, relaunch, set_mount_manager};

/// 读取并校验当前持久化配置。
#[tauri::command]
pub fn config_load() -> AppResult<AppConfig> {
    ConfigStore::load()
}

/// 切换托盘图标显示：持久化到配置并立即生效（对齐开机自启开关的即时生效模式）。
#[tauri::command]
pub fn tray_set_visible(app: AppHandle, visible: bool) -> AppResult<()> {
    let mut config = ConfigStore::load()?;
    config.show_tray_icon = visible;
    ConfigStore::save(&config)?;
    crate::platform::tray::set_tray_visible(&app, visible);
    Ok(())
}

/// 保存配置；挂载目录变化时停止旧运行时、清理缓存并重启，首次配置时启动同步引擎。
#[tauri::command]
pub async fn config_save(app: AppHandle, config: AppConfig) -> AppResult<()> {
    let old = ConfigStore::load().ok();
    let old_configured = old.as_ref().map(|c| c.mount_configured).unwrap_or(false);
    let old_abs = old.as_ref().map(|c| c.expanded_mount_dir());
    let new_abs = config.expanded_mount_dir();
    let dir_changed =
        old_configured && config.mount_configured && old_abs.as_ref() != Some(&new_abs);

    ConfigStore::save(&config)?;
    // 保存/导入路径也可能改托盘可见性，与运行时状态对齐
    crate::platform::tray::set_tray_visible(&app, config.show_tray_icon);

    // 切换或取消挂载目录
    if old_configured && (!config.mount_configured || dir_changed) {
        drop_runtime_async().await;
        if let Some(old_abs) = old_abs {
            crate::core::cache_paths::clear_cache_files(&old_abs.to_string_lossy());
        }
        crate::core::cache_paths::clear_cache_files(&new_abs.to_string_lossy());
        tracing::info!("挂载目录变更，relaunch");
        relaunch(&app);
        return Ok(());
    }

    // 首次配置并启动引擎
    if !old_configured && config.mount_configured {
        let m = Arc::new(MountManager::new(&new_abs));
        m.ensure_mount_dir()?;
        set_mount_manager(m);
        ensure_engine_started(&app)?;
        return Ok(());
    }

    // 更新挂载管理器
    let m = Arc::new(MountManager::new(&new_abs));
    m.ensure_mount_dir()?;
    set_mount_manager(m);
    Ok(())
}

/// 将当前配置序列化为可导入的 JSON 文本。
#[tauri::command]
pub fn config_export_json() -> AppResult<String> {
    let config = ConfigStore::load()?;
    ConfigStore::export_to_json(&config)
}

/// 解析并校验 JSON 配置，但不在此入口直接覆盖当前配置文件。
#[tauri::command]
pub fn config_import_json(json_str: String) -> AppResult<AppConfig> {
    ConfigStore::import_from_json(&json_str)
}
