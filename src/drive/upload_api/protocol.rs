//! 断点上传会话地址、服务端偏移与错误恢复语义。

use reqwest::header::{LOCATION, RETRY_AFTER};
use serde_json::Value;

use crate::drive::models::DriveFile;
use crate::error::{parse_retry_after, AppError, AppResult, DriveTransportKind, RequestSemantics};

use super::{
    ProgressFn, ResumeProgressFn, ResumeSession, UploadApi, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE,
    MIN_CHUNK_SIZE,
};

impl UploadApi {
    /// 返回服务端会话地址；缺少两种合法身份时按远端不确定失败。
    pub(super) fn session_request_url(&self, session: &ResumeSession) -> AppResult<String> {
        if !session.session_url.is_empty() {
            return Ok(session.session_url.clone());
        }
        if !session.server_id.is_empty() && !session.upload_id.is_empty() {
            return Ok(format!(
                "{}/files/{}?uploadId={}",
                self.upload_base, session.server_id, session.upload_id
            ));
        }
        Err(remote_ambiguity(
            "断点上传会话缺少 Location，也缺少完整 serverId/uploadId",
            false,
        ))
    }

    /// 从响应中的非空 Location 更新会话地址。
    pub(super) fn update_session_location(
        &self,
        session: &mut ResumeSession,
        response: &reqwest::Response,
    ) -> AppResult<()> {
        let Some(location) = response.headers().get(LOCATION) else {
            return Ok(());
        };
        let location = location.to_str().map_err(|error| {
            remote_ambiguity(&format!("上传响应 Location 不是有效文本：{error}"), false)
        })?;
        if location.trim().is_empty() {
            return Err(remote_ambiguity("上传响应返回空 Location", false));
        }
        session.session_url = location.to_string();
        Ok(())
    }

    /// 接受并校验服务端建议的分片大小。
    pub(super) fn update_session_chunk_size(
        &self,
        session: &mut ResumeSession,
        body: &Value,
    ) -> AppResult<()> {
        if let Some(slice_size) = body.get("sliceSize").and_then(Value::as_u64) {
            validated_chunk_size(slice_size)?;
            session.chunk_size = slice_size;
        }
        Ok(())
    }
}

/// 仅在文件身份、长度及可选名称完整匹配时接受最终结果。
pub(super) fn complete_upload_file(
    body: &Value,
    expected_size: u64,
    expected_name: Option<&str>,
) -> Option<DriveFile> {
    let file = DriveFile::from_json(body)?;
    let size_matches = file.size >= 0 && file.size as u64 == expected_size;
    let name_matches = !file.name.trim().is_empty()
        && expected_name.map_or(true, |expected| file.name.as_str() == expected);
    (!file.id.trim().is_empty() && size_matches && name_matches).then_some(file)
}

/// 只接受从 0 开始、连续、无重叠且不越界的已接收范围。空数组明确表示 0；
/// 缺字段、hole、重叠或格式异常都不能回退成本地 `offset + chunk_len`。
pub(super) fn parse_confirmed_offset(body: &Value, total_size: u64) -> AppResult<u64> {
    let ranges = body
        .get("rangeList")
        .and_then(Value::as_array)
        .ok_or_else(|| remote_ambiguity("308 响应缺少 rangeList", false))?;
    if ranges.is_empty() {
        return Ok(0);
    }

    let mut expected_start = 0_u64;
    for range in ranges {
        let raw = range
            .as_str()
            .ok_or_else(|| remote_ambiguity("rangeList 含非字符串元素", false))?;
        let (start, end) = raw
            .split_once('-')
            .ok_or_else(|| remote_ambiguity(&format!("非法上传范围：{raw}"), false))?;
        if end.contains('-') {
            return Err(remote_ambiguity(&format!("非法上传范围：{raw}"), false));
        }
        let start = start
            .parse::<u64>()
            .map_err(|_| remote_ambiguity(&format!("非法上传范围起点：{raw}"), false))?;
        let end = end
            .parse::<u64>()
            .map_err(|_| remote_ambiguity(&format!("非法上传范围终点：{raw}"), false))?;
        if start != expected_start || end < start || end >= total_size {
            return Err(remote_ambiguity(
                &format!(
                    "上传范围不连续或越界：{raw}，期望起点 {expected_start}，总长度 {total_size}"
                ),
                false,
            ));
        }
        expected_start = end
            .checked_add(1)
            .ok_or_else(|| remote_ambiguity("上传范围终点溢出", false))?;
    }
    Ok(expected_start)
}

