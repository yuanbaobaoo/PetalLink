//! 统一异常类型 —— 业务层抛出这些类型，UI 层据此渲染友好提示。
//!
//! 对齐 `legacy/lib/core/errors.dart`。
//!
//! # 安全
//! 所有 Display/serde 输出均不泄露 token（§3.2）。错误消息只包含用户可读的中文描述。

use serde::Serialize;
use thiserror::Error;

/// 请求的副作用语义。传输层据此保守记录失败时写入是否可能已到达服务端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSemantics {
    Read,
    Write,
}

impl RequestSemantics {
    pub const fn is_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// reqwest 传输失败发生的阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveTransportKind {
    Network,
    Connect,
    Timeout,
    Request,
    ResponseBody,
    Decode,
    Other,
}

/// 已解析的 HTTP `Retry-After`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfter {
    DelaySeconds(u64),
    AtUnixMs(i64),
}

impl RetryAfter {
    pub fn next_retry_at(self, now_ms: i64) -> i64 {
        match self {
            Self::DelaySeconds(seconds) => {
                now_ms.saturating_add((seconds.min(i64::MAX as u64 / 1_000) as i64) * 1_000)
            }
            Self::AtUnixMs(timestamp_ms) => timestamp_ms.max(now_ms),
        }
    }
}

/// 解析 `Retry-After` 的 delta-seconds 或 IMF-fixdate 形式。
pub fn parse_retry_after(value: &str) -> Option<RetryAfter> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(RetryAfter::DelaySeconds(seconds));
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| RetryAfter::AtUnixMs(date.timestamp_millis()))
}

/// 所有自定义异常基类。序列化为前端可解析的扁平结构。
///
/// 自定义 Serialize 把字段提到顶层（`kind`/`code`/`message`/`status_code`/`error_code`），
/// `message` 始终是字符串。这样前端 `AppError.message: string` 直接可读，
/// 避免默认 tagged-enum 序列化把 payload 嵌套进 `message` 导致渲染成 `[object Object]`。
///
/// `code` 字段供前端按错误类别渲染（登录态切换 / toast 文案 / 阻塞弹窗）。
#[derive(Debug, Clone, Error)]
pub enum AppError {
    /// OAuth 流程相关（取消 / state 不匹配 / 超时 / 被拒绝 / 浏览器打不开）
    #[error("{message}")]
    Auth {
        code: AuthErrorCode,
        message: String,
    },

    /// Token 相关（未登录 / 刷新失败）
    #[error("{message}")]
    Token {
        code: TokenErrorCode,
        message: String,
    },

    /// Drive API 调用异常（状态码 / 华为错误码 / 网络）
    #[error("{message}")]
    DriveApi {
        code: DriveApiErrorCode,
        message: String,
        status_code: Option<u16>,
        error_code: Option<String>,
        retry_after: Option<RetryAfter>,
        transport_kind: Option<DriveTransportKind>,
        request_may_have_reached_server: bool,
        auth_already_replayed: bool,
    },

    /// 配置相关
    #[error("{message}")]
    Config { message: String },

    /// 配额不足（上传前校验，需求 §2.8 第三阶段）
    #[error("{message}")]
    QuotaExceeded {
        required: i64,
        remaining: i64,
        message: String,
    },

    /// 通用错误（文件系统、序列化等）
    #[error("{message}")]
    Generic { message: String },
}

