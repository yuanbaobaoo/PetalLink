//! Auth 编排服务 —— OAuth 授权流程 + code 交换 + token 生命周期管理。
//!
//! 对齐 `legacy/lib/auth/auth_service.dart`。
//!
//! # 关键编码细节（华为 API 怪癖）
//! - scope 用空格分隔，空格替换为 `%20`（**不**用 URL 编码整个 scope，`/` 不编码）
//! - code 交换时手工拼接 form body，用 [`percent_encoding`] 精确编码每个值
//!   （authorization_code 含 `+ / =`，form-urlencoded 会把 `+` 当空格 → invalid code 1101）

use std::sync::Arc;
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// dart `Uri.encodeComponent` 等价集：仅 A-Za-z0-9-_.~ 不编码（RFC 3986 unreserved）。
/// 其余字符（含 / + = 空格等）全部百分号编码。
/// 关键：'+' 必须编码为 %2B，否则 form-urlencoded 会把 '+' 当空格 → invalid code 1101。
const URI_ENCODE_COMPONENT: &AsciiSet = &CONTROLS
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
    .add(b'}');

/// 编码辅助：等价于 dart `Uri.encodeComponent`。
fn enc(s: &str) -> String {
    utf8_percent_encode(s, URI_ENCODE_COMPONENT).to_string()
}
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth::models::TokenPair;
use crate::auth::oauth_server::{OauthCallbackResult, OauthServer, OauthServerStopHandle};
use crate::auth::pkce::{generate_pkce, generate_state};
use crate::auth::token_refresher::TokenRefresher;
use crate::auth::token_store::{global_store, TokenStore};
use crate::constants;
use crate::error::{AppError, AppResult};

/// Auth 服务：编排授权流程、token 刷新、登出。
///
/// 对齐 dart `AuthService`。
pub struct AuthService {
    token_store: Arc<dyn TokenStore>,
    refresher: Arc<TokenRefresher>,
    /// 当前授权流程的 PKCE verifier（仅在 authorize() 期间有效）
    current_verifier: Mutex<Option<String>>,
    /// 是否被用户取消授权
    cancelled: Mutex<bool>,
    /// 当前 OAuth 回调 server 停止句柄
    current_oauth_stop: Mutex<Option<OauthServerStopHandle>>,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreRefreshFailureAction {
    ReturnError,
    ClearLogin,
}

fn restore_refresh_failure_action(error: &AppError) -> RestoreRefreshFailureAction {
    match error {
        AppError::DriveApi {
            code: crate::error::DriveApiErrorCode::Network,
            ..
        } => RestoreRefreshFailureAction::ReturnError,
        _ => RestoreRefreshFailureAction::ClearLogin,
    }
}

impl AuthService {
    /// 使用全局 token store 构造单例。
    pub fn new() -> Self {
        let token_store: Arc<dyn TokenStore> = Arc::new(GlobalStoreWrapper);
        let refresher = Arc::new(TokenRefresher::new(token_store.clone()));
        Self {
            token_store,
            refresher,
            current_verifier: Mutex::new(None),
            cancelled: Mutex::new(false),
            current_oauth_stop: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("构建 reqwest client 失败"),
        }
    }

    /// 启动时恢复登录态：加载 token，若将过期则刷新。
    /// 对齐 dart `restore()`。返回是否已登录。
    pub async fn restore(&self) -> AppResult<bool> {
        match self.token_store.load()? {
            Some(token) => {
                self.refresher.set_current(token.clone());
                if token.will_expire_within(constants::TOKEN_EXPIRY_BUFFER_SECS) {
                    match self.refresher.refresh().await {
                        Ok(_) => Ok(true),
                        Err(e) => match restore_refresh_failure_action(&e) {
                            RestoreRefreshFailureAction::ReturnError => {
                                tracing::warn!(error = %e, "恢复登录态时刷新失败，保留本地 token 并返回错误");
                                Err(e)
                            }
                            RestoreRefreshFailureAction::ClearLogin => {
                                tracing::warn!(error = %e, "恢复登录态时 token 被拒绝，登出");
                                self.logout().await?;
                                Ok(false)
                            }
                        },
                    }
                } else {
                    Ok(true)
                }
            }
            None => Ok(false),
        }
    }

