//! DriveClient —— reqwest 客户端 + Auth 注入 + 401 自动刷新重放。
//!
//! 对齐 `legacy/lib/drive/drive_client.dart`。
//!
//! - baseURL = `driveapis.cloud.huawei.com.cn/drive/v1`
//! - connect 15s / receive 60s / send 60s
//! - 每个请求注入 Bearer token；401 时强制刷新并重放
//! - 网络错误归一化为 [`AppError::drive_network`]
//!
//! 设计：不使用 reqwest-middleware（避免版本耦合），改用「构造带 token 的 RequestBuilder
//! + execute_with_retry 统一发送」模式。401 重放在 execute_with_retry 内处理。

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, StatusCode};

use crate::auth::service::AuthService;
use crate::constants;
use crate::error::{AppError, AppResult};

/// 共享的 reqwest 客户端（连接池 maxConnectionsPerHost=15，对齐 dart）。
pub struct DriveClient {
    http: Client,
    auth: Arc<AuthService>,
    /// Drive API base URL（默认 `DRIVE_API_BASE`；测试可注入 wiremock 地址）。
    base_url: String,
}

impl DriveClient {
    pub fn new(auth: Arc<AuthService>) -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(15)
            .build()
            .expect("构建 reqwest client 失败");
        Self { http, auth, base_url: constants::DRIVE_API_BASE.to_string() }
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
        Self { http, auth, base_url }
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
    async fn execute_with_retry(&self, method: Method, url: &str, apply: impl Fn(RequestBuilder) -> RequestBuilder + Clone) -> AppResult<reqwest::Response> {
        // 第一次尝试
        let req = apply(self.build_authed(method.clone(), url).await?);
        let resp = req
            .send()
            .await
            .map_err(|e| classify_error(&e))?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }

        // 401：强制刷新后重放
        tracing::warn!("收到 401，刷新 token 后重放");
        let new_token = self.auth.refresher().refresh().await?;
        let req = apply(self.build_authed_with_token(method, url, &new_token.access_token)?);
        let resp = req.send().await.map_err(|e| classify_error(&e))?;
        Ok(resp)
    }

    /// 构造带指定 token 的请求（重放用，不再 ensureValidAccessToken）。
    fn build_authed_with_token(&self, method: Method, url: &str, token: &str) -> AppResult<RequestBuilder> {
        Ok(self.http.request(method, url).bearer_auth(token))
    }

    /// GET 请求（相对 driveApiBase 路径）。返回原始响应（调用方处理状态码）。
    pub async fn get(&self, path: &str) -> AppResult<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        self.execute_with_retry(Method::GET, &url, |r| r).await
    }

    /// POST 请求。
    pub async fn post(&self, path: &str, body: Option<Vec<u8>>, content_type: &str) -> AppResult<reqwest::Response> {
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
    pub async fn patch(&self, path: &str, body: Vec<u8>, content_type: &str) -> AppResult<reqwest::Response> {
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
    pub async fn post_full(&self, url: &str, body: Option<Vec<u8>>, content_type: &str) -> AppResult<reqwest::Response> {
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
    pub async fn patch_full(&self, url: &str, body: Vec<u8>, content_type: &str) -> AppResult<reqwest::Response> {
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
    if err.is_timeout() || err.is_connect() {
        AppError::drive_network(Some(&err.to_string()))
    } else {
        AppError::drive_from_status(0, &err.to_string())
    }
}

/// 处理非 2xx 响应，返回 AppError（读取 body 用于错误码）。
pub async fn handle_error_response(resp: reqwest::Response) -> AppError {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    tracing::warn!(status, body = %body, "Drive API 错误响应");
    AppError::drive_from_status(status, &body)
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
/// - JSON 解析失败 → `AppError::generic("解析{ctx}响应失败：{e}")`
pub async fn parse_json_response(resp: reqwest::Response, ctx: &str) -> AppResult<serde_json::Value> {
    if !resp.status().is_success() {
        return Err(handle_error_response(resp).await);
    }
    resp.json()
        .await
        .map_err(|e| AppError::generic(format!("解析{ctx}响应失败：{e}")))
}
