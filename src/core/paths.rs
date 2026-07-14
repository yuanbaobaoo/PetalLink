//! 通用路径工具 —— 跨模块复用的文件系统路径辅助函数。

use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};

/// 展开路径开头的 `~/` 为 `$HOME/`。
///
/// 替代散布在 `engine.rs` 等处的 `path.replace("~/", &format!("{}/", env::var("HOME")...))`。
/// 若 HOME 环境变量缺失，`~/` 替换为空串前缀（与原内联实现一致）。
/// 不以 `~/` 开头的路径原样返回。
pub fn expand_tilde(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    path.replace("~/", &format!("{home}/"))
}

/// 校验单个路径片段，拒绝空值、路径分隔符和特殊目录。
pub fn validate_path_segment(segment: &str) -> AppResult<()> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains('\0')
    {
        return Err(AppError::config(format!("路径片段不安全：{segment}")));
    }
    Ok(())
}

/// 校验挂载目录内使用的相对路径，拒绝绝对路径、上跳、空段和反斜杠。
pub fn validate_relative_path(rel_path: &str, allow_empty: bool) -> AppResult<()> {
    if rel_path.is_empty() {
        return if allow_empty {
            Ok(())
        } else {
            Err(AppError::config("相对路径不能为空".to_string()))
        };
    }
    if rel_path.contains('\\') || rel_path.contains('\0') || rel_path.contains("//") {
        return Err(AppError::config(format!("相对路径不安全：{rel_path}")));
    }
    let path = Path::new(rel_path);
    if path.is_absolute() {
        return Err(AppError::config(format!(
            "相对路径不能是绝对路径：{rel_path}"
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_str().ok_or_else(|| {
                    AppError::config(format!("路径包含非 UTF-8 片段：{rel_path}"))
                })?;
                validate_path_segment(segment)?;
            }
            _ => return Err(AppError::config(format!("相对路径不安全：{rel_path}"))),
        }
    }
    Ok(())
}

/// 在校验后把相对路径拼到指定根目录下。
pub fn safe_join_under(base: &Path, rel_path: &str, allow_empty: bool) -> AppResult<PathBuf> {
    validate_relative_path(rel_path, allow_empty)?;
    Ok(base.join(rel_path))
}

/// 把前端给出的绝对路径转换为挂载根下的安全相对路径。
pub fn relative_path_from_mount(mount_dir: &Path, candidate: &Path) -> AppResult<String> {
    if !candidate.is_absolute() {
        return Err(AppError::config(format!(
            "本地路径必须是绝对路径：{}",
            candidate.display()
        )));
    }
    let rel = candidate
        .strip_prefix(mount_dir)
        .map_err(|_| AppError::config(format!("路径不在同步目录内：{}", candidate.display())))?;
    let rel = rel.to_string_lossy().to_string();
    validate_relative_path(&rel, false)?;
    Ok(rel)
}
