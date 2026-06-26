//! ASCII-only JSON 编码 —— 华为 Drive API 中文文件名 400 错误的核心修复。
//!
//! 对齐 `legacy/lib/drive/api/files_api.dart` 的 `asciiJsonEncode`。
//!
//! # 背景（v1.4 联调修正 §10.6.1）
//! 华为 Drive API 服务端（Java 系，疑似 Jackson 默认配置）JSON 解析器**不接受**
//! UTF-8 多字节字符直接出现在 JSON 字符串值中——即使 Content-Type 声明 charset=utf-8，
//! 含中文的 `"fileName":"那你"` 会被解析为空，返回 400 + `errorCode: 21004002`。
//!
//! 解决：把所有 > 0x7F 的码点转义为 `\uXXXX` ASCII-only JSON。
//! 适用于 createFolder / update 等 application/json 请求体。
//! （upload 的 multipart/related metadata 路径容忍 UTF-8，无需此处理。）

/// 把任意可序列化值编码为 ASCII-only JSON 字符串：
/// 所有 > 0x7F 的 Unicode 码点转义为 `\uXXXX`。
///
/// 对齐 dart `asciiJsonEncode(Object? obj)`。
pub fn ascii_json_encode<T: serde::Serialize>(obj: &T) -> String {
    let raw = serde_json::to_string(obj).unwrap_or_default();
    escape_non_ascii(&raw)
}

/// 把已序列化的 JSON 字符串中的非 ASCII 字符转义为 \uXXXX。
///
/// 对于 > 0xFFFF 的字符（如 emoji），先转为 UTF-16 代理对，
/// 每个代理码元转义为 \uXXXX（对齐 dart 按 codeUnit 遍历的行为）。
pub fn escape_non_ascii(raw: &str) -> String {
    let mut buf = String::with_capacity(raw.len());
    for c in raw.chars() {
        let code = c as u32;
        if code > 0x7F {
            if code <= 0xFFFF {
                // BMP 内字符：直接 \uXXXX
                buf.push_str(&format!("\\u{:04x}", code));
            } else {
                // 辅助平面字符：转 UTF-16 代理对
                let v = code - 0x10000;
                let high = 0xD800 + (v >> 10);
                let low = 0xDC00 + (v & 0x3FF);
                buf.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
            }
        } else {
            buf.push(c);
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_ascii_only_no_chinese() {
        // 中文字符必须被转义为 \uXXXX
        let input = json!({ "fileName": "那你", "mimeType": "folder" });
        let encoded = ascii_json_encode(&input);
        // 编码后不应含原始中文字节
        assert!(!encoded.contains('你'));
        assert!(!encoded.contains('那'));
        // 应含转义形式
        assert!(encoded.contains("\\u90a3")); // 那 = U+90A3
        assert!(encoded.contains("\\u4f60")); // 你 = U+4F60
    }

    #[test]
    fn test_ascii_keeps_english() {
        let input = json!({ "fileName": "test", "size": 100 });
        let encoded = ascii_json_encode(&input);
        assert_eq!(encoded, r#"{"fileName":"test","size":100}"#);
    }

    #[test]
    fn test_ascii_mixed() {
        // 中英混合
        let input = json!({ "fileName": "报告 report.txt" });
        let encoded = ascii_json_encode(&input);
        assert!(encoded.contains("report.txt"));
        assert!(encoded.contains("\\u62a5")); // 报 = U+62A5
        assert!(encoded.contains("\\u544a")); // 告 = U+544A
    }

    #[test]
    fn test_escape_non_ascii_emoji() {
        // emoji（> 0xFFFF）由 serde_json 拆为代理对，每个代理码点被转义
        let raw = "\"😀\"";
        let escaped = escape_non_ascii(raw);
        // 😀 = U+1F600，UTF-16 代理对 D83D DE00
        assert!(escaped.contains("\\ud83d"));
        assert!(escaped.contains("\\ude00"));
    }

    #[test]
    fn test_escape_preserves_json_structure() {
        let input = json!({ "fileName": "新建文件夹" });
        let encoded = ascii_json_encode(&input);
        // 转义后仍是合法 JSON
        let reparsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(reparsed["fileName"], "新建文件夹");
    }

    #[test]
    fn test_special_chars_preserved() {
        // JSON 结构字符（{}[]:,）应保留（用多字段对象确保有逗号）
        let input = json!({ "a": "b", "c": "d" });
        let encoded = ascii_json_encode(&input);
        assert!(encoded.contains('{'));
        assert!(encoded.contains('}'));
        assert!(encoded.contains(':'));
        assert!(encoded.contains(','));
    }
}
