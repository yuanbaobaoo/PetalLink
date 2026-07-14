//! Changes API —— 华为 Drive 增量变更接口（GET /drive/v1/changes）。
//!
//! `nextCursor` 只用于同一轮增量拉取的续页；末页的 `newStartCursor`
//! 才是下一轮轮询可提交的 checkpoint。任一页面或变更项无法严格解释时，
//! 本模块直接失败，由调用方保留旧 checkpoint 并回退可信全量刷新。

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::drive::client::{parse_json_response, DriveClient};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

/// 生产环境单轮增量追平允许请求的最大页数。
const DEFAULT_MAX_CHANGE_PAGES: usize = 10_000;

/// 变更类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// 官方 `deleted=true` 硬删除，或真机兼容的 trashDone/recycled 软删除。
    Removed,
    /// 文件新增或元数据修改。
    Modified,
}

/// 单条严格解析后的变更。
///
/// 删除事件可能只有顶层 `fileId`，因此不能用伪造的空 `DriveFile` 表示 tombstone。
#[derive(Debug, Clone)]
pub struct Change {
    pub kind: ChangeKind,
    pub file_id: String,
    pub file: Option<DriveFile>,
}

impl Change {
    /// 返回变更对应的云端文件标识。
    pub fn file_id(&self) -> &str {
        &self.file_id
    }

    /// 返回非删除变更携带的完整文件元数据。
    pub fn file(&self) -> Option<&DriveFile> {
        self.file.as_ref()
    }

    /// 严格解析单条 change。无法安全解释的字段或语义让整页失败。
    pub fn from_json(value: &Value) -> AppResult<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| protocol_error("change 条目不是对象"))?;

        validate_category(object, "category", "drive#change", "change")?;
        validate_category(object, "type", "File", "change")?;
        validate_optional_rfc3339(object, "time", "change")?;

        let file_id = required_non_empty_string(object, "fileId", "change")?.to_string();
        let deleted = required_bool(object, "deleted", "change")?;
        let change_type = optional_non_empty_string(object, "changeType", "change")?;

        let parsed_file = match object.get("file") {
            None | Some(Value::Null) => None,
            Some(file) => Some(parse_change_file(file)?),
        };

        if let Some((parsed_id, _, _)) = parsed_file.as_ref() {
            if parsed_id != &file_id {
                return Err(protocol_error(format!(
                    "change.file.id 与 fileId 不一致：{} != {}",
                    parsed_id, file_id
                )));
            }
        }

        let recycled = parsed_file
            .as_ref()
            .is_some_and(|(_, recycled, _)| *recycled);
        let soft_deleted = change_type == Some("trashDone") || recycled;
        let kind = if deleted || soft_deleted {
            ChangeKind::Removed
        } else {
            ChangeKind::Modified
        };

        let file = parsed_file.and_then(|(_, _, file)| file);
        if kind == ChangeKind::Modified && file.is_none() {
            return Err(protocol_error(format!(
                "非删除 change 缺少可完整解析的 file：{file_id}"
            )));
        }
        if kind == ChangeKind::Modified {
            let parent_count = file
                .as_ref()
                .and_then(|file| file.parent_folder.as_ref())
                .map_or(0, Vec::len);
            if parent_count != 1 {
                return Err(protocol_error(format!(
                    "非删除 change 必须且只能有一个 parentFolder：{file_id}"
                )));
            }
        }

        Ok(Self {
            kind,
            file_id,
            file,
        })
    }
}

/// Changes 单页。两个 cursor 字段有不同含义，禁止合并。
#[derive(Debug, Clone)]
pub struct ChangesPage {
    pub changes: Vec<Change>,
    /// 同一轮 catch-up 的下一页 cursor；非空时必须继续。
    pub next_cursor: Option<String>,
    /// 仅末页可提交为下一轮 checkpoint。
    pub new_start_cursor: Option<String>,
}

/// 兼容旧调用名称；语义已严格升级为 [`ChangesPage`]。
pub type ChangeListResult = ChangesPage;

impl ChangesPage {
    /// 严格解析单页变更及两个用途不同的游标。
    pub fn from_json(json: &Value) -> AppResult<Self> {
        let object = json
            .as_object()
            .ok_or_else(|| protocol_error("Changes:list 顶层响应不是对象"))?;
        validate_category(object, "category", "drive#changeList", "Changes:list")?;

        let raw_changes = object
            .get("changes")
            .ok_or_else(|| protocol_error("Changes:list 响应缺少 changes 数组"))?
            .as_array()
            .ok_or_else(|| protocol_error("Changes:list 的 changes 不是数组"))?;

        let mut changes = Vec::with_capacity(raw_changes.len());
        for (index, value) in raw_changes.iter().enumerate() {
            changes.push(Change::from_json(value).map_err(|error| {
                protocol_error(format!(
                    "Changes:list 第 {} 个 change 无效：{error}",
                    index + 1
                ))
            })?);
        }

        // 两个字段都先严格解析，再由 paginator 决定续页或终止。
        let next_cursor = optional_cursor(object, "nextCursor")?;
        let new_start_cursor = optional_cursor(object, "newStartCursor")?;

        Ok(Self {
            changes,
            next_cursor,
            new_start_cursor,
        })
    }
}

