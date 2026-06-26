//! 系统托盘 —— NSStatusItem + 菜单项。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift` 的 setupStatusItem + buildStatusMenu。
//!
//! 菜单项：版本标识 / 显示主窗口 / 退出 PetalLink
//! 图标：assets/menubar-icon.png（双环+同步点，对齐 Flutter MenubarIcon）
//!       以 template 方式加载（icon_as_template），macOS 按明暗自动着色，对齐 Flutter isTemplate
//! tooltip：动态，联动同步状态
//!
//! 说明：Flutter 版「立即同步」为条件显示（登录 + 配好同步目录才出现），默认隐藏；
//!       此处按需求直接不提供该项，菜单与 Flutter 默认态（canSync=false）一致。

use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::image::Image;

/// menubar-icon.png（编译期嵌入，对齐 Flutter Asset Catalog 的 MenubarIcon）
/// SVG 经 qlmanage 转 PNG（Tauri 2 Image::from_bytes 不支持 SVG）
const MENUBAR_ICON_PNG: &[u8] = include_bytes!("../../assets/menubar-icon.png");

/// 创建系统托盘图标 + 菜单（对齐 Flutter buildStatusMenu）。
pub fn setup(app: &AppHandle) {
    // 构建菜单（对齐 AppDelegate.buildStatusMenu）
    let version_item = MenuItem::with_id(
        app, "version", "PetalLink - 华为云盘 Mac 客户端开源版 v1.0", false, None::<&str>,
    ).expect("创建版本菜单项失败");
    let sep1 = PredefinedMenuItem::separator(app).expect("创建分隔符失败");
    let show_item = MenuItem::with_id(
        app, "show_window", "显示主窗口", true, None::<&str>,
    ).expect("创建显示窗口菜单项失败");
    let sep2 = PredefinedMenuItem::separator(app).expect("创建分隔符失败");
    let quit_item = MenuItem::with_id(
        app, "quit", "退出 PetalLink", true, None::<&str>,
    ).expect("创建退出菜单项失败");

    let menu = Menu::with_items(app, &[
        &version_item, &sep1, &show_item, &sep2, &quit_item,
    ]).expect("创建托盘菜单失败");

    // 加载 menubar 图标（PNG，对齐 Flutter MenubarIcon）
    let icon = Image::from_bytes(MENUBAR_ICON_PNG)
        .unwrap_or_else(|_| {
            tracing::warn!("menubar PNG 加载失败，回退到应用图标");
            app.default_window_icon().cloned().unwrap()
        });

    let _ = TrayIconBuilder::with_id("PetalLink-tray")
        .icon(icon)
        .icon_as_template(true) // 对齐 Flutter isTemplate=true：macOS 按明暗自动着色
        .tooltip("PetalLink — 后台同步中")
        .menu(&menu)
        .show_menu_on_left_click(true) // 左键显示菜单（对齐 Flutter item.menu = menu）
        .on_menu_event(|app_handle, event| {
            match event.id().as_ref() {
                "show_window" => show_main_window(app_handle),
                "quit" => quit_app(app_handle),
                _ => {}
            }
        })
        .build(app);

    tracing::info!("系统托盘图标+菜单已创建");
}

/// 更新托盘 tooltip（联动同步状态）。
pub fn update_tooltip(app: &AppHandle, tooltip: &str) {
    if let Some(tray) = app.tray_by_id("PetalLink-tray") {
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.show().ok();
        window.set_focus().ok();
    }
    #[cfg(target_os = "macos")]
    {
        crate::platform::activation::set_regular();
    }
}

fn quit_app(app: &AppHandle) {
    tracing::info!("菜单栏「退出 PetalLink」— 真退出");
    #[cfg(target_os = "macos")]
    crate::platform::activation::mark_real_quit();
    app.exit(0);
}
