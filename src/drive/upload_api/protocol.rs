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

/// 收尾响应是否显式声明了 `size`。
///
/// 华为收尾 2xx 有时只回 `id`/`fileName`，缺 `size` 与「回填 0」语义完全不同：
/// 前者是元数据未生成，后者是真实的 0 字节文件，必须分开判定。
pub(super) fn declares_size(body: &Value) -> bool {
    body.get("size").is_some_and(|value| !value.is_null())
}

/// 仅在文件身份、长度及可选名称完整匹配时接受最终结果。
///
/// `size` 缺失时按 `id + name` 接受，并用本地已发送长度补全；显式 `size`
/// 与本地不一致时一律拒绝——只容忍字段缺失，不容忍数值矛盾。
pub(super) fn complete_upload_file(
    body: &Value,
    expected_size: u64,
    expected_name: Option<&str>,
) -> Option<DriveFile> {
    let mut file = DriveFile::from_json(body)?;
    let name_matches = !file.name.trim().is_empty()
        && expected_name.is_none_or(|expected| file.name.as_str() == expected);
    // 身份不完整时一律拒绝：无法证明这次写入属于本任务。
    if file.id.trim().is_empty() || !name_matches {
        return None;
    }
    if declares_size(body) {
        // 服务端明确给出长度就必须与本地一致，0 字节也不能放过。
        if file.size < 0 || file.size as u64 != expected_size {
            return None;
        }
    } else {
        // 未回填长度：按任务快照补全，避免结算层把 0 当成真实长度。
        tracing::warn!(
            id = %file.id,
            expected_size,
            "收尾响应未回填 size，按本地快照补全"
        );
        file.size = i64::try_from(expected_size).unwrap_or(i64::MAX);
    }
    Some(file)
}

/// 服务端是否已给出可证明的对象身份（`id` 与非空名称）。
///
/// 用于区分「收尾元数据尚未生成」与「响应根本不是一次成功的写入」。
pub(super) fn has_object_identity(body: &Value) -> bool {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());
    let name = body
        .get("fileName")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .is_some_and(|name| !name.trim().is_empty());
    id && name
}

/// 服务端报告的已接收范围解析结果。
///
/// 华为 308 的 `rangeList` 可能出现空洞（如只收到 0–17MB 与 64MB 之后），
/// 因此不能再用单一偏移表达：调用方需要知道「连续前缀」和「首个空洞起点」。
pub(super) struct ConfirmedRanges {
    /// 从 0 起连续确认到的偏移；空洞之后的字节不计入。
    pub prefix: u64,
    /// 服务端报告的首个空洞起点；无空洞时为 None。
    pub gap: Option<u64>,
}

/// 只接受不重叠、不倒序且不越界的已接收范围。
///
/// 空数组明确表示 0；缺字段、重叠、倒序或格式异常都不能回退成本地
/// `offset + chunk_len`。空洞不算协议错误，由调用方按 `gap` 回退重发。
pub(super) fn parse_confirmed_ranges(body: &Value, total_size: u64) -> AppResult<ConfirmedRanges> {
    let ranges = body
        .get("rangeList")
        .and_then(Value::as_array)
        .ok_or_else(|| remote_ambiguity("308 响应缺少 rangeList", false))?;
    if ranges.is_empty() {
        return Ok(ConfirmedRanges {
            prefix: 0,
            gap: None,
        });
    }

    let mut prefix = 0_u64;
    let mut gap: Option<u64> = None;
    // 上一条范围的排他终点，用于拒绝重叠与倒序。
    let mut previous_end = 0_u64;
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
        // 重叠或倒序说明服务端报告自相矛盾，不放宽。
        if end < start || end >= total_size || start < previous_end {
            return Err(remote_ambiguity(
                &format!(
                    "上传范围重叠、倒序或越界：{raw}，上一条终点 {}，总长度 {total_size}",
                    previous_end.saturating_sub(1)
                ),
                false,
            ));
        }
        if start <= prefix {
            // 与已有前缀相接：连续前缀继续向前推进。
            prefix = end
                .checked_add(1)
                .ok_or_else(|| remote_ambiguity("上传范围终点溢出", false))?;
        } else if gap.is_none() {
            // 首次出现断层：记录空洞起点，之后的字节都不计入前缀。
            gap = Some(prefix);
            tracing::warn!(
                prefix,
                next_start = start,
                "服务端已接收范围出现空洞，回退到连续前缀重发"
            );
        }
        previous_end = end
            .checked_add(1)
            .ok_or_else(|| remote_ambiguity("上传范围终点溢出", false))?;
    }
    Ok(ConfirmedRanges { prefix, gap })
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
///
/// cause 不会进入对用户可见的 message（`AppError` 的 Display 只输出稳定文案），
/// 必须在此落日志，否则所有歧义失败在结算日志里都坍缩成一句“云端响应异常”。
pub(super) fn remote_ambiguity(cause: &str, auth_already_replayed: bool) -> AppError {
    tracing::warn!(
        cause,
        auth_already_replayed,
        "上传写入结果歧义，转入远端核验"
    );
    AppError::drive_transport_with_submission(
        DriveTransportKind::Decode,
        true,
        auth_already_replayed,
    )
}