/// 自定义序列化：扁平结构，`message` 始终为字符串，匹配前端 `AppError` 接口。
/// 形如 `{"kind":"Token","code":"refresh_failed","message":"...","status_code":null,"error_code":null}`。
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // 前端兼容形状固定为五个字段。
        match self {
            AppError::Auth { code, message } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "Auth")?;
                s.serialize_field("code", code)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", &None::<u16>)?;
                s.serialize_field("error_code", &None::<String>)?;
                s.end()
            }
            AppError::Token { code, message } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "Token")?;
                s.serialize_field("code", code)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", &None::<u16>)?;
                s.serialize_field("error_code", &None::<String>)?;
                s.end()
            }
            AppError::DriveApi {
                code,
                message,
                status_code,
                error_code,
                ..
            } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "DriveApi")?;
                s.serialize_field("code", code)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", status_code)?;
                s.serialize_field("error_code", error_code)?;
                s.end()
            }
            AppError::Config { message } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "Config")?;
                s.serialize_field("code", &None::<&str>)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", &None::<u16>)?;
                s.serialize_field("error_code", &None::<String>)?;
                s.end()
            }
            AppError::QuotaExceeded {
                required,
                remaining,
                message,
            } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "QuotaExceeded")?;
                s.serialize_field("code", &None::<&str>)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", &None::<u16>)?;
                s.serialize_field("error_code", &None::<String>)?;
                // required/remaining 不暴露到前端（前端不消费，避免冗余）
                let _ = (required, remaining);
                s.end()
            }
            AppError::Generic { message } => {
                let mut s = serializer.serialize_struct("AppError", 5)?;
                s.serialize_field("kind", "Generic")?;
                s.serialize_field("code", &None::<&str>)?;
                s.serialize_field("message", message)?;
                s.serialize_field("status_code", &None::<u16>)?;
                s.serialize_field("error_code", &None::<String>)?;
                s.end()
            }
        }
    }
}

/// OAuth 错误子码（对齐 dart `AuthException` 工厂）
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorCode {
    Cancelled,
    StateMismatch,
    Timeout,
    Denied,
    BrowserLaunchFailed,
    InvalidCode,
    TokenResponseInvalid,
    ScopeInvalid,
}

/// Token 错误子码（对齐 dart `TokenException` 工厂）
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenErrorCode {
    NotLoggedIn,
    RefreshFailed,
}

/// Drive API 错误子码（对齐 dart `DriveApiException` 工厂）
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriveApiErrorCode {
    /// 通用 HTTP 状态码错误
    FromStatus,
    /// 配额不足
    QuotaExceeded,
    /// 网络连接失败
    Network,
}

impl AppError {
    /// 仅从结构化 Drive 元数据读取 HTTP 状态；绝不解析用户可读消息。
    pub const fn drive_status(&self) -> Option<u16> {
        match self {
            Self::DriveApi { status_code, .. } => *status_code,
            _ => None,
        }
    }

    // ===== Auth 工厂 =====
    /// 用户主动取消授权（非错误，UI 不应显示为失败）
    pub fn auth_cancelled() -> Self {
        Self::Auth {
            code: AuthErrorCode::Cancelled,
            message: "用户取消授权".to_string(),
        }
    }

    /// state 不匹配（防 CSRF）
    pub fn auth_state_mismatch() -> Self {
        Self::Auth {
            code: AuthErrorCode::StateMismatch,
            message: "授权回调 state 校验失败，请重试".to_string(),
        }
    }

    /// 回调超时
    pub fn auth_timeout() -> Self {
        Self::Auth {
            code: AuthErrorCode::Timeout,
            message: "登录超时，请重新登录".to_string(),
        }
    }

    /// 华为返回 error 参数
    pub fn auth_denied(error_description: Option<&str>) -> Self {
        let message = match error_description {
            Some(desc) => format!("授权失败：{desc}"),
            None => "授权被拒绝".to_string(),
        };
        Self::Auth {
            code: AuthErrorCode::Denied,
            message,
        }
    }

    /// 浏览器无法打开
    pub fn auth_browser_launch_failed() -> Self {
        Self::Auth {
            code: AuthErrorCode::BrowserLaunchFailed,
            message: "无法打开浏览器，请检查系统设置".to_string(),
        }
    }

    /// 未收到授权码
    pub fn auth_invalid_code() -> Self {
        Self::Auth {
            code: AuthErrorCode::InvalidCode,
            message: "未收到授权码".to_string(),
        }
    }

