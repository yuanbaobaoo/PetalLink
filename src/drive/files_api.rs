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

use std::collections::HashSet;
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
const PRODUCTION_PAGE_SIZE: u32 = 100;
const PRODUCTION_MAX_PAGES: usize = 1_000;

/// Files:list 的客户端分页上限。
///
/// 华为只定义单页大小上限，没有定义目录总页数。客户端仍需要有限上限来避免服务端
/// cursor 循环或异常数据导致永久索引；达到上限且仍有下一页时必须失败，不能返回部分树。
#[derive(Debug, Clone, Copy)]
pub struct PaginationPolicy {
    max_pages: usize,
}

impl PaginationPolicy {
    pub fn new(max_pages: usize) -> AppResult<Self> {
        if max_pages == 0 {
            return Err(AppError::generic("Files 分页上限必须大于 0"));
        }
        Ok(Self { max_pages })
    }

    const fn production() -> Self {
        Self {
            max_pages: PRODUCTION_MAX_PAGES,
        }
    }
}

pub struct FilesApi {
    client: Arc<crate::drive::client::DriveClient>,
    pagination: PaginationPolicy,
}

impl FilesApi {
    pub fn new(client: Arc<crate::drive::client::DriveClient>) -> Self {
        Self {
            client,
            pagination: PaginationPolicy::production(),
        }
    }

    /// 使用可控的分页上限构造真实 Files API wrapper。
    ///
    /// 该 seam 仍走 [`DriveClient`] 的生产请求链，只替换防无限分页的客户端上限。
    pub fn with_pagination_policy(
        client: Arc<crate::drive::client::DriveClient>,
        pagination: PaginationPolicy,
    ) -> Self {
        Self { client, pagination }
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
        validate_page_size(page_size)?;
        let folder_token = match parent_id {
            Some(id) if !id.is_empty() => id,
            _ => "root",
        };
        validate_query_literal(folder_token, "parentFolder")?;
        let query_param = format!("'{folder_token}' in parentFolder");
        let mut path = format!(
            "/files?fields=*&pageSize={page_size}&queryParam={}",
            urlencoding(&query_param)
        );
        if let Some(c) = cursor {
            if !c.is_empty() {
                path.push_str(&format!("&cursor={}", urlencoding(c)));
            }
        }

        let resp = self.send_get(&path).await?;
        let auth_already_replayed =
            response_metadata(&resp, RequestSemantics::Read).auth_already_replayed;
        let body: Value = parse_json_response(resp, "list").await?;
        parse_file_list_page(&body, "list", auth_already_replayed)
    }

    /// 列举目录全部内容（自动翻页）。
    /// 固定使用华为官方上限 pageSize=100；空的非终止页仍按 nextCursor 继续。
    pub async fn list_all(&self, parent_id: Option<&str>) -> AppResult<Vec<DriveFile>> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for page_index in 0..self.pagination.max_pages {
            let result = self
                .list(parent_id, cursor.as_deref(), PRODUCTION_PAGE_SIZE)
                .await?;
            all.extend(result.files);

            match result.next_cursor {
                None => return Ok(all),
                Some(next_cursor) => {
                    if !seen_cursors.insert(next_cursor.clone()) {
                        return Err(files_protocol_error(
                            "listAll",
                            "nextCursor 重复或形成循环",
                            false,
                        ));
                    }
                    if page_index + 1 >= self.pagination.max_pages {
                        return Err(files_protocol_error(
                            "listAll",
                            "达到分页上限时服务端仍返回 nextCursor，结果不完整",
                            false,
                        ));
                    }
                    cursor = Some(next_cursor);
                }
            }
        }

