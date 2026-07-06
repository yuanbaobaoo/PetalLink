//! 缓存文件路径管理。
//!
//! 对齐 `legacy/lib/core/cache_paths.dart`。
//!
//! 同步快照与云端树缓存统一放在 Application Support 工作目录（与 config.json /
//! petal_link.db 同目录），避免污染用户同步目录。文件名用同步目录绝对路径转义后
//! 作标识符，区分不同同步目录的缓存。

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config_store::support_dir;
use crate::error::AppResult;

/// 把绝对路径转义为文件名安全的字符串：非 `[A-Za-z0-9._-]` 字符替换为 `_`。
///
/// 例：`/Users/me/hwcloud-drive` → `_Users_me_hwcloud-drive`
/// 对齐 dart `escapeMountPath`。
pub fn escape_mount_path(abs_path: &str) -> String {
    abs_path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 缓存基准目录（Application Support 工作目录）。
pub fn cache_base_dir() -> AppResult<PathBuf> {
    support_dir()
}

/// 同步状态快照缓存文件：`<base>/syncstate_<escaped>.json`
pub fn sync_state_cache_file(abs_mount_dir: &str) -> AppResult<PathBuf> {
    Ok(cache_base_dir()?.join(format!(
        "syncstate_{}.json",
        escape_mount_path(abs_mount_dir)
    )))
}

/// 云端树缓存文件：`<base>/cloudtree_<escaped>.json`
pub fn cloud_tree_cache_file(abs_mount_dir: &str) -> AppResult<PathBuf> {
    Ok(cache_base_dir()?.join(format!(
        "cloudtree_{}.json",
        escape_mount_path(abs_mount_dir)
    )))
}

/// changes 增量游标缓存文件：`<base>/changes_cursor_<escaped>.txt`
///
/// 存放华为 /drive/v1/changes 的分页游标，跨重启复用以走增量路径。
/// cursor 失效或文件缺失 → 调用方回退全量 BFS。
pub fn changes_cursor_file(abs_mount_dir: &str) -> AppResult<PathBuf> {
    Ok(cache_base_dir()?.join(format!(
        "changes_cursor_{}.txt",
        escape_mount_path(abs_mount_dir)
    )))
}

/// 删除指定同步目录对应的缓存文件（快照 + 云端树 + changes 游标）。
/// 供 CacheService.clearAll / 更换目录重置复用。错误仅忽略。
pub fn clear_cache_files(abs_mount_dir: &str) {
    if let Ok(f) = sync_state_cache_file(abs_mount_dir) {
        let _ = fs::remove_file(&f);
    }
    if let Ok(f) = cloud_tree_cache_file(abs_mount_dir) {
        let _ = fs::remove_file(&f);
    }
    if let Ok(f) = changes_cursor_file(abs_mount_dir) {
        let _ = fs::remove_file(&f);
    }
}

/// 删除工作目录下**所有**缓存文件（全部挂载目录的 syncstate_*/cloudtree_*）。
/// `clear_cache_files` 只删指定挂载目录的；「清空缓存」应调用本函数，
/// 否则历史遗留的旧挂载目录（如临时目录 /var/folders/.../T/.tmpXXX）缓存不会被清。
pub fn clear_all_cache_files() {
    let Ok(dir) = support_dir() else { return };
    let Ok(entries) = fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("syncstate_") || name.starts_with("cloudtree_") || name.starts_with("changes_cursor_") {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// 旧版缓存文件名（迁移用，存放在同步目录内）。
pub const LEGACY_SYNC_STATE_NAME: &str = ".hwcloud_syncstate";
pub const LEGACY_CLOUD_TREE_NAME: &str = ".hwcloud_cloudtree.json";

/// 一次性迁移：把同步目录下的旧缓存文件移动到工作目录的新路径。
/// 旧版升级场景：缓存还在同步目录里的 `.hwcloud_*`。若新路径不存在但旧路径
/// 存在，则迁移过来；迁移后删除旧文件。失败仅忽略（最坏情况是重建缓存）。
pub fn migrate_legacy_cache(abs_mount_dir: &str) {
    // 同步状态快照
    if let (Ok(new_file), Ok(old_file)) = (
        sync_state_cache_file(abs_mount_dir),
        std::result::Result::<PathBuf, ()>::Ok(Path::new(abs_mount_dir).join(LEGACY_SYNC_STATE_NAME)),
    ) {
        if !new_file.exists() && old_file.exists() {
            if let Some(parent) = new_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&old_file, &new_file);
        }
    }

    // 云端树缓存
    if let (Ok(new_file), Ok(old_file)) = (
        cloud_tree_cache_file(abs_mount_dir),
        std::result::Result::<PathBuf, ()>::Ok(Path::new(abs_mount_dir).join(LEGACY_CLOUD_TREE_NAME)),
    ) {
        if !new_file.exists() && old_file.exists() {
            if let Some(parent) = new_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&old_file, &new_file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_mount_path() {
        assert_eq!(
            escape_mount_path("/Users/me/hwcloud-drive"),
            "_Users_me_hwcloud-drive"
        );
    }

    #[test]
    fn test_escape_keeps_safe_chars() {
        assert_eq!(
            escape_mount_path("/Users/a.b-c_d/data"),
            "_Users_a.b-c_d_data"
        );
    }

    #[test]
    fn test_cache_file_naming() {
        let f = sync_state_cache_file("/Users/me/drive").unwrap();
        assert!(f
            .to_string_lossy()
            .ends_with("syncstate__Users_me_drive.json"));
        let f = cloud_tree_cache_file("/Users/me/drive").unwrap();
        assert!(f
            .to_string_lossy()
            .ends_with("cloudtree__Users_me_drive.json"));
    }
}
