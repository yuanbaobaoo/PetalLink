//! 同步状态快照公开合同测试。

use std::collections::HashMap;

use petal_link_lib::core::cache_paths;
use petal_link_lib::sync::sync_state_store::{
    LocalFileSnapshotEntry, SnapshotEntry, SyncStateStore,
};
use tempfile::tempdir;

/// 验证公开缓存格式可被状态存储完整加载。
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

    // 直接写入公开缓存格式。
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
    let cache_file = cache_paths::sync_state_cache_file(&dir_str).unwrap();
    std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
    std::fs::write(
        &cache_file,
        serde_json::to_string_pretty(&snapshot).unwrap(),
    )
    .unwrap();

    // 加载验证
    let loaded = store.load();
    assert_eq!(loaded.len(), 1);
    let entry = loaded.get("file1.txt").unwrap();
    assert_eq!(entry.mtime, 1000);
    assert_eq!(entry.size, 2048);
    assert_eq!(entry.sha256.as_deref(), Some("abc123"));
}
