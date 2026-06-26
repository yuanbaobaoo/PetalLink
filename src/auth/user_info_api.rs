//! 华为账号信息客户端（合并多端点）。
//!
//! 对齐 `legacy/lib/auth/user_info_api.dart`。
//!
//! 流程：并行调三个端点（任一失败不影响其他），合并为单一 UserInfo：
//! 1. `POST rest.php?nsp_svc=GOpen.User.getInfo` → displayName / openID / headPictureURL（需 profile scope）
//! 2. `POST rest.php?nsp_svc=GOpen.User.getPhone` → 纯文本手机号（需 mobile scope，仅中国大陆）
//! 3. `GET /oauth2/v3/userinfo`（OIDC）→ sub 标识（尽力而为，华为该端点常 404）
//!
//! 合并优先级：oidc < info < phone（phone 最优先，覆盖 info 的脱敏手机号）。

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::auth::models::UserInfo;
use crate::auth::service::AuthService;
use crate::constants;
use crate::error::AppResult;

const REST_PHP_URL: &str = "https://account.cloud.huawei.com/rest.php";

/// 华为账号信息客户端。
pub struct UserInfoApi {
    auth: Arc<AuthService>,
    http: reqwest::Client,
}

impl UserInfoApi {
    pub fn new(auth: Arc<AuthService>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("构建 reqwest client 失败");
        Self { auth, http }
    }

    /// 拉取完整账号信息（合并三端点）。任一端点失败不影响其他。
    pub async fn get(&self) -> AppResult<UserInfo> {
        let token = self.auth.ensure_valid_access_token().await?;
        tracing::info!("开始拉取账号信息");

        // 并行三端点（失败返回空 map，不影响其他）
        let (info, phone, oidc) = tokio::join!(
            self.get_display_info(&token),
            self.get_phone_number(&token),
            self.get_oidc_user_info(&token),
        );
        let info_json = info.unwrap_or_default();
        let phone_json = phone.unwrap_or_default();
        let oidc_json = oidc.unwrap_or_default();

        // 合并：oidc 先放，info 覆盖，phone 最后覆盖（最优先）
        let mut merged = serde_json::Map::new();
        if oidc_json.is_object() {
            if let Some(obj) = oidc_json.as_object() {
                merged.extend(obj.clone());
            }
        }
        if info_json.is_object() {
            if let Some(obj) = info_json.as_object() {
                merged.extend(obj.clone());
            }
        }
        if phone_json.is_object() {
            if let Some(obj) = phone_json.as_object() {
                merged.extend(obj.clone());
            }
        }

        let user_info = UserInfo::from_json(&Value::Object(merged));
        Ok(user_info.resolve_anonymous_as_mobile())
    }

    /// POST GOpen.User.getInfo → displayName / openID / headPictureURL / displayNameFlag。
    async fn get_display_info(&self, token: &str) -> reqwest::Result<Value> {
        let resp = self
            .http
            .post(REST_PHP_URL)
            .query(&[("nsp_svc", "GOpen.User.getInfo")])
            .form(&[
                ("access_token", token),
                ("getNickName", "1"), // 1=返回真实昵称；0=匿名化
            ])
            .send()
            .await?;
        let json: Value = resp.json().await?;
        if !json.is_object() {
            tracing::warn!("GOpen.User.getInfo 返回非对象");
            return Ok(Value::Null);
        }
        Ok(json)
    }

    /// POST GOpen.User.getPhone → 纯文本手机号（无字段名），也可能在 JSON 字段中。
    async fn get_phone_number(&self, token: &str) -> reqwest::Result<Value> {
        let resp = self
            .http
            .post(REST_PHP_URL)
            .query(&[("nsp_svc", "GOpen.User.getPhone")])
            .form(&[("access_token", token)])
            .send()
            .await?;
        // 响应可能是纯文本（手机号）也可能是 JSON
        let text = resp.text().await?;
        let trimmed = text.trim();
        // 先尝试 JSON 解析
        if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
            if json.is_object() {
                return Ok(json);
            }
        }
        // 纯文本形式：包装为 {mobile: <text>}
        if !trimmed.is_empty() {
            return Ok(serde_json::json!({ "mobile": trimmed }));
        }
        Ok(Value::Null)
    }

    /// GET OIDC userinfo（尽力而为，常 404）。
    async fn get_oidc_user_info(&self, token: &str) -> reqwest::Result<Value> {
        let resp = self
            .http
            .get(constants::USER_INFO_URL)
            .bearer_auth(token)
            .send()
            .await?;
        let json: Value = resp.json().await?;
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_priority_phone_wins() {
        // 模拟合并：oidc < info < phone，phone 最优先
        let mut merged = serde_json::Map::new();
        merged.extend(serde_json::json!({"mobile": "匿名脱敏", "sub": "oidc-sub"}).as_object().unwrap().clone());
        merged.extend(serde_json::json!({"displayName": "昵称", "mobile": "info-phone"}).as_object().unwrap().clone());
        merged.extend(serde_json::json!({"mobile": "13800000000"}).as_object().unwrap().clone());

        let user = UserInfo::from_json(&Value::Object(merged));
        // phone 最后覆盖
        assert_eq!(user.mobile.as_deref(), Some("13800000000"));
        assert_eq!(user.display_name.as_deref(), Some("昵称"));
        assert_eq!(user.sub.as_deref(), Some("oidc-sub"));
    }
}