/// 按严格游标协议拉取云盘增量变更。
pub struct ChangesApi {
    client: Arc<DriveClient>,
    max_pages: usize,
}

impl ChangesApi {
    /// 使用生产页数上限创建增量变更接口。
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self {
            client,
            max_pages: DEFAULT_MAX_CHANGE_PAGES,
        }
    }

    /// 使用受控页数上限构造 paginator。生产默认值与测试/诊断值相互独立。
    pub fn with_page_limit(client: Arc<DriveClient>, max_pages: usize) -> AppResult<Self> {
        if max_pages == 0 {
            return Err(AppError::generic("Changes 分页上限必须大于 0"));
        }
        Ok(Self { client, max_pages })
    }

    /// 获取初始游标。官方要求显式请求 `fields=*`。
    pub async fn get_start_cursor(&self) -> AppResult<String> {
        let resp = self.client.get("/changes/getStartCursor?fields=*").await?;
        let body: Value = parse_json_response(resp, "startCursor").await?;
        let object = body
            .as_object()
            .ok_or_else(|| protocol_error("getStartCursor 顶层响应不是对象"))?;
        validate_category(object, "category", "drive#startCursor", "getStartCursor")?;
        Ok(required_non_empty_string(object, "startCursor", "getStartCursor")?.to_string())
    }

    /// 拉取一页增量变更。cursor 是华为接口必填项。
    pub async fn list_changes(&self, cursor: &str) -> AppResult<ChangesPage> {
        let cursor = required_cursor(cursor, "Changes:list")?;
        let url = format!(
            "/changes?fields=*&pageSize=100&includeDeleted=true&cursor={}",
            crate::drive::files_api::urlencoding(cursor)
        );
        let resp = self.client.get(&url).await?;
        let body: Value = parse_json_response(resp, "changes").await?;
        ChangesPage::from_json(&body)
    }

    /// 拉取完整的一轮增量变更。
    ///
    /// 空 `changes` 不代表终页；只要 `nextCursor` 非空就继续。只有无非空
    /// `nextCursor` 且存在非空 `newStartCursor` 时才成功返回 checkpoint。
    pub async fn list_all_changes(&self, start_cursor: &str) -> AppResult<(Vec<Change>, String)> {
        let mut cursor = required_cursor(start_cursor, "Changes:list_all")?.to_string();
        let mut seen = HashSet::new();
        seen.insert(cursor.clone());
        let mut all = Vec::new();

        for page_number in 1..=self.max_pages {
            let page = self.list_changes(&cursor).await?;
            let page_count = page.changes.len();
            all.extend(page.changes);

            if let Some(next_cursor) = page.next_cursor {
                if page_number == self.max_pages {
                    return Err(protocol_error(format!(
                        "Changes:list 达到页数上限 {} 时仍有 nextCursor，拒绝返回部分结果",
                        self.max_pages
                    )));
                }
                if !seen.insert(next_cursor.clone()) {
                    return Err(protocol_error(format!(
                        "Changes:list cursor 未推进或形成循环：{next_cursor}"
                    )));
                }
                tracing::debug!(
                    page_number,
                    page_count,
                    total = all.len(),
                    "Changes:list 继续翻页"
                );
                cursor = next_cursor;
                continue;
            }

            let final_cursor = page.new_start_cursor.ok_or_else(|| {
                protocol_error("Changes:list 终页缺少非空 newStartCursor，无法提交 checkpoint")
            })?;
            if (final_cursor == cursor && page_count > 0)
                || (final_cursor != cursor && seen.contains(&final_cursor))
            {
                return Err(protocol_error(format!(
                    "Changes:list 已累计 {} 条变更，但 newStartCursor 未推进或形成循环：{final_cursor}",
                    all.len()
                )));
            }
            tracing::info!(
                pages = page_number,
                total = all.len(),
                "Changes:list 已完整追平"
            );
            return Ok((all, final_cursor));
        }

        Err(protocol_error("Changes:list 未能在分页上限内结束"))
    }
}

/// 要求调用游标非空，否则返回协议错误。
fn required_cursor<'a>(cursor: &'a str, operation: &str) -> AppResult<&'a str> {
    if cursor.trim().is_empty() {
        Err(protocol_error(format!("{operation} 缺少非空 cursor")))
    } else {
        Ok(cursor)
    }
}

/// 解析可缺失游标，并将空字符串视为未提供。
fn optional_cursor(object: &Map<String, Value>, field: &str) -> AppResult<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(cursor)) if cursor.trim().is_empty() => Ok(None),
        Some(Value::String(cursor)) => Ok(Some(cursor.clone())),
        Some(_) => Err(protocol_error(format!(
            "Changes:list 的 {field} 必须是字符串、null 或缺失"
        ))),
    }
}

