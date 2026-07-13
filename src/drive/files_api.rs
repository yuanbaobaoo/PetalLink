//! Files API —— 列举 / 创建 / 更新 / 删除 / 搜索。
//!
//! 对齐 `legacy/lib/drive/api/files_api.dart`。
//!
//! # 华为 API 怪癖
//! - **parentFolder 查询语法**：不用 parentFolder 参数，而用 `queryParam='root' in parentFolder`
//!   （单引号包裹 token）。列出根目录用 `'root'`，列出子目录用 `'<id>'`。
//! - **asciiJsonEncode**：createFolder / update 的 application/json 请求体必须用 ASCII-only 编码，
//!   否则中文名报 400 `21004002 fileName can not be blank`。
//! - **createFolder**：mimeType 必填，root 目录省略 parentFolder。

use std::sync::Arc;

use serde_json::Value;

use crate::drive::ascii_json::ascii_json_encode;
use crate::drive::client::{
    parse_json_response, parse_json_response_with_semantics, response_decode_error,
    response_metadata,
};
use crate::drive::models::{DriveFile, FileListResult};
use crate::error::{AppError, AppResult, RequestSemantics};

/// 华为文件夹 mimeType
const FOLDER_MIME_TYPE: &str = "application/vnd.huawei-apps.folder";

pub struct FilesApi {
    client: Arc<crate::drive::client::DriveClient>,
}

impl FilesApi {
    pub fn new(client: Arc<crate::drive::client::DriveClient>) -> Self {
        Self { client }
    }

    /// 列举目录内容（单页）。
    /// 对齐 dart `FilesApi.list({parentId?, cursor?, pageSize=100})`。
    ///
    /// 关键：用 `queryParam='root' in parentFolder` 语法（华为不接受 parentFolder 参数）。
    pub async fn list(
        &self,
        parent_id: Option<&str>,
        cursor: Option<&str>,
        page_size: u32,
    ) -> AppResult<FileListResult> {
        let folder_token = match parent_id {
            Some(id) if !id.is_empty() => id,
            _ => "root",
        };
        let query_param = format!("'{folder_token}' in parentFolder");
        let mut url = format!(
            "{}/files?fields=*&pageSize={}&queryParam={}",
            crate::constants::DRIVE_API_BASE,
            page_size,
            urlencoding(&query_param)
        );
        if let Some(c) = cursor {
            if !c.is_empty() {
                url.push_str(&format!("&cursor={}", urlencoding(c)));
            }
        }

        let resp = self.send_get(&url).await?;
        let body: Value = parse_json_response(resp, "list").await?;
        Ok(FileListResult::from_json(&body))
    }