    /// 启动 OAuth 授权流程：打开浏览器 → 等待回调 → 换 token → 持久化。
    /// 对齐 dart `authorize()`。成功后 token 已存存储，currentToken 可用。
    pub async fn authorize(&self, port: u16) -> AppResult<TokenPair> {
        tracing::info!(port, "开始 OAuth 授权流程");

        let state = generate_state();
        let pkce = generate_pkce();
        *self.current_verifier.lock().await = Some(pkce.code_verifier.clone());
        let redirect_uri = build_redirect_uri(port);
        *self.cancelled.lock().await = false;

        // 1. 启动 loopback 监听
        let server = OauthServer::start(port).await?;
        *self.current_oauth_stop.lock().await = Some(server.stop_handle());

        // 2. 构造授权 URL 并打开浏览器
        let auth_url = build_authorize_url(&redirect_uri, &state, &pkce);
        tracing::info!("打开授权页：{auth_url}");
        let launched = open_browser(&auth_url);
        if !launched {
            *self.current_oauth_stop.lock().await = None;
            server.stop().await;
            return Err(AppError::auth_browser_launch_failed());
        }

        // 3. 等待回调
        let callback_result = server.wait_for_callback().await;
        *self.current_oauth_stop.lock().await = None;
        // finally：server 已在 wait_for_callback 内 stop

        // 4. 用户取消检测
        if *self.cancelled.lock().await {
            return Err(AppError::auth_cancelled());
        }

        let callback = callback_result?;

        // 5. 校验回调
        self.validate_callback(&callback, &state)?;

        // 6. 换 token（带 PKCE code_verifier）
        tracing::info!("收到授权码，换取 token...");
        let verifier = self.current_verifier.lock().await.clone();
        let token = self
            .exchange_code_for_token(
                &callback.code.clone().unwrap(),
                &redirect_uri,
                verifier.as_deref(),
            )
            .await?;

        // 7. 持久化
        self.token_store.save(&token)?;
        self.refresher.set_current(token.clone());
        tracing::info!("OAuth 授权流程完成 ✓");
        Ok(token)
    }

    /// 取消正在进行的授权流程。对齐 dart `cancelAuthorize()`。
    pub async fn cancel_authorize(&self) {
        *self.cancelled.lock().await = true;
        if let Some(stop) = self.current_oauth_stop.lock().await.take() {
            stop.stop();
        }
        tracing::info!("用户取消授权");
    }

    /// 退出登录：清空存储 + 内存（F-AUTH-05）。对齐 dart `logout()`。
    pub async fn logout(&self) -> AppResult<()> {
        self.token_store.clear()?;
        self.refresher.clear_current();
        tracing::info!("已退出登录");
        Ok(())
    }

    /// 确保拥有有效 token，必要时刷新（供 Drive 客户端调用）。
    /// 对齐 dart `ensureValidAccessToken()`。返回 access_token 字符串。
    pub async fn ensure_valid_access_token(&self) -> AppResult<String> {
        let token = self.refresher.current_token().await?;
        let token = token.ok_or_else(AppError::token_not_logged_in)?;
        if token.will_expire_within(constants::TOKEN_EXPIRY_BUFFER_SECS) {
            let refreshed = self.refresher.refresh().await?;
            return Ok(refreshed.access_token);
        }
        Ok(token.access_token)
    }

    /// 校验回调结果。对齐 dart authorize() 第 5 步。
    fn validate_callback(
        &self,
        callback: &OauthCallbackResult,
        expected_state: &str,
    ) -> AppResult<()> {
        if let Some(error) = &callback.error {
            // 华为 OAuth 错误码识别（1101 + invalid scope → 明确指引）
            if error == "1101" {
                let desc_lower = callback
                    .error_description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase();
                if desc_lower.contains("scope") {
                    let scope_list = constants::SCOPES.join(", ");
                    return Err(AppError::Auth {
                        code: crate::error::AuthErrorCode::Denied,
                        message: format!(
                            "授权失败：scope 未在 AppGallery Connect 后台授权\n\
                             错误码：error=1101 sub_error={}\n\
                             当前请求的 scope：{scope_list}\n\n\
                             请在 AGC 后台「API 管理」和「OAuth 2.0 凭据 → 作用域」两处\
                             勾选上述所有 scope 后重试。",
                            callback.sub_error.as_deref().unwrap_or("N/A")
                        ),
                    });
                }
            }
            return Err(AppError::auth_denied(
                callback.error_description.as_deref().or(Some(error)),
            ));
        }
        if callback.code.is_none() {
            return Err(AppError::auth_invalid_code());
        }
        if callback.state.as_deref() != Some(expected_state) {
            tracing::warn!(
                expected = expected_state,
                got = ?callback.state,
                "state 不匹配"
            );
            return Err(AppError::auth_state_mismatch());
        }
        Ok(())
    }

