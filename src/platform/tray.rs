//! 系统托盘 —— NSStatusItem + 菜单项。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift` 的 setupStatusItem + buildStatusMenu。
//!
//! 菜单项：版本标识 / 显示主窗口 / 正在传输汇总（动态） / 退出 PetalLink
//! 图标：assets/menubar-icon.png（双环+同步点，对齐 Flutter MenubarIcon）
//!       以 template 方式加载（icon_as_template），macOS 按明暗自动着色，对齐 Flutter isTemplate
//! tooltip：动态，联动同步状态
//!
//! # 刷新策略（重要）
//!
//! macOS 上替换 `NSStatusItem.menu` 会结束当前菜单追踪，**已展开的菜单会被立即关闭**。
//! 因此这里绝不在刷新时 `set_menu`：
//!
//! 1. 菜单结构在启动时建成固定三段式（版本 / 显示主窗口 / 退出）；
//! 2. 有进行中传输时用 `insert_items` 就地插入「分隔 + 汇总行 + 分隔」，结束后用
//!    `remove_at` 就地移除，两者都不替换 NSMenu，不会关闭已展开的菜单；
//! 3. 传输进度只更新汇总行的文本（`MenuItem::set_text`），并按 `MENU_TEXT_INTERVAL_MS`
//!    节流，避免高频查库与重绘。
//!
//! 「正在传输」段只显示一行汇总（方向 + 项数 + 百分比），不再逐条列出任务：
//! 逐条列表会让菜单高度随批量任务数暴涨，且每来一条新任务就要改结构。
//!
//! # 线程约束（强制）
//!
//! `tray_icon::TrayIcon` 与 `muda` 的菜单对象内部都是 `Rc<RefCell<_>>`，tauri 只是
//! 用 `unsafe impl Send/Sync` 把调用转发到主线程。**句柄本身**的 clone/drop 不会被
//! 转发：在 tokio 线程上 `tray_by_id()` 或丢弃句柄会与主线程竞争 Rc 引用计数，
//! debug 构建会直接命中 `hint::assert_unchecked` 前置条件并 abort 整个进程
//! （2026-11-23 批量上传压测实际发生）。因此本模块约定：
//!
//! 1. 托盘与菜单句柄只在 `run_on_main_thread` 闭包内获取、持有和释放；
//! 2. 跨线程只传递普通数据（`String`/`bool`），不传 `TrayIcon`/`Menu`；
//! 3. 数据库查询留在调用线程，避免主线程被 SQLite I/O 阻塞。

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::data::repository::{self, transfer_direction};
use crate::error::AppResult;

/// 托盘唯一标识
const TRAY_ID: &str = "PetalLink-tray";
/// 汇总行菜单项 id（就地改文本时按此查找）
const MENU_ID_TRANSFER_SUMMARY: &str = "transfer_summary";
/// 传输段插入位置：版本 / 分隔线 / 显示主窗口 之后
const TRANSFER_BLOCK_POSITION: usize = 3;
/// 汇总文案最小刷新间隔（毫秒）：进度展示允许滞后，避免高频查库与重绘
const MENU_TEXT_INTERVAL_MS: i64 = 2000;
/// 传输段结构变化的最小间隔（毫秒）：合并同一批任务开始/结束时的抖动
const STRUCTURE_CHANGE_INTERVAL_MS: i64 = 1500;
/// 空闲时的托盘 tooltip 模板（展示名在前，dev 包会带 `-Dev`）。
fn idle_tooltip(app: &AppHandle) -> String {
    format!("{} — 后台同步中", crate::constants::display_name(app))
}
/// menubar-icon.png（编译期嵌入，对齐 Flutter Asset Catalog 的 MenubarIcon）
/// SVG 经 qlmanage 转 PNG（Tauri 2 Image::from_bytes 不支持 SVG）
const MENUBAR_ICON_PNG: &[u8] = include_bytes!("../../assets/menubar-icon.png");

/// 托盘菜单句柄：创建后保存，所有后续刷新都在同一份菜单上就地修改。
static TRAY_MENU: OnceLock<Mutex<Option<Menu<Wry>>>> = OnceLock::new();
/// 上次刷新时是否存在进行中传输（决定传输段是否插入）
static LAST_HAS_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 上次传输段结构变化的时间（毫秒），用于合并抖动
static LAST_STRUCTURE_CHANGE_MS: AtomicI64 = AtomicI64::new(0);
/// 上次汇总文案刷新的时间（毫秒）
static LAST_TEXT_UPDATE_MS: AtomicI64 = AtomicI64::new(0);
/// 上次设置的 tooltip，相同则不重复写入
static LAST_TOOLTIP: OnceLock<Mutex<String>> = OnceLock::new();