    /// 列举目录全部内容（自动翻页）。
    /// 对齐 dart `FilesApi.listAll({parentId?})`：pageSize=500，最多 100 页（~50K 项）。
    pub async fn list_all(&self, parent_id: Option<&str>) -> AppResult<Vec<DriveFile>> {
        const MAX_PAGES: usize = 100;
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let result = self.list(parent_id, cursor.as_deref(), 500).await?;
            let has_next = result.has_next();
            all.extend(result.files);
            cursor = result.next_cursor;
            if !has_next {
                return Ok(all);
            }
        }
        tracing::warn!("listAll 超过 {MAX_PAGES} 页，截断");
        Ok(all)
    }

    /// 获取单个文件元数据。对齐 dart `FilesApi.get(id)`。
    pub async fn get(&self, id: &str) -> AppResult<DriveFile> {
        let url = format!("{}/files/{id}?fields=*", crate::constants::DRIVE_API_BASE);
        let resp = self.send_get(&url).await?;
        let body: Value = parse_json_response(resp, "get").await?;
        DriveFile::from_json(&body).ok_or_else(|| AppError::generic("文件元数据格式异常"))
    }

    /// 创建文件夹。对齐 dart `FilesApi.createFolder({name, parentId?})`。
    ///
    /// 关键：body 用 [`ascii_json_encode`] 编码（中文名必须 ASCII 转义）。
    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> AppResult<DriveFile> {
        let body = build_create_folder_body(name, parent_id);
        let encoded = ascii_json_encode(&body);
        let url = format!("{}/files?fields=*", crate::constants::DRIVE_API_BASE);
        let resp = self
            .send_post(&url, encoded.into_bytes(), "application/json")
            .await?;
        let metadata = response_metadata(&resp, RequestSemantics::Write);
        let body_json: Value =
            parse_json_response_with_semantics(resp, "createFolder", RequestSemantics::Write)
                .await?;
        parse_written_drive_file(&body_json, "createFolder", metadata.auth_already_replayed)
    }

    /// 删除文件（软删除，移入回收站"最近删除"）。
    ///
    /// **重要**：华为 Drive API 的 `DELETE /drive/v1/files/{id}` 是**永久删除**，不进回收站。
    /// 要实现软删除（进"最近删除"），必须用 PATCH 更新 `recycled: true`。
    /// 对齐华为官方文档 Files:update → recycled 字段。
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let path = delete_path(id);
        let mut body = serde_json::Map::new();
        body.insert("recycled".into(), Value::Bool(true));
        let encoded = ascii_json_encode(&Value::Object(body));
        let resp = self
            .client
            .patch(&path, encoded.into_bytes(), "application/json")
            .await?;
        // 消费 body
        let _ = resp.text().await;
        Ok(())
    }

    /// 更新文件（重命名/移动/改描述）。
    /// 对齐 dart `FilesApi.update(id, {newName?, newParentFolder?, description?})`。
    ///
    /// 关键：body 用 [`ascii_json_encode`] 编码。
    pub async fn update(
        &self,
        id: &str,
        new_name: Option<&str>,
        new_parent_folder: Option<&str>,
        description: Option<&str>,
    ) -> AppResult<DriveFile> {
        let mut body = serde_json::Map::new();
        if let Some(name) = new_name {
            body.insert("fileName".into(), Value::String(name.to_string()));
        }
        if let Some(parent) = new_parent_folder {
            body.insert(
                "parentFolder".into(),
                Value::Array(vec![Value::String(parent.to_string())]),
            );
        }
        if let Some(desc) = description {
            body.insert("description".into(), Value::String(desc.to_string()));
        }
        let encoded = ascii_json_encode(&Value::Object(body));
        let url = format!("{}/files/{id}", crate::constants::DRIVE_API_BASE);
        let resp = self
            .send_patch(&url, encoded.into_bytes(), "application/json")
            .await?;
        let metadata = response_metadata(&resp, RequestSemantics::Write);
        let body_json: Value =
            parse_json_response_with_semantics(resp, "update", RequestSemantics::Write).await?;
        parse_written_drive_file(&body_json, "update", metadata.auth_already_replayed)
    }

    /// 搜索文件。对齐 dart `FilesApi.search(keyword, {parentId?, ...})`。
    ///
    /// 关键：用 `queryParam=fileName:contains:"keyword"`，叠加 parentFolder 语法。
    pub async fn search(
        &self,
        keyword: &str,
        parent_id: Option<&str>,
        page_size: u32,
    ) -> AppResult<FileListResult> {
        let mut query = format!("fileName:contains:\"{keyword}\"");
        if let Some(pid) = parent_id {
            if !pid.is_empty() {
                query = format!("{query} and '{pid}' in parentFolder");
            }
        }
        let url = format!(
            "{}/files?fields=*&pageSize={}&queryParam={}",
            crate::constants::DRIVE_API_BASE,
            page_size,
            urlencoding(&query)
        );
        let resp = self.send_get(&url).await?;
        let body: Value = parse_json_response(resp, "search").await?;
        Ok(FileListResult::from_json(&body))
    }

    // ===== 内部：委托 DriveClient 统一的 auth + 401 重放逻辑 =====

    /// 发送 GET（完整 URL，注入 token + 401 重放）
    async fn send_get(&self, url: &str) -> AppResult<reqwest::Response> {
        self.client.get_full(url).await
    }

    /// 发送 POST（带 body）
    async fn send_post(
        &self,
        url: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.post_full(url, Some(body), content_type).await
    }

    /// 发送 PATCH
    async fn send_patch(
        &self,
        url: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.patch_full(url, body, content_type).await
    }
}

fn delete_path(id: &str) -> String {
    let encoded_id = percent_encoding::utf8_percent_encode(id, &URL_PATH_SEGMENT_ENCODE_SET);
    format!("/files/{encoded_id}")
}

/// URL 编码（query 参数用），对齐 dart `Uri.encodeQueryComponent`。
/// 仅不编码 RFC 3986 unreserved 字符：A-Za-z0-9-_.~
///
/// `pub` 以便 `changes_api` 等同模块复用（cursor 同为 query 参数）。
pub fn urlencoding(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, &URL_QUERY_ENCODE_SET).to_string()
}

/// 模块级编码集（避免临时值生命周期问题）。
static URL_QUERY_ENCODE_SET: once_cell::sync::Lazy<percent_encoding::AsciiSet> =
    once_cell::sync::Lazy::new(|| {
        percent_encoding::CONTROLS
            .add(b' ')
            .add(b'!')
            .add(b'"')
            .add(b'#')
            .add(b'$')
            .add(b'%')
            .add(b'&')
            .add(b'\'')
            .add(b'(')
            .add(b')')
            .add(b'*')
            .add(b'+')
            .add(b',')
            .add(b'/')
            .add(b':')
            .add(b';')
            .add(b'<')
            .add(b'=')
            .add(b'>')
            .add(b'?')
            .add(b'@')
            .add(b'[')
            .add(b'\\')
            .add(b']')
            .add(b'^')
            .add(b'`')
            .add(b'{')
            .add(b'|')
            .add(b'}')
    });

