//! PetalLink 应用装配 —— 状态构造、命令注册、事件桥接。
//!
//! 对齐 `legacy/lib/app.dart` 的 ProviderScope 装配职责。

use std::sync::Arc;
use tauri::Manager;

pub mod auth;
pub mod commands;
pub mod constants;
pub mod core;
mod data;
pub mod drive;
pub mod error;
mod mount;
pub mod platform;
pub mod sync;

/// 日志初始化：三路输出（默认 INFO，对齐 dart `initLogger`）。
/// - stdout fmt（控制台，debug 带颜色）
/// - 滚动文件 fmt（`<support_dir>/logs`，每日轮转，完整持久日志供导出）
/// - 环形缓冲 Layer（供设置页日志查看，最近 1000 条）
///
/// 之前只装 fmt(stdout)，缓冲恒空 → 日志查看页空白；文件 sink 也缺位 → 无可导出。
pub fn init_logger() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,petal_link_lib=info,tauri_plugin_updater=warn"));

    let stdout_layer = fmt::layer()
        .with_target(false)
        .with_ansi(cfg!(debug_assertions));

    let buffer_layer = crate::core::logging::LogBufferLayer;

    // 滚动文件日志（每日轮转）。目录不可用时降级为 stdout+buffer，不阻断启动。
    // 用 match 两分支各自构造 + try_init，规避静态类型下条件层无法统一类型的约束。
    let _ = match crate::core::logging::log_dir() {
        Ok(dir) => {
            let _ = std::fs::create_dir_all(&dir);
            let appender = tracing_appender::rolling::daily(&dir, "PetalLink.log");
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(
                    fmt::layer()
                        .with_writer(appender)
                        .with_ansi(false)
                        .with_target(true),
                )
                .with(buffer_layer)
                .try_init()
        }
        Err(e) => {
            eprintln!("日志目录不可用，跳过文件日志：{e}");
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout_layer)
                .with(buffer_layer)
                .try_init()
        }
    };
    // 启动时清理超期日志（保留 30 天）
    crate::core::logging::cleanup_old_logs();
}

/// 加载 .env（开发期便利）。
pub fn load_env() {
    if let Ok(env_vars) = dotenvy::dotenv() {
        tracing::info!(path = ?env_vars, "已加载 .env 配置");
    } else {
        tracing::debug!(".env 不存在或加载失败，使用默认/构建期注入的配置");
    }
    if let Ok(secret) = std::env::var("HWCLOUD_CLIENT_SECRET") {
        constants::set_env_secret(secret);
    }
    if let Ok(client_id) = std::env::var("HWCLOUD_CLIENT_ID") {
        constants::set_env_client_id(client_id);
    }
}

