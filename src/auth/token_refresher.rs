//! Token 刷新器（需求 F-AUTH-04）。
//!
//! 对齐 `legacy/lib/auth/token_refresher.dart`。
//!
//! 用 refresh_token 换新的 access_token。并发调用共享同一个 in-flight 结果。
//! 华为刷新响应可能不含新 refresh_token → 沿用旧的。

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::Notify;

use crate::auth::models::{now_ms, TokenPair};
use crate::auth::token_store::TokenStore;
use crate::constants;
use crate::error::{AppError, AppResult};

/// 将刷新请求的传输阶段标志映射为稳定错误类别。
fn classify_refresh_transport_flags(
    is_timeout: bool,
    is_connect: bool,
    is_body: bool,
    cause: &str,
) -> AppError {
    if is_connect {
        return AppError::drive_transport(
            crate::error::DriveTransportKind::Connect,
            crate::error::RequestSemantics::Read,
            false,
            Some(cause),
        );
    }
    if is_timeout {
        return AppError::drive_transport(
            crate::error::DriveTransportKind::Timeout,
            crate::error::RequestSemantics::Read,
            false,
            Some(cause),
        );
    }
    if is_body {
        return AppError::drive_transport(
            crate::error::DriveTransportKind::ResponseBody,
            crate::error::RequestSemantics::Read,
            false,
            Some(cause),
        );
    }
    AppError::token_refresh_failed(Some(cause))
}

/// 保存当前共享刷新任务，确保并发请求只执行一次刷新。
#[derive(Default)]
struct RefreshSingleflight {
    active: Mutex<Option<Arc<RefreshFlight>>>,
}

/// 承载一次共享刷新任务的结果与完成通知。
struct RefreshFlight {
    result: Mutex<Option<AppResult<TokenPair>>>,
    completed: Notify,
}

/// 保证刷新主任务退出时始终发布结果并清理活动槽位。
struct RefreshLeaderGuard<'a> {
    singleflight: &'a RefreshSingleflight,
    flight: Arc<RefreshFlight>,
    armed: bool,
}

impl<'a> RefreshLeaderGuard<'a> {
    /// 为指定共享刷新任务创建仍处于生效状态的守卫。
    fn new(singleflight: &'a RefreshSingleflight, flight: Arc<RefreshFlight>) -> Self {
        Self {
            singleflight,
            flight,
            armed: true,
        }
    }

    /// 发布刷新结果、唤醒等待者并释放活动槽位。
    fn complete(&mut self, result: AppResult<TokenPair>) {
        *self.flight.result.lock() = Some(result);
        self.flight.completed.notify_waiters();
        self.singleflight.clear_if_active(&self.flight);
        self.armed = false;
    }
}

impl Drop for RefreshLeaderGuard<'_> {
    /// 主任务被取消时向等待者发布取消错误，避免永久等待。
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let cancellation = AppError::token_refresh_failed(Some("刷新任务被取消"));
        let mut result = self.flight.result.lock();
        if result.is_none() {
            *result = Some(Err(cancellation));
        }
        drop(result);
        self.flight.completed.notify_waiters();
        self.singleflight.clear_if_active(&self.flight);
    }
}

impl RefreshFlight {
    /// 创建尚未完成的共享刷新任务状态。
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            completed: Notify::new(),
        }
    }
}

impl RefreshSingleflight {
    /// 由首个调用者执行刷新，其余调用者等待并复用同一结果。
    async fn run<F, Fut>(&self, operation: F) -> AppResult<TokenPair>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<TokenPair>>,
    {
        let (flight, is_leader) = {
            let mut active = self.active.lock();
            match active.as_ref() {
                Some(flight) => (flight.clone(), false),
                None => {
                    let flight = Arc::new(RefreshFlight::new());
                    *active = Some(flight.clone());
                    (flight, true)
                }
            }
        };

        if is_leader {
            let mut leader_guard = RefreshLeaderGuard::new(self, flight);
            let result = operation().await;
            leader_guard.complete(result.clone());
            return result;
        }

        loop {
            let completed = flight.completed.notified();
            if let Some(result) = flight.result.lock().clone() {
                return result;
            }
            completed.await;
        }
    }

    /// 仅在给定任务仍为当前活动任务时清空槽位。
    fn clear_if_active(&self, flight: &Arc<RefreshFlight>) {
        let mut active = self.active.lock();
        if active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, flight))
        {
            *active = None;
        }
    }
}

