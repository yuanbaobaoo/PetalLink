//! 文件跳过逻辑（.hwcloud_ 前缀 / .tmp / glob 跳过统一）。
//!
//! 对齐 `legacy/lib/mount/mount_manager.dart` 的 `_shouldSkipNameTopLevel` +
//! `local_watcher.dart` 的 `_shouldSkip` + `sync_engine.dart` 的 `_shouldSkipName`。
//!
//! 四处硬编码过滤（v1.8 全局过滤，无论用户如何配置 skipPatterns）：
//! 1. `.hwcloud_` 前缀（内部缓存/快照文件）
//! 2. `.hwcloud_placeholder` 后缀（旧版占位符）
//! 3. `.tmp` 后缀（下载原子写临时文件）
//! 4. 用户配置的 skipPatterns（简化 glob）

use regex::Regex;

/// 判断文件名是否应被跳过（不参与同步）。
///
/// 统一逻辑，供 scanLocal / local_watcher / sync_engine 复用。
pub fn should_skip(name: &str, skip_patterns: &[String]) -> bool {
    // 1. .hwcloud_ 前缀（内部文件，硬编码全局过滤）
    if name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
        return true;
    }
    // 2. 旧版占位符后缀
    if name.ends_with(".hwcloud_placeholder") {
        return true;
    }
    // 3. .tmp 后缀（下载原子写临时文件）
    if name.ends_with(crate::constants::TMP_SUFFIX) {
        return true;
    }
    // 4. 用户配置的 skipPatterns（简化 glob 匹配）
    for pattern in skip_patterns {
        if glob_matches(pattern, name) {
            return true;
        }
    }
    false
}

/// 简化 glob 匹配（对齐 dart `_shouldSkipNameTopLevel` 的 glob 实现）。
/// `*` → `.*`，`?` → `.`，转义 `\` 和 `.`，全匹配。
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    // 构建 regex：* → .*, ? → ., 转义特殊字符
    let mut regex_str = String::with_capacity(pattern.len() + 4);
    regex_str.push('^');
    for c in pattern.chars() {
        match c {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '\\' | '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }
    regex_str.push('$');
    match Regex::new(&regex_str) {
        Ok(re) => re.is_match(name),
        Err(_) => false,
    }
}
