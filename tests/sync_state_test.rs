//! 同步进度公开合同测试。

use petal_link_lib::sync::state::SyncGlobalState;

/// 验证全部任务完成时进度为百分之百。
#[test]
fn test_progress_all_completed() {
    let state = SyncGlobalState {
        total: 100,
        completed: 100,
        ..Default::default()
    };
    assert_eq!(state.progress(), 1.0);
}

/// 验证完成一半任务时进度为百分之五十。
#[test]
fn test_progress_half() {
    let state = SyncGlobalState {
        total: 100,
        completed: 50,
        ..Default::default()
    };
    assert_eq!(state.progress(), 0.5);
}

/// 验证任务总数为零时进度安全归零。
#[test]
fn test_progress_zero_total() {
    let state = SyncGlobalState::default();
    assert_eq!(state.progress(), 1.0);
}
