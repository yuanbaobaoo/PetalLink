//! Drive 数据模型 —— DriveFile / FileCategory / DriveAbout / FileListResult。
//!
//! 对齐 `legacy/lib/drive/models/`（drive_file.dart + about.dart + file_list_result.dart）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 文件分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory {
    Folder,
    Audio,
    Video,
    Image,
    Document,
    Package,
    Archive,
    Executable,
    None,
}

impl FileCategory {
    /// 基于 mimeType 判断文件分类。
    ///
    /// 华为的 category 字段对所有资源都返回 "drive#file"（无类型信息），
    /// 真正的类型在 mimeType。文件夹：`application/vnd.huawei-apps.folder`。
    /// 对齐 dart `FileCategory.fromMimeType`。
    pub fn from_mime_type(mime_type: Option<&str>) -> Self {
        let m = match mime_type {
            Some(s) => s.to_lowercase(),
            None => return Self::None,
        };
        // 文件夹（华为/Google Drive 兼容）
        if matches!(
            m.as_str(),
            "application/vnd.huawei-apps.folder"
                | "application/vnd.huawei-app.folder"
                | "application/vnd.google-apps.folder"
                | "application/x-folder"
        ) {
            return Self::Folder;
        }
        if m.starts_with("image/") {
            return Self::Image;
        }
        if m.starts_with("video/") {
            return Self::Video;
        }
        if m.starts_with("audio/") {
            return Self::Audio;
        }
        // 文档类
        if m.starts_with("text/")
            || m.contains("pdf")
            || m.contains("word")
            || m.contains("msword")
            || m.contains("officedocument.wordprocessing")
            || m.contains("spreadsheet")
            || m.contains("excel")
            || m.contains("presentation")
            || m.contains("powerpoint")
        {
            return Self::Document;
        }
        // 压缩包
        if m.contains("zip")
            || m.contains("rar")
            || m.contains("7z")
            || m.contains("tar")
            || m.contains("gzip")
            || m.contains("x-tar")
        {
            return Self::Archive;
        }
        // 安装包
        if m.contains("apk")
            || m.contains("dmg")
            || m.contains("pkg")
            || m.contains("debian")
            || m.contains("rpm")
        {
            return Self::Package;
        }
        // 可执行
        if m.contains("executable") || m.contains("x-msdownload") || m.ends_with("x-mach-binary") {
            return Self::Executable;
        }
        Self::None
    }
}

/// Drive 文件 DTO（对应华为云盘 File 资源）。
/// 对齐 dart `DriveFile`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub category: FileCategory,
    pub size: i64,
    pub parent_folder: Option<Vec<String>>,
    pub description: Option<String>,
    pub created_time: Option<DateTime<Utc>>,
    pub edited_time: Option<DateTime<Utc>>,
    pub mime_type: Option<String>,
    /// 云端内容 hash（md5/sha256，字段名兼容多种）。
    /// 若华为返回则为内容指纹，用于精确变更检测；为 null 时降级用 editedTime。
    pub content_hash: Option<String>,
    pub thumbnail_link: Option<String>,
}

impl DriveFile {
    /// 是否文件夹
    pub fn is_folder(&self) -> bool {
        self.category == FileCategory::Folder
    }

