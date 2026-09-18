//! DriveClient —— reqwest 客户端 + Auth 注入 + 401 自动刷新重放。
//!
//! 对齐 `legacy/lib/drive/drive_client.dart`。
//!
//! - baseURL = `driveapis.cloud.huawei.com.cn/drive/v1`
//! - 常规请求：connect 15s / 总超时 60s
//! - 流式内容传输（整文件下载）：独立 client，仅 connect 15s，无总超时
//! - 每个请求注入 Bearer token；401 时强制刷新并重放
//! - 网络/响应错误保留结构化恢复元数据
//!
//! 设计：不使用 reqwest-middleware（避免版本耦合），改用「构造带 token 的 RequestBuilder
//! + execute_with_retry 统一发送」模式。401 重放在 execute_with_retry 内处理。

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::{Client, Method, RequestBuilder, StatusCode};

use crate::auth::service::AuthService;
use crate::constants;
use crate::error::{parse_retry_after, AppError, AppResult, DriveTransportKind, RequestSemantics};

/// 共享的 reqwest 客户端（连接池 maxConnectionsPerHost=15，对齐 dart）。
pub struct DriveClient {
    http: Client,
    /// 流式内容传输专用客户端：无总超时（总超时会在大文件下载中途掐断流），
    /// 仅保留 connect 超时；断流/僵死由 TCP 层与任务恢复机制兜底。
    http_stream: Client,
    auth: Arc<AuthService>,
    /// Drive API base URL（默认 `DRIVE_API_BASE`）。
    base_url: String,
}

/// 随成功响应传递的请求语义与认证重放信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResponseMetadata {
    pub semantics: RequestSemantics,
    pub auth_already_replayed: bool,
}

impl DriveClient {
    /// 创建带超时、连接池和认证服务的 Drive 客户端。
    pub fn new(auth: Arc<AuthService>) -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(15)
            .build()
            .expect("构建 reqwest client 失败");
        // 流式传输（整文件下载）不能使用总超时：reqwest 的 timeout 覆盖到响应体流读完，
        // 60s 内下载不完的大文件会被强制掐断并误判为网络错误。
        let http_stream = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(15)
            .build()
            .expect("构建流式 reqwest client 失败");
        Self {
            http,
            http_stream,
            auth,
            base_url: constants::DRIVE_API_BASE.to_string(),
        }
    }

    /// 获取 auth service 引用。
    pub fn auth(&self) -> &Arc<AuthService> {
        &self.auth
    }

    /// 获取流式传输专用 reqwest client（整文件内容下载，无总超时）。
    pub fn streaming_http(&self) -> &Client {
        &self.http_stream
    }

    /// 构造请求并注入 Bearer token，返回 RequestBuilder。
    /// token 端点（含 oauth2/v3/token）不注入 auth。
    async fn build_authed(&self, method: Method, url: &str) -> AppResult<RequestBuilder> {
        let req = self.http.request(method, url);
        if url.contains("oauth2/v3/token") {
            return Ok(req);
        }
        let token = self.auth.ensure_valid_access_token().await?;
        Ok(req.bearer_auth(token))
    }

    /// 发送请求，401 时刷新 token 并重放一次。
    /// 对齐 dart AuthInterceptor.onError 的 401 重放逻辑。
    async fn execute_with_retry(
        &self,
        method: Method,
        url: &str,
        apply: impl Fn(RequestBuilder) -> RequestBuilder + Clone,
    ) -> AppResult<reqwest::Response> {
        let semantics = request_semantics(&method);
        // 第一次尝试
        let req = apply(self.build_authed(method.clone(), url).await?);
        let resp = req
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, semantics, false))?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return ensure_success_response(resp, semantics, false).await;
        }

        // 401：强制刷新后重放
        tracing::warn!("收到 401，刷新 token 后重放");
        let new_token = self.auth.refresher().refresh().await?;
        let req = apply(self.build_authed_with_token(method, url, &new_token.access_token)?);
        let resp = req
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, semantics, true))?;
        ensure_success_response(resp, semantics, true).await
    }

    /// 构造带指定 token 的请求（重放用，不再 ensureValidAccessToken）。
    fn build_authed_with_token(
        &self,
        method: Method,
        url: &str,
        token: &str,
    ) -> AppResult<RequestBuilder> {
        Ok(self.http.request(method, url).bearer_auth(token))
    }

    /// 用任意 reqwest client 执行一次带单次 401 刷新重放的请求。
    /// `build` 必须能以最新 token 重建整个请求（body/头不可复用的场景）。
    /// 返回 (最终响应, 是否已发生认证重放)；响应状态码由调用方自行处理。
    pub async fn send_with_auth_replay(
        &self,
        http: &Client,
        semantics: RequestSemantics,
        build: impl Fn(&Client, &str) -> RequestBuilder,
    ) -> AppResult<(reqwest::Response, bool)> {
        let token = self.auth.ensure_valid_access_token().await?;
        let resp = build(http, &token)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, semantics, false))?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok((resp, false));
        }
        // 401：强制刷新后重放一次
        tracing::warn!("收到 401，刷新 token 后重放");
        let new_token = self.auth.refresher().refresh().await?;
        let resp = build(http, &new_token.access_token)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, semantics, true))?;
        Ok((resp, true))
    }

    /// GET 请求（相对 driveApiBase 路径）。只返回最终 2xx 响应。
    pub async fn get(&self, path: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.execute_with_retry(Method::GET, &url, |r| r).await
    }

    /// 向相对路径发送 POST；仅返回最终 2xx，401 最多刷新重放一次。
    pub async fn post(
        &self,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let ct = content_type.to_string();
        self.execute_with_retry(Method::POST, &url, move |r| {
            let mut r = r.header("Content-Type", &ct);
            if let Some(b) = &body {
                r = r.body(b.clone());
            }
            r
        })
        .await
    }

    /// 向相对路径发送 PATCH；传输失败会保留写请求是否可能已提交的语义。
    pub async fn patch(
        &self,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let ct = content_type.to_string();
        self.execute_with_retry(Method::PATCH, &url, move |r| {
            r.header("Content-Type", &ct).body(body.clone())
        })
        .await
    }

    /// 向相对路径发送 DELETE；仅返回最终 2xx，401 最多刷新重放一次。
    pub async fn delete(&self, path: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.execute_with_retry(Method::DELETE, &url, |r| r).await
    }
}

