//! 系统托盘 —— NSStatusItem + 菜单项。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift` 的 setupStatusItem + buildStatusMenu。
//!
//! 菜单项：版本标识 / 显示主窗口 / 正在传输的文件记录（动态） / 退出 PetalLink
//! 图标：assets/menubar-icon.png（双环+同步点，对齐 Flutter MenubarIcon）
//!       以 template 方式加载（icon_as_template），macOS 按明暗自动着色，对齐 Flutter isTemplate
//! tooltip：动态，联动同步状态
//!
//! 说明：Flutter 版「立即同步」为条件显示（登录 + 配好同步目录才出现），默认隐藏；
//!       此处按需求直接不提供该项，菜单与 Flutter 默认态（canSync=false）一致。
//!
//! 「正在传输」段：每次传输变化（transfer_update 广播）时整体重建菜单，
//! 列出 transfer_queue 中 state IN (PENDING, RUNNING) 的全部任务，每个任务两行
//! （文件名 / 正在上传…N% 或 正在下载…N%），disabled 仅展示。
//! macOS 原生菜单项超出屏幕高度时自动出现上下箭头滚动，无需手写滚动条。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::data::repository::{self, transfer_direction, transfer_state};
use crate::error::AppResult;

/// 托盘唯一标识
const TRAY_ID: &str = "PetalLink-tray";
/// 菜单中传输文件名最大显示字符数（超出截断加省略号）
const MAX_NAME_CHARS: usize = 20;
/// 菜单重建最小间隔（毫秒）。传输进度高频变化，过频重建会让已展开的菜单闪烁消失。
/// 节流到 5 秒一次：用户点开菜单后有足够时间查看，不被重绘打断。
const MENU_REBUILD_INTERVAL_MS: i64 = 5000;
/// 上次菜单重建的 epoch 毫秒（0=从未重建，首条传输出现时立即重建）
static LAST_MENU_REBUILD_MS: AtomicI64 = AtomicI64::new(0);
/// 上次传输段签名（项目数 + 每个 task 的 id+state+transferred 组合 hash），相同则跳过菜单重建
/// 防止无传输状态变化时高频重建触发 muda icon 渲染 panic
static LAST_TRANSFER_SIGNATURE: AtomicU64 = AtomicU64::new(0);
/// menubar-icon.png（编译期嵌入，对齐 Flutter Asset Catalog 的 MenubarIcon）
/// SVG 经 qlmanage 转 PNG（Tauri 2 Image::from_bytes 不支持 SVG）
const MENUBAR_ICON_PNG: &[u8] = include_bytes!("../../assets/menubar-icon.png");

/// 创建系统托盘图标 + 菜单（对齐 Flutter buildStatusMenu）。
pub fn setup(app: &AppHandle) {
    // 构建菜单（对齐 AppDelegate.buildStatusMenu，内含动态「正在传输」段）
    let menu = build_menu(app).expect("创建托盘菜单失败");

    // 加载 menubar 图标（PNG，对齐 Flutter MenubarIcon）
    let icon = Image::from_bytes(MENUBAR_ICON_PNG).unwrap_or_else(|_| {
        tracing::warn!("menubar PNG 加载失败，回退到应用图标");
        app.default_window_icon().cloned().unwrap()
    });

    let _ = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true) // 对齐 Flutter isTemplate=true：macOS 按明暗自动着色
        .tooltip("PetalLink — 后台同步中")
        .menu(&menu)
        .show_menu_on_left_click(true) // 左键显示菜单（对齐 Flutter item.menu = menu）
        .on_menu_event(|app_handle, event| {
            match event.id().as_ref() {
                "show_window" => show_main_window(app_handle),
                "quit" => quit_app(app_handle),
                _ => {} // 传输记录项 disabled，不会触发事件
            }
        })
        .build(app);

    tracing::info!("系统托盘图标+菜单已创建");
}