    /// token 响应格式异常
    pub fn auth_token_response_invalid() -> Self {
        Self::Auth {
            code: AuthErrorCode::TokenResponseInvalid,
            message: "token 响应格式异常".to_string(),
        }
    }

    // ===== Token 工厂 =====
    /// 尚未登录
    pub fn token_not_logged_in() -> Self {
        Self::Token {
            code: TokenErrorCode::NotLoggedIn,
            message: "尚未登录".to_string(),
        }
    }

    /// Token 刷新失败
    pub fn token_refresh_failed(cause: Option<&str>) -> Self {
        let message = match cause {
            Some(c) => format!("Token 刷新失败：{c}"),
            None => "Token 刷新失败，请重新登录".to_string(),
        };
        Self::Token {
            code: TokenErrorCode::RefreshFailed,
            message,
        }
    }

    // ===== DriveApi 工厂 =====
    /// 从 HTTP 状态码构造（华为 4xx 错误体在 body 里携带 code/description）
    pub fn drive_from_status(status_code: u16, body: &str) -> Self {
        Self::drive_from_response(status_code, body, None, RequestSemantics::Read, false)
    }

    /// 从服务端错误响应构造，保留恢复策略需要的结构化元数据。
    pub fn drive_from_response(
        status_code: u16,
        body: &str,
        retry_after: Option<RetryAfter>,
        semantics: RequestSemantics,
        auth_already_replayed: bool,
    ) -> Self {
        Self::drive_from_response_with_submission(
            status_code,
            body,
            retry_after,
            semantics.is_write(),
            auth_already_replayed,
        )
    }

    /// 从已知提交阶段的服务端响应构造，供直接请求迁移时复用。
    pub fn drive_from_response_with_submission(
        status_code: u16,
        body: &str,
        retry_after: Option<RetryAfter>,
        request_may_have_reached_server: bool,
        auth_already_replayed: bool,
    ) -> Self {
        Self::DriveApi {
            code: DriveApiErrorCode::FromStatus,
            message: format!("云端请求失败 ({status_code})"),
            status_code: Some(status_code),
            error_code: parse_huawei_error_code(body),
            retry_after,
            transport_kind: None,
            request_may_have_reached_server,
            auth_already_replayed,
        }
        .with_cause_body(body)
    }

    /// 配额不足
    pub fn drive_quota_exceeded() -> Self {
        Self::DriveApi {
            code: DriveApiErrorCode::QuotaExceeded,
            message: "云盘空间不足".to_string(),
            status_code: None,
            error_code: Some("quota_exceeded".to_string()),
            retry_after: None,
            transport_kind: None,
            request_may_have_reached_server: false,
            auth_already_replayed: false,
        }
    }

    /// 网络连接失败
    pub fn drive_network(cause: Option<&str>) -> Self {
        Self::drive_transport(
            DriveTransportKind::Network,
            RequestSemantics::Read,
            false,
            cause,
        )
    }

    /// 从传输失败构造，供 DriveClient 及直接上传/下载请求复用。
    pub fn drive_transport(
        transport_kind: DriveTransportKind,
        semantics: RequestSemantics,
        auth_already_replayed: bool,
        cause: Option<&str>,
    ) -> Self {
        Self::drive_transport_with_submission(
            transport_kind,
            semantics.is_write() && transport_kind != DriveTransportKind::Connect,
            auth_already_replayed,
            cause,
        )
    }

    /// 从已知提交阶段的传输失败构造；直接流式请求可显式保留提交不确定性。
    pub fn drive_transport_with_submission(
        transport_kind: DriveTransportKind,
        request_may_have_reached_server: bool,
        auth_already_replayed: bool,
        cause: Option<&str>,
    ) -> Self {
        let message = match transport_kind {
            DriveTransportKind::Decode => "云端响应异常",
            _ => "网络连接失败，请检查网络",
        };
        Self::DriveApi {
            code: DriveApiErrorCode::Network,
            message: message.to_string(),
            status_code: None,
            error_code: None,
            retry_after: None,
            transport_kind: Some(transport_kind),
            request_may_have_reached_server,
            auth_already_replayed,
        }
        .with_cause_body(cause.unwrap_or(""))
    }