/// 上传收尾 2xx 响应未通过完整 File 校验时，打印原始 body（截断 2KB）与解析摘要。
/// 华为响应 schema 漂移的取证手段：2026-09-15 起 finalize 大面积校验失败，
/// 真实响应字段必须先落日志，才能据此修正解析器。
pub(super) fn log_incomplete_final_body(
    context: &str,
    body: &Value,
    expected_size: u64,
    expected_name: Option<&str>,
) {
    let raw = body.to_string();
    let truncated: String = raw.chars().take(2048).collect();
    let parsed = DriveFile::from_json(body);
    tracing::warn!(
        context,
        expected_size,
        expected_name = expected_name.unwrap_or(""),
        parsed_id = parsed.as_ref().map(|f| f.id.as_str()).unwrap_or(""),
        parsed_name = parsed.as_ref().map(|f| f.name.as_str()).unwrap_or(""),
        parsed_size = parsed.as_ref().map(|f| f.size).unwrap_or(-1),
        body = %truncated,
        "上传收尾响应未通过完整 File 校验"
    );
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
///
/// 4xx/5xx 的响应体是唯一能区分 403 成因（配额/风控/权限）的现场证据，
/// 必须在返回前截断落日志；否则结算日志里只剩一句“云端请求失败 (403)”。
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
    let truncated: String = body.chars().take(1024).collect();
    tracing::warn!(
        status,
        session_sensitive,
        auth_already_replayed,
        error_code = upload_error_code(&body).unwrap_or_default(),
        forbidden_reason = if status == 403 {
            classify_forbidden_reason(&body)
        } else {
            ""
        },
        body = %truncated,
        "上传请求返回错误响应"
    );
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

/// 取上传错误响应里的稳定错误码，取不到时返回 None。
fn upload_error_code(body: &str) -> Option<String> {
    crate::error::parse_huawei_error_code(body)
}

/// 把 403 按响应体特征归为配额、风控或权限，仅用于日志归因。
///
/// 终态策略暂不改动：403 仍按权限类永久失败处理，避免在风控场景下形成死循环。
fn classify_forbidden_reason(body: &str) -> &'static str {
    let upper = body.to_ascii_uppercase();
    if upper.contains("QUOTA") || upper.contains("SPACE") || upper.contains("CAPACITY") {
        "quota"
    } else if upper.contains("RISK")
        || upper.contains("SECURITY")
        || upper.contains("SENSITIVE")
        || upper.contains("RATE")
    {
        "risk"
    } else {
        "permission"
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 收尾响应缺 `size` 时按 id + name 接受，并用本地已发送长度补全。
    #[test]
    fn complete_upload_file_tolerates_missing_size() {
        let body = json!({"id": "f1", "fileName": "a.mp4"});
        let file = complete_upload_file(&body, 1024, Some("a.mp4")).expect("缺 size 也应接受");
        assert_eq!(file.id, "f1");
        assert_eq!(file.size, 1024);
    }

    /// 显式回填 0 与缺失 `size` 语义不同，长度不符必须拒绝。
    #[test]
    fn complete_upload_file_rejects_explicit_zero_size() {
        let body = json!({"id": "f1", "fileName": "a.mp4", "size": 0});
        assert!(complete_upload_file(&body, 1024, Some("a.mp4")).is_none());
    }

    /// 名称不符或 id 缺失时无法证明本次写入归属，一律拒绝。
    #[test]
    fn complete_upload_file_rejects_mismatched_identity() {
        let wrong_name = json!({"id": "f1", "fileName": "b.mp4", "size": 1024});
        assert!(complete_upload_file(&wrong_name, 1024, Some("a.mp4")).is_none());
        let no_id = json!({"fileName": "a.mp4", "size": 1024});
        assert!(complete_upload_file(&no_id, 1024, Some("a.mp4")).is_none());
    }

    /// 连续区间推进到末尾时前缀等于总长度且无空洞。
    #[test]
    fn confirmed_ranges_accept_contiguous() {
        let body = json!({"rangeList": ["0-99", "100-199"]});
        let ranges = parse_confirmed_ranges(&body, 200).expect("连续区间合法");
        assert_eq!(ranges.prefix, 200);
        assert!(ranges.gap.is_none());
    }

    /// 出现空洞时只返回连续前缀，并给出空洞起点，供上层回退补发。
    #[test]
    fn confirmed_ranges_report_first_gap() {
        let body = json!({"rangeList": ["0-99", "65536-131071"]});
        let ranges = parse_confirmed_ranges(&body, 200_000).expect("有空洞不算协议错误");
        assert_eq!(ranges.prefix, 100);
        assert_eq!(ranges.gap, Some(100));
    }

    /// 重叠、倒序或越界仍是协议错误，不放宽。
    #[test]
    fn confirmed_ranges_reject_overlap_and_overflow() {
        let overlap = json!({"rangeList": ["0-99", "50-149"]});
        assert!(parse_confirmed_ranges(&overlap, 200).is_err());
        let overflow = json!({"rangeList": ["0-200"]});
        assert!(parse_confirmed_ranges(&overflow, 200).is_err());
        let reversed = json!({"rangeList": ["100-199", "0-99"]});
        assert!(parse_confirmed_ranges(&reversed, 200).is_err());
    }
}