/// 返回当前 epoch 毫秒。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 返回托盘菜单的全局句柄容器。
fn tray_menu_slot() -> &'static Mutex<Option<Menu<Wry>>> {
    TRAY_MENU.get_or_init(|| Mutex::new(None))
}

/// 返回上次 tooltip 的全局容器。
fn last_tooltip_slot() -> &'static Mutex<String> {
    LAST_TOOLTIP.get_or_init(|| Mutex::new(String::new()))
}

/// 创建系统托盘图标 + 菜单（对齐 Flutter buildStatusMenu）。
pub fn setup(app: &AppHandle) {
    // 启动时只建固定三段式；传输段由 refresh_menu 就地插入。
    let menu = match build_base_menu(app) {
        Ok(menu) => menu,
        Err(error) => {
            crate::platform::activation::set_tray_icon_visible(false);
            tracing::error!(%error, "创建托盘菜单失败，应用将以无托盘模式运行");
            return;
        }
    };
    // 保存句柄：之后所有刷新都在这份菜单上就地修改，绝不 set_menu 替换。
    if let Ok(mut slot) = tray_menu_slot().lock() {
        *slot = Some(menu.clone());
    }

    // 加载 menubar 图标（PNG，对齐 Flutter MenubarIcon）
    let Some(icon) = load_menubar_icon(app) else {
        crate::platform::activation::set_tray_icon_visible(false);
        tracing::error!("可用托盘图标缺失，将以无托盘模式运行");
        return;
    };

    match TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true) // 对齐 Flutter isTemplate=true：macOS 按明暗自动着色
        .tooltip(idle_tooltip(app))
        .menu(&menu)
        .show_menu_on_left_click(true) // 左键显示菜单（对齐 Flutter item.menu = menu）
        .on_menu_event(|app_handle, event| {
            match event.id().as_ref() {
                "show_window" => show_main_window(app_handle),
                "quit" => quit_app(app_handle),
                _ => {} // 传输汇总项 disabled，不会触发事件
            }
        })
        .build(app)
    {
        Ok(_) => tracing::info!("系统托盘图标+菜单已创建"),
        Err(e) => {
            crate::platform::activation::set_tray_icon_visible(false);
            tracing::error!(error = %e, "系统托盘图标创建失败，关窗将直接退出");
        }
    }
}

/// 加载 menubar 图标；内嵌 PNG 与兜底的应用图标都必须是非零尺寸。
///
/// 空尺寸图标会触发 muda 的 `ZeroWidth` panic（2026-11-22 实际崩溃），
/// 因此这里逐一校验，全部不可用时返回 None 由调用方退回无托盘模式。
fn load_menubar_icon<'a>(app: &'a AppHandle) -> Option<Image<'a>> {
    if let Ok(icon) = Image::from_bytes(MENUBAR_ICON_PNG) {
        if icon.width() > 0 && icon.height() > 0 {
            return Some(icon);
        }
        tracing::warn!("menubar PNG 解码为 0 尺寸，回退到应用图标");
    } else {
        tracing::warn!("menubar PNG 加载失败，回退到应用图标");
    }
    app.default_window_icon()
        .cloned()
        .filter(|icon| icon.width() > 0 && icon.height() > 0)
}

/// 构建固定三段式托盘菜单：版本 → 分隔 → 显示主窗口 → 分隔 → 退出。
fn build_base_menu(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    // 版本项（disabled，纯展示）
    // 首行展示名跟随 productName：dev 包显示 `PetalLink-Dev`，与正式版一眼区分。
    let header = format!(
        "{} - 华为云盘桌面客户端开源版",
        crate::constants::display_name(app)
    );
    let version_item = MenuItem::with_id(app, "version", header, false, None::<&str>)?;
    let sep_top = PredefinedMenuItem::separator(app)?;
    let show_item = MenuItem::with_id(app, "show_window", "显示主窗口", true, None::<&str>)?;
    let sep_bottom = PredefinedMenuItem::separator(app)?;
    // 退出项同样带展示名：dev 包显示「退出 PetalLink-Dev」，避免误退正式版。
    let quit_label = format!("退出 {}", crate::constants::display_name(app));
    let quit_item = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;

    // 异构菜单项统一收集为 &dyn IsMenuItem 交给 with_items
    let items: Vec<&dyn IsMenuItem<Wry>> =
        vec![&version_item, &sep_top, &show_item, &sep_bottom, &quit_item];
    Menu::with_items(app, &items)
}