        Err(files_protocol_error("listAll", "分页策略无可用页数", false))
    }

    /// 获取单个文件元数据。对齐 dart `FilesApi.get(id)`。
    pub async fn get(&self, id: &str) -> AppResult<DriveFile> {
        let path = format!("{}?fields=*", file_path(id));
        let resp = self.send_get(&path).await?;
        let auth_already_replayed =
            response_metadata(&resp, RequestSemantics::Read).auth_already_replayed;
        let body: Value = parse_json_response(resp, "get").await?;
        parse_drive_file_strict(&body, "get", auth_already_replayed, None)
    }

    /// 创建文件夹。对齐 dart `FilesApi.createFolder({name, parentId?})`。
    ///
    /// 关键：body 用 [`ascii_json_encode`] 编码（中文名必须 ASCII 转义）。
    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> AppResult<DriveFile> {
        let body = build_create_folder_body(name, parent_id);
        let encoded = ascii_json_encode(&body);
        let path = "/files?fields=*";
        let resp = self
            .send_post(path, encoded.into_bytes(), "application/json")
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
        let path = format!("{}?fields=*", file_path(id));
        let resp = self
            .send_patch(&path, encoded.into_bytes(), "application/json")
            .await?;
        let metadata = response_metadata(&resp, RequestSemantics::Write);
        let body_json: Value =
            parse_json_response_with_semantics(resp, "update", RequestSemantics::Write).await?;
        parse_written_drive_file(&body_json, "update", metadata.auth_already_replayed)
    }

    /// 搜索文件。对齐 dart `FilesApi.search(keyword, {parentId?, ...})`。
    ///
    /// 关键：用官方 `fileName contains 'keyword'` 单引号 DSL，整段只编码一次。
    /// 官方未定义单引号和反斜线的转义规则，因此这些输入在发请求前 fail closed。
    pub async fn search(
        &self,
        keyword: &str,
        parent_id: Option<&str>,
        page_size: u32,
    ) -> AppResult<FileListResult> {
        validate_page_size(page_size)?;
        validate_query_literal(keyword, "搜索关键词")?;
        let mut query = format!("fileName contains '{keyword}'");
        if let Some(pid) = parent_id {
            if !pid.is_empty() {
                validate_query_literal(pid, "parentFolder")?;
                query = format!("{query} and '{pid}' in parentFolder");
            }
        }
        let path = format!(
            "/files?fields=*&pageSize={page_size}&queryParam={}",
            urlencoding(&query)
        );
        let resp = self.send_get(&path).await?;
        let auth_already_replayed =
            response_metadata(&resp, RequestSemantics::Read).auth_already_replayed;
        let body: Value = parse_json_response(resp, "search").await?;
        parse_file_list_page(&body, "search", auth_already_replayed)
    }

    // ===== 内部：委托 DriveClient 统一的 auth + 401 重放逻辑 =====

    /// 发送 GET（相对 Drive API 路径，保留可注入 base URL + 401 重放）。
    async fn send_get(&self, path: &str) -> AppResult<reqwest::Response> {
        self.client.get(path).await
    }

    /// 发送 POST（带 body）
    async fn send_post(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.post(path, Some(body), content_type).await
    }

    /// 发送 PATCH
    async fn send_patch(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        self.client.patch(path, body, content_type).await
    }
}

fn validate_page_size(page_size: u32) -> AppResult<()> {
    if (1..=PRODUCTION_PAGE_SIZE).contains(&page_size) {
        Ok(())
    } else {
        Err(AppError::generic("Files pageSize 必须在 1..=100 范围内"))
    }
}

fn validate_query_literal(value: &str, field: &str) -> AppResult<()> {
    if value.contains(['\'', '\\']) {
        return Err(AppError::generic(format!(
            "{field} 包含华为 queryParam 尚未定义转义规则的字符"
        )));
    }
    Ok(())
}

fn files_protocol_error(ctx: &str, cause: &str, auth_already_replayed: bool) -> AppError {
    response_decode_error(ctx, RequestSemantics::Read, auth_already_replayed, cause)
}

/// 严格解析 Files:list/search 单页。
///
/// `files` 缺失、类型错误或任一条目不完整时整页失败；`nextCursor` 只接受
/// 缺失/null/string，空字符串按终页处理。这样 schema 歧义永远不会变成可信空页。
fn parse_file_list_page(
    body: &Value,
    ctx: &str,
    auth_already_replayed: bool,
) -> AppResult<FileListResult> {
    let object = body
        .as_object()
        .ok_or_else(|| files_protocol_error(ctx, "响应顶层必须是对象", auth_already_replayed))?;

    if let Some(category) = object.get("category") {
        match category {
            Value::String(value) if value == "drive#fileList" => {}
            _ => {
                return Err(files_protocol_error(
                    ctx,
                    "category 不是 drive#fileList",
                    auth_already_replayed,
                ));
            }
        }
    }

    let raw_files = object
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| files_protocol_error(ctx, "files 缺失或不是数组", auth_already_replayed))?;
    let mut files = Vec::with_capacity(raw_files.len());
    for (index, value) in raw_files.iter().enumerate() {
        files.push(parse_drive_file_strict(
            value,
            ctx,
            auth_already_replayed,
            Some(index),
        )?);
    }

    let next_cursor = match object.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.is_empty() => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(files_protocol_error(
                ctx,
                "nextCursor 必须是字符串、null 或缺失",
                auth_already_replayed,
            ));
        }
    };

    Ok(FileListResult { files, next_cursor })
}

