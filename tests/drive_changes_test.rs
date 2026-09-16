//! Drive 增量变更协议解析测试。

use petal_link_lib::drive::changes_api::{ChangeKind, ChangesPage};
use serde_json::json;

/// 验证完整非删除变更解析为修改事件。
#[test]
fn test_parse_modified_change() {
    let json = json!({
        "category": "drive#changeList",
        "changes": [{
            "category": "drive#change",
            "changeType": "update",
            "deleted": false,
            "file": {
                "id": "f1",
                "fileName": "a.txt",
                "mimeType": "text/plain",
                "size": 100,
                "parentFolder": ["root-folder-id"]
            },
            "fileId": "f1",
            "type": "File"
        }],
        "newStartCursor": "311298"
    });
    let result = ChangesPage::from_json(&json).expect("strict modified change");
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].kind, ChangeKind::Modified);
    assert_eq!(
        result.changes[0].file().map(|file| file.name.as_str()),
        Some("a.txt")
    );
    assert_eq!(result.next_cursor, None);
    assert_eq!(result.new_start_cursor.as_deref(), Some("311298"));
}

/// 验证删除 tombstone 解析为移除事件。
#[test]
fn test_parse_removed_change() {
    let json = json!({
        "category": "drive#changeList",
        "changes": [{
            "category": "drive#change",
            "changeType": "trashDone",
            "deleted": false,
            "file": { "id": "f9", "fileName": "del.txt", "mimeType": "text/plain", "size": 10, "recycled": true },
            "fileId": "f9",
            "type": "File"
        }],
        "newStartCursor": "311299"
    });
    let result = ChangesPage::from_json(&json).expect("strict soft-delete change");
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].kind, ChangeKind::Removed);
    assert_eq!(result.changes[0].file_id(), "f9");
    assert_eq!(
        result.changes[0].file().map(|file| file.name.as_str()),
        Some("del.txt")
    );
    assert_eq!(result.new_start_cursor.as_deref(), Some("311299"));
}

/// 验证空终页保留新的起始游标。
#[test]
fn test_parse_empty_terminal_page() {
    let json = json!({
        "category": "drive#changeList",
        "changes": [],
        "newStartCursor": "311296"
    });
    let result = ChangesPage::from_json(&json).expect("strict empty terminal page");
    assert!(result.changes.is_empty());
    assert_eq!(result.next_cursor, None);
    assert_eq!(result.new_start_cursor.as_deref(), Some("311296"));
}

/// 验证 change 事件容忍字符串形式的 size（2026-09 华为响应 schema 漂移）。
#[test]
fn test_parse_change_with_string_size() {
    let json = json!({
        "category": "drive#changeList",
        "changes": [{
            "category": "drive#change",
            "changeType": "update",
            "deleted": false,
            "file": { "id": "fs1", "fileName": "漂移.txt", "mimeType": "text/plain", "size": "2048", "parentFolder": ["root-folder-id"] },
            "fileId": "fs1",
            "type": "File"
        }],
        "newStartCursor": "311300"
    });
    let result = ChangesPage::from_json(&json).expect("字符串 size 的 change 应解析成功");
    assert_eq!(result.changes.len(), 1);
    assert_eq!(result.changes[0].file().map(|file| file.size), Some(2048));
}

/// 验证非数字字符串 size 仍被拒绝，不能静默吞掉坏数据。
#[test]
fn test_parse_change_rejects_invalid_string_size() {
    let json = json!({
        "category": "drive#changeList",
        "changes": [{
            "category": "drive#change",
            "changeType": "update",
            "deleted": false,
            "file": { "id": "fs2", "fileName": "坏.txt", "mimeType": "text/plain", "size": "abc" },
            "fileId": "fs2",
            "type": "File"
        }],
        "newStartCursor": "311301"
    });
    assert!(ChangesPage::from_json(&json).is_err());
}
