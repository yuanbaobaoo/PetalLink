//! 平台进程启动、退出与重启公开状态测试。

use petal_link_lib::platform::activation::{is_launched_manually, is_restarting, should_real_quit};

/// 验证测试进程被识别为手动启动。
#[test]
fn test_manual_launch_detection() {
    assert!(is_launched_manually());
}

/// 验证测试环境默认不允许真实退出。
#[test]
fn test_real_quit_default() {
    assert!(!should_real_quit());
}

/// 验证平台默认不处于重启状态。
#[test]
fn test_restarting_default() {
    assert!(!is_restarting());
}