/// 从全局 DB 查询进行中的传输任务（state IN PENDING/RUNNING，created_at 升序）。
fn load_active_transfers() -> AppResult<Vec<repository::TransferTask>> {
    let conn = crate::commands::DB.lock();
    repository::list_active_transfers(&conn)
}

/// 生成汇总文案：方向 + 项数 + 总体百分比。
fn summary_text(tasks: &[repository::TransferTask]) -> String {
    // 汇总总量与已传字节；总量为 0 时百分比按 0 显示。
    let total: i64 = tasks.iter().map(|task| task.total_size.max(0)).sum();
    let transferred: i64 = tasks.iter().map(|task| task.transferred.max(0)).sum();
    // 百分比：只做展示，不参与任何调度判断。
    let pct = if total > 0 {
        ((transferred.min(total) * 100) / total) as i32
    } else {
        0
    };
    // 全部同向时才用具体方向，混合方向用「正在传输」。
    let directions = tasks
        .iter()
        .map(|task| task.direction)
        .collect::<std::collections::BTreeSet<_>>();
    let label = match directions.len() {
        1 if directions.contains(&transfer_direction::UPLOAD) => "正在上传",
        1 if directions.contains(&transfer_direction::DOWNLOAD) => "正在下载",
        1 if directions.contains(&transfer_direction::DOWNLOAD_UPDATE) => "正在更新",
        1 => "正在删除",
        _ => "正在传输",
    };
    format!("{label} {} 项 · {pct}%", tasks.len())
}

/// 按 id 取回汇总菜单项。
fn summary_item(menu: &Menu<Wry>) -> Option<MenuItem<Wry>> {
    match menu.get(MENU_ID_TRANSFER_SUMMARY) {
        Some(MenuItemKind::MenuItem(item)) => Some(item),
        _ => None,
    }
}

/// 就地插入传输段（分隔线 + 汇总行 + 分隔线）。
fn insert_transfer_block(app: &AppHandle<Wry>, menu: &Menu<Wry>, text: &str) -> tauri::Result<()> {
    let sep_top = PredefinedMenuItem::separator(app)?;
    let summary = MenuItem::with_id(app, MENU_ID_TRANSFER_SUMMARY, text, false, None::<&str>)?;
    let sep_bottom = PredefinedMenuItem::separator(app)?;
    let items: [&dyn IsMenuItem<Wry>; 3] = [&sep_top, &summary, &sep_bottom];
    menu.insert_items(&items, TRANSFER_BLOCK_POSITION)
}

/// 就地移除传输段（汇总行两侧的分隔线一并移除）。
fn remove_transfer_block(menu: &Menu<Wry>) -> tauri::Result<()> {
    let Ok(items) = menu.items() else {
        return Ok(());
    };
    let Some(index) = items
        .iter()
        .position(|item| item.id() == MENU_ID_TRANSFER_SUMMARY)
    else {
        return Ok(());
    };
    // 先删汇总行之后的分隔线，再删汇总行，最后删它之前的分隔线，索引依次回收。
    if index + 1 < items.len() {
        menu.remove_at(index + 1)?;
    }
    menu.remove_at(index)?;
    if index > 0 {
        menu.remove_at(index - 1)?;
    }
    Ok(())
}

