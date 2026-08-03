//! 激活策略 —— activationPolicy 切换 + --hidden 检测 + 退出拦截标志。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift`。
//! activationPolicy 通过 objc2-app-kit 直接调 NSApp.setActivationPolicy，
//! 不再依赖 osascript + System Events（需辅助功能权限，常静默失败）。
//!
//! # 退出拦截
//! Tao（Tauri 底层的 macOS 窗口库）的 AppDelegate 未实现
//! `applicationShouldTerminate:`，导致 Dock 右键「退出」/ Cmd+Q 直接
//! 绕过后端 RunEvent::ExitRequested 把进程杀掉。
//!
//! 我们 swizzle `-[NSApplication terminate:]`，在原始方法之前检查：
//! 1. `should_real_quit()` → 托盘「退出 PetalLink」
//! 2. 当前 Apple Event 是否为 kAEQuitApplication → 系统关机/登出
//! 3. 托盘图标已隐藏（后台保活无退出入口）→ 直接放行
//!    若都非 → 拦截退出，隐藏窗口 + accessory 模式，保持后台运行。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

#[cfg(target_os = "macos")]
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use objc2::runtime::Imp;

/// 用户在界面触发重启时写入 Application Support 的一次性前台启动标记。
const RELAUNCH_VISIBLE_MARKER: &str = ".relaunch-visible";
/// 一次性重启标记的最长有效期，避免崩溃遗留影响后续登录自启动。
const RELAUNCH_VISIBLE_MAX_AGE_MS: u64 = 120_000;
/// 仅 tray「退出 PetalLink」/ relaunch 置 true，放开退出拦截。
static REAL_QUIT: AtomicBool = AtomicBool::new(false);
/// relaunch 场景置 true，跳过关机 flush（缓存已清）。
static RESTARTING: AtomicBool = AtomicBool::new(false);
/// 当前是否处于 accessory 模式（关窗/Cmd+Q 拦截后）。供窗口获焦时判断是否需切回 regular。
#[cfg(target_os = "macos")]
static IS_ACCESSORY: AtomicBool = AtomicBool::new(false);
/// 托盘图标是否可见（默认可见）。托盘隐藏时后台保活失去退出入口，
/// Cmd+Q/Dock 退出应直接放行真退出，不再拦截转后台。
static TRAY_ICON_VISIBLE: AtomicBool = AtomicBool::new(true);
/// 当前进程启动来源只探测一次，保证 setup 内多次查询得到同一结果。
static LAUNCHED_MANUALLY: OnceLock<bool> = OnceLock::new();

/// 记录托盘图标可见性（启动时按配置初始化，切换开关时更新）。
pub fn set_tray_icon_visible(visible: bool) {
    TRAY_ICON_VISIBLE.store(visible, Ordering::SeqCst);
}
/// 托盘图标当前是否可见。
pub fn is_tray_icon_visible() -> bool {
    TRAY_ICON_VISIBLE.load(Ordering::SeqCst)
}

/// 标记本次退出为用户确认的真实退出。
pub fn mark_real_quit() {
    REAL_QUIT.store(true, Ordering::SeqCst);
}
/// 判断当前退出请求是否应真正终止进程。
pub fn should_real_quit() -> bool {
    REAL_QUIT.load(Ordering::SeqCst)
}
/// 标记应用正在重启，以跳过退出期重复清理。
pub fn mark_restarting() {
    RESTARTING.store(true, Ordering::SeqCst);
    REAL_QUIT.store(true, Ordering::SeqCst);
    // Tauri 重启会保留原参数；登录启动的进程仍带 --hidden。写一次性标记让新进程
    // 覆盖该参数，避免用户主动重启后窗口再次隐藏。
    if let Err(error) = write_relaunch_visible_marker() {
        tracing::warn!(%error, "写入重启前台标记失败，新进程可能保持隐藏");
    }
}

/// 窗口获焦时调用：若当前处于 accessory 模式（关窗/Cmd+Q 后台），切回 regular 恢复可交互。
/// 不在 accessory 模式时 no-op，避免正常前台获焦重复调用。
#[cfg(target_os = "macos")]
pub fn ensure_regular_if_was_accessory() {
    if IS_ACCESSORY.swap(false, Ordering::SeqCst) {
        set_regular();
        tracing::info!("窗口获焦：从 accessory 切回 regular（恢复可交互）");
    }
}

/// 非 macOS 平台无需调整激活策略。
#[cfg(not(target_os = "macos"))]
pub fn ensure_regular_if_was_accessory() {}
/// 判断当前退出是否由应用重启触发。
pub fn is_restarting() -> bool {
    RESTARTING.load(Ordering::SeqCst)
}

/// 未携带 `--hidden` 参数时视为用户手动启动。
pub fn is_launched_manually() -> bool {
    *LAUNCHED_MANUALLY.get_or_init(|| {
        if !std::env::args().any(|argument| argument == "--hidden") {
            remove_relaunch_visible_marker();
            true
        } else {
            consume_relaunch_visible_marker()
        }
    })
}