/// Token 刷新器。
///
/// 并发去重：刷新期间所有并发调用共享同一次成功或失败结果。
pub struct TokenRefresher {
    token_store: Arc<dyn TokenStore>,
    http: reqwest::Client,
    /// 同一时刻的并发刷新共享同一个完成结果（成功或失败）。
    refresh_flight: RefreshSingleflight,
    /// 当前持有的 token（内存缓存，避免每次刷新都读存储）
    current: Mutex<Option<TokenPair>>,
}

impl TokenRefresher {
    /// 使用指定令牌存储构造刷新器。
    pub fn new(token_store: Arc<dyn TokenStore>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            token_store,
            http,
            refresh_flight: RefreshSingleflight::default(),
            current: Mutex::new(None),
        }
    }

    /// 更新内存中的 token 缓存。
    pub fn set_current(&self, token: TokenPair) {
        *self.current.lock() = Some(token);
    }

    /// 清空内存中的 token 缓存。
    pub fn clear_current(&self) {
        *self.current.lock() = None;
    }

    /// 获取当前 token（优先内存缓存，回退存储）。
    pub async fn current_token(&self) -> AppResult<Option<TokenPair>> {
        if let Some(t) = self.current.lock().clone() {
            return Ok(Some(t));
        }
        self.token_store.load()
    }

    /// 刷新 token 并持久化。返回新 token。
    /// 并发调用共享同一次刷新结果（成功与失败均共享）。
    /// 对齐 dart `TokenRefresher.refresh()`。
    pub async fn refresh(&self) -> AppResult<TokenPair> {
        self.refresh_with(|| self.perform_refresh()).await
    }

    /// 通过并发去重器执行给定刷新操作。
    async fn refresh_with<F, Fut>(&self, operation: F) -> AppResult<TokenPair>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<TokenPair>>,
    {
        self.refresh_flight.run(operation).await
    }

    /// 请求新访问令牌，先原子替换持久化文件，成功后再更新内存副本。
    async fn perform_refresh(&self) -> AppResult<TokenPair> {
        let current = self
            .current_token()
            .await?
            .ok_or_else(AppError::token_not_logged_in)?;

        tracing::info!("开始刷新 token...");
        let body = [
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", current.refresh_token.clone()),
            ("client_id", constants::resolved_client_id().to_string()),
            ("client_secret", constants::resolved_client_secret()),
        ];
        // 刷新用 form-urlencoded（refresh_token 无特殊字符，对齐 dart）
        let resp = self
            .http
            .post(constants::TOKEN_URL)
            .form(&body)
            .send()
            .await
            // 区分网络错误 vs token 刷新失败：超时/连接/响应体中断 → 网络连接失败，
            // 其余（含真正的 token 拒绝）→ token 刷新失败。对齐 drive::client::classify_error
            .map_err(|error| {
                classify_refresh_transport_flags(
                    error.is_timeout(),
                    error.is_connect(),
                    error.is_body(),
                    &error.to_string(),
                )
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|error| {
            classify_refresh_transport_flags(
                error.is_timeout(),
                error.is_connect(),
                error.is_body(),
                &error.to_string(),
            )
        })?;
        let data: Value = serde_json::from_str(&text)
            .map_err(|e| AppError::token_refresh_failed(Some(&e.to_string())))?;

        let access_token = data
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                let cause = data
                    .get("error_description")
                    .and_then(Value::as_str)
                    .or_else(|| data.get("error").and_then(Value::as_str));
                AppError::token_refresh_failed(cause)
            })?;

        // 华为刷新响应可能不含新 refresh_token → 沿用旧的
        let refresh_token = data
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or(&current.refresh_token)
            .to_string();
        let expires_in = data
            .get("expires_in")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(3600);
        let token_type = data
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or("Bearer")
            .to_string();
        let scope = data
            .get("scope")
            .and_then(Value::as_str)
            .map(String::from)
            .or(current.scope);

        let new_token = TokenPair {
            access_token: access_token.to_string(),
            refresh_token,
            expires_at: now_ms() + expires_in * 1000,
            token_type,
            scope,
        };

        self.token_store.save(&new_token)?;
        *self.current.lock() = Some(new_token.clone());
        tracing::info!("token 刷新成功");
        let _ = status; // 状态码已隐含在 data 解析中
        Ok(new_token)
    }
}