/// 应用启动 —— Tauri Builder。
pub fn run() {
    init_logger();
    load_env();

    tracing::info!(
        bundle_id = constants::BUNDLE_IDENTIFIER,
        version = constants::APP_VERSION,
        "PetalLink 启动中"
    );

    let app = tauri::Builder::default()
        // 单实例守护：第二个进程启动时直接退出（已运行实例聚焦到前台）。
        // 防止双进程各自创建 FSEvents watcher 监听同一挂载目录 → 互相触发 sync cycle
        // → 基于 stale cloud_tree 误判「本地新建」疯狂上传。必须最先注册。
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // 第二实例尝试启动。
            // 若新实例带 --hidden（LaunchAgent 重复触发），不显示窗口；
            // 否则是用户手动打开 → 聚焦已运行实例的主窗口。
            // 关键：若 app 此前因关窗拦截进入 accessory 模式，必须切回 regular 才能响应用户输入，
            // 否则窗口虽 show 但所有按钮点不动（accessory app 不接收鼠标事件）。
            let is_hidden = argv.iter().any(|a| a == "--hidden");
            if let Some(w) = app.get_webview_window("main") {
                if !is_hidden {
                    let _ = w.show();
                    let _ = w.set_focus();
                    #[cfg(target_os = "macos")]
                    crate::platform::activation::set_regular();
                }
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            // Auth
            commands::auth_check_secret,
            commands::auth_restore,
            commands::auth_login,
            commands::auth_cancel_login,
            commands::auth_logout,
            commands::auth_get_user_info,
            commands::auth_is_logged_in,
            // Drive
            commands::drive_list,
            commands::drive_list_all,
            commands::drive_get_file,
            commands::drive_create_folder,
            commands::drive_delete_file,
            commands::drive_rename_file,
            commands::drive_move_file,
            commands::drive_search,
            commands::drive_get_thumbnail,
            commands::drive_get_about,
            commands::drive_download_file,
            commands::drive_upload_file,
            // Sync
            commands::sync_manual_refresh,
            commands::sync_check_safe_free_up,
            commands::sync_check_file_local_status,
            commands::sync_batch_file_status,
            commands::sync_free_up_space,
            commands::sync_download_on_demand,
            commands::sync_folder_recursive,
            commands::sync_retry_failed,
            commands::sync_state,
            commands::sync_items_by_folder,
            // Config
            commands::config_load,
            commands::config_save,
            commands::config_export_json,
            commands::config_import_json,
            // Transfer
            commands::transfer_list_all,
            commands::transfer_has_active,
            commands::transfer_clear_completed,
            commands::transfer_clear_failed,
            commands::transfer_clear_finished,
            commands::transfer_retry,
            // Platform
            commands::open_in_finder,
            commands::launch_at_login_is_enabled,
            commands::launch_at_login_set_enabled,
            commands::app_clear_cache,
            commands::logs_list,
            commands::logs_export,
            commands::logs_clear,
            commands::app_get_version,
        ])
        // 关窗拦截：关闭按钮/Cmd+W → 隐藏到后台 accessory（不退出），仅 tray 退出放行
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if !platform::activation::should_real_quit() {
                        api.prevent_close();
                        let _ = window.hide();
                        #[cfg(target_os = "macos")]
                        platform::activation::set_accessory();
                    }
                }
                // 窗口获焦：若此前因关窗/Cmd+Q 拦截进入 accessory 模式，切回 regular 恢复可交互。
                // 覆盖所有恢复路径（最小化恢复、Dock 点击、单实例 show、托盘点击等），
                // 确保 accessory 模式下窗口恢复后能正常接收输入。
                tauri::WindowEvent::Focused(true) => {
                    #[cfg(target_os = "macos")]
                    platform::activation::ensure_regular_if_was_accessory();
                }
                _ => {}
            }
        })
        .setup(|app| {
            // 最早阶段：根据 --hidden 参数设置 activationPolicy
            platform::activation::init_activation_policy();
            // ★ 必须最早安装：拦截 Dock/Cmd+Q 退出，防止 macOS 直接杀进程
            platform::activation::install_terminate_interceptor();
            // ★ 检测上次崩溃标记（panic hook 写入），记录到日志后清理
            if let Ok(support) = crate::core::config_store::support_dir() {
                let marker = support.join("last_crash.marker");
                if marker.exists() {
                    if let Ok(content) = std::fs::read_to_string(&marker) {
                        tracing::warn!(
                            crash = %content.trim(),
                            "检测到上次启动异常退出（last_crash.marker），详情见上次日志末尾"
                        );
                    }
                    let _ = std::fs::remove_file(&marker);
                }
            }
            // 创建系统托盘
            platform::tray::setup(app.handle());

            // ★ 启动期清理：若 LaunchAgent 已启用，移除 Login Items 中的重复项。
            //   避免「LaunchAgent 带 --hidden」+「Login Items 不带 --hidden」双重启动，
            //   Login Items 触发 single-instance 回调顶出窗口。
            if platform::launch_at_login::is_enabled() {
                platform::launch_at_login::remove_from_login_items();
            }

            // 手动启动 → 显示主窗口并聚焦；开机自启（--hidden）→ 窗口保持隐藏（visible:false 默认），仅菜单栏图标后台运行
            if platform::activation::is_launched_manually() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    tracing::info!("手动启动：显示主窗口");
                }
            } else {
                tracing::info!("--hidden 模式：主窗口保持隐藏，仅保留菜单栏图标");
            }

            // 加载配置（仅一次，token 检测 + 引擎初始化共用）
            let mut config = core::config_store::ConfigStore::load().unwrap_or_default();
            tracing::info!(mount = %config.mount_dir, configured = config.mount_configured, "配置加载成功");

            // ★ 最优先：token 丢失但旧配置/缓存仍在 → 先清空再走后续流程。
            // 首次安装 config 默认 mount_configured=false → 跳过清理。
            {
                use crate::auth::token_store::{global_store, TokenStore};
                let logged_in = global_store()
                    .load()
                    .ok()
                    .flatten()
                    .is_some();
                if !logged_in && config.mount_configured {
                    commands::cleanup_orphan_state();
                    // 清理后磁盘 config 已变（mount_configured=false），重新加载
                    config = core::config_store::ConfigStore::load().unwrap_or_default();
                }
            }

            // 初始化 MountManager + SyncEngine（仅当已配置目录且已登录）
            if config.mount_configured {
                let abs_dir = config.expanded_mount_dir();
                let m = Arc::new(mount::manager::MountManager::new(&abs_dir));
                if m.ensure_mount_dir().is_ok() {
                    commands::set_mount_manager(m);
                    tracing::info!("MountManager 已初始化");
                    if let Err(e) = commands::ensure_engine_started(app.handle()) {
                        tracing::error!(error = %e, "SyncEngine 初始化失败");
                    }
                }
            }

            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            tracing::info!("PetalLink 初始化完成");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("运行 PetalLink 时出错");

    // Cmd+Q/Dock Quit → 隐藏到后台；tray 退出(code=Some)放行；关机/真退出 flush
    app.run(|handle, event| match event {
        tauri::RunEvent::ExitRequested { api, code, .. } => {
            if !platform::activation::should_real_quit() && code.is_none() {
                api.prevent_exit();
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.hide();
                }
                #[cfg(target_os = "macos")]
                platform::activation::set_accessory();
            }
        }
        tauri::RunEvent::Exit => {
            crate::core::net_guard::shutdown_probe();
            platform::shutdown::flush_with_timeout(handle);
        }
        _ => {}
    });
}