/// URL path segment 编码集；与 query 参数分别命名，避免未来两种语义误混。
static URL_PATH_SEGMENT_ENCODE_SET: once_cell::sync::Lazy<percent_encoding::AsciiSet> =
    once_cell::sync::Lazy::new(|| {
        percent_encoding::CONTROLS
            .add(b' ')
            .add(b'!')
            .add(b'"')
            .add(b'#')
            .add(b'$')
            .add(b'%')
            .add(b'&')
            .add(b'\'')
            .add(b'(')
            .add(b')')
            .add(b'*')
            .add(b'+')
            .add(b',')
            .add(b'/')
            .add(b':')
            .add(b';')
            .add(b'<')
            .add(b'=')
            .add(b'>')
            .add(b'?')
            .add(b'@')
            .add(b'[')
            .add(b'\\')
            .add(b']')
            .add(b'^')
            .add(b'`')
            .add(b'{')
            .add(b'|')
            .add(b'}')
    });

/// 构造 createFolder 请求体。
/// 对齐 dart `buildCreateFolderBody`：mimeType 必填，root 目录省略 parentFolder。
pub fn build_create_folder_body(name: &str, parent_id: Option<&str>) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("fileName".into(), Value::String(name.to_string()));
    body.insert(
        "mimeType".into(),
        Value::String(FOLDER_MIME_TYPE.to_string()),
    );
    if let Some(pid) = parent_id {
        if !pid.is_empty() && pid != "root" {
            body.insert(
                "parentFolder".into(),
                Value::Array(vec![Value::String(pid.to_string())]),
            );
        }
    }
    Value::Object(body)
}

fn parse_written_drive_file(
    body: &Value,
    ctx: &str,
    auth_already_replayed: bool,
) -> AppResult<DriveFile> {
    DriveFile::from_json(body).ok_or_else(|| {
        response_decode_error(
            ctx,
            RequestSemantics::Write,
            auth_already_replayed,
            "响应缺少文件必填字段",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::models::{now_ms, TokenPair};
    use crate::auth::service::AuthService;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn authenticated_files_api(base_url: String) -> FilesApi {
        let auth = Arc::new(AuthService::new());
        auth.refresher().set_current(TokenPair {
            access_token: "access-token".into(),
            refresh_token: "refresh-token".into(),
            expires_at: now_ms() + 3_600_000,
            token_type: "Bearer".into(),
            scope: None,
        });
        let client = Arc::new(crate::drive::client::DriveClient::with_base_url(
            auth, base_url,
        ));
        FilesApi::new(client)
    }

    #[test]
    fn test_build_create_folder_body_root() {
        let body = build_create_folder_body("我的文件夹", None);
        let obj = body.as_object().unwrap();
        assert_eq!(obj["fileName"], "我的文件夹");
        assert_eq!(obj["mimeType"], "application/vnd.huawei-apps.folder");
        assert!(obj.get("parentFolder").is_none());
    }

    #[test]
    fn test_build_create_folder_body_subfolder() {
        let body = build_create_folder_body("子文件夹", Some("parent-id-123"));
        let obj = body.as_object().unwrap();
        assert_eq!(obj["parentFolder"], json!(["parent-id-123"]));
    }

    #[test]
    fn test_build_create_folder_body_root_id_omitted() {
        let body = build_create_folder_body("根文件夹", Some("root"));
        assert!(body.as_object().unwrap().get("parentFolder").is_none());
    }

    #[test]
    fn test_build_create_folder_body_mimetype_mandatory() {
        let body = build_create_folder_body("f", None);
        assert_eq!(body["mimeType"], "application/vnd.huawei-apps.folder");
    }

    #[test]
    fn test_urlencoding() {
        // 单引号与空格应被编码（华为 queryParam 语法 'root' in parentFolder）
        let encoded = urlencoding("'root' in parentFolder");
        assert!(!encoded.contains(' '));
        assert!(!encoded.contains('\''));
    }

    #[test]
    fn invalid_written_drive_file_response_is_post_submit_decode_error() {
        let error = parse_written_drive_file(&json!({"fileName": "created"}), "createFolder", true)
            .unwrap_err();

        assert!(matches!(
            error,
            AppError::DriveApi {
                transport_kind: Some(crate::error::DriveTransportKind::Decode),
                request_may_have_reached_server: true,
                auth_already_replayed: true,
                ..
            }
        ));
    }

    #[test]
    fn delete_uses_injectable_relative_drive_path() {
        assert_eq!(
            delete_path("file id/with spaces"),
            "/files/file%20id%2Fwith%20spaces"
        );
    }

    #[tokio::test]
    async fn irreversible_delete_never_reports_final_500_as_success() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/files/victim"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "errorCode": "delete-failed"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let api = authenticated_files_api(server.uri());

        let error = api.delete("victim").await.unwrap_err();

        assert!(matches!(
            error,
            AppError::DriveApi {
                status_code: Some(500),
                error_code: Some(ref code),
                request_may_have_reached_server: true,
                ..
            } if code == "delete-failed"
        ));
    }
}
