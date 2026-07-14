//! Drive 文件、配额与分页领域模型测试。

use petal_link_lib::drive::models::{DriveAbout, DriveFile, FileCategory, FileListResult};
use serde_json::json;

/// 验证配额容量判断覆盖充足与不足场景。
#[test]
fn test_ensure_capacity_logic() {
    let about = DriveAbout {
        user_capacity: 1000,
        used_space: 600,
        user_display_name: None,
    };
    assert!(about.can_fit(400));
    assert!(!about.can_fit(401));
}

/// 验证官方文件夹 MIME 类型映射为目录类别。
#[test]
fn test_file_category_folder_by_mime() {
    assert_eq!(
        FileCategory::from_mime_type(Some("application/vnd.huawei-apps.folder")),
        FileCategory::Folder
    );
    assert_eq!(
        FileCategory::from_mime_type(Some("application/vnd.google-apps.folder")),
        FileCategory::Folder
    );
}

/// 验证常见 MIME 前缀映射为对应文件类别。
#[test]
fn test_file_category_by_mime_prefix() {
    assert_eq!(
        FileCategory::from_mime_type(Some("image/png")),
        FileCategory::Image
    );
    assert_eq!(
        FileCategory::from_mime_type(Some("video/mp4")),
        FileCategory::Video
    );
    assert_eq!(
        FileCategory::from_mime_type(Some("audio/mpeg")),
        FileCategory::Audio
    );
    assert_eq!(
        FileCategory::from_mime_type(Some("application/pdf")),
        FileCategory::Document
    );
    assert_eq!(
        FileCategory::from_mime_type(Some("application/zip")),
        FileCategory::Archive
    );
    assert_eq!(FileCategory::from_mime_type(None), FileCategory::None);
}

/// 验证 Drive 文件解析优先使用官方 `fileName` 字段。
#[test]
#[allow(non_snake_case)]
fn test_drive_file_from_json_uses_fileName() {
    let json = json!({
        "id": "file-1",
        "fileName": "测试.txt",
        "mimeType": "text/plain",
        "size": 1024,
        "category": "drive#file",
        "sha256": "abc123",
    });
    let file = DriveFile::from_json(&json).unwrap();
    assert_eq!(file.id, "file-1");
    assert_eq!(file.name, "测试.txt");
    assert_eq!(file.category, FileCategory::Document);
    assert_eq!(file.size, 1024);
    assert!(!file.is_folder());
    assert_eq!(file.content_hash.as_deref(), Some("abc123"));
}

/// 验证内容哈希兼容字段按预期解析。
#[test]
fn test_drive_file_content_hash_aliases() {
    for key in [
        "sha256",
        "md5",
        "md5Checksum",
        "fileSha256",
        "hash",
        "contentHash",
    ] {
        let json = json!({ "id": "f", "fileName": "n", key: "hash-value" });
        let file = DriveFile::from_json(&json).unwrap();
        assert_eq!(
            file.content_hash.as_deref(),
            Some("hash-value"),
            "字段 {key} 应被识别"
        );
    }
}

/// 验证 Drive 文件 JSON 往返保留关键字段。
#[test]
fn test_drive_file_roundtrip_json() {
    let json = json!({
        "id": "f1",
        "fileName": "报告.docx",
        "mimeType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "size": 2048,
        "editedTime": "2026-06-18T10:30:00Z",
    });
    let file = DriveFile::from_json(&json).unwrap();
    let reencoded = file.to_json();
    let reparsed = DriveFile::from_json(&reencoded).unwrap();
    assert_eq!(reparsed.id, "f1");
    assert_eq!(reparsed.name, "报告.docx");
    assert_eq!(reparsed.size, 2048);
}

/// 验证非对象配额响应回退为默认值。
#[test]
fn test_drive_about_default_on_non_object() {
    let about = DriveAbout::from_json(&json!("string"));
    assert_eq!(about.user_capacity, 0);
    assert_eq!(about.used_space, 0);
}

/// 验证文件列表分页 cursor 判定。
#[test]
fn test_file_list_result_pagination() {
    let json = json!({
        "files": [
            { "id": "f1", "fileName": "a" },
            { "id": "f2", "fileName": "b" },
        ],
        "nextCursor": "next-page-token",
    });
    let result = FileListResult::from_json(&json);
    assert_eq!(result.files.len(), 2);
    assert!(result.has_next());
    assert_eq!(result.next_cursor.as_deref(), Some("next-page-token"));

    let result = FileListResult::from_json(&json!({ "files": [] }));
    assert!(!result.has_next());
}
