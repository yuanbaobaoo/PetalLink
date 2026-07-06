//! 通用路径工具 —— 跨模块复用的文件系统路径辅助函数。

/// 展开路径开头的 `~/` 为 `$HOME/`。
///
/// 替代散布在 `engine.rs` 等处的 `path.replace("~/", &format!("{}/", env::var("HOME")...))`。
/// 若 HOME 环境变量缺失，`~/` 替换为空串前缀（与原内联实现一致）。
/// 不以 `~/` 开头的路径原样返回。
pub fn expand_tilde(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    path.replace("~/", &format!("{home}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde_with_home() {
        // 注意：HOME 由测试环境提供（CI/本地通常都有），仅验证 ~ 被展开为 $HOME 前缀
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            assert_eq!(expand_tilde("~/drive"), format!("{home}/drive"));
            assert_eq!(expand_tilde("~/"), format!("{home}/"));
        }
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        // 不含 ~ 的路径原样返回，与 HOME 无关
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
        assert_eq!(expand_tilde(""), "");
    }
}
