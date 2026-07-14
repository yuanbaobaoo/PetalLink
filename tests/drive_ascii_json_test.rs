//! Drive ASCII JSON 编码与非 ASCII 转义测试。

use petal_link_lib::drive::ascii_json::{ascii_json_encode, escape_non_ascii};
use serde_json::json;

/// 验证纯英文 JSON 编码保持 ASCII 内容。
#[test]
fn test_ascii_keeps_english() {
    let input = json!({ "fileName": "test", "size": 100 });
    let encoded = ascii_json_encode(&input);
    assert_eq!(encoded, r#"{"fileName":"test","size":100}"#);
}

/// 验证中英文混合 JSON 将非 ASCII 字符转义。
#[test]
fn test_ascii_mixed() {
    let input = json!({ "fileName": "报告 report.txt" });
    let encoded = ascii_json_encode(&input);
    assert!(encoded.contains("report.txt"));
    assert!(encoded.contains("\\u62a5"));
    assert!(encoded.contains("\\u544a"));
}

/// 验证表情符号按 UTF-16 代理对转义。
#[test]
fn test_escape_non_ascii_emoji() {
    let escaped = escape_non_ascii("\"😀\"");
    assert!(escaped.contains("\\ud83d"));
    assert!(escaped.contains("\\ude00"));
}