fn parse_drive_file_strict(
    value: &Value,
    ctx: &str,
    auth_already_replayed: bool,
    index: Option<usize>,
) -> AppResult<DriveFile> {
    let prefix = index
        .map(|index| format!("files[{index}]"))
        .unwrap_or_else(|| "file".to_string());
    let object = value.as_object().ok_or_else(|| {
        files_protocol_error(ctx, &format!("{prefix} 必须是对象"), auth_already_replayed)
    })?;

    require_nonempty_string(object.get("id"), ctx, &prefix, "id", auth_already_replayed)?;
    let name_value = object.get("fileName").or_else(|| object.get("name"));
    require_nonempty_string(name_value, ctx, &prefix, "fileName", auth_already_replayed)?;
    require_nonempty_string(
        object.get("mimeType"),
        ctx,
        &prefix,
        "mimeType",
        auth_already_replayed,
    )?;
    if let Some(category) = object.get("category") {
        match category {
            Value::String(value) if value == "drive#file" => {}
            _ => {
                return Err(files_protocol_error(
                    ctx,
                    &format!("{prefix}.category 不是 drive#file"),
                    auth_already_replayed,
                ));
            }
        }
    }

    validate_optional_nonnegative_i64(
        object.get("size"),
        ctx,
        &prefix,
        "size",
        auth_already_replayed,
    )?;
    validate_optional_string(
        object.get("description"),
        ctx,
        &prefix,
        "description",
        auth_already_replayed,
    )?;
    validate_optional_string(
        object.get("thumbnailLink"),
        ctx,
        &prefix,
        "thumbnailLink",
        auth_already_replayed,
    )?;
    for field in [
        "sha256",
        "md5",
        "md5Checksum",
        "fileSha256",
        "hash",
        "contentHash",
    ] {
        validate_optional_string(
            object.get(field),
            ctx,
            &prefix,
            field,
            auth_already_replayed,
        )?;
    }
    for field in ["createdTime", "editedTime"] {
        validate_optional_timestamp(
            object.get(field),
            ctx,
            &prefix,
            field,
            auth_already_replayed,
        )?;
    }
    if let Some(parent_folder) = object.get("parentFolder") {
        match parent_folder {
            Value::Null => {}
            Value::Array(values)
                if values
                    .iter()
                    .all(|value| value.as_str().is_some_and(|value| !value.is_empty())) => {}
            _ => {
                return Err(files_protocol_error(
                    ctx,
                    &format!("{prefix}.parentFolder 必须是字符串数组（元素不能为空）或 null"),
                    auth_already_replayed,
                ));
            }
        }
    }

    DriveFile::from_json(value).ok_or_else(|| {
        files_protocol_error(
            ctx,
            &format!("{prefix} 无法构造 DriveFile"),
            auth_already_replayed,
        )
    })
}

fn require_nonempty_string(
    value: Option<&Value>,
    ctx: &str,
    prefix: &str,
    field: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    if value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
    {
        Ok(())
    } else {
        Err(files_protocol_error(
            ctx,
            &format!("{prefix}.{field} 缺失、类型错误或为空"),
            auth_already_replayed,
        ))
    }
}

fn validate_optional_nonnegative_i64(
    value: Option<&Value>,
    ctx: &str,
    prefix: &str,
    field: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Number(number)) if number.as_i64().is_some_and(|value| value >= 0) => Ok(()),
        _ => Err(files_protocol_error(
            ctx,
            &format!("{prefix}.{field} 必须是非负整数或 null"),
            auth_already_replayed,
        )),
    }
}

fn validate_optional_string(
    value: Option<&Value>,
    ctx: &str,
    prefix: &str,
    field: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    match value {
        None | Some(Value::Null | Value::String(_)) => Ok(()),
        _ => Err(files_protocol_error(
            ctx,
            &format!("{prefix}.{field} 必须是字符串或 null"),
            auth_already_replayed,
        )),
    }
}

fn validate_optional_timestamp(
    value: Option<&Value>,
    ctx: &str,
    prefix: &str,
    field: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    match value {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(timestamp))
            if chrono::DateTime::parse_from_rfc3339(timestamp).is_ok() =>
        {
            Ok(())
        }
        _ => Err(files_protocol_error(
            ctx,
            &format!("{prefix}.{field} 必须是 RFC3339 字符串或 null"),
            auth_already_replayed,
        )),
    }
}

fn delete_path(id: &str) -> String {
    file_path(id)
}

fn file_path(id: &str) -> String {
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
