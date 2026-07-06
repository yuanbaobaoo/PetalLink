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
use crate::drive::client::{classify_error, handle_error_response};
use crate::drive::models::{DriveFile, FileListResult};
use crate::error::{AppError, AppResult};

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
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 list 响应失败：{e}")))?;
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
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 get 响应失败：{e}")))?;
        DriveFile::from_json(&body).ok_or_else(|| AppError::generic("文件元数据格式异常"))
    }

    /// 创建文件夹。对齐 dart `FilesApi.createFolder({name, parentId?})`。
    ///
    /// 关键：body 用 [`ascii_json_encode`] 编码（中文名必须 ASCII 转义）。
    pub async fn create_folder(&self, name: &str, parent_id: Option<&str>) -> AppResult<DriveFile> {
        let body = build_create_folder_body(name, parent_id);
        let encoded = ascii_json_encode(&body);
        let url = format!("{}/files?fields=*", crate::constants::DRIVE_API_BASE);
        let resp = self.send_post(&url, encoded.into_bytes(), "application/json").await?;
        let body_json: Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 createFolder 响应失败：{e}")))?;
        DriveFile::from_json(&body_json).ok_or_else(|| AppError::generic("创建文件夹响应异常"))
    }

    /// 删除文件（软删除，进回收站）。对齐 dart `FilesApi.delete(id)`。
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let url = format!("{}/files/{id}", crate::constants::DRIVE_API_BASE);
        let resp = self.send_delete(&url).await?;
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
        let resp = self.send_patch(&url, encoded.into_bytes(), "application/json").await?;
        let body_json: Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 update 响应失败：{e}")))?;
        DriveFile::from_json(&body_json).ok_or_else(|| AppError::generic("更新响应异常"))
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
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 search 响应失败：{e}")))?;
        Ok(FileListResult::from_json(&body))
    }

    // ===== 内部：带 auth + 401 重放的请求发送 =====

    /// 发送 GET（注入 token + 401 重放）
    async fn send_get(&self, url: &str) -> AppResult<reqwest::Response> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let resp = self
            .client
            .raw_http()
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| classify_error(&e))?;
        self.handle_401_retry(reqwest::Method::GET, url, resp, None, "").await
    }

    /// 发送 POST（带 body）
    async fn send_post(&self, url: &str, body: Vec<u8>, content_type: &str) -> AppResult<reqwest::Response> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let resp = self
            .client
            .raw_http()
            .post(url)
            .bearer_auth(&token)
            .header("Content-Type", content_type)
            .body(body.clone())
            .send()
            .await
            .map_err(|e| classify_error(&e))?;
        self.handle_401_retry(reqwest::Method::POST, url, resp, Some(body), content_type).await
    }

    /// 发送 PATCH
    async fn send_patch(&self, url: &str, body: Vec<u8>, content_type: &str) -> AppResult<reqwest::Response> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let resp = self
            .client
            .raw_http()
            .request(reqwest::Method::PATCH, url)
            .bearer_auth(&token)
            .header("Content-Type", content_type)
            .body(body.clone())
            .send()
            .await
            .map_err(|e| classify_error(&e))?;
        self.handle_401_retry(reqwest::Method::PATCH, url, resp, Some(body), content_type).await
    }

    /// 发送 DELETE
    async fn send_delete(&self, url: &str) -> AppResult<reqwest::Response> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let resp = self
            .client
            .raw_http()
            .delete(url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| classify_error(&e))?;
        self.handle_401_retry(reqwest::Method::DELETE, url, resp, None, "").await
    }

    /// 401 重放：若响应为 401，刷新 token 后重放一次。
    async fn handle_401_retry(
        &self,
        method: reqwest::Method,
        url: &str,
        resp: reqwest::Response,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
            if !resp.status().is_success() {
                return Err(handle_error_response(resp).await);
            }
            return Ok(resp);
        }
        tracing::warn!("Files API 收到 401，刷新 token 后重放");
        let new_token = self.client.auth().refresher().refresh().await?;
        let mut req = self
            .client
            .raw_http()
            .request(method, url)
            .bearer_auth(new_token.access_token);
        if !content_type.is_empty() {
            req = req.header("Content-Type", content_type);
        }
        if let Some(b) = body {
            req = req.body(b);
        }
        let resp = req.send().await.map_err(|e| classify_error(&e))?;
        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }
        Ok(resp)
    }
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

/// 构造 createFolder 请求体。
/// 对齐 dart `buildCreateFolderBody`：mimeType 必填，root 目录省略 parentFolder。
pub fn build_create_folder_body(name: &str, parent_id: Option<&str>) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("fileName".into(), Value::String(name.to_string()));
    body.insert("mimeType".into(), Value::String(FOLDER_MIME_TYPE.to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