    /// 用授权码换 token。手工拼接 form body 防止 `+` 被当空格。
    /// 对齐 dart `_exchangeCodeForToken`。
    async fn exchange_code_for_token(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: Option<&str>,
    ) -> AppResult<TokenPair> {
        // 关键：authorization_code 含 '+' '/' '='，form-urlencoded 会把 '+' 当空格。
        // 手工拼接 form body，用 enc（dart Uri.encodeComponent 等价）对每个值精确编码。
        let mut parts = vec![
            format!("grant_type={}", enc("authorization_code")),
            format!("code={}", enc(code)),
            format!("client_id={}", enc(constants::resolved_client_id())),
            format!(
                "client_secret={}",
                enc(&constants::resolved_client_secret())
            ),
            format!("redirect_uri={}", enc(redirect_uri)),
        ];
        if let Some(verifier) = code_verifier {
            parts.push(format!("code_verifier={}", enc(verifier)));
        }
        let body = parts.join("&");

        let resp = self
            .http
            .post(constants::TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::generic(format!("换 token 失败：{e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::generic(format!("换 token 失败：{e}")))?;
        let data: Value =
            serde_json::from_str(&text).map_err(|_| AppError::auth_token_response_invalid())?;

        if data.get("access_token").is_none() {
            let desc = data
                .get("error_description")
                .and_then(Value::as_str)
                .or_else(|| data.get("error").and_then(Value::as_str))
                .unwrap_or(&text);
            tracing::error!(status = %status, "换 token 失败：{desc}");
            return Err(AppError::generic(format!("换 token 失败：{desc}")));
        }

        TokenPair::from_token_response(&data).ok_or_else(AppError::auth_token_response_invalid)
    }

    /// 获取 token refresher 引用（供 DriveClient 401 重放用）。
    pub fn refresher(&self) -> &Arc<TokenRefresher> {
        &self.refresher
    }
}

impl Default for AuthService {
    fn default() -> Self {
        Self::new()
    }
}

/// 构造 redirect_uri：`http://127.0.0.1:<port>/oauth/callback`。
/// 对齐 dart `_buildRedirectUri`。
fn build_redirect_uri(port: u16) -> String {
    format!(
        "http://{}:{}{}",
        constants::LOOPBACK_HOST,
        port,
        constants::CALLBACK_PATH
    )
}

/// 构造授权 URL。
///
/// 关键：scope 用空格分隔，空格替换为 `%20`（不整体编码，`/` 保留）。
/// 对齐 dart `_buildAuthorizeUrl`。
pub fn build_authorize_url(
    redirect_uri: &str,
    state: &str,
    pkce: &crate::auth::pkce::PkcePair,
) -> String {
    let scope_raw = constants::SCOPES.join(" ");
    // 其余参数用 enc（dart Uri.encodeComponent 等价）编码
    let query = [
        format!("response_type={}", enc("code")),
        format!("client_id={}", enc(constants::resolved_client_id())),
        format!("redirect_uri={}", enc(redirect_uri)),
        format!("state={}", enc(state)),
        format!("access_type={}", enc("offline")),
        format!("code_challenge={}", enc(&pkce.code_challenge)),
        format!("code_challenge_method={}", enc("S256")),
    ]
    .join("&");
    // scope 不整体编码，空格用 %20（华为接受）
    let scope_encoded = scope_raw.replace(' ', "%20");
    format!("{}?{query}&scope={scope_encoded}", constants::AUTHORIZE_URL)
}

/// 打开系统浏览器（macOS 用 `open` 命令）。
fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        false
    }
}

/// 包装全局加密 token 存储为 Arc<dyn TokenStore>。
/// global_store 返回 &'static，但 trait object 需要 Arc；此处每次调用转发。
struct GlobalStoreWrapper;

impl TokenStore for GlobalStoreWrapper {
    fn load(&self) -> AppResult<Option<TokenPair>> {
        global_store().load()
    }
    fn save(&self, token: &TokenPair) -> AppResult<()> {
        global_store().save(token)
    }
    fn clear(&self) -> AppResult<()> {
        global_store().clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::pkce::generate_pkce;

    #[test]
    fn test_build_redirect_uri() {
        let uri = build_redirect_uri(9999);
        assert_eq!(uri, "http://127.0.0.1:9999/oauth/callback");
    }

    #[test]
    fn test_build_authorize_url_scope_not_encoded() {
        let pkce = generate_pkce();
        let url = build_authorize_url("http://127.0.0.1:9999/oauth/callback", "mystate", &pkce);
        // 应包含授权端点
        assert!(url.starts_with(constants::AUTHORIZE_URL));
        // scope 中的 / 不应被编码（华为要求）
        assert!(url.contains("scope=openid%20profile%20https://www.huawei.com/auth/drive"));
        assert!(!url.contains("drive%2F"));
        // 含 PKCE challenge
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={}", pkce.code_challenge)));
    }

    #[test]
    fn test_build_authorize_url_params() {
        let pkce = generate_pkce();
        let url = build_authorize_url("http://127.0.0.1:9999/oauth/callback", "st", &pkce);
        assert!(url.contains("response_type=code"));
        assert!(url.contains(&format!("client_id={}", constants::resolved_client_id())));
        assert!(url.contains("state=st"));
        assert!(url.contains("access_type=offline"));
    }

    #[test]
    fn test_validate_callback_success() {
        let svc = AuthService::new();
        let cb = OauthCallbackResult {
            code: Some("abc".into()),
            state: Some("st".into()),
            ..Default::default()
        };
        assert!(svc.validate_callback(&cb, "st").is_ok());
    }

    #[test]
    fn test_validate_callback_state_mismatch() {
        let svc = AuthService::new();
        let cb = OauthCallbackResult {
            code: Some("abc".into()),
            state: Some("other".into()),
            ..Default::default()
        };
        let err = svc.validate_callback(&cb, "st").unwrap_err();
        assert!(
            matches!(err, AppError::Auth { code, .. } if code == crate::error::AuthErrorCode::StateMismatch)
        );
    }

    #[test]
    fn test_validate_callback_1101_scope_error() {
        let svc = AuthService::new();
        let cb = OauthCallbackResult {
            error: Some("1101".into()),
            error_description: Some("invalid scope".into()),
            sub_error: Some("20042".into()),
            ..Default::default()
        };
        let err = svc.validate_callback(&cb, "st").unwrap_err();
        match err {
            AppError::Auth { message, .. } => {
                assert!(message.contains("scope 未在 AppGallery Connect"));
                assert!(message.contains("20042"));
            }
            _ => panic!("应为 Auth 错误"),
        }
    }

    #[test]
    fn test_validate_callback_no_code() {
        let svc = AuthService::new();
        let cb = OauthCallbackResult {
            code: None,
            state: Some("st".into()),
            ..Default::default()
        };
        let err = svc.validate_callback(&cb, "st").unwrap_err();
        assert!(
            matches!(err, AppError::Auth { code, .. } if code == crate::error::AuthErrorCode::InvalidCode)
        );
    }

    #[test]
    fn test_restore_refresh_failure_action_keeps_token_on_network_error() {
        let err = AppError::drive_network(Some("timeout"));
        assert_eq!(
            restore_refresh_failure_action(&err),
            RestoreRefreshFailureAction::ReturnError
        );
    }

    #[test]
    fn test_restore_refresh_failure_action_clears_token_on_refresh_failure() {
        let err = AppError::token_refresh_failed(Some("invalid_grant"));
        assert_eq!(
            restore_refresh_failure_action(&err),
            RestoreRefreshFailureAction::ClearLogin
        );
    }
}