/// 传输任务变化时刷新托盘菜单（就地改文本 / 就地增删传输段，绝不替换 NSMenu）。
///
/// 这里只做「查库 + 计算 + 节流判断」；所有托盘/菜单句柄都在主线程闭包内获取，
/// 见文件头的线程约束。
pub fn refresh_menu(app: &AppHandle) {
    // 先查进行中传输：决定传输段是否存在以及汇总文案。
    let tasks = match load_active_transfers() {
        Ok(tasks) => tasks,
        Err(error) => {
            tracing::warn!(%error, "查询进行中传输失败，托盘菜单保持上次状态");
            return;
        }
    };
    let has_active = !tasks.is_empty();
    let text = summary_text(&tasks);
    let now = now_ms();
    let last_active = LAST_HAS_ACTIVE.load(Ordering::Relaxed);

    // 结构变化：合并短时间内的抖动；进入空闲必须立即清理，不留在菜单里。
    let structure_due = if has_active {
        !last_active
            && now - LAST_STRUCTURE_CHANGE_MS.load(Ordering::Relaxed)
                >= STRUCTURE_CHANGE_INTERVAL_MS
    } else {
        last_active
    };
    // 文本刷新只对已存在的传输段生效，并按间隔节流。
    let text_due = has_active
        && !structure_due
        && now - LAST_TEXT_UPDATE_MS.load(Ordering::Relaxed) >= MENU_TEXT_INTERVAL_MS;

    // tooltip 跟随当前活跃状态，内容相同不重复写入。
    let tooltip = if has_active {
        text.clone()
    } else {
        idle_tooltip(app)
    };
    let tooltip_due = last_tooltip_slot()
        .lock()
        .map(|last| last.as_str() != tooltip)
        .unwrap_or(false);

    if !structure_due && !text_due && !tooltip_due {
        return;
    }
    // 先占住节流窗口再提交，多个并发事件只会产生一次主线程更新。
    LAST_STRUCTURE_CHANGE_MS.store(now, Ordering::Relaxed);
    LAST_TEXT_UPDATE_MS.store(now, Ordering::Relaxed);
    if structure_due {
        LAST_HAS_ACTIVE.store(has_active, Ordering::Relaxed);
    }

    // 只在主线程获取并释放托盘/菜单句柄：底层是 Rc<RefCell<_>>，跨线程 clone/drop 不安全。
    let app_handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(tray) = app_handle.tray_by_id(TRAY_ID) else {
            return;
        };
        if tooltip_due {
            set_tooltip_if_changed(&tray, &tooltip);
        }
        let Some(menu) = tray_menu_slot()
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
        else {
            return;
        };
        let outcome = if structure_due {
            if has_active {
                insert_transfer_block(&app_handle, &menu, &text)
            } else {
                remove_transfer_block(&menu)
            }
        } else if let Some(item) = summary_item(&menu) {
            item.set_text(&text)
        } else {
            Ok(())
        };
        match outcome {
            Ok(()) => tracing::debug!(has_active, structure_due, "托盘菜单已就地更新"),
            Err(error) => tracing::warn!(%error, has_active, "就地更新托盘菜单失败"),
        }
    }) {
        tracing::warn!(%error, "提交托盘菜单更新到主线程失败");
    }
}

/// 仅在 tooltip 内容变化时写入，避免高频调用。
///
/// 只允许在主线程调用：`tray` 句柄本身不是跨线程安全的。
fn set_tooltip_if_changed(tray: &tauri::tray::TrayIcon<Wry>, tooltip: &str) {
    let Ok(mut last) = last_tooltip_slot().lock() else {
        return;
    };
    if last.as_str() == tooltip {
        return;
    }
    if let Err(error) = tray.set_tooltip(Some(tooltip)) {
        tracing::warn!(%error, "设置托盘 tooltip 失败");
        return;
    }
    *last = tooltip.to_string();
}

/// 更新托盘 tooltip（联动同步状态）。
pub fn update_tooltip(app: &AppHandle, tooltip: &str) {
    let app_handle = app.clone();
    let tooltip = tooltip.to_string();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = app_handle.tray_by_id(TRAY_ID) {
            set_tooltip_if_changed(&tray, &tooltip);
        }
    });
}

/// 切换托盘图标可见性，并同步更新退出拦截依据的全局标志。
///
/// 托盘隐藏后后台保活失去退出入口，activation 层的 Cmd+Q/Dock 退出拦截
/// 会据此放行真退出（见 `activation::is_tray_icon_visible`）。
pub fn set_tray_visible(app: &AppHandle, visible: bool) {
    let app_handle = app.clone();
    let _ = app.run_on_main_thread(move || match app_handle.tray_by_id(TRAY_ID) {
        Some(tray) => {
            if let Err(e) = tray.set_visible(visible) {
                crate::platform::activation::set_tray_icon_visible(false);
                tracing::error!(error = %e, visible, "切换托盘图标可见性失败");
            } else {
                crate::platform::activation::set_tray_icon_visible(visible);
                tracing::info!(visible, "托盘图标可见性已切换");
            }
        }
        None => {
            crate::platform::activation::set_tray_icon_visible(false);
            tracing::warn!("托盘图标不存在，关窗将直接退出");
        }
    });
}

/// 显示主窗口并恢复普通应用激活策略。
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

/// 标记真实退出并终止应用。
fn quit_app(app: &AppHandle) {
    tracing::info!("托盘「退出 PetalLink」— 真退出");
    crate::platform::activation::mark_real_quit();
    app.exit(0);
}
