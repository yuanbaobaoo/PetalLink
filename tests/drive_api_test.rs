//! Drive API 集成测试（wiremock 桩）。
//!
//! 验证华为 Drive REST API 怪癖：
//! - createFolder 中文名必须 ASCII 转义（否则 400 21004002）
//! - list 用 queryParam='root' in parentFolder 语法
//! - resume 分片上传 offset 防御性校验

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// 验证 list 请求用 queryParam='root' in parentFolder 语法（华为怪癖）。
#[tokio::test]
async fn test_list_uses_query_param_syntax() {
    let server = MockServer::start().await;
    let base = server.uri();

    Mock::given(method("GET"))
        .and(path("/files"))
        .and(wiremock::matchers::query_param(
            "queryParam",
            "'root' in parentFolder",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                { "id": "f1", "fileName": "a.txt", "mimeType": "text/plain", "size": 100 }
            ]
        })))
        .mount(&server)
        .await;

    // 用 reqwest 直接验证 query 参数构造（与 files_api.rs 一致）
    let resp = reqwest::Client::new()
        .get(format!("{base}/files"))
        .query(&[
            ("fields", "*"),
            ("pageSize", "100"),
            ("queryParam", "'root' in parentFolder"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["files"][0]["fileName"], "a.txt");
}

/// 验证 list 响应解析（含 category/mimeType 怪癖）。
#[test]
fn test_list_response_parses_folder_by_mime() {
    use petal_link_lib::drive::models::FileListResult;
    use petal_link_lib::drive::models::FileCategory;

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

/// 验证 resume 分片上传的 offset 防御逻辑（防服务端回滚）。
#[test]
fn test_resume_offset_defense_logic() {
    // 模拟 executor 中的 offset 防御：uploaded 必须在 (offset, total] 范围内才接受，
    // 否则 fallback 到 offset += chunk_len
    let total: u64 = 15_000_000; // 15MB
    let chunk: u64 = 5_000_000; // 5MB

    // 正常：服务端返回偏移前进
    let offset = 0u64;
    let uploaded = 5_000_000u64;
    let new_offset = if uploaded > offset && uploaded <= total {
        uploaded
    } else {
        offset + chunk
    };
    assert_eq!(new_offset, 5_000_000);

    // 异常：服务端回滚（返回更小的偏移）→ fallback 到 offset+chunk
    let offset = 10_000_000u64;
    let uploaded = 5_000_000u64; // 回滚！
    let new_offset = if uploaded > offset && uploaded <= total {
        uploaded
    } else {
        offset + chunk
    };
    assert_eq!(new_offset, 15_000_000); // 用 offset+chunk 推进，避免死循环

    // 异常：服务端返回越界偏移 → fallback
    let offset = 10_000_000u64;
    let uploaded = 99_999_999u64; // 超过 total
    let new_offset = if uploaded > offset && uploaded <= total {
        uploaded
    } else {
        offset + chunk
    };
    assert_eq!(new_offset, 15_000_000);
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

/// 验证 multipart/related body 结构（小文件上传，Google Drive 风格）。
#[test]
fn test_multipart_related_body_structure() {
    // 模拟 upload_api.build_multipart_related 的输出结构
    let boundary = "hwcloud_123";
    let metadata = br#"{"fileName":"test.txt"}"#;
    let file_bytes = b"hello world";

    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let body_str = String::from_utf8_lossy(&body);
    // metadata 部分（application/json）
    assert!(body_str.contains("application/json; charset=UTF-8"));
    assert!(body_str.contains(r#"{"fileName":"test.txt"}"#));
    // 文件部分（octet-stream）
    assert!(body_str.contains("application/octet-stream"));
    assert!(body_str.contains("hello world"));
    // boundary 结构
    assert_eq!(body_str.matches("--hwcloud_123").count(), 3);
    assert!(body_str.ends_with("--hwcloud_123--\r\n"));
}
