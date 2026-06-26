//! 激活策略 —— activationPolicy 切换 + --hidden 检测 + 退出拦截标志。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift`。
//! 注：activationPolicy 通过 osascript 切换（System Events）。
//!     正式版可改 objc2-app-kit 直接调 NSApp.setActivationPolicy 更可靠，但需对应 feature 门控。

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
    run_osascript("regular");
    tracing::info!("已设 .regular policy");
}

#[cfg(not(target_os = "macos"))]
pub fn set_regular() {}

#[cfg(target_os = "macos")]
pub fn set_accessory() {
    run_osascript("accessory");
    tracing::info!("已设 .accessory policy");
}

#[cfg(not(target_os = "macos"))]
pub fn set_accessory() {}

pub fn init_activation_policy() {
    if is_launched_manually() { set_regular(); }
    else { set_accessory(); }
}

#[cfg(target_os = "macos")]
fn run_osascript(policy: &str) {
    let exe_name = std::env::current_exe()
        .ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "PetalLink".to_string());
    let script = format!(
        r#"tell application "System Events" to set activation policy of process "{}" to {}"#,
        exe_name, policy
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

#[cfg(not(target_os = "macos"))]
fn run_osascript(_policy: &str) {}

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
