/// ASCII-only JSON 编码 —— 华为 Drive API 中文文件名 400 错误的核心修复。
///
/// 严格对齐 Rust 原版 `src/drive/ascii_json.rs`。
///
/// 背景（v1.4 联调修正 §10.6.1）：华为 Drive API 服务端（Java 系，疑似 Jackson
/// 默认配置）JSON 解析器**不接受** UTF-8 多字节字符直接出现在 JSON 字符串值中
/// ——即使 Content-Type 声明 charset=utf-8，含中文的 `"fileName":"那你"` 会被
/// 解析为空，返回 400 + `errorCode: 21004002`。
///
/// 解决：把所有 > 0x7F 的码点转义为 `\uXXXX` ASCII-only JSON。
/// 适用于 createFolder / update / delete 等 application/json 请求体。
/// （upload 的 multipart/related metadata 路径容忍 UTF-8，无需此处理。）
library;

import 'dart:convert';

/// 把任意可 JSON 序列化值编码为 ASCII-only JSON 字符串：
/// 所有 > 0x7F 的 UTF-16 code unit 转义为 `\uXXXX`（小写 hex，对齐 Rust）。
///
/// 对齐 Rust `ascii_json_encode`。Dart 字符串按 UTF-16 code unit 遍历，
/// > 0xFFFF 的字符（如 emoji）天然已是代理对，逐个转义，与 Rust 的
/// 「先转 UTF-16 代理对再逐码元转义」行为一致。
String asciiJsonEncode(Object? obj) => escapeNonAscii(jsonEncode(obj));

/// 把已序列化 JSON 字符串中的非 ASCII 字符转义为 `\uXXXX`。
///
/// 对齐 Rust `escape_non_ascii`。
String escapeNonAscii(String raw) {
  final buf = StringBuffer();
  for (final unit in raw.codeUnits) {
    if (unit > 0x7F) {
      buf.write('\\u${unit.toRadixString(16).padLeft(4, '0')}');
    } else {
      buf.writeCharCode(unit);
    }
  }
  return buf.toString();
}
