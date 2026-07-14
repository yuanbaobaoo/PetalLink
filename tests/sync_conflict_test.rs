//! 同步冲突处理公开合同测试。

use chrono::{DateTime, TimeZone, Utc};
use petal_link_lib::drive::models::{DriveFile, FileCategory};
use petal_link_lib::sync::conflict::{dedupe_copy_path, ConflictResolver, ConflictSide};
use tempfile::tempdir;

/// 构造带指定编辑时间的云端文件。
fn sample_cloud_file(edited_time: DateTime<Utc>) -> DriveFile {
    DriveFile {
        id: "f1".into(),
        name: "test.txt".into(),
        category: FileCategory::None,
        size: 100,
        parent_folder: None,
        description: None,
        created_time: None,
        edited_time: Some(edited_time),
        mime_type: Some("text/plain".into()),
        content_hash: None,
        thumbnail_link: None,
    }
}

/// 验证本地晚六十秒时本地版本胜出。
#[test]
fn test_local_wins_when_60s_later() {
    let mut resolver = ConflictResolver::new();
    let local_mtime = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let cloud_time = Utc.with_ymd_and_hms(2026, 6, 20, 11, 58, 0).unwrap(); // 提前 120s
    let cloud = sample_cloud_file(cloud_time);
    let dir = tempdir().unwrap().keep();
    let local_path = dir.join("test.txt");

    let resolution = resolver.resolve(&local_path, &cloud, &local_mtime);
    assert_eq!(resolution.winner, ConflictSide::Local);
    assert!(resolution.copy_path.to_string_lossy().contains("云端副本"));
}

/// 验证时间差小于六十秒时云端版本胜出。
#[test]
fn test_cloud_wins_when_diff_less_than_60s() {
    let mut resolver = ConflictResolver::new();
    let local_mtime = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let cloud_time = Utc.with_ymd_and_hms(2026, 6, 20, 11, 59, 30).unwrap(); // 提前 30s
    let cloud = sample_cloud_file(cloud_time);
    let dir = tempdir().unwrap().keep();
    let local_path = dir.join("test.txt");

    let resolution = resolver.resolve(&local_path, &cloud, &local_mtime);
    assert_eq!(resolution.winner, ConflictSide::Cloud);
    assert!(resolution.copy_path.to_string_lossy().contains("本地副本"));
}

/// 验证本地时间更早时云端版本胜出。
#[test]
fn test_cloud_wins_when_local_is_earlier() {
    let mut resolver = ConflictResolver::new();
    let local_mtime = Utc.with_ymd_and_hms(2026, 6, 20, 11, 0, 0).unwrap();
    let cloud_time = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap(); // 云端更晚
    let cloud = sample_cloud_file(cloud_time);
    let dir = tempdir().unwrap().keep();
    let local_path = dir.join("test.txt");

    let resolution = resolver.resolve(&local_path, &cloud, &local_mtime);
    assert_eq!(resolution.winner, ConflictSide::Cloud);
}

/// 验证冲突副本路径包含来源标签与原扩展名。
#[test]
fn test_dedupe_copy_path_format() {
    let dir = tempdir().unwrap().keep();
    let path = dir.join("report.txt");
    let stamp = Utc.with_ymd_and_hms(2026, 6, 20, 14, 30, 0).unwrap();
    let copy = dedupe_copy_path(&path, "本地副本", &stamp);
    assert_eq!(
        copy.file_name().unwrap().to_string_lossy(),
        "report (本地副本 2026-06-20 14-30-00).txt"
    );
}

/// 验证冲突副本撞名时追加序号。
#[test]
fn test_dedupe_copy_path_adds_sequence() {
    let dir = tempdir().unwrap().keep();
    // 先创建一个同名副本
    std::fs::write(dir.join("report (本地副本 2026-06-20 14-30-00).txt"), "dup").unwrap();
    let path = dir.join("report.txt");
    let stamp = Utc.with_ymd_and_hms(2026, 6, 20, 14, 30, 0).unwrap();
    let copy = dedupe_copy_path(&path, "本地副本", &stamp);
    // 应生成序号版本
    assert!(copy.to_string_lossy().contains("(1)"));
}