/// 从协议对象读取必需的非空字符串字段。
fn required_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> AppResult<&'a str> {
    match object.get(field) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
        Some(Value::String(_)) => Err(protocol_error(format!("{context} 的 {field} 不能为空"))),
        Some(_) => Err(protocol_error(format!("{context} 的 {field} 必须是字符串"))),
        None => Err(protocol_error(format!("{context} 缺少 {field}"))),
    }
}

/// 从协议对象读取可选但一旦出现就必须非空的字符串。
fn optional_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> AppResult<Option<&'a str>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value)),
        Some(Value::String(_)) => Err(protocol_error(format!(
            "{context} 的 {field} 不能为空字符串"
        ))),
        Some(_) => Err(protocol_error(format!(
            "{context} 的 {field} 必须是字符串或 null"
        ))),
    }
}

/// 从协议对象读取必需布尔字段。
fn required_bool(object: &Map<String, Value>, field: &str, context: &str) -> AppResult<bool> {
    match object.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(protocol_error(format!("{context} 的 {field} 必须是布尔值"))),
        None => Err(protocol_error(format!("{context} 缺少 {field}"))),
    }
}

/// 校验可选类别字段与官方预期值一致。
fn validate_category(
    object: &Map<String, Value>,
    field: &str,
    expected: &str,
    context: &str,
) -> AppResult<()> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(value)) if value == expected => Ok(()),
        Some(Value::String(value)) => Err(protocol_error(format!(
            "{context} 的 {field} 非预期：{value}"
        ))),
        Some(_) => Err(protocol_error(format!(
            "{context} 的 {field} 必须是字符串或 null"
        ))),
    }
}

/// 校验可选字段只能是字符串或空值。
fn validate_optional_string(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> AppResult<()> {
    match object.get(field) {
        None | Some(Value::Null) | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(protocol_error(format!(
            "{context} 的 {field} 必须是字符串或 null"
        ))),
    }
}

/// 校验可选时间字段符合 RFC 3339。
fn validate_optional_rfc3339(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> AppResult<()> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(value)) => chrono::DateTime::parse_from_rfc3339(value)
            .map(|_| ())
            .map_err(|_| protocol_error(format!("{context} 的 {field} 不是 RFC3339 时间"))),
        Some(_) => Err(protocol_error(format!(
            "{context} 的 {field} 必须是 RFC3339 字符串或 null"
        ))),
    }
}

/// 返回 `(id, recycled, 完整文件)`。删除 tombstone 的 file 可以只含 id；
/// 非删除事件由调用方要求第三项必须存在。
fn parse_change_file(value: &Value) -> AppResult<(String, bool, Option<DriveFile>)> {
    let object = value
        .as_object()
        .ok_or_else(|| protocol_error("change.file 不是对象"))?;
    validate_category(object, "category", "drive#file", "change.file")?;

    let id = required_non_empty_string(object, "id", "change.file")?.to_string();
    let name = match object.get("fileName") {
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.as_str()),
        Some(Value::String(_)) => {
            return Err(protocol_error("change.file 的 fileName 不能为空"));
        }
        Some(Value::Null) | None => optional_non_empty_string(object, "name", "change.file")?,
        Some(_) => {
            return Err(protocol_error(
                "change.file 的 fileName 必须是字符串或 null",
            ));
        }
    };

    validate_optional_string(object, "mimeType", "change.file")?;
    validate_optional_string(object, "description", "change.file")?;
    validate_optional_string(object, "thumbnailLink", "change.file")?;
    for hash_field in [
        "sha256",
        "md5",
        "md5Checksum",
        "fileSha256",
        "hash",
        "contentHash",
    ] {
        validate_optional_string(object, hash_field, "change.file")?;
    }
    validate_optional_rfc3339(object, "createdTime", "change.file")?;
    validate_optional_rfc3339(object, "editedTime", "change.file")?;

    if let Some(parent_folder) = object.get("parentFolder") {
        match parent_folder {
            Value::Null => {}
            Value::Array(parents)
                if parents.iter().all(|parent| {
                    parent
                        .as_str()
                        .is_some_and(|parent| !parent.trim().is_empty())
                }) => {}
            Value::Array(_) => {
                return Err(protocol_error(
                    "change.file 的 parentFolder 必须只包含非空字符串",
                ));
            }
            _ => {
                return Err(protocol_error(
                    "change.file 的 parentFolder 必须是数组或 null",
                ));
            }
        }
    }

    if let Some(size) = object.get("size") {
        match size {
            Value::Null => {}
            Value::Number(number) if number.as_i64().is_some() => {}
            _ => {
                return Err(protocol_error("change.file 的 size 必须是 i64 整数或 null"));
            }
        }
    }

    let recycled = match object.get("recycled") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(protocol_error(
                "change.file 的 recycled 必须是布尔值或 null",
            ))
        }
    };

    let file = if name.is_some() {
        Some(
            DriveFile::from_json(value)
                .ok_or_else(|| protocol_error("change.file 无法解析为 DriveFile"))?,
        )
    } else {
        None
    };

    Ok((id, recycled, file))
}

/// 构造带 Changes API 上下文的协议错误。
fn protocol_error(message: impl Into<String>) -> AppError {
    AppError::generic(format!("华为 Changes API 协议错误：{}", message.into()))
}