    /// 从华为 JSON 响应构造。
    /// 对齐 dart `DriveFile.fromJson`（含 category/mimeType 怪癖 + contentHash 字段别名）。
    pub fn from_json(json: &Value) -> Option<Self> {
        let id = json.get("id").and_then(Value::as_str)?.to_string();
        // 华为用 fileName，标准用 name
        let name = json
            .get("fileName")
            .and_then(Value::as_str)
            .or_else(|| json.get("name").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let mime_type = json
            .get("mimeType")
            .and_then(Value::as_str)
            .map(String::from);
        let category = FileCategory::from_mime_type(mime_type.as_deref());
        let size = json
            .get("size")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(0);
        let parent_folder = json
            .get("parentFolder")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });
        let description = json
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);
        let created_time = parse_time(json.get("createdTime"));
        let edited_time = parse_time(json.get("editedTime"));
        // 内容 hash：兼容华为多种字段名
        let content_hash = [
            "sha256",
            "md5",
            "md5Checksum",
            "fileSha256",
            "hash",
            "contentHash",
        ]
        .iter()
        .find_map(|k| json.get(*k).and_then(Value::as_str).map(String::from));
        let thumbnail_link = json
            .get("thumbnailLink")
            .and_then(Value::as_str)
            .map(String::from);

        Some(Self {
            id,
            name,
            category,
            size,
            parent_folder,
            description,
            created_time,
            edited_time,
            mime_type,
            content_hash,
            thumbnail_link,
        })
    }

    /// 序列化为华为 JSON（用于云端树缓存持久化）。
    /// 对齐 dart `DriveFile.toJson`。
    pub fn to_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("id".into(), Value::String(self.id.clone()));
        map.insert("fileName".into(), Value::String(self.name.clone()));
        if self.size > 0 {
            map.insert("size".into(), Value::Number(self.size.into()));
        }
        if let Some(pf) = &self.parent_folder {
            map.insert(
                "parentFolder".into(),
                Value::Array(pf.iter().map(|s| Value::String(s.clone())).collect()),
            );
        }
        if let Some(d) = &self.description {
            map.insert("description".into(), Value::String(d.clone()));
        }
        if let Some(t) = self.created_time {
            map.insert("createdTime".into(), Value::String(t.to_rfc3339()));
        }
        if let Some(t) = self.edited_time {
            map.insert("editedTime".into(), Value::String(t.to_rfc3339()));
        }
        if let Some(m) = &self.mime_type {
            map.insert("mimeType".into(), Value::String(m.clone()));
        }
        if let Some(h) = &self.content_hash {
            map.insert("sha256".into(), Value::String(h.clone()));
        }
        Value::Object(map)
    }
}

/// 解析 ISO8601 时间字符串。对齐 dart `parseTime`。
fn parse_time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    let s = v.and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Drive 配额信息。对齐 dart `DriveAbout`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveAbout {
    pub user_capacity: i64,
    pub used_space: i64,
    pub user_display_name: Option<String>,
}

impl DriveAbout {
    /// 剩余空间
    pub fn remaining_space(&self) -> i64 {
        self.user_capacity - self.used_space
    }

    /// 是否能容纳 n 字节
    pub fn can_fit(&self, n: i64) -> bool {
        self.remaining_space() >= n
    }

    /// 从华为 JSON 构造。
    /// 配额字段在 `storageQuota` 子对象下，且华为返回为 String（容忍解析）。
    /// 对齐 dart `DriveAbout.fromJson`（含 storageQuota 嵌套 + String 容忍）。
    pub fn from_json(json: &Value) -> Self {
        let default = Self {
            user_capacity: 0,
            used_space: 0,
            user_display_name: None,
        };
        if !json.is_object() {
            return default;
        }
        // 配额优先取 storageQuota 子对象，回退顶层
        let quota = json.get("storageQuota").unwrap_or(json);
        let user_capacity = tolerant_parse_int(quota.get("userCapacity")).unwrap_or(0);
        let used_space = tolerant_parse_int(quota.get("usedSpace")).unwrap_or(0);
        // 用户名在 user.displayName 嵌套
        let user_display_name = json
            .get("user")
            .and_then(|u| u.get("displayName"))
            .and_then(Value::as_str)
            .map(String::from);
        Self {
            user_capacity,
            used_space,
            user_display_name,
        }
    }
}

