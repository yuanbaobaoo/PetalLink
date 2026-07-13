//! DriveClient —— reqwest 客户端 + Auth 注入 + 401 自动刷新重放。
//!
//! 对齐 `legacy/lib/drive/drive_client.dart`。
//!
//! - baseURL = `driveapis.cloud.huawei.com.cn/drive/v1`
//! - connect 15s / receive 60s / send 60s
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
    auth: Arc<AuthService>,
    /// Drive API base URL（默认 `DRIVE_API_BASE`；测试可注入 wiremock 地址）。
    base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResponseMetadata {
    pub semantics: RequestSemantics,
    pub auth_already_replayed: bool,
}

impl DriveClient {
    pub fn new(auth: Arc<AuthService>) -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(15)
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            http,
            auth,
            base_url: constants::DRIVE_API_BASE.to_string(),
        }
    }

    /// 测试用：注入自定义 base URL（如 wiremock 地址）。
    #[cfg(test)]
    pub fn with_base_url(auth: Arc<AuthService>, base_url: String) -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(15)
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            http,
            auth,
            base_url,
        }
    }

    /// 获取 auth service 引用。
    pub fn auth(&self) -> &Arc<AuthService> {
        &self.auth
    }

    /// 获取底层 reqwest client（upload/download 等需自定义 URL 时用）。
    pub fn raw_http(&self) -> &Client {
        &self.http
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

    /// GET 请求（相对 driveApiBase 路径）。只返回最终 2xx 响应。
    pub async fn get(&self, path: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.execute_with_retry(Method::GET, &url, |r| r).await
    }

    /// POST 请求。
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

    /// PATCH 请求。
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

    /// DELETE 请求。
    pub async fn delete(&self, path: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.execute_with_retry(Method::DELETE, &url, |r| r).await
    }

    /// GET 请求（完整 URL，不拼接 base_url）。
    /// 供 FilesApi 等已自行构造完整 URL 的调用方使用，复用统一的 auth + 401 重放逻辑。
    pub async fn get_full(&self, url: &str) -> AppResult<reqwest::Response> {
        self.execute_with_retry(Method::GET, url, |r| r).await
    }

    /// POST 请求（完整 URL）。
    pub async fn post_full(
        &self,
        url: &str,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        let ct = content_type.to_string();
        self.execute_with_retry(Method::POST, url, move |r| {
            let mut r = r.header("Content-Type", &ct);
            if let Some(b) = &body {
                r = r.body(b.clone());
            }
            r
        })
        .await
    }

    /// PATCH 请求（完整 URL）。
    pub async fn patch_full(
        &self,
        url: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> AppResult<reqwest::Response> {
        let ct = content_type.to_string();
        self.execute_with_retry(Method::PATCH, url, move |r| {
            r.header("Content-Type", &ct).body(body.clone())
        })
        .await
    }

    /// DELETE 请求（完整 URL）。
    pub async fn delete_full(&self, url: &str) -> AppResult<reqwest::Response> {
        self.execute_with_retry(Method::DELETE, url, |r| r).await
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
    AppError::drive_transport(
        kind,
        semantics,
        auth_already_replayed,
        Some(&error.to_string()),
    )
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

fn request_semantics(method: &Method) -> RequestSemantics {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        RequestSemantics::Read
    } else {
        RequestSemantics::Write
    }
}

fn must_reject_final_status(status: StatusCode) -> bool {
    !status.is_success()
}

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
    let diagnostic = format!("解析{ctx}响应失败：{cause}");
    AppError::drive_transport(
        DriveTransportKind::Decode,
        semantics,
        auth_already_replayed,
        Some(&diagnostic),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DriveTransportKind;

    #[test]
    fn final_status_rejects_every_non_success_including_second_401() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(must_reject_final_status(status), "status={status}");
        }
        assert!(!must_reject_final_status(StatusCode::OK));
        assert!(!must_reject_final_status(StatusCode::NO_CONTENT));
    }

    #[test]
    fn response_decode_error_preserves_write_submission_uncertainty() {
        let write =
            response_decode_error("createFolder", RequestSemantics::Write, true, "missing id");
        let read = response_decode_error("list", RequestSemantics::Read, false, "invalid json");

        assert_eq!(write.to_string(), "云端响应异常");
        assert!(matches!(
            write,
            AppError::DriveApi {
                transport_kind: Some(DriveTransportKind::Decode),
                request_may_have_reached_server: true,
                auth_already_replayed: true,
                ..
            }
        ));
        assert!(matches!(
            read,
            AppError::DriveApi {
                transport_kind: Some(DriveTransportKind::Decode),
                request_may_have_reached_server: false,
                auth_already_replayed: false,
                ..
            }
        ));
    }

    #[test]
    fn response_extension_preserves_auth_replay_for_later_decode() {
        let raw = http::Response::builder()
            .status(StatusCode::OK)
            .body(reqwest::Body::from("{}"))
            .unwrap();
        let mut response: reqwest::Response = raw.into();
        attach_response_metadata(&mut response, RequestSemantics::Write, true);

        let metadata = response_metadata(&response, RequestSemantics::Read);
        assert_eq!(metadata.semantics, RequestSemantics::Write);
        assert!(metadata.auth_already_replayed);

        let error = response_decode_error(
            "update",
            metadata.semantics,
            metadata.auth_already_replayed,
            "missing id",
        );
        assert!(matches!(
            error,
            AppError::DriveApi {
                auth_already_replayed: true,
                request_may_have_reached_server: true,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn second_401_is_structured_and_marked_as_already_replayed() {
        let raw = http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(reqwest::Body::from(r#"{"errorCode":"still-unauthorized"}"#))
            .unwrap();
        let response: reqwest::Response = raw.into();

        let error = ensure_success_response(response, RequestSemantics::Write, true)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AppError::DriveApi {
                status_code: Some(401),
                error_code: Some(ref code),
                request_may_have_reached_server: true,
                auth_already_replayed: true,
                ..
            } if code == "still-unauthorized"
        ));
    }

    #[tokio::test]
    async fn final_429_preserves_retry_after_and_numeric_huawei_code() {
        let raw = http::Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header(RETRY_AFTER, "12")
            .body(reqwest::Body::from(r#"{"errorCode":21004002}"#))
            .unwrap();
        let response: reqwest::Response = raw.into();

        let error = ensure_success_response(response, RequestSemantics::Read, false)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AppError::DriveApi {
                status_code: Some(429),
                error_code: Some(ref code),
                retry_after: Some(crate::error::RetryAfter::DelaySeconds(12)),
                ..
            } if code == "21004002"
        ));
    }
}
