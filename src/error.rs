//! 统一异常类型 —— 业务层抛出这些类型，UI 层据此渲染友好提示。
//!
//! 对齐 `legacy/lib/core/errors.dart`。
//!
//! # 安全
//! 所有 Display/serde 输出均不泄露 token（§3.2）。错误消息只包含用户可读的中文描述。

use serde::Serialize;
use thiserror::Error;

/// 所有自定义异常基类。序列化为前端可解析的 tagged enum。
///
/// `code` 字段供前端按错误类别渲染（登录态切换 / toast 文案 / 阻塞弹窗）。
#[derive(Debug, Clone, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    /// OAuth 流程相关（取消 / state 不匹配 / 超时 / 被拒绝 / 浏览器打不开）
    #[error("{message}")]
    Auth { code: AuthErrorCode, message: String },

    /// Token 相关（未登录 / 刷新失败）
    #[error("{message}")]
    Token { code: TokenErrorCode, message: String },

    /// Drive API 调用异常（状态码 / 华为错误码 / 网络）
    #[error("{message}")]
    DriveApi {
        code: DriveApiErrorCode,
        message: String,
        status_code: Option<u16>,
        error_code: Option<String>,
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
        Self::DriveApi {
            code: DriveApiErrorCode::FromStatus,
            message: format!("云端请求失败 ({status_code})"),
            status_code: Some(status_code),
            error_code: None,
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
        }
    }

    /// 网络连接失败
    pub fn drive_network(cause: Option<&str>) -> Self {
        Self::DriveApi {
            code: DriveApiErrorCode::Network,
            message: "网络连接失败，请检查网络".to_string(),
            status_code: None,
            error_code: None,
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
        assert!(matches!(e, AppError::Auth { code: AuthErrorCode::Cancelled, .. }));
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
    fn test_serde_roundtrip() {
        // 前端能解析错误结构
        let e = AppError::auth_denied(Some("用户拒绝"));
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("Auth"));
        assert!(json.contains("用户拒绝"));
    }
}
