//! 同步状态快照 —— `.hwcloud_syncstate` JSON 持久化。
//!
//! 对齐 `legacy/lib/sync/sync_state_store.dart`。
//!
//! 记录每个文件的：相对路径 → mtime / size / sha256 快照。
//! 供下次启动与当前本地扫描做三方 diff，识别退出期间改动并自动同步（F-MOUNT-14）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::cache_paths;
use crate::error::AppResult;

/// 快照条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// 修改时间（毫秒 epoch）
    pub mtime: i64,
    /// 文件大小（字节）
    pub size: u64,
    /// SHA256 hex（可选）
    pub sha256: Option<String>,
}

/// 同步状态快照持久化。
pub struct SyncStateStore {
    /// 挂载目录（绝对路径）
    mount_dir: String,
}

impl SyncStateStore {
    pub fn new(mount_dir: &str) -> Self {
        Self {
            mount_dir: mount_dir.to_string(),
        }
    }

    /// 快照文件是否存在（供首次启动判断）。
    pub fn exists(&self) -> bool {
        cache_paths::sync_state_cache_file(&self.mount_dir)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// 加载本地快照（不存在或解析失败则返回空）。
    pub fn load(&self) -> HashMap<String, SnapshotEntry> {
        let path = match cache_paths::sync_state_cache_file(&self.mount_dir) {
            Ok(p) => p,
            Err(_) => return HashMap::new(),
        };
        if !path.exists() {
            return HashMap::new();
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };
        let files: HashMap<String, SnapshotEntry> =
            serde_json::from_str(&raw).unwrap_or_default();
        tracing::info!(count = files.len(), "已加载同步状态快照");
        files
    }

    /// 保存快照（扫描本地文件并持久化）。
    pub async fn save(&self, local_files: &HashMap<String, LocalFileSnapshotEntry>) -> AppResult<()> {
        let cache_file = cache_paths::sync_state_cache_file(&self.mount_dir)?;
        if let Some(parent) = cache_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let snapshot: HashMap<String, SnapshotEntry> = local_files
            .iter()
            .map(|(path, entry)| {
                (
                    path.clone(),
                    SnapshotEntry {
                        mtime: entry.mtime,
                        size: entry.size,
                        sha256: entry.sha256.clone(),
                    },
                )
            })
            .collect();
        let json = serde_json::to_string_pretty(&snapshot)?;
        std::fs::write(&cache_file, json)?;
        tracing::info!(count = snapshot.len(), "已保存同步状态快照");
        Ok(())
    }

    /// 删除快照文件（退出登录/清空缓存时调用）。
    pub fn clear(&self) {
        cache_paths::clear_cache_files(&self.mount_dir);
    }
}

/// 本地文件快照条目（供 save 使用）。
#[derive(Debug, Clone)]
pub struct LocalFileSnapshotEntry {
    pub mtime: i64,
    pub size: u64,
    pub sha256: Option<String>,
    pub is_folder: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_snapshot_roundtrip() {
        let dir = tempdir().unwrap().keep();
        let dir_str = dir.to_string_lossy().to_string();
        let store = SyncStateStore::new(&dir_str);

        // 初始不存在
        assert!(!store.exists());

        // 保存一些条目
        let mut local: HashMap<String, LocalFileSnapshotEntry> = HashMap::new();
        local.insert(
            "file1.txt".into(),
            LocalFileSnapshotEntry {
                mtime: 1000,
                size: 2048,
                sha256: Some("abc123".into()),
                is_folder: false,
            },
        );

        // 保存需要 async runtime，此处简化为直接写文件
        let snapshot: HashMap<String, SnapshotEntry> = local
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    SnapshotEntry {
                        mtime: v.mtime,
                        size: v.size,
                        sha256: v.sha256.clone(),
                    },
                )
            })
            .collect();
        let cache_file =
            crate::core::cache_paths::sync_state_cache_file(&dir_str).unwrap();
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, serde_json::to_string_pretty(&snapshot).unwrap()).unwrap();

        // 加载验证
        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        let entry = loaded.get("file1.txt").unwrap();
        assert_eq!(entry.mtime, 1000);
        assert_eq!(entry.size, 2048);
        assert_eq!(entry.sha256.as_deref(), Some("abc123"));
    }
}