/// 归一化 HTTP 错误为 AppError。
/// 对齐 dart `_throwDriveError`。
pub fn classify_error(err: &reqwest::Error) -> AppError {
    classify_transport_error(err, RequestSemantics::Read, false)
}

/// 将 reqwest 传输失败映射为可供恢复策略消费的结构化错误。
pub fn classify_transport_error(
    error: &reqwest::Error,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppError {
    let kind = if error.is_connect() {
        DriveTransportKind::Connect
    } else if error.is_timeout() {
        DriveTransportKind::Timeout
    } else if error.is_body() {
        DriveTransportKind::ResponseBody
    } else if error.is_decode() {
        DriveTransportKind::Decode
    } else if error.is_request() {
        DriveTransportKind::Request
    } else {
        DriveTransportKind::Other
    };
    // Decode/ResponseBody/Request/Other 的用户文案统一坍缩为“云端响应异常”或“网络连接失败”，
    // 原始 reqwest 错误文本不落日志就无法定位（2026-09-15 上传收尾大面积失败即因此卡排查）。
    // Connect/Timeout/Network 是断网期常态，保持静默避免刷屏。
    if matches!(
        kind,
        DriveTransportKind::Decode
            | DriveTransportKind::ResponseBody
            | DriveTransportKind::Request
            | DriveTransportKind::Other
    ) {
        tracing::warn!(
            ?kind,
            ?semantics,
            auth_already_replayed,
            cause = %error,
            "请求传输层失败（解码/协议类），按结构化错误上抛"
        );
    }
    AppError::drive_transport(kind, semantics, auth_already_replayed)
}

/// 处理非 2xx 响应，返回 AppError（读取 body 用于错误码）。
pub async fn handle_error_response(resp: reqwest::Response) -> AppError {
    handle_error_response_with_metadata(resp, RequestSemantics::Read, false).await
}

/// 处理非 2xx 响应并保留请求语义、Retry-After 与认证重放状态。
pub async fn handle_error_response_with_metadata(
    resp: reqwest::Response,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let body = resp.text().await.unwrap_or_default();
    tracing::warn!(status, body = %body, "Drive API 错误响应");
    AppError::drive_from_response(status, &body, retry_after, semantics, auth_already_replayed)
}

/// 按 HTTP 方法区分只读请求与可能已提交的写请求。
fn request_semantics(method: &Method) -> RequestSemantics {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        RequestSemantics::Read
    } else {
        RequestSemantics::Write
    }
}

