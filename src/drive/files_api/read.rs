//! 云盘文件读取、搜索与受限分页。

use std::collections::HashSet;

use serde_json::Value;

use super::request::{
    file_path, urlencoding, validate_page_size, validate_query_literal, PRODUCTION_PAGE_SIZE,
};
use super::response::{files_protocol_error, parse_drive_file_strict, parse_file_list_page};
use super::FilesApi;
use crate::drive::client::{parse_json_response, response_metadata};
use crate::drive::models::{DriveFile, FileListResult};
use crate::error::{AppResult, RequestSemantics};

impl FilesApi {
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

    /// 发送 GET（相对 Drive API 路径，保留可注入 base URL + 401 重放）。
    async fn send_get(&self, path: &str) -> AppResult<reqwest::Response> {
        self.client.get(path).await
    }
}
