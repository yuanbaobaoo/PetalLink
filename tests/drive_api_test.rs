//! Drive API 公开合同测试。
//!
//! 验证请求体、模型解析与容量判定。

use serde_json::json;

/// 验证 createFolder 请求体中文名被 ASCII 转义（华为 400 根因）。
#[tokio::test]
async fn test_create_folder_body_escaped_for_chinese() {
    use petal_link_lib::drive::ascii_json::ascii_json_encode;
    use petal_link_lib::drive::files_api::build_create_folder_body;
    use serde_json::Value;

    let body = build_create_folder_body("我的文件夹", Some("parent-1"));
    let encoded = ascii_json_encode(&body);

    // 编码后不应含原始中文
    assert!(!encoded.contains("我的文件夹"));
    // 应含转义形式（我=U+6211, 的=U+7684, 文=U+6587, 件=U+4EF6, 夹=U+5939）
    assert!(encoded.contains("\\u6211"));
    assert!(encoded.contains("\\u7684"));
    assert!(encoded.contains("\\u6587"));
    assert!(encoded.contains("\\u5939"));

    // 编码后仍是合法 JSON，反序列化应还原中文
    let reparsed: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(reparsed["fileName"], "我的文件夹");
    assert_eq!(reparsed["parentFolder"], json!(["parent-1"]));
    assert_eq!(reparsed["mimeType"], "application/vnd.huawei-apps.folder");
}

/// 验证 createFolder 根目录省略 parentFolder（华为要求）。
#[tokio::test]
async fn test_create_folder_root_omits_parent() {
    use petal_link_lib::drive::files_api::build_create_folder_body;
    let body = build_create_folder_body("root-folder", None);
    assert!(body.as_object().unwrap().get("parentFolder").is_none());
    // mimeType 必填（否则 21004001）
    assert_eq!(body["mimeType"], "application/vnd.huawei-apps.folder");
}

/// 验证显式 `root` 父目录同样从创建请求体省略。
#[test]
fn test_build_create_folder_body_root_id_omitted() {
    use petal_link_lib::drive::files_api::build_create_folder_body;

    let body = build_create_folder_body("根文件夹", Some("root"));
    assert!(body.as_object().unwrap().get("parentFolder").is_none());
}

/// 验证查询参数编码遵守 RFC 3986 非保留字符规则。
#[test]
fn test_urlencoding() {
    use petal_link_lib::drive::files_api::urlencoding;

    // 单引号与空格应被编码（华为 queryParam 语法 'root' in parentFolder）
    let encoded = urlencoding("'root' in parentFolder");
    assert!(!encoded.contains(' '));
    assert!(!encoded.contains('\''));
}

/// 验证 list 响应解析（含 category/mimeType 怪癖）。
#[test]
fn test_list_response_parses_folder_by_mime() {
    use petal_link_lib::drive::models::FileCategory;
    use petal_link_lib::drive::models::FileListResult;

    let json = json!({
        "files": [
            {
                "id": "folder-1",
                "fileName": "文档",
                "mimeType": "application/vnd.huawei-apps.folder",
                "category": "drive#file"
            },
            {
                "id": "file-1",
                "fileName": "报告.pdf",
                "mimeType": "application/pdf",
                "category": "drive#file",
                "size": 2048
            }
        ]
    });
    let result = FileListResult::from_json(&json);
    assert_eq!(result.files.len(), 2);
    // 文件夹检测靠 mimeType（category 恒为 drive#file，无类型信息）
    assert!(result.files[0].is_folder());
    assert_eq!(result.files[0].category, FileCategory::Folder);
    assert!(!result.files[1].is_folder());
    assert_eq!(result.files[1].category, FileCategory::Document);
}

/// 验证 about 配额字段容忍 String 类型（华为怪癖）。
#[test]
fn test_about_quota_string_tolerance() {
    use petal_link_lib::drive::models::DriveAbout;
    use serde_json::json;

    // 华为返回配额为 String 类型
    let json = json!({
        "storageQuota": {
            "userCapacity": "107374182400",
            "usedSpace": "5368709120"
        },
        "user": { "displayName": "测试用户" }
    });
    let about = DriveAbout::from_json(&json);
    assert_eq!(about.user_capacity, 107374182400);
    assert_eq!(about.used_space, 5368709120);
    assert_eq!(about.remaining_space(), 107374182400 - 5368709120);
    assert!(about.can_fit(100_000_000));
    assert_eq!(about.user_display_name.as_deref(), Some("测试用户"));
}
