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

#[derive(Default)]
struct RefreshSingleflight {
    active: Mutex<Option<Arc<RefreshFlight>>>,
}

struct RefreshFlight {
    result: Mutex<Option<AppResult<TokenPair>>>,
    completed: Notify,
}

struct RefreshLeaderGuard<'a> {
    singleflight: &'a RefreshSingleflight,
    flight: Arc<RefreshFlight>,
    armed: bool,
}

impl<'a> RefreshLeaderGuard<'a> {
    fn new(singleflight: &'a RefreshSingleflight, flight: Arc<RefreshFlight>) -> Self {
        Self {
            singleflight,
            flight,
            armed: true,
        }
    }

    fn complete(&mut self, result: AppResult<TokenPair>) {
        *self.flight.result.lock() = Some(result);
        self.flight.completed.notify_waiters();
        self.singleflight.clear_if_active(&self.flight);
        self.armed = false;
    }
}

impl Drop for RefreshLeaderGuard<'_> {
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
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            completed: Notify::new(),
        }
    }
}

impl RefreshSingleflight {
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

    fn clear_if_active(&self, flight: &Arc<RefreshFlight>) {
        let mut active = self.active.lock();
        if active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, flight))
        {
            *active = None;
        }
    }

    #[cfg(test)]
    fn has_waiter(&self) -> bool {
        self.active
            .lock()
            .as_ref()
            .is_some_and(|flight| Arc::strong_count(flight) >= 3)
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

    async fn refresh_with<F, Fut>(&self, operation: F) -> AppResult<TokenPair>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<TokenPair>>,
    {
        self.refresh_flight.run(operation).await
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::token_store::EncryptedFileStore;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct MemoryTokenStore {
        token: Mutex<Option<TokenPair>>,
    }

    impl TokenStore for MemoryTokenStore {
        fn load(&self) -> AppResult<Option<TokenPair>> {
            Ok(self.token.lock().clone())
        }

        fn save(&self, token: &TokenPair) -> AppResult<()> {
            *self.token.lock() = Some(token.clone());
            Ok(())
        }

        fn clear(&self) -> AppResult<()> {
            *self.token.lock() = None;
            Ok(())
        }
    }

    fn sample_token() -> TokenPair {
        TokenPair {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: now_ms() + 3_600_000,
            token_type: "Bearer".to_string(),
            scope: None,
        }
    }

    #[tokio::test]
    async fn test_clear_current_removes_memory_token() {
        let store = Arc::new(MemoryTokenStore::default());
        let refresher = TokenRefresher::new(store.clone());
        refresher.set_current(sample_token());
        store.clear().expect("清理存储成功");

        assert!(refresher
            .current_token()
            .await
            .expect("读取 token 成功")
            .is_some());

        refresher.clear_current();

        assert!(refresher
            .current_token()
            .await
            .expect("读取 token 成功")
            .is_none());
    }

    #[tokio::test]
    async fn test_refresh_requires_logged_in() {
        // 无 token 时刷新应返回 not_logged_in
        // 用加密文件存储：无 token.bin → load 为 None → 刷新报 NotLoggedIn
        let store = Arc::new(EncryptedFileStore);
        // 确保无 token（幂等清除，文件不存在也成功）
        let _ = store.clear();
        let refresher = TokenRefresher::new(store);
        let result = refresher.refresh().await;
        assert!(matches!(
            result,
            Err(AppError::Token { code, .. }) if code == crate::error::TokenErrorCode::NotLoggedIn
        ));
    }

    #[tokio::test]
    async fn failed_refresh_leader_shares_error_with_waiter_instead_of_old_token() {
        let store = Arc::new(MemoryTokenStore::default());
        let refresher = Arc::new(TokenRefresher::new(store));
        refresher.set_current(sample_token());

        let leader_started = Arc::new(Notify::new());
        let release_leader = Arc::new(Notify::new());
        let actual_calls = Arc::new(AtomicUsize::new(0));
        let started_wait = leader_started.notified();

        let leader = {
            let refresher = refresher.clone();
            let leader_started = leader_started.clone();
            let release_leader = release_leader.clone();
            let actual_calls = actual_calls.clone();
            tokio::spawn(async move {
                refresher
                    .refresh_with(move || async move {
                        actual_calls.fetch_add(1, AtomicOrdering::SeqCst);
                        leader_started.notify_one();
                        release_leader.notified().await;
                        Err(AppError::token_refresh_failed(Some("leader failed")))
                    })
                    .await
            })
        };
        started_wait.await;

        let waiter = {
            let refresher = refresher.clone();
            let actual_calls = actual_calls.clone();
            tokio::spawn(async move {
                refresher
                    .refresh_with(move || async move {
                        actual_calls.fetch_add(1, AtomicOrdering::SeqCst);
                        Ok(sample_token())
                    })
                    .await
            })
        };
        while !refresher.refresh_flight.has_waiter() {
            tokio::task::yield_now().await;
        }

        release_leader.notify_waiters();
        let leader_result = leader.await.unwrap();
        let waiter_result = waiter.await.unwrap();

        assert!(matches!(
            leader_result,
            Err(AppError::Token {
                code: crate::error::TokenErrorCode::RefreshFailed,
                ..
            })
        ));
        assert!(matches!(
            waiter_result,
            Err(AppError::Token {
                code: crate::error::TokenErrorCode::RefreshFailed,
                ..
            })
        ));
        assert_eq!(actual_calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            refresher
                .current_token()
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "access"
        );
    }

    #[tokio::test]
    async fn aborted_refresh_leader_notifies_waiter_and_allows_new_leader() {
        let store = Arc::new(MemoryTokenStore::default());
        let refresher = Arc::new(TokenRefresher::new(store));
        refresher.set_current(sample_token());
        let leader_started = Arc::new(Notify::new());
        let started_wait = leader_started.notified();
        let actual_calls = Arc::new(AtomicUsize::new(0));

        let leader = {
            let refresher = refresher.clone();
            let leader_started = leader_started.clone();
            let actual_calls = actual_calls.clone();
            tokio::spawn(async move {
                refresher
                    .refresh_with(move || async move {
                        actual_calls.fetch_add(1, AtomicOrdering::SeqCst);
                        leader_started.notify_one();
                        std::future::pending::<AppResult<TokenPair>>().await
                    })
                    .await
            })
        };
        started_wait.await;

        let waiter = {
            let refresher = refresher.clone();
            let actual_calls = actual_calls.clone();
            tokio::spawn(async move {
                refresher
                    .refresh_with(move || async move {
                        actual_calls.fetch_add(1, AtomicOrdering::SeqCst);
                        Ok(sample_token())
                    })
                    .await
            })
        };
        while !refresher.refresh_flight.has_waiter() {
            tokio::task::yield_now().await;
        }

        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        let waiter_result = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter must be released when leader is aborted")
            .unwrap();
        assert!(matches!(
            waiter_result,
            Err(AppError::Token {
                code: crate::error::TokenErrorCode::RefreshFailed,
                ..
            })
        ));

        let next = refresher
            .refresh_with({
                let actual_calls = actual_calls.clone();
                move || async move {
                    actual_calls.fetch_add(1, AtomicOrdering::SeqCst);
                    let mut token = sample_token();
                    token.access_token = "next-access".into();
                    Ok(token)
                }
            })
            .await
            .unwrap();
        assert_eq!(next.access_token, "next-access");
        assert_eq!(actual_calls.load(AtomicOrdering::SeqCst), 2);
    }

    #[test]
    fn refresh_transport_classification_keeps_connect_timeout_and_body_typed_as_network() {
        let connect = classify_refresh_transport_flags(false, true, false, "connect");
        let timeout = classify_refresh_transport_flags(true, false, false, "timeout");
        let response_body = classify_refresh_transport_flags(false, false, true, "body");
        let other = classify_refresh_transport_flags(false, false, false, "protocol");

        assert!(matches!(
            connect,
            AppError::DriveApi {
                transport_kind: Some(crate::error::DriveTransportKind::Connect),
                ..
            }
        ));
        assert!(matches!(
            timeout,
            AppError::DriveApi {
                transport_kind: Some(crate::error::DriveTransportKind::Timeout),
                ..
            }
        ));
        assert!(matches!(
            response_body,
            AppError::DriveApi {
                transport_kind: Some(crate::error::DriveTransportKind::ResponseBody),
                ..
            }
        ));
        assert!(matches!(
            other,
            AppError::Token {
                code: crate::error::TokenErrorCode::RefreshFailed,
                ..
            }
        ));
    }
}