/// 构建托盘菜单（含动态「正在传输」段）。
///
/// 顺序：版本 → 分隔 → 显示主窗口 → [分隔 → 正在传输项… → 分隔] → 退出。
/// 方括号段仅当存在进行中的传输任务时才出现，否则菜单回退到三段式（与历史一致）。
fn build_menu(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    // 版本项（disabled，纯展示）
    let version_item = MenuItem::with_id(
        app,
        "version",
        "PetalLink - 华为云盘 Mac 客户端开源版 v1.0",
        false,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let show_item = MenuItem::with_id(app, "show_window", "显示主窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 PetalLink", true, None::<&str>)?;

    // 进行中的传输项（每个任务两行 disabled MenuItem）
    let transfer_items = active_transfer_menu_items(app);

    // 上分隔线：仅当存在传输段时创建（分隔「显示主窗口」与传输段）
    // 下分隔线：无条件创建（始终分隔上方内容与「退出」，对齐历史 sep2，避免无传输时丢线）
    let has_transfers = !transfer_items.is_empty();
    let sep_top = if has_transfers {
        Some(PredefinedMenuItem::separator(app)?)
    } else {
        None
    };
    let sep_bottom = PredefinedMenuItem::separator(app)?;

    // 异构菜单项统一收集为 &dyn IsMenuItem 交给 with_items
    let mut items: Vec<&dyn IsMenuItem<Wry>> = Vec::new();
    items.push(&version_item);
    items.push(&sep1);
    items.push(&show_item);

    // 传输段：仅当有任务时插入，由 sep_top 与 sep_bottom 包夹
    if has_transfers {
        if let Some(ref sep) = sep_top {
            items.push(sep);
        }
        for it in &transfer_items {
            items.push(it);
        }
    }

    items.push(&sep_bottom);
    items.push(&quit_item);

    Menu::with_items(app, &items)
}

/// 查询进行中的传输任务并构造菜单项（每个任务两行：文件名 / 正在上传…N% 或 正在下载…N%）。
///
/// 数据源：transfer_queue 表，state IN (PENDING, RUNNING)，按 created_at 升序
/// （最早入队的排前面，符合「先开始先显示」直觉）。
/// 全部列出，不截断 —— macOS 原生菜单超出屏幕时自动上下箭头滚动。
fn active_transfer_menu_items(app: &AppHandle<Wry>) -> Vec<MenuItem<Wry>> {
    match load_active_transfers() {
        Ok(tasks) => tasks
            .iter()
            .flat_map(|t| build_one_transfer_item(app, t))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "查询进行中传输失败，托盘菜单省略传输段");
            Vec::new()
        }
    }
}

/// 为单个传输任务构造两行菜单项（文件名行 + 状态行），均 disabled。
///
/// @param app - 应用句柄（构造 MenuItem 必需）
/// @param task - 传输任务
fn build_one_transfer_item(
    app: &AppHandle<Wry>,
    task: &repository::TransferTask,
) -> Vec<MenuItem<Wry>> {
    // 名字行：文件名（disabled 灰色展示），超长截断为最多 10 字符 + 省略号
    let display_name = truncate_name(&task.name, MAX_NAME_CHARS);
    let name_item = match MenuItem::with_id(
        app,
        format!("transfer_name_{}", task.id),
        display_name.as_str(),
        false,
        None::<&str>,
    ) {
        Ok(item) => item,
        Err(e) => {
            tracing::warn!(error = %e, id = task.id, "创建传输文件名菜单项失败，跳过该项");
            return Vec::new();
        }
    };

    // 状态行：正在上传…N% / 正在下载…N% / 正在删除…N%（disabled 灰色展示）
    let status_text = format_transfer_status(task);
    let status_item = match MenuItem::with_id(
        app,
        format!("transfer_status_{}", task.id),
        status_text.as_str(),
        false,
        None::<&str>,
    ) {
        Ok(item) => item,
        Err(e) => {
            tracing::warn!(error = %e, id = task.id, "创建传输状态菜单项失败，跳过该项");
            return Vec::new();
        }
    };

    vec![name_item, status_item]
}

/// 生成传输状态文本：方向 + 百分比。
///
/// - UPLOAD → 「正在上传…N%」
/// - DOWNLOAD → 「正在下载…N%」
/// - DELETE → 「正在删除…N%」
///
/// total_size 缺失（0）时百分比按 0 显示，避免 0/0 噪声。
fn format_transfer_status(task: &repository::TransferTask) -> String {
    // 方向标签（与 transfer_direction 常量对齐）
    let label = if task.direction == transfer_direction::UPLOAD {
        "正在上传"
    } else if task.direction == transfer_direction::DOWNLOAD {
        "正在下载"
    } else if task.direction == transfer_direction::DOWNLOAD_UPDATE {
        "正在更新"
    } else {
        "正在删除"
    };
    // 百分比：transferred/total_size，缺失或为 0 时记 0%
    let pct = if task.total_size > 0 {
        std::cmp::min(100, (task.transferred * 100 / task.total_size) as i32)
    } else {
        0
    };
    format!("{label}…{pct}%")
}

