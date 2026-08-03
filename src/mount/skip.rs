//! 文件跳过逻辑（.hwcloud_ 前缀 / .tmp / glob 跳过统一）。
//!
//! 对齐 `legacy/lib/mount/mount_manager.dart` 的 `_shouldSkipNameTopLevel` +
//! `local_watcher.dart` 的 `_shouldSkip` + `sync_engine.dart` 的 `_shouldSkipName`。
//!
//! 四类统一过滤（v1.8 全局过滤，无论用户如何配置 skipPatterns）：
//! 1. `.hwcloud_` 前缀（内部缓存/快照文件）
//! 2. `.hwcloud_placeholder` 后缀（旧版占位符）
//! 3. `.tmp` 后缀（下载原子写临时文件）
//! 4. 用户配置的 skipPatterns（简化 glob）

use std::collections::HashSet;

use regex::Regex;

/// 预编译并统一执行内部规则与用户 skipPatterns。
///
/// 对 crate 外开放是为了 `tests/` 集成测试能以真实匹配器驱动拖拽导入复制的公开合同。
#[derive(Clone, Debug, Default)]
pub struct SkipMatcher {
    exact_patterns: HashSet<String>,
    wildcard_patterns: Vec<Regex>,
}

impl SkipMatcher {
    /// 将配置规则编译为可跨扫描、watcher 与规划阶段复用的匹配器。
    pub fn new(skip_patterns: &[String]) -> Self {
        let mut matcher = Self::default();
        for pattern in skip_patterns {
            if pattern.contains('*') || pattern.contains('?') {
                let regex_pattern = glob_regex_pattern(pattern);
                match Regex::new(&regex_pattern) {
                    Ok(regex) => matcher.wildcard_patterns.push(regex),
                    Err(error) => {
                        tracing::warn!(pattern, %error, "skipPattern 编译失败，本条规则不生效");
                    }
                }
            } else {
                matcher.exact_patterns.insert(pattern.clone());
            }
        }
        matcher
    }

    /// 判断单个文件名是否命中内部规则或用户配置。
    pub fn should_skip(&self, name: &str) -> bool {
        // 内部缓存、旧占位符和下载临时文件不受用户配置影响。
        if name.starts_with(crate::constants::INTERNAL_FILE_PREFIX)
            || name.ends_with(".hwcloud_placeholder")
            || name.ends_with(crate::constants::TMP_SUFFIX)
        {
            return true;
        }
        self.exact_patterns.contains(name)
            || self
                .wildcard_patterns
                .iter()
                .any(|pattern| pattern.is_match(name))
    }

    /// 判断规范相对路径中是否包含任一应跳过的目录或文件名。
    pub(crate) fn should_skip_relative_path(&self, relative_path: &str) -> bool {
        relative_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .any(|segment| self.should_skip(segment))
    }
}

/// 将简化 glob 转换为全匹配正则；所有正则元字符在通配转换前转义。
fn glob_regex_pattern(pattern: &str) -> String {
    let mut regex_str = String::with_capacity(pattern.len() + 4);
    regex_str.push('^');
    for character in pattern.chars() {
        match character {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '\\' | '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                regex_str.push('\\');
                regex_str.push(character);
            }
            _ => regex_str.push(character),
        }
    }
    regex_str.push('$');
    regex_str
}

/// 覆盖只能通过内部统一规则入口验证的路径级跳过合同。
#[cfg(test)]
mod tests {
    use super::SkipMatcher;

    /// 文件名与任意层级相对路径必须使用同一组 skipPatterns。
    #[test]
    fn relative_path_skip_matches_entry_name_rules() {
        let patterns = vec![".DS_Store".to_string(), "~$*".to_string()];
        let matcher = SkipMatcher::new(&patterns);

        assert!(matcher.should_skip(".DS_Store"));
        assert!(matcher.should_skip_relative_path("projects/legal/.DS_Store"));
        assert!(matcher.should_skip_relative_path("projects/~$contract.docx"));
        assert!(matcher.should_skip_relative_path("projects/cache.tmp"));
        assert!(!matcher.should_skip_relative_path("projects/contract.docx"));
    }

    /// 正则元字符必须保持普通文件名语义，仅星号和问号作为通配符。
    #[test]
    fn matcher_escapes_regex_metacharacters() {
        let patterns = vec![
            "report+[1](draft).*".to_string(),
            "literal+(copy)".to_string(),
        ];
        let matcher = SkipMatcher::new(&patterns);

        assert!(matcher.should_skip("report+[1](draft).pdf"));
        assert!(matcher.should_skip("literal+(copy)"));
        assert!(!matcher.should_skip("report1draft.pdf"));
    }

    /// 问号、Unicode 与内建规则必须在预编译后保持原有名称匹配语义。
    #[test]
    fn matcher_supports_question_unicode_and_builtin_rules() {
        let patterns = vec!["报告-?.txt".to_string()];
        let matcher = SkipMatcher::new(&patterns);

        assert!(matcher.should_skip("报告-甲.txt"));
        assert!(!matcher.should_skip("报告-甲乙.txt"));
        assert!(matcher.should_skip(".hwcloud_cache.json"));
        assert!(matcher.should_skip("legacy.hwcloud_placeholder"));
        assert!(matcher.should_skip("download.tmp"));
        assert!(!matcher.should_skip("download.tmp.txt"));
    }
}
