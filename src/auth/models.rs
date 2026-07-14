//! Auth 模型 —— TokenPair + UserInfo。
//!
//! 对齐 `legacy/lib/auth/models/token_pair.dart` + `models/user_info.dart`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OAuth Token 对（需求 F-AUTH-03）。
/// access_token + refresh_token + 过期时间，加密持久化到本地文件（机器码绑定）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// access_token 过期时间（**毫秒**时间戳，对齐 dart）
    pub expires_at: i64,
    #[serde(default = "default_token_type")]
    pub token_type: String,
    pub scope: Option<String>,
}

/// 返回缺省的 Bearer 令牌类型。
fn default_token_type() -> String {
    "Bearer".to_string()
}

impl TokenPair {
    /// 是否已过期
    pub fn is_expired(&self) -> bool {
        now_ms() >= self.expires_at
    }

    /// 距过期是否小于 buffer_secs 秒（用于提前刷新，默认 60 秒）。
    /// 对齐 dart `willExpireWithin(Duration buffer)`。
    pub fn will_expire_within(&self, buffer_secs: i64) -> bool {
        let threshold = now_ms() + buffer_secs * 1000;
        threshold >= self.expires_at
    }

    /// 从华为 token 端点响应构造（expires_in 为**秒**）。
    /// 对齐 dart `TokenPair.fromTokenResponse`。
    pub fn from_token_response(json: &Value) -> Option<Self> {
        let access_token = json.get("access_token")?.as_str()?.to_string();
        let refresh_token = json
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let expires_in_sec = json
            .get("expires_in")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(3600);
        let expires_at = now_ms() + expires_in_sec * 1000;
        let token_type = json
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or("Bearer")
            .to_string();
        let scope = json.get("scope").and_then(Value::as_str).map(String::from);
        Some(Self {
            access_token,
            refresh_token,
            expires_at,
            token_type,
            scope,
        })
    }
}

/// 华为账号信息 DTO（合并自多个端点响应）。
/// 对齐 dart `UserInfo`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserInfo {
    pub sub: Option<String>,
    pub open_id: Option<String>,
    pub union_id: Option<String>,
    pub display_name: Option<String>,
    pub name: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub mobile: Option<String>,
    pub avatar_url: Option<String>,
    /// displayName 是否为匿名账号（displayNameFlag=1）
    #[serde(default)]
    pub is_anonymized: bool,
}

impl UserInfo {
    /// 用户主要展示名（按优先级）：真实昵称 > 手机号 > OIDC name > openId/sub。
    /// 对齐 dart `primaryLabel`。
    pub fn primary_label(&self) -> Option<String> {
        if let Some(d) = non_empty_trimmed(&self.display_name) {
            return Some(d);
        }
        if let Some(m) = non_empty_trimmed(&self.mobile) {
            return Some(m);
        }
        for c in [&self.name, &self.nickname, &self.open_id, &self.sub] {
            if let Some(v) = non_empty_trimmed(c) {
                return Some(v);
            }
        }
        None
    }

    /// 副标题：邮箱（如果和主标不一样且非空）；否则手机号（同样不和主标重复）；
    /// 否则匿名账号提示 / null。对齐 dart `secondaryLabel`。
    pub fn secondary_label(&self) -> Option<String> {
        let pri = self.primary_label();
        if let Some(e) = non_empty_trimmed(&self.email) {
            if Some(&e) != pri.as_ref() {
                return Some(e);
            }
        }
        if let Some(m) = non_empty_trimmed(&self.mobile) {
            if Some(&m) != pri.as_ref() {
                return Some(m);
            }
        }
        if self.is_anonymized {
            return Some("匿名账号".to_string());
        }
        None
    }

    /// 头像首字符（取主标第一个字符）。对齐 dart `initial`。
    /// Rust String 按 char 取首字符（CJK 安全；复杂 grapheme cluster 简化为首个 Unicode scalar）。
    pub fn initial(&self) -> Option<String> {
        let label = self.primary_label()?;
        label.chars().next().map(|c| c.to_string())
    }

    /// 从合并后的 JSON 构造（兼容多种字段命名）。
    /// 对齐 dart `UserInfo.fromJson`。
    pub fn from_json(json: &Value) -> Self {
        let flag = json.get("displayNameFlag");
        let is_anon = flag
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .map(|n| n == 1)
            .unwrap_or(false);

        Self {
            sub: pick(json, &["sub", "user_id", "userId"]),
            open_id: pick(json, &["openID", "openId", "open_id"]),
            union_id: pick(json, &["unionID", "unionId", "union_id"]),
            display_name: pick(json, &["displayName", "display_name"]),
            name: pick(json, &["name"]),
            nickname: pick(json, &["nickname", "nick_name", "preferred_username"]),
            email: pick(json, &["email"]),
            mobile: pick(json, &["mobile", "phone", "phone_number", "mobile_number"]),
            avatar_url: pick(json, &["headPictureURL", "picture", "avatar", "avatar_url"]),
            is_anonymized: is_anon,
        }
    }

    /// 把"匿名 displayName + 真实手机号"合并为最优展示。
    /// 对齐 dart `resolveAnonymousAsMobile`：清空匿名名让 primary 走 mobile。
    pub fn resolve_anonymous_as_mobile(self) -> Self {
        if !self.is_anonymized {
            return self;
        }
        if non_empty_trimmed(&self.mobile).is_none() {
            return self;
        }
        Self {
            display_name: None, // 顶掉匿名名
            ..self
        }
    }
}

/// 取首个非空 trim 后的字符串
fn non_empty_trimmed(s: &Option<String>) -> Option<String> {
    s.as_ref().and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// 从 JSON 按 keys 顺序取首个非空字符串
fn pick(json: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = json.get(*k).and_then(Value::as_str) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// 当前时间（毫秒 epoch）
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
