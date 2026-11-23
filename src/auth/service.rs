//! Auth 编排服务 —— OAuth 授权流程 + code 交换 + token 生命周期管理。
//!
//! 对齐 `legacy/lib/auth/auth_service.dart`。
//!
//! # 关键编码细节（华为 API 怪癖）
//! - scope 用空格分隔，空格替换为 `%20`（**不**用 URL 编码整个 scope，`/` 不编码）
//! - code 交换时手工拼接 form body，用 [`percent_encoding`] 精确编码每个值
//!   （authorization_code 含 `+ / =`，form-urlencoded 会把 `+` 当空格 → invalid code 1101）

use std::sync::Arc;

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
use crate::auth::token_store::global_store;
use crate::constants;
use crate::error::{AppError, AppResult};

/// Auth 服务：编排授权流程、token 刷新、登出。
///
/// 对齐 dart `AuthService`。
pub struct AuthService {
    refresher: Arc<TokenRefresher>,
    /// 当前授权流程的 PKCE verifier（仅在 authorize() 期间有效）
    current_verifier: Mutex<Option<String>>,
    /// 是否被用户取消授权
    cancelled: Mutex<bool>,
    /// 当前 OAuth 回调 server 停止句柄
    current_oauth_stop: Mutex<Option<OauthServerStopHandle>>,
    /// 最近一次构造的授权页 URL。
    ///
    /// 内嵌登录窗口被身份提供方拦截时，前端可以一键改用系统浏览器继续同一次流程
    /// （loopback 服务仍在等待，code 仍会回到这里）。
    last_authorize_url: Mutex<Option<String>>,
    http: reqwest::Client,
}

/// 启动恢复阶段刷新失败后的登录态处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreRefreshFailureAction {
    ReturnError,
    ClearLogin,
}