/// 容忍解析 int：接受 int/num/String（华为配额字段可能返回 String）。
/// 对齐 dart `_tolerantParseInt`。
fn tolerant_parse_int(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 文件列表结果。对齐 dart `FileListResult`。
#[derive(Debug, Clone, Serialize)]
pub struct FileListResult {
    pub files: Vec<DriveFile>,
    pub next_cursor: Option<String>,
}

impl FileListResult {
    /// 是否还有下一页
    pub fn has_next(&self) -> bool {
        self.next_cursor
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// 从华为 list 响应构造。
    pub fn from_json(json: &Value) -> Self {
        let files = json
            .get("files")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(DriveFile::from_json).collect())
            .unwrap_or_default();
        let next_cursor = json
            .get("nextCursor")
            .or_else(|| json.get("cursor"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from);
        Self { files, next_cursor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    #[allow(non_snake_case)]
    fn test_drive_file_from_json_uses_fileName() {
        // 华为用 fileName（非标准 name）
        let json = json!({
            "id": "file-1",
            "fileName": "测试.txt",
            "mimeType": "text/plain",
            "size": 1024,
            "category": "drive#file",  // 华为恒为此值，无类型信息
            "sha256": "abc123",
        });
        let f = DriveFile::from_json(&json).unwrap();
        assert_eq!(f.id, "file-1");
        assert_eq!(f.name, "测试.txt");
        assert_eq!(f.category, FileCategory::Document);
        assert_eq!(f.size, 1024);
        assert!(!f.is_folder());
        assert_eq!(f.content_hash.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_drive_file_folder_detection() {
        let json = json!({
            "id": "folder-1",
            "fileName": "文档",
            "mimeType": "application/vnd.huawei-apps.folder",
            "category": "drive#file",
        });
        let f = DriveFile::from_json(&json).unwrap();
        assert!(f.is_folder());
        assert_eq!(f.category, FileCategory::Folder);
    }

    #[test]
    fn test_drive_file_content_hash_aliases() {
        // contentHash 字段别名兼容
        for key in [
            "sha256",
            "md5",
            "md5Checksum",
            "fileSha256",
            "hash",
            "contentHash",
        ] {
            let json = json!({ "id": "f", "fileName": "n", key: "hash-value" });
            let f = DriveFile::from_json(&json).unwrap();
            assert_eq!(
                f.content_hash.as_deref(),
                Some("hash-value"),
                "字段 {key} 应被识别"
            );
        }
    }

    #[test]
    fn test_drive_file_roundtrip_json() {
        let json = json!({
            "id": "f1",
            "fileName": "报告.docx",
            "mimeType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "size": 2048,
            "editedTime": "2026-06-18T10:30:00Z",
        });
        let f = DriveFile::from_json(&json).unwrap();
        let reencoded = f.to_json();
        // 反序列化应还原关键字段
        let f2 = DriveFile::from_json(&reencoded).unwrap();
        assert_eq!(f2.id, "f1");
        assert_eq!(f2.name, "报告.docx");
        assert_eq!(f2.size, 2048);
    }

    #[test]
    fn test_drive_about_quota_nested_string() {
        // 配额在 storageQuota 子对象，且为 String 类型（华为怪癖）
        let json = json!({
            "storageQuota": {
                "userCapacity": "53687091200",
                "usedSpace": "1073741824"
            },
            "user": { "displayName": "张三" }
        });
        let about = DriveAbout::from_json(&json);
        assert_eq!(about.user_capacity, 53687091200);
        assert_eq!(about.used_space, 1073741824);
        assert_eq!(about.remaining_space(), 53687091200 - 1073741824);
        assert!(about.can_fit(1000));
        assert_eq!(about.user_display_name.as_deref(), Some("张三"));
    }

    #[test]
    fn test_drive_about_default_on_non_object() {
        let about = DriveAbout::from_json(&json!("string"));
        assert_eq!(about.user_capacity, 0);
        assert_eq!(about.used_space, 0);
    }

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

        // 无 cursor
        let json2 = json!({ "files": [] });
        let result2 = FileListResult::from_json(&json2);
        assert!(!result2.has_next());
    }
}
