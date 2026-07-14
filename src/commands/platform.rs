//! 平台与应用命令。

use tauri::AppHandle;

use crate::auth::token_store::TokenStore;
use crate::data::repository;
use crate::error::{AppError, AppResult};

use super::{drop_runtime_async, relaunch, DB};

/// 在 Finder 中打开路径。
#[tauri::command]
pub async fn open_in_finder(path: String) -> AppResult<bool> {
    #[cfg(target_os = "macos")]
    {
        let result = std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map(|_| true)
            .map_err(|e| AppError::generic(format!("打开 Finder 失败：{e}")))?;
        Ok(result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(false)
    }
}

/// 检查开机自启。
#[tauri::command]
pub fn launch_at_login_is_enabled() -> bool {
    crate::platform::launch_at_login::is_enabled()
}

/// 设置开机自启。
#[tauri::command]
pub fn launch_at_login_set_enabled(enabled: bool) -> bool {
    match crate::platform::launch_at_login::set_enabled(enabled) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(error = %e, "设置开机自启失败");
            false
        }
    }
}

/// 清空应用缓存。
#[tauri::command]
pub async fn app_clear_cache(app: AppHandle) -> AppResult<()> {
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
pub fn logs_list() -> AppResult<Vec<crate::core::logging::LogRecord>> {
    Ok(crate::core::logging::snapshot())
}

/// 导出完整日志。
#[tauri::command]
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
pub fn logs_clear() -> AppResult<()> {
    crate::core::logging::clear();
    Ok(())
}

/// 获取应用版本。
#[tauri::command]
pub fn app_get_version() -> String {
    crate::constants::APP_VERSION.to_string()
}
