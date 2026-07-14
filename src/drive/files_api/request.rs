//! Files API 请求路径、查询字面量与请求体编码。

use serde_json::Value;

use crate::error::{AppError, AppResult};

/// 华为文件夹 mimeType。
pub(super) const FOLDER_MIME_TYPE: &str = "application/vnd.huawei-apps.folder";
/// 华为 Files:list 单页允许的生产请求条数。
pub(super) const PRODUCTION_PAGE_SIZE: u32 = 100;

/// 校验单页大小位于华为接口允许的 `1..=100` 范围。
pub(super) fn validate_page_size(page_size: u32) -> AppResult<()> {
    if (1..=PRODUCTION_PAGE_SIZE).contains(&page_size) {
        Ok(())
    } else {
        Err(AppError::generic("Files pageSize 必须在 1..=100 范围内"))
    }
}

/// 拒绝官方 DSL 未定义转义规则的查询字面量。
pub(super) fn validate_query_literal(value: &str, field: &str) -> AppResult<()> {
    if value.contains(['\'', '\\']) {
        return Err(AppError::generic(format!(
            "{field} 包含华为 queryParam 尚未定义转义规则的字符"
        )));
    }
    Ok(())
}

/// 构造软删除写操作使用的文件资源路径。
pub(super) fn delete_path(id: &str) -> String {
    file_path(id)
}

/// 构造更新路径，并在移动时附加成对父目录参数。
pub(super) fn update_path(id: &str, move_parents: Option<(&str, &str)>) -> String {
    let mut path = format!("{}?fields=*", file_path(id));
    if let Some((old_parent, new_parent)) = move_parents {
        path.push_str("&addParentFolder=");
        path.push_str(&urlencoding(new_parent));
        path.push_str("&removeParentFolder=");
        path.push_str(&urlencoding(old_parent));
    }
    path
}

/// 将文件标识按单一路径段编码为资源路径。
pub(super) fn file_path(id: &str) -> String {
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

/// 校验通用文件标识非空。
pub(super) fn validate_file_id(id: &str) -> AppResult<()> {
    validate_file_id_value(id, "fileId")
}

/// 校验指定语义字段中的文件标识非空。
pub(super) fn validate_file_id_value(id: &str, field: &str) -> AppResult<()> {
    if id.trim().is_empty() {
        Err(AppError::generic(format!("{field} 不能为空")))
    } else {
        Ok(())
    }
}

/// 将缺失或空根目录标识归一化为 `root`，并拒绝其他空白标识。
pub(super) fn canonical_parent_id(parent_id: Option<&str>) -> AppResult<&str> {
    match parent_id {
        None | Some("") | Some("root") => Ok("root"),
        Some(parent_id) => {
            validate_file_id_value(parent_id, "parentFolder")?;
            Ok(parent_id)
        }
    }
}
