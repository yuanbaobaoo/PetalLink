//! 文件稳定阈值公开合同测试。

use petal_link_lib::sync::stability::EDITING_THRESHOLD_SECS;

/// 验证编辑状态阈值固定为五分钟。
#[test]
fn test_editing_threshold() {
    assert_eq!(EDITING_THRESHOLD_SECS, 300); // 5 分钟
}
