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
//! 若都非 → 拦截退出，隐藏窗口 + accessory 模式，保持后台运行。

use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "macos")]
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use objc2::runtime::Imp;

/// 仅 tray「退出 PetalLink」/ relaunch 置 true，放开退出拦截。
static REAL_QUIT: AtomicBool = AtomicBool::new(false);
/// relaunch 场景置 true，跳过关机 flush（缓存已清）。
static RESTARTING: AtomicBool = AtomicBool::new(false);
/// 当前是否处于 accessory 模式（关窗/Cmd+Q 拦截后）。供窗口获焦时判断是否需切回 regular。
static IS_ACCESSORY: AtomicBool = AtomicBool::new(false);

pub fn mark_real_quit() { REAL_QUIT.store(true, Ordering::SeqCst); }
pub fn should_real_quit() -> bool { REAL_QUIT.load(Ordering::SeqCst) }
pub fn mark_restarting() { RESTARTING.store(true, Ordering::SeqCst); REAL_QUIT.store(true, Ordering::SeqCst); }

/// 窗口获焦时调用：若当前处于 accessory 模式（关窗/Cmd+Q 后台），切回 regular 恢复可交互。
/// 不在 accessory 模式时 no-op，避免正常前台获焦重复调用。
#[cfg(target_os = "macos")]
pub fn ensure_regular_if_was_accessory() {
    if IS_ACCESSORY.swap(false, Ordering::SeqCst) {
        set_regular();
        tracing::info!("窗口获焦：从 accessory 切回 regular（恢复可交互）");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_regular_if_was_accessory() {}
pub fn is_restarting() -> bool { RESTARTING.load(Ordering::SeqCst) }

pub fn is_launched_manually() -> bool {
    !std::env::args().any(|a| a == "--hidden")
}

#[cfg(target_os = "macos")]
pub fn set_regular() {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2::rc::Retained;
    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 0i32] };
    let _: () = unsafe { msg_send![&app, activateIgnoringOtherApps: true] };
    IS_ACCESSORY.store(false, Ordering::SeqCst);
    tracing::info!("已设 .regular policy");
}

#[cfg(not(target_os = "macos"))]
pub fn set_regular() {}

#[cfg(target_os = "macos")]
pub fn set_accessory() {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2::rc::Retained;
    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 1i32] };
    IS_ACCESSORY.store(true, Ordering::SeqCst);
    tracing::info!("已设 .accessory policy");
}

#[cfg(not(target_os = "macos"))]
pub fn set_accessory() {}

pub fn init_activation_policy() {
    if is_launched_manually() { set_regular(); }
    else { set_accessory(); }
}

// ──  退出拦截  ──────────────────────────────────────────────

/// kCoreEventClass = 'aevt'
#[cfg(target_os = "macos")]
const K_CORE_EVENT_CLASS: u32 = 0x61657674;
/// kAEQuitApplication = 'quit'
#[cfg(target_os = "macos")]
const K_AE_QUIT_APPLICATION: u32 = 0x71756974;

/// 存储原始的 `-[NSApplication terminate:]` IMP。
#[cfg(target_os = "macos")]
static ORIGINAL_TERMINATE: Mutex<Option<Imp>> = Mutex::new(None);

/// 检测当前 Apple Event 是否为系统关机/登出触发的 Quit。
///
/// macOS 在关机/登出时会向每个运行中的应用**先**发送
/// `kAEQuitApplication` Apple Event，再由此触发
/// `-[NSApplication terminate:]`。
///
/// 检查 `[[NSAppleEventManager sharedAppleEventManager] currentAppleEvent]`：
/// - 系统关机/登出 → eventClass==kCoreEventClass, eventID==kAEQuitApplication
/// - 用户 Dock Quit / Cmd+Q → currentAppleEvent 返回 nil
#[cfg(target_os = "macos")]
fn is_apple_event_system_quit() -> bool {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2::rc::Retained;

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
    use objc2::runtime::AnyObject;
    use objc2::rc::Retained;

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
///
/// 拦截条件：Dock 右键退出 / Cmd+Q → 隐藏窗口 + accessory 模式
#[cfg(target_os = "macos")]
extern "C-unwind" fn terminate_override(
    this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    _sender: *mut objc2::runtime::AnyObject,
) {
    if should_real_quit() || is_apple_event_system_quit() {
        // 真正退出：调用原始 terminate:
        if let Some(orig) = *ORIGINAL_TERMINATE.lock().unwrap() {
            let orig_fn: unsafe extern "C-unwind" fn(
                *mut objc2::runtime::AnyObject,
                objc2::runtime::Sel,
                *mut objc2::runtime::AnyObject,
            ) = unsafe { std::mem::transmute(orig) };
            unsafe { orig_fn(this, _cmd, _sender); }
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
    let method = cls.instance_method(sel)
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

#[cfg(not(target_os = "macos"))]
pub fn install_terminate_interceptor() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manual_launch_detection() { assert!(is_launched_manually()); }
    #[test]
    fn test_real_quit_default() { assert!(!should_real_quit()); }
    #[test]
    fn test_restarting_default() { assert!(!is_restarting()); }
}