/// 返回当前 Application Support 中的一次性重启标记路径。
fn relaunch_visible_marker_path() -> crate::error::AppResult<std::path::PathBuf> {
    Ok(crate::core::config_store::support_dir()?.join(RELAUNCH_VISIBLE_MARKER))
}

/// 写入当前时间，供紧随其后的新进程以前台模式恢复。
fn write_relaunch_visible_marker() -> crate::error::AppResult<()> {
    let path = relaunch_visible_marker_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let now = current_unix_millis().ok_or_else(|| {
        crate::error::AppError::generic("系统时间早于 Unix epoch，无法写入重启标记")
    })?;
    std::fs::write(path, now.to_string())?;
    Ok(())
}

/// 手动启动时清理可能由异常重启遗留的标记。
fn remove_relaunch_visible_marker() {
    if let Ok(path) = relaunch_visible_marker_path() {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::debug!(%error, "清理重启前台标记失败"),
        }
    }
}

/// 读取后立即删除一次性标记，仅接受足够新的合法时间戳。
fn consume_relaunch_visible_marker() -> bool {
    let Ok(path) = relaunch_visible_marker_path() else {
        return false;
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            tracing::debug!(%error, "读取重启前台标记失败");
            return false;
        }
    };
    let _ = std::fs::remove_file(&path);
    let Some(now) = current_unix_millis() else {
        return false;
    };
    content
        .trim()
        .parse::<u64>()
        .map(|created_at| relaunch_marker_is_recent(created_at, now))
        .unwrap_or(false)
}

/// 获取当前 Unix 毫秒时间戳。
fn current_unix_millis() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

/// 判断标记时间是否位于允许的重启窗口内，并容忍最多五秒的时钟偏差。
fn relaunch_marker_is_recent(created_at: u64, now: u64) -> bool {
    created_at <= now.saturating_add(5_000)
        && now.saturating_sub(created_at) <= RELAUNCH_VISIBLE_MAX_AGE_MS
}

/// 切换为普通应用模式，使 Dock 与窗口恢复交互。
#[cfg(target_os = "macos")]
pub fn set_regular() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 0i32] };
    let _: () = unsafe { msg_send![&app, activateIgnoringOtherApps: true] };
    IS_ACCESSORY.store(false, Ordering::SeqCst);
    tracing::info!("已设 .regular policy");
}

/// 非 macOS 平台无需切换普通应用模式。
#[cfg(not(target_os = "macos"))]
pub fn set_regular() {}

/// 切换为附件应用模式，隐藏 Dock 图标但保留托盘进程。
#[cfg(target_os = "macos")]
pub fn set_accessory() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 1i32] };
    IS_ACCESSORY.store(true, Ordering::SeqCst);
    tracing::info!("已设 .accessory policy");
}

/// 非 macOS 平台无需切换附件应用模式。
#[cfg(not(target_os = "macos"))]
pub fn set_accessory() {}

/// 根据启动参数设置初始应用激活策略。
pub fn init_activation_policy() {
    if is_launched_manually() {
        set_regular();
    } else {
        set_accessory();
    }
}

// ──  退出拦截  ──────────────────────────────────────────────

/// Apple Event 核心事件类别常量。
/// 常量标识：kCoreEventClass = 'aevt'。
#[cfg(target_os = "macos")]
const K_CORE_EVENT_CLASS: u32 = 0x61657674;
/// Apple Event 退出应用事件常量。
/// 常量标识：kAEQuitApplication = 'quit'。
#[cfg(target_os = "macos")]
const K_AE_QUIT_APPLICATION: u32 = 0x71756974;

/// 存储原始的 `-[NSApplication terminate:]` IMP。
#[cfg(target_os = "macos")]
static ORIGINAL_TERMINATE: Mutex<Option<Imp>> = Mutex::new(None);

/// 检测当前 Apple Event 是否为系统关机/登出触发的 Quit。
///
/// macOS 在关机/登出时会向每个运行中的应用**先**发送
/// `kAEQuitApplication` Apple Event，再由此触发
/// 方法标识：`-[NSApplication terminate:]`。
///
/// 检查 `[[NSAppleEventManager sharedAppleEventManager] currentAppleEvent]`：
/// - 系统关机/登出 → eventClass==kCoreEventClass, eventID==kAEQuitApplication
/// - 用户 Dock Quit / Cmd+Q → currentAppleEvent 返回 nil
#[cfg(target_os = "macos")]
fn is_apple_event_system_quit() -> bool {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;

    // NSAppleEventManager 是 Foundation 的一部分，通过 msg_send! 调用
    let mgr_cls = objc2::class!(NSAppleEventManager);
    let mgr: Retained<AnyObject> = unsafe { msg_send![mgr_cls, sharedAppleEventManager] };
    let event: Option<Retained<AnyObject>> = unsafe { msg_send![&mgr, currentAppleEvent] };

    if let Some(event) = event {
        let event_class: u32 = unsafe { msg_send![&event, eventClass] };
        let event_id: u32 = unsafe { msg_send![&event, eventID] };
        let is_quit = event_class == K_CORE_EVENT_CLASS && event_id == K_AE_QUIT_APPLICATION;
        if is_quit {
            tracing::info!("检测到系统关机/登出 Apple Event（kAEQuitApplication），放行退出");
        }
        return is_quit;
    }

    false
}

