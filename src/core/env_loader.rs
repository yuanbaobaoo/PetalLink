//! .env 加载器（开发期便利，对齐 `legacy/lib/core/config/env_loader.dart`）。
//!
//! client_secret 解析优先级（高 → 低）：
//! 1. 构建期环境变量 `TAURI_CLIENT_SECRET`（编译期注入，覆盖最强）→ 见 `constants.rs`
//! 2. `.env` 文件（开发期通过 dotenvy 加载）→ 本模块
//! 3. 默认占位符（登录会被拒绝）
//!
//! 文件不存在或解析失败时静默跳过，secret 回退到占位符。

use std::path::PathBuf;

/// 从磁盘加载 .env 文件，解析 KEY=VALUE 写入返回的 Map。
/// 对齐 dart `loadEnvFile`。搜索顺序（首个命中即用）：
/// 1. 当前工作目录
/// 2. 可执行文件所在目录
/// 3. 可执行文件父目录
///
/// 文件不存在/空 → 返回空 Map，不报错。
#[allow(dead_code)]
pub fn load_env_file() -> std::collections::HashMap<String, String> {
    let mut result = std::collections::HashMap::new();

    // 候选路径列表
    let candidates: Vec<PathBuf> = candidate_paths();
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            parse_env_content(&content, &mut result);
            if !result.is_empty() {
                tracing::debug!(path = %path.display(), "已加载 .env");
                break;
            }
        }
    }

    result
}

/// 候选 .env 路径：当前目录 / 可执行文件目录 / 可执行文件父目录。
#[allow(dead_code)]
fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(".env")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join(".env"));
            if let Some(parent) = dir.parent() {
                paths.push(parent.join(".env"));
            }
        }
    }
    paths
}

/// 解析 .env 内容（简化版：支持 KEY=VALUE、引号、注释、export 前缀）。
/// 对齐 flutter_dotenv 的 Parser 行为（足够覆盖 client_secret 场景）。
#[allow(dead_code)]
fn parse_env_content(content: &str, out: &mut std::collections::HashMap<String, String>) {
    for line in content.lines() {
        let line = line.trim();
        // 跳过空行与注释
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // 去除可选的 export 前缀
        let line = line.strip_prefix("export ").unwrap_or(line);
        // 分割 KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let value = clean_value(value.trim());
            out.insert(key, value);
        }
    }
}

/// 清理值：去除首尾引号（单/双），保留内部内容。
#[allow(dead_code)]
fn clean_value(raw: &str) -> String {
    if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || (raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2)
    {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let mut map = std::collections::HashMap::new();
        parse_env_content("HWCLOUD_CLIENT_SECRET=abc123", &mut map);
        assert_eq!(map.get("HWCLOUD_CLIENT_SECRET"), Some(&"abc123".to_string()));
    }

    #[test]
    fn test_parse_with_quotes() {
        let mut map = std::collections::HashMap::new();
        parse_env_content(r#"KEY="value with spaces""#, &mut map);
        assert_eq!(map.get("KEY"), Some(&"value with spaces".to_string()));
    }

    #[test]
    fn test_parse_skips_comments_and_empty() {
        let mut map = std::collections::HashMap::new();
        parse_env_content(
            "# comment\n\nexport FOO=bar\nBAZ=qux\n",
            &mut map,
        );
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(map.get("BAZ"), Some(&"qux".to_string()));
    }

    #[test]
    fn test_parse_skips_malformed() {
        let mut map = std::collections::HashMap::new();
        parse_env_content("=novalue\nNOEQUALS\nGOOD=ok", &mut map);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("GOOD"), Some(&"ok".to_string()));
    }
}