/// 网络故障保留 token，其余刷新失败清理失效登录态。
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
        Self {
            refresher: Arc::new(TokenRefresher::new()),
            current_verifier: Mutex::new(None),
            cancelled: Mutex::new(false),
            current_oauth_stop: Mutex::new(None),
            last_authorize_url: Mutex::new(None),
            http: super::http_client(),
        }
    }

    /// 启动时恢复登录态：加载 token，若将过期则刷新。
    /// 对齐 dart `restore()`。返回是否已登录。
    pub async fn restore(&self) -> AppResult<bool> {
        match global_store().load()? {
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

    /// 启动 OAuth 授权流程：拉起授权页 → 等待回调 → 换 token → 持久化。
    ///
    /// 授权页优先用应用内嵌窗口打开（`login_window`），创建失败时自动回退系统浏览器；
    /// 成功后 token 已落盘，`current_token` 可用。对齐 dart `authorize()`。
    pub async fn authorize(
        &self,
        app: Option<&tauri::AppHandle>,
        port: u16,
        sheet_bounds: Option<super::login_window::SheetBounds>,
    ) -> AppResult<TokenPair> {
        tracing::info!(port, "开始 OAuth 授权流程");

        let state = generate_state();
        let pkce = generate_pkce();
        *self.current_verifier.lock().await = Some(pkce.code_verifier.clone());
        let redirect_uri = build_redirect_uri(port);
        *self.cancelled.lock().await = false;

        // 1. 启动 loopback 监听
        let server = OauthServer::start(port).await?;
        *self.current_oauth_stop.lock().await = Some(server.stop_handle());

        // 2. 构造授权 URL 并拉起授权页（内嵌优先，失败回退浏览器）
        let auth_url = build_authorize_url(&redirect_uri, &state, &pkce);
        *self.last_authorize_url.lock().await = Some(auth_url.clone());
        tracing::info!("打开授权页：{auth_url}");
        if !self.launch_authorize_page(app, &auth_url, sheet_bounds) {
            self.abort_oauth_server(server).await;
            return Err(AppError::auth_browser_launch_failed());
        }

        // 3. 等待回调并完成交换；无论成功、失败还是取消，都要收掉登录窗口。
        let outcome = self
            .complete_authorization(server, &redirect_uri, &state)
            .await;
        if let Some(app) = app {
            super::login_window::close(app);
        }
        *self.last_authorize_url.lock().await = None;
        outcome
    }

    /// 拉起授权页：优先内嵌窗口，创建失败时回退系统浏览器。
    ///
    /// 返回 false 表示两种方式都失败，调用方应直接报错并收掉 loopback 服务。
    fn launch_authorize_page(
        &self,
        app: Option<&tauri::AppHandle>,
        auth_url: &str,
        sheet_bounds: Option<super::login_window::SheetBounds>,
    ) -> bool {
        if let Some(app) = app {
            if let Some(surface) = super::login_window::open(app, auth_url, sheet_bounds) {
                // 告知前端实际形态：非抽屉时它要把抽屉外壳收起来。
                emit_login_launched(app, surface.as_str());
                return true;
            }
            tracing::warn!("内嵌登录不可用，回退系统浏览器");
            emit_login_launched(app, "browser");
        }
        open_browser(auth_url)
    }

    /// 启动失败时收掉 loopback 服务，避免端口与监听任务泄漏。
    async fn abort_oauth_server(&self, server: OauthServer) {
        *self.current_oauth_stop.lock().await = None;
        server.stop().await;
    }

    /// 等待回调并完成校验、换 token、持久化。
    async fn complete_authorization(
        &self,
        server: OauthServer,
        redirect_uri: &str,
        state: &str,
    ) -> AppResult<TokenPair> {
        // 等待回调。
        let callback_result = server.wait_for_callback().await;
        *self.current_oauth_stop.lock().await = None;
        // 取消检测优先于回调结果：用户可能先取消、回调随后才到。
        if *self.cancelled.lock().await {
            return Err(AppError::auth_cancelled());
        }
        let callback = callback_result?;

        // 校验回调（state 必须与本次授权一致）。
        self.validate_callback(&callback, state)?;

        // 换 token（带 PKCE code_verifier）。
        tracing::info!("收到授权码，换取 token...");
        let verifier = self.current_verifier.lock().await.clone();
        let code = callback.code.clone().ok_or_else(AppError::auth_cancelled)?;
        let token = self
            .exchange_code_for_token(&code, redirect_uri, verifier.as_deref())
            .await?;

        // 持久化。
        global_store().save(&token)?;
        self.refresher.set_current(token.clone());
        tracing::info!("OAuth 授权流程完成 ✓");
        Ok(token)
    }

    /// 用系统浏览器继续当前授权流程（内嵌窗口被拦截时的回退入口）。
    ///
    /// 返回是否真的打开了浏览器；没有进行中的授权流程时返回 false。
    pub async fn reopen_in_browser(&self) -> bool {
        let url = self.last_authorize_url.lock().await.clone();
        match url {
            Some(url) => open_browser(&url),
            None => false,
        }
    }

    /// 取消正在进行的授权流程。对齐 dart `cancelAuthorize()`。
    pub async fn cancel_authorize(&self) {
        let stop = self.current_oauth_stop.lock().await.take();
        let Some(stop) = stop else {
            // 没有进行中的授权流程：登录刚成功或已超时，忽略这次取消。
            tracing::debug!("忽略无进行中流程的取消请求");
            return;
        };
        *self.cancelled.lock().await = true;
        stop.stop();
        tracing::info!("用户取消授权");
    }

    /// 退出登录：清空存储 + 内存（F-AUTH-05）。对齐 dart `logout()`。
    pub async fn logout(&self) -> AppResult<()> {
        global_store().clear()?;
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
    /// 使用全局 token store 构造默认授权服务。
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

/// 广播登录界面的实际形态，供前端收起不适用的抽屉外壳。
fn emit_login_launched(app: &tauri::AppHandle, mode: &str) {
    use tauri_specta::Event;
    let _ = crate::ipc::AuthLoginLaunchedEvent {
        mode: mode.to_string(),
    }
    .emit(app);
}

/// 使用桌面环境的默认浏览器打开授权 URL。
fn open_browser(url: &str) -> bool {
    crate::platform::opener::open(url).is_ok()
}