    // ===== Config / Quota 工厂 =====
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    pub fn quota_exceeded(required: i64, remaining: i64) -> Self {
        Self::QuotaExceeded {
            required,
            remaining,
            message: format!("空间不足：需要 {required} 字节，剩余 {remaining} 字节"),
        }
    }

    pub fn generic(message: impl Into<String>) -> Self {
        Self::Generic {
            message: message.into(),
        }
    }

    /// 附加 cause（仅用于内部诊断，不序列化到前端）
    fn with_cause_body(self, _body: &str) -> Self {
        // body 仅记录到日志，不透出到前端 message（避免泄露）
        self
    }
}

fn parse_huawei_error_code(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let error_code = value
        .get("errorCode")
        .or_else(|| value.get("error").and_then(|error| error.get("errorCode")))?;
    match error_code {
        serde_json::Value::String(code) => Some(code.clone()),
        serde_json::Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

/// Tauri 命令统一返回 Result<T, AppError>。
/// AppError 已实现 Serialize，可直接作为 command 的错误类型。
pub type AppResult<T> = Result<T, AppError>;

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Generic {
            message: format!("文件操作失败：{e}"),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Generic {
            message: format!("数据解析失败：{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_cancelled_message() {
        let e = AppError::auth_cancelled();
        assert!(matches!(
            e,
            AppError::Auth {
                code: AuthErrorCode::Cancelled,
                ..
            }
        ));
        assert_eq!(e.to_string(), "用户取消授权");
    }

    #[test]
    fn test_quota_message() {
        let e = AppError::quota_exceeded(100, 50);
        assert!(matches!(e, AppError::QuotaExceeded { .. }));
        assert!(e.to_string().contains("需要 100"));
    }

    #[test]
    fn test_drive_from_status() {
        let e = AppError::drive_from_status(404, "not found body");
        match e {
            AppError::DriveApi { status_code, .. } => {
                assert_eq!(status_code, Some(404));
            }
            _ => panic!("应为 DriveApi"),
        }
    }

    #[test]
    fn test_serde_flat_structure() {
        // 序列化后 message 必须是字符串（非嵌套对象），kind/code 在顶层
        let e = AppError::auth_denied(Some("用户拒绝"));
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "Auth");
        assert_eq!(v["code"], "denied");
        // message 是字符串而非嵌套对象（修复 [object Object] 渲染 bug）
        assert_eq!(v["message"], "授权失败：用户拒绝");
        assert!(v["message"].is_string());
        assert!(v.get("status_code").is_some());
    }

    #[test]
    fn test_serde_network_vs_refresh_distinct() {
        // 网络错误 → DriveApi/network（「网络连接失败」）；token 刷新失败 → Token/refresh_failed
        // 两者 kind/code 不同，前端据此渲染不同文案
        let net = AppError::drive_network(Some("timeout"));
        let refresh = AppError::token_refresh_failed(Some("invalid_grant"));
        let nv: serde_json::Value = serde_json::to_value(&net).unwrap();
        let rv: serde_json::Value = serde_json::to_value(&refresh).unwrap();
        assert_eq!(nv["kind"], "DriveApi");
        assert_eq!(nv["code"], "network");
        assert_eq!(nv["message"], "网络连接失败，请检查网络");
        assert_eq!(rv["kind"], "Token");
        assert_eq!(rv["code"], "refresh_failed");
        assert!(rv["message"].as_str().unwrap().contains("Token 刷新失败"));
        assert_ne!(nv["kind"], rv["kind"]);
    }

    #[test]
    fn test_serde_driveapi_carries_status_code() {
        // DriveApi 变体透出 status_code / error_code（顶层）
        let e = AppError::drive_from_status(404, "not found");
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["kind"], "DriveApi");
        assert_eq!(v["status_code"], 404);
        assert!(v["message"].is_string());
    }

    #[test]
    fn drive_response_preserves_numeric_huawei_code_and_retry_after() {
        let error = AppError::drive_from_response(
            429,
            r#"{"errorCode":21004002,"errorDescription":"slow down"}"#,
            Some(RetryAfter::DelaySeconds(17)),
            RequestSemantics::Write,
            true,
        );

        match error {
            AppError::DriveApi {
                status_code,
                error_code,
                retry_after,
                transport_kind,
                request_may_have_reached_server,
                auth_already_replayed,
                ..
            } => {
                assert_eq!(status_code, Some(429));
                assert_eq!(error_code.as_deref(), Some("21004002"));
                assert_eq!(retry_after, Some(RetryAfter::DelaySeconds(17)));
                assert_eq!(transport_kind, None);
                assert!(request_may_have_reached_server);
                assert!(auth_already_replayed);
            }
            other => panic!("expected DriveApi, got {other:?}"),
        }
    }

    #[test]
    fn drive_response_preserves_string_huawei_code() {
        let error = AppError::drive_from_response(
            400,
            r#"{"errorCode":"21004002"}"#,
            None,
            RequestSemantics::Read,
            false,
        );

        assert!(matches!(
            error,
            AppError::DriveApi {
                error_code: Some(ref code),
                ..
            } if code == "21004002"
        ));
    }

    #[test]
    fn retry_after_parser_accepts_delta_seconds_and_http_date() {
        assert_eq!(
            parse_retry_after("120"),
            Some(RetryAfter::DelaySeconds(120))
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(RetryAfter::AtUnixMs(784_111_777_000))
        );
        assert_eq!(parse_retry_after("not-a-retry-after"), None);
    }

    #[test]
    fn write_transport_metadata_distinguishes_connect_from_timeout() {
        let connect = AppError::drive_transport(
            DriveTransportKind::Connect,
            RequestSemantics::Write,
            false,
            Some("connect failed"),
        );
        let timeout = AppError::drive_transport(
            DriveTransportKind::Timeout,
            RequestSemantics::Write,
            true,
            Some("timed out"),
        );

        assert!(matches!(
            connect,
            AppError::DriveApi {
                transport_kind: Some(DriveTransportKind::Connect),
                request_may_have_reached_server: false,
                auth_already_replayed: false,
                ..
            }
        ));
        assert!(matches!(
            timeout,
            AppError::DriveApi {
                transport_kind: Some(DriveTransportKind::Timeout),
                request_may_have_reached_server: true,
                auth_already_replayed: true,
                ..
            }
        ));
    }

    #[test]
    fn internal_drive_metadata_does_not_change_frontend_shape() {
        let error = AppError::drive_from_response(
            503,
            r#"{"errorCode":"busy"}"#,
            Some(RetryAfter::DelaySeconds(3)),
            RequestSemantics::Write,
            true,
        );
        let value = serde_json::to_value(error).unwrap();
        let object = value.as_object().unwrap();

        assert_eq!(object.len(), 5);
        assert!(object.get("retry_after").is_none());
        assert!(object.get("transport_kind").is_none());
        assert!(object.get("request_may_have_reached_server").is_none());
        assert!(object.get("auth_already_replayed").is_none());
    }

    #[test]
    fn structured_status_matching_never_reads_display_message() {
        let fake_status = AppError::generic("云端请求失败 (404), please retry 409");
        let structured = AppError::drive_from_status(404, "{}");

        assert_eq!(fake_status.drive_status(), None);
        assert_eq!(structured.drive_status(), Some(404));
    }
}
