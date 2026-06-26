//! Token 刷新器（需求 F-AUTH-04）。
//!
//! 对齐 `legacy/lib/auth/token_refresher.dart`。
//!
//! 用 refresh_token 换新的 access_token。带并发去重锁，防止多个并发请求同时触发刷新。
//! 华为刷新响应可能不含新 refresh_token → 沿用旧的。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::auth::models::{now_ms, TokenPair};
use crate::auth::token_store::TokenStore;
use crate::constants;
use crate::error::{AppError, AppResult};

/// Token 刷新器。
///
/// 并发去重：刷新期间所有并发调用共享同一次结果（通过对 `_refresh_guard` 加异步锁，
/// 锁持有期间执行刷新，后续调用等待锁释放后读取已刷新的新 token）。
pub struct TokenRefresher {
    token_store: Arc<dyn TokenStore>,
    http: reqwest::Client,
    /// 刷新串行化锁：同一时刻只有一个刷新在执行
    refresh_lock: AsyncMutex<()>,
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
            refresh_lock: AsyncMutex::new(()),
            current: Mutex::new(None),
        }
    }

    /// 更新内存中的 token 缓存。
    pub fn set_current(&self, token: TokenPair) {
        *self.current.lock() = Some(token);
    }

    /// 获取当前 token（优先内存缓存，回退存储）。
    pub async fn current_token(&self) -> AppResult<Option<TokenPair>> {
        if let Some(t) = self.current.lock().clone() {
            return Ok(Some(t));
        }
        self.token_store.load()
    }

    /// 刷新 token 并持久化。返回新 token。
    /// 并发调用共享同一次刷新结果（refresh_lock 串行化）。
    /// 对齐 dart `TokenRefresher.refresh()`。
    pub async fn refresh(&self) -> AppResult<TokenPair> {
        // 并发去重：拿不到锁说明已有刷新在进行，等它完成后重新读 token
        let waited = self.refresh_lock.try_lock().is_err();
        let _guard = self.refresh_lock.lock().await;
        if waited {
            // 之前有刷新在跑，复用其结果（直接读最新 token）
            if let Some(t) = self.current.lock().clone() {
                tracing::debug!("已有刷新在进行中，等待复用结果");
                return Ok(t);
            }
        }

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
            .map_err(|e| AppError::token_refresh_failed(Some(&e.to_string())))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::token_refresh_failed(Some(&e.to_string())))?;
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
    use crate::auth::token_store::AdaptiveTokenStore;

    #[tokio::test]
    async fn test_refresh_requires_logged_in() {
        // 无 token 时刷新应返回 not_logged_in
        // 用真实 AdaptiveTokenStore（CI 环境通常无 Keychain → 降级文件，load 为 None）
        let store = Arc::new(AdaptiveTokenStore::new());
        // 确保无 token
        let _ = store.clear();
        let refresher = TokenRefresher::new(store);
        let result = refresher.refresh().await;
        assert!(matches!(
            result,
            Err(AppError::Token { code, .. }) if code == crate::error::TokenErrorCode::NotLoggedIn
        ));
    }
}