/// 判断最终响应是否必须转换为结构化 HTTP 错误。
fn must_reject_final_status(status: StatusCode) -> bool {
    !status.is_success()
}

/// 仅放行最终 2xx 响应；其余状态读取错误体后返回失败。
async fn ensure_success_response(
    mut response: reqwest::Response,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) -> AppResult<reqwest::Response> {
    if must_reject_final_status(response.status()) {
        return Err(handle_error_response_with_metadata(
            response,
            semantics,
            auth_already_replayed,
        )
        .await);
    }
    attach_response_metadata(&mut response, semantics, auth_already_replayed);
    Ok(response)
}

/// 将请求语义与认证重放状态附加到成功响应。
fn attach_response_metadata(
    response: &mut reqwest::Response,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
) {
    response.extensions_mut().insert(ResponseMetadata {
        semantics,
        auth_already_replayed,
    });
}

/// 读取响应元数据；外部响应缺失扩展时使用给定语义且视为未重放。
pub(crate) fn response_metadata(
    response: &reqwest::Response,
    fallback_semantics: RequestSemantics,
) -> ResponseMetadata {
    response
        .extensions()
        .get::<ResponseMetadata>()
        .copied()
        .unwrap_or(ResponseMetadata {
            semantics: fallback_semantics,
            auth_already_replayed: false,
        })
}

/// 统一「检查状态码 → 解析 JSON」两步模式。
///
/// 替代散布在 about_api / changes_api / files_api 等处的重复代码：
/// ```ignore
/// if !resp.status().is_success() { return Err(handle_error_response(resp).await); }
/// let body: Value = resp.json().await.map_err(|e| AppError::generic(format!("解析XX响应失败：{e}")))?;
/// ```
///
/// - 非 2xx → `handle_error_response` 归一化为 AppError
/// - JSON 解析失败 → 带 Decode/提交语义的 `AppError::DriveApi`
pub async fn parse_json_response(
    resp: reqwest::Response,
    ctx: &str,
) -> AppResult<serde_json::Value> {
    parse_json_response_with_semantics(resp, ctx, RequestSemantics::Read).await
}

/// 检查状态并按请求语义解析 JSON；写响应解析失败保留 post-submit 不确定性。
pub async fn parse_json_response_with_semantics(
    resp: reqwest::Response,
    ctx: &str,
    semantics: RequestSemantics,
) -> AppResult<serde_json::Value> {
    let metadata = response_metadata(&resp, semantics);
    if !resp.status().is_success() {
        return Err(handle_error_response_with_metadata(
            resp,
            metadata.semantics,
            metadata.auth_already_replayed,
        )
        .await);
    }
    resp.json().await.map_err(|error| {
        response_decode_error(
            ctx,
            metadata.semantics,
            metadata.auth_already_replayed,
            &error.to_string(),
        )
    })
}

/// 构造成功响应后的 JSON/schema 解码错误。
pub fn response_decode_error(
    ctx: &str,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
    cause: &str,
) -> AppError {
    // 用户可见错误保持稳定且不回显响应体；把无凭据的解析上下文写入本地日志，
    // 否则所有 schema/身份校验失败都会坍缩成无法定位的“云端响应异常”。
    tracing::warn!(
        context = ctx,
        ?semantics,
        auth_already_replayed,
        cause,
        "Drive API 响应解码或协议校验失败"
    );
    AppError::drive_transport(DriveTransportKind::Decode, semantics, auth_already_replayed)
}
