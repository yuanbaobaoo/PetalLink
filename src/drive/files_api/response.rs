//! Files API 响应的严格模式校验与写后身份核验。

use serde_json::Value;

use super::request::FOLDER_MIME_TYPE;
use crate::drive::client::{response_decode_error, response_metadata};
use crate::drive::models::{DriveFile, FileListResult};
use crate::error::{AppError, AppResult, RequestSemantics};

/// 构造保留认证重放状态的只读协议错误。
pub(super) fn files_protocol_error(
    ctx: &str,
    cause: &str,
    auth_already_replayed: bool,
) -> AppError {
    response_decode_error(ctx, RequestSemantics::Read, auth_already_replayed, cause)
}

/// 严格解析 Files:list/search 单页。
///
/// `files` 缺失、类型错误或任一条目不完整时整页失败；`nextCursor` 只接受
/// 缺失/null/string，空字符串按终页处理。这样 schema 歧义永远不会变成可信空页。
pub(super) fn parse_file_list_page(
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

/// 严格校验单个 File 的身份、类型、时间及父目录字段。
pub(super) fn parse_drive_file_strict(
    value: &Value,
    ctx: &str,
    auth_already_replayed: bool,
    index: Option<usize>,
) -> AppResult<DriveFile> {
    // 错误前缀保留列表索引，方便定位具体异常元素。
    let prefix = index
        .map(|index| format!("files[{index}]"))
        .unwrap_or_else(|| "file".to_string());
    let object = value.as_object().ok_or_else(|| {
        files_protocol_error(ctx, &format!("{prefix} 必须是对象"), auth_already_replayed)
    })?;

    // 身份、名称和 MIME 是后续同步规划必需字段。
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

    // 可选字段出现时必须严格匹配协议类型，不能静默丢弃坏数据。
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
    // 父目录数组中的空 ID 会破坏路径解析，因此直接拒绝。
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

    // 字段合同通过后再交给模型解析，失败仍作为协议错误返回。
    DriveFile::from_json(value).ok_or_else(|| {
        files_protocol_error(
            ctx,
            &format!("{prefix} 无法构造 DriveFile"),
            auth_already_replayed,
        )
    })
}

/// 要求字段存在且为非空字符串。
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

/// 校验可选字段为非负整数或空值。
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

/// 校验可选字段为字符串或空值。
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

/// 校验可选字段为 RFC 3339 时间或空值。
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

/// 将写响应解析为文件，并把解码失败标记为写后不确定性。
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

/// 写接口使用 `fields=*`，因此成功结果必须是可识别、非空的 Huawei File，而不能只凭
/// 任意 JSON/任意 2xx 推进本地状态。
pub(super) fn parse_verified_written_drive_file(
    body: &Value,
    ctx: &str,
    auth_already_replayed: bool,
) -> AppResult<DriveFile> {
    let object = body.as_object().ok_or_else(|| {
        write_protocol_error(ctx, auth_already_replayed, "响应顶层不是 File 对象")
    })?;
    if object
        .get("category")
        .is_some_and(|category| category.as_str() != Some("drive#file"))
    {
        return Err(write_protocol_error(
            ctx,
            auth_already_replayed,
            "响应 category 不是 drive#file",
        ));
    }
    let file = parse_written_drive_file(body, ctx, auth_already_replayed)?;
    if file.id.trim().is_empty()
        || file.name.trim().is_empty()
        || !file
            .mime_type
            .as_deref()
            .is_some_and(|mime_type| !mime_type.trim().is_empty())
    {
        return Err(write_protocol_error(
            ctx,
            auth_already_replayed,
            "File 缺少非空 id/fileName/mimeType",
        ));
    }
    if let Some(parent_folder) = object.get("parentFolder") {
        match parent_folder {
            Value::Null => {}
            Value::Array(parents)
                if parents
                    .iter()
                    .all(|parent| parent.as_str().is_some_and(|id| !id.is_empty())) => {}
            _ => {
                return Err(write_protocol_error(
                    ctx,
                    auth_already_replayed,
                    "File.parentFolder 不是非空字符串数组或 null",
                ));
            }
        }
    }
    Ok(file)
}

/// 仅接受华为 Files 写接口的 200 合同，并返回认证是否已重放。
pub(super) fn require_official_write_ok(resp: &reqwest::Response, ctx: &str) -> AppResult<bool> {
    let metadata = response_metadata(resp, RequestSemantics::Write);
    if resp.status() != reqwest::StatusCode::OK {
        return Err(response_decode_error(
            ctx,
            metadata.semantics,
            metadata.auth_already_replayed,
            &format!(
                "Huawei Files 写操作成功状态必须是 200，实际为 {}",
                resp.status().as_u16()
            ),
        ));
    }
    Ok(metadata.auth_already_replayed)
}

/// 按读写语义构造 Files 协议错误。
pub(super) fn protocol_error(
    ctx: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
    cause: &str,
) -> AppError {
    response_decode_error(ctx, semantics, auth_already_replayed, cause)
}

/// 构造可能发生在远端已提交之后的写协议错误。
pub(super) fn write_protocol_error(
    ctx: &str,
    auth_already_replayed: bool,
    cause: &str,
) -> AppError {
    protocol_error(ctx, RequestSemantics::Write, auth_already_replayed, cause)
}

/// 核验响应文件身份与请求标识一致。
pub(super) fn verify_file_id(
    file: &DriveFile,
    expected_id: &str,
    ctx: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppResult<()> {
    if file.id == expected_id {
        Ok(())
    } else {
        Err(protocol_error(
            ctx,
            semantics,
            auth_already_replayed,
            "响应 File.id 与请求 fileId 不一致",
        ))
    }
}

/// 以写请求语义核验响应文件身份。
pub(super) fn verify_written_file_id(
    file: &DriveFile,
    expected_id: &str,
    ctx: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    verify_file_id(
        file,
        expected_id,
        ctx,
        RequestSemantics::Write,
        auth_already_replayed,
    )
}

/// 核验新建目录的身份、名称、类型与唯一父目录。
pub(super) fn verify_created_folder(
    file: &DriveFile,
    expected_name: &str,
    expected_parent: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppResult<()> {
    if file.id.trim().is_empty() {
        return Err(protocol_error(
            "createFolder",
            semantics,
            auth_already_replayed,
            "响应 File.id 为空",
        ));
    }
    if file.name != expected_name {
        return Err(protocol_error(
            "createFolder",
            semantics,
            auth_already_replayed,
            "响应 fileName 与请求名称不一致",
        ));
    }
    if file.mime_type.as_deref() != Some(FOLDER_MIME_TYPE) {
        return Err(protocol_error(
            "createFolder",
            semantics,
            auth_already_replayed,
            "响应 mimeType 不是 Huawei 文件夹类型",
        ));
    }
    // 根目录请求使用协议别名 `root`，但 Huawei File 响应中的 parentFolder 返回账号
    // 实际根目录 ID。该 ID 是服务端分配的 opaque 值，不能与字面量 `root` 比较。
    // 此处仍要求响应恰有一个非空父目录；非根目录继续精确匹配请求中的稳定 fileId。
    if expected_parent == "root" {
        single_parent(file, "createFolder", semantics, auth_already_replayed)?;
        return Ok(());
    }
    verify_parent(
        file,
        expected_parent,
        "createFolder",
        semantics,
        auth_already_replayed,
    )
}

/// 返回唯一非空父目录；多父或缺失时拒绝继续移动。
pub(super) fn single_parent<'a>(
    file: &'a DriveFile,
    ctx: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppResult<&'a str> {
    match file.parent_folder.as_deref() {
        Some([parent]) if !parent.is_empty() => Ok(parent),
        _ => Err(protocol_error(
            ctx,
            semantics,
            auth_already_replayed,
            "当前只支持一个非空 parentFolder，响应无法安全用于移动",
        )),
    }
}

/// 核验文件唯一父目录与预期值一致。
pub(super) fn verify_parent(
    file: &DriveFile,
    expected_parent: &str,
    ctx: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppResult<()> {
    if single_parent(file, ctx, semantics, auth_already_replayed)? == expected_parent {
        Ok(())
    } else {
        Err(protocol_error(
            ctx,
            semantics,
            auth_already_replayed,
            "响应 parentFolder 与目标父目录不一致",
        ))
    }
}

/// 以写请求语义核验文件的最终父目录。
pub(super) fn verify_written_parent(
    file: &DriveFile,
    expected_parent: &str,
    ctx: &str,
    auth_already_replayed: bool,
) -> AppResult<()> {
    verify_parent(
        file,
        expected_parent,
        ctx,
        RequestSemantics::Write,
        auth_already_replayed,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::verify_created_folder;
    use crate::drive::models::DriveFile;
    use crate::error::RequestSemantics;

    fn folder(parent_folder: serde_json::Value) -> DriveFile {
        DriveFile::from_json(&json!({
            "id": "folder-id",
            "fileName": "new-folder",
            "mimeType": "application/vnd.huawei-apps.folder",
            "parentFolder": parent_folder,
        }))
        .expect("valid folder")
    }

    #[test]
    fn root_alias_accepts_unique_opaque_root_parent_id() {
        let result = verify_created_folder(
            &folder(json!(["opaque-account-root-id"])),
            "new-folder",
            "root",
            RequestSemantics::Read,
            false,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn root_alias_still_rejects_missing_or_ambiguous_parent() {
        for parent_folder in [json!([]), json!(["root-a", "root-b"])] {
            let result = verify_created_folder(
                &folder(parent_folder),
                "new-folder",
                "root",
                RequestSemantics::Read,
                false,
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn nested_folder_parent_still_requires_exact_file_id() {
        let result = verify_created_folder(
            &folder(json!(["other-parent-id"])),
            "new-folder",
            "expected-parent-id",
            RequestSemantics::Read,
            false,
        );

        assert!(result.is_err());
    }
}