/// 隐藏所有可见窗口，切换到 accessory 模式（Dock 图标消失）。
#[cfg(target_os = "macos")]
fn hide_windows_and_go_accessory() {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;

    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    let windows: Retained<AnyObject> = unsafe { msg_send![&app, windows] };
    let count: usize = unsafe { msg_send![&windows, count] };
    for i in 0..count {
        let window: Retained<AnyObject> = unsafe { msg_send![&windows, objectAtIndex: i] };
        let is_visible: bool = unsafe { msg_send![&window, isVisible] };
        if is_visible {
            let _: () = unsafe { msg_send![&window, orderOut: std::ptr::null::<AnyObject>()] };
        }
    }
    set_accessory();
}

/// 替换 `-[NSApplication terminate:]` 的实现。
///
/// 放行条件（任一满足即调用原始方法）：
/// 1. `should_real_quit()` — 托盘「退出 PetalLink」
/// 2. `is_apple_event_system_quit()` — 系统关机/登出
/// 3. `!is_tray_icon_visible()` — 托盘图标已隐藏，后台保活无退出入口
///
/// 拦截条件：Dock 右键退出 / Cmd+Q 且托盘可见 → 隐藏窗口 + accessory 模式
#[cfg(target_os = "macos")]
extern "C-unwind" fn terminate_override(
    this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    _sender: *mut objc2::runtime::AnyObject,
) {
    if should_real_quit() || is_apple_event_system_quit() || !is_tray_icon_visible() {
        // 真正退出：调用原始 terminate:
        if let Some(orig) = *ORIGINAL_TERMINATE.lock().unwrap() {
            let orig_fn: unsafe extern "C-unwind" fn(
                *mut objc2::runtime::AnyObject,
                objc2::runtime::Sel,
                *mut objc2::runtime::AnyObject,
            ) = unsafe { std::mem::transmute(orig) };
            unsafe {
                orig_fn(this, _cmd, _sender);
            }
        }
    } else {
        tracing::info!("Dock/Cmd+Q 退出已拦截：隐藏窗口，保持后台运行");
        hide_windows_and_go_accessory();
    }
}

/// 安装退出拦截。
///
/// Swizzle `-[NSApplication terminate:]` 以在 Dock/Cmd+Q 退出时保持后台运行，
/// 同时确保系统关机/登出正常退出。
///
/// 必须在 Tauri setup 阶段尽早调用（在创建托盘之前）。
#[cfg(target_os = "macos")]
pub fn install_terminate_interceptor() {
    let cls = objc2::class!(NSApplication);
    let sel = objc2::sel!(terminate:);
    let method = cls
        .instance_method(sel)
        .expect("NSApplication must respond to terminate:");

    let original = unsafe {
        method.set_implementation(std::mem::transmute::<
            unsafe extern "C-unwind" fn(
                *mut objc2::runtime::AnyObject,
                objc2::runtime::Sel,
                *mut objc2::runtime::AnyObject,
            ),
            Imp,
        >(terminate_override))
    };
    *ORIGINAL_TERMINATE.lock().unwrap() = Some(original);
    tracing::info!("已安装 NSApplication terminate: 拦截器（含系统关机检测）");
}

/// 非 macOS 平台不安装原生退出拦截器。
#[cfg(not(target_os = "macos"))]
pub fn install_terminate_interceptor() {}

#[cfg(test)]
/// 跨平台启动来源辅助逻辑测试。
mod tests {
    use super::{relaunch_marker_is_recent, RELAUNCH_VISIBLE_MAX_AGE_MS};

    /// 重启标记只在短窗口内有效，并容忍小幅时钟偏差。
    #[test]
    fn relaunch_marker_has_bounded_lifetime() {
        let now = 1_000_000;
        assert!(relaunch_marker_is_recent(now, now));
        assert!(relaunch_marker_is_recent(now + 5_000, now));
        assert!(relaunch_marker_is_recent(
            now - RELAUNCH_VISIBLE_MAX_AGE_MS,
            now
        ));
        assert!(!relaunch_marker_is_recent(now + 5_001, now));
        assert!(!relaunch_marker_is_recent(
            now - RELAUNCH_VISIBLE_MAX_AGE_MS - 1,
            now
        ));
    }
}