/// 截断文件名至最多 `max_chars` 个字符（按字符计，非字节），超出追加省略号。
/// 用 chars() 而非字节切片，避免中文字符被切断。
fn truncate_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let truncated: String = name.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// 从全局 DB 查询进行中的传输任务（state IN PENDING/RUNNING，created_at 升序）。
fn load_active_transfers() -> AppResult<Vec<repository::TransferTask>> {
    let conn = crate::commands::DB.lock();
    let mut stmt = conn
        .prepare("SELECT * FROM transfer_queue WHERE state IN (?1, ?2) ORDER BY created_at ASC")
        .map_err(|e| crate::error::AppError::generic(format!("查询传输任务失败：{e}")))?;
    let rows = stmt
        .query_map(
            rusqlite::params![transfer_state::PENDING, transfer_state::RUNNING],
            repository::TransferTask::from_row,
        )
        .map_err(|e| crate::error::AppError::generic(format!("查询传输任务失败：{e}")))?;
    let mut tasks = Vec::new();
    for row in rows {
        tasks.push(row.map_err(|error| {
            crate::error::AppError::generic(format!("读取传输任务失败：{error}"))
        })?);
    }
    Ok(tasks)
}

/// 重建托盘菜单（在传输变化时调用，刷新「正在传输」段）。
///
/// 复用 transfer_update 广播触发：入队/进度更新/结算/引擎状态推送均会调用。
/// **节流**：传输进度高频变化（500ms 一次进度回调），过频重建会让已展开的菜单闪烁消失。
/// 故按 MENU_REBUILD_INTERVAL_MS（5 秒）节流。仅当「无进行中传输」时不节流，
/// 以保证任务完成时立即清掉传输段（不残留已完成项）。
/// 容错：重建失败仅告警不中断，对齐 update_tooltip 的容错风格。
/// 计算传输任务的签名（项目数 + 每个 task 的 id+state+transferred 组合 hash）。
/// 签名相同 → 传输段无变化 → 可跳过重建。
fn transfer_signature(tasks: &[repository::TransferTask]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tasks.len().hash(&mut hasher);
    for t in tasks {
        t.id.hash(&mut hasher);
        t.state.hash(&mut hasher);
        t.transferred.hash(&mut hasher);
        t.total_size.hash(&mut hasher);
    }
    hasher.finish()
}

pub fn refresh_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    // 先查传输任务（用于签名判等 + 节流）
    let tasks = match load_active_transfers() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "查询进行中传输失败，托盘菜单省略传输段");
            Vec::new()
        }
    };
    let has_active = !tasks.is_empty();

    // ★ 签名判等：传输段无变化时跳过重建（防止高频 sync_state 广播触发 muda icon panic）
    let sig = transfer_signature(&tasks);
    if sig == LAST_TRANSFER_SIGNATURE.load(Ordering::Relaxed) {
        return; // 传输段完全没变，不重建
    }

    // 有进行中传输 → 节流（避免高频重绘打断用户查看菜单）；无传输 → 立即重建（清场）
    if has_active {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let last = LAST_MENU_REBUILD_MS.load(Ordering::Relaxed);
        if last != 0 && now - last < MENU_REBUILD_INTERVAL_MS {
            return; // 节流窗口内，跳过本次重建
        }
        LAST_MENU_REBUILD_MS.store(now, Ordering::Relaxed);
    }

    match build_menu(app) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                tracing::warn!(error = %e, "托盘菜单重建失败");
            } else {
                // ★ 签名仅在成功重建后更新，避免"签名已更新但菜单未重建"的语义偏差
                LAST_TRANSFER_SIGNATURE.store(sig, Ordering::Relaxed);
            }
        }
        Err(e) => tracing::warn!(error = %e, "构造托盘菜单失败"),
    }
}

/// 更新托盘 tooltip（联动同步状态）。
pub fn update_tooltip(app: &AppHandle, tooltip: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
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
