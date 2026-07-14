//! 云端 checkpoint 公开合同测试。

use std::collections::HashMap;

use petal_link_lib::core::cache_paths;
use petal_link_lib::drive::models::{DriveFile, FileCategory};
use petal_link_lib::sync::cloud_tree::{
    load_persisted_cloud_tree, persist_cloud_checkpoint, CloudTreeCache,
};

/// 构造固定身份的云端缓存文件。
fn sample_file() -> DriveFile {
    DriveFile {
        id: "f1".into(),
        name: "学习".into(),
        category: FileCategory::Folder,
        ..Default::default()
    }
}

/// 向指定挂载目录写入原始云端树缓存。
fn write_cache_raw(abs_mount_dir: &str, json: &str) {
    let cache_file = cache_paths::cloud_tree_cache_file(abs_mount_dir).unwrap();
    std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
    std::fs::write(&cache_file, json).unwrap();
}

/// 验证不完整缓存被拒绝。
#[test]
fn test_load_rejects_incomplete_cache() {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().to_string_lossy().to_string();
    let mut tree = HashMap::new();
    tree.insert("学习".into(), sample_file());
    let cache = CloudTreeCache {
        root_folder_id: Some("root".into()),
        tree,
        path_to_id: HashMap::new(),
        cursor: Some("c1".into()),
        complete: false,
    };
    write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
    assert!(
        load_persisted_cloud_tree(&abs).is_none(),
        "complete=false 的缓存必须被拒绝，否则 startup 会拿残缺缓存触发文件同步"
    );
}

/// 验证完整空云盘缓存可以加载。
#[test]
fn test_load_accepts_complete_empty_tree() {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().to_string_lossy().to_string();
    let cache = CloudTreeCache {
        root_folder_id: Some("root".into()),
        tree: HashMap::new(),
        path_to_id: HashMap::new(),
        cursor: Some("c-empty".into()),
        complete: true,
    };
    write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
    assert!(
        load_persisted_cloud_tree(&abs).is_some(),
        "完整空盘必须可作为可信 checkpoint"
    );
}

/// 验证完整非空云盘缓存可以加载。
#[test]
fn test_load_accepts_complete_cache() {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().to_string_lossy().to_string();
    let mut tree = HashMap::new();
    tree.insert("学习".into(), sample_file());
    let mut path_to_id = HashMap::new();
    path_to_id.insert("学习".into(), "f1".into());
    let cache = CloudTreeCache {
        root_folder_id: Some("root".into()),
        tree,
        path_to_id,
        cursor: Some("c1".into()),
        complete: true,
    };
    write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
    let loaded = load_persisted_cloud_tree(&abs);
    assert!(loaded.is_some(), "完整缓存应被接受");
    assert_eq!(loaded.unwrap().tree.len(), 1);
}

/// 验证缺少完成标记的旧缓存被视为不完整。
#[test]
fn test_old_cache_without_complete_field_treated_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().to_string_lossy().to_string();
    // 手写无 complete 字段的旧格式 JSON
    let old_json = r#"{
        "root_folder_id": "root",
        "tree": {"学习": {"id": "f1", "name": "学习"}},
        "path_to_id": {"学习": "f1"}
    }"#;
    write_cache_raw(&abs, old_json);
    assert!(
        load_persisted_cloud_tree(&abs).is_none(),
        "旧格式缓存（无 complete 字段）应被视为不完整，强制重跑 BFS"
    );
}

/// 验证检查点持久化后正式文件可读且无临时文件残留。
#[test]
fn test_persist_internal_atomic_and_readable() {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().to_string_lossy().to_string();
    let mut tree = HashMap::new();
    tree.insert("学习".into(), sample_file());
    let mut p2i = HashMap::new();
    p2i.insert("学习".into(), "f1".into());
    let checkpoint =
        CloudTreeCache::new_trusted(Some("root".into()), tree, p2i, "c1".into()).unwrap();
    persist_cloud_checkpoint(&abs, &checkpoint).unwrap();

    let cache_file = cache_paths::cloud_tree_cache_file(&abs).unwrap();
    assert!(cache_file.exists(), "缓存文件应存在");
    // 无残留 .tmp
    let tmp = cache_file.with_extension("json.tmp");
    assert!(!tmp.exists(), "原子写后不应残留 .tmp 文件");
    // 可被 load 读回
    let loaded = load_persisted_cloud_tree(&abs);
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().root_folder_id.as_deref(), Some("root"));
}