/// 归一化并校验服务端分片大小，越界时拒绝分配缓冲区。
pub(super) fn validated_chunk_size(chunk_size: u64) -> AppResult<u64> {
    let chunk_size = if chunk_size == 0 {
        DEFAULT_CHUNK_SIZE
    } else {
        chunk_size
    };
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
        return Err(remote_ambiguity(
            &format!(
                "服务端分片大小 {chunk_size} 不在允许范围 {MIN_CHUNK_SIZE}..={MAX_CHUNK_SIZE}"
            ),
            false,
        ));
    }
    Ok(chunk_size)
}

/// 发布比例及可持久化会话偏移，不自行推进偏移。
pub(super) fn notify_resume_progress(
    session: &ResumeSession,
    offset: u64,
    total_size: u64,
    on_progress: Option<&ProgressFn>,
    on_resume_progress: Option<&ResumeProgressFn>,
) {
    if let Some(callback) = on_progress {
        let ratio = if total_size == 0 {
            1.0
        } else {
            offset as f64 / total_size as f64
        };
        callback(ratio.clamp(0.0, 1.0));
    }
    if let Some(callback) = on_resume_progress {
        callback(
            &session.server_id,
            &session.upload_id,
            offset,
            &session.session_url,
        );
    }
}

/// 构造“写请求可能已到达服务端”的恢复型错误。
pub(super) fn remote_ambiguity(cause: &str, auth_already_replayed: bool) -> AppError {
    AppError::drive_transport_with_submission(
        DriveTransportKind::Decode,
        true,
        auth_already_replayed,
        Some(cause),
    )
}

/// 判断错误是否要求沿同一会话远端核验而非重新新建。
pub(super) fn is_remote_ambiguity(error: &AppError) -> bool {
    match error {
        AppError::DriveApi {
            request_may_have_reached_server: true,
            transport_kind: Some(_),
            ..
        } => true,
        AppError::DriveApi {
            error_code: Some(error_code),
            ..
        } => error_code == "upload_session_expired",
        _ => false,
    }
}

/// 只在请求明确未到服务端的连接失败上做短暂本地重试。401 已在 `put_chunk` 内完成
/// 唯一一次刷新重放；任何可能已提交的写入都必须交回持久化恢复策略。
pub(super) fn should_retry_chunk_locally(error: &AppError) -> bool {
    matches!(
        error,
        AppError::DriveApi {
            status_code: None,
            transport_kind: Some(DriveTransportKind::Connect),
            request_may_have_reached_server: false,
            ..
        }
    )
}

/// 判断错误是否已消耗唯一一次认证刷新重放。
pub(super) fn auth_already_replayed(error: &AppError) -> bool {
    matches!(
        error,
        AppError::DriveApi {
            auth_already_replayed: true,
            ..
        }
    )
}

/// 读取上传错误响应，并将失效会话与普通 HTTP 失败区分。
pub(super) async fn upload_response_error(
    response: reqwest::Response,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
    session_sensitive: bool,
) -> AppError {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let body = response.text().await.unwrap_or_default();
    let upper = body.to_ascii_uppercase();
    if session_sensitive
        && (status == 404
            || status == 410
            || upper.contains("CONTENT_NOT_FOUND")
            || upper.contains("UPLOAD_ID_NOT_FOUND"))
    {
        return AppError::drive_upload_session_expired(status, auth_already_replayed);
    }
    AppError::drive_from_response(status, &body, retry_after, semantics, auth_already_replayed)
}

/// 构造 metadata JSON（multipart 路径用普通 JSON，容忍 UTF-8，不需 asciiJsonEncode）。
pub(super) fn build_metadata_json(file_name: &str, parent_id: Option<&str>) -> String {
    let mut meta = serde_json::Map::new();
    meta.insert("fileName".into(), Value::String(file_name.to_string()));
    if let Some(pid) = parent_id {
        if !pid.is_empty() {
            meta.insert(
                "parentFolder".into(),
                Value::Array(vec![Value::String(pid.to_string())]),
            );
        }
    }
    Value::Object(meta).to_string()
}
