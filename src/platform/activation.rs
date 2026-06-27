//! 激活策略 —— activationPolicy 切换 + --hidden 检测 + 退出拦截标志。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift`。
//! activationPolicy 通过 objc2-app-kit 直接调 NSApp.setActivationPolicy，
//! 不再依赖 osascript + System Events（需辅助功能权限，常静默失败）。

use std::sync::atomic::{AtomicBool, Ordering};

/// 仅 tray「退出 PetalLink」/ relaunch 置 true，放开退出拦截。
static REAL_QUIT: AtomicBool = AtomicBool::new(false);
/// relaunch 场景置 true，跳过关机 flush（缓存已清）。
static RESTARTING: AtomicBool = AtomicBool::new(false);

pub fn mark_real_quit() { REAL_QUIT.store(true, Ordering::SeqCst); }
pub fn should_real_quit() -> bool { REAL_QUIT.load(Ordering::SeqCst) }
pub fn mark_restarting() { RESTARTING.store(true, Ordering::SeqCst); REAL_QUIT.store(true, Ordering::SeqCst); }
pub fn is_restarting() -> bool { RESTARTING.load(Ordering::SeqCst) }

pub fn is_launched_manually() -> bool {
    !std::env::args().any(|a| a == "--hidden")
}

#[cfg(target_os = "macos")]
pub fn set_regular() {
    // 使用 objc2 msg_send! 直接调 NSApp.setActivationPolicy，不再依赖 osascript
    // （osascript 需辅助功能权限，常静默失败导致 Dock 图标不消失）。
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2::rc::Retained;
    let cls = objc2::class!(NSApplication);
    let app: Retained<AnyObject> = unsafe { msg_send![cls, sharedApplication] };
    // NSApplicationActivationPolicyRegular = 0
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 0i32] };
    let _: () = unsafe { msg_send![&app, activateIgnoringOtherApps: true] };
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
    // NSApplicationActivationPolicyAccessory = 1
    let _: () = unsafe { msg_send![&app, setActivationPolicy: 1i32] };
    tracing::info!("已设 .accessory policy");
}

#[cfg(not(target_os = "macos"))]
pub fn set_accessory() {}

pub fn init_activation_policy() {
    if is_launched_manually() { set_regular(); }
    else { set_accessory(); }
}

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
