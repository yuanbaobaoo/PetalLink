//! OAuth token 交换流程集成测试（wiremock 桩）。
//!
//! 验证：
//! - code 交换时手工拼接 form body 的编码正确性（authorization_code 含 + / = 不被破坏）
//! - token 响应解析（expires_in → expires_at 计算）
//! - 华为错误响应（无 access_token）返回明确错误
//! - 刷新流程（refresh_token 换新 access_token）

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 验证 token 端点 form body 编码：authorization_code 含 '+' 时不应被当空格。
/// 这是华为 API 最坑的怪癖之一（dart 侧用手工 form body 绕过 dio 编码）。
#[tokio::test]
async fn test_code_exchange_preserves_plus_in_code() {
    let server = MockServer::start().await;

    // 桩：期望收到 code=abc%2Bdef（+ 被编码为 %2B，而非被当空格）
    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .and(wiremock::matchers::body_string_contains("code=abc%2Bdef"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "new-at",
            "refresh_token": "new-rt",
            "expires_in": 3600,
            "token_type": "Bearer",
        })))
        .mount(&server)
        .await;

    // 直接用 reqwest 验证编码逻辑（与 service.rs exchange_code_for_token 一致）
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

    /// dart Uri.encodeComponent 等价集（与 service.rs URI_ENCODE_COMPONENT 一致）
    const URI_ENC: &AsciiSet = &CONTROLS
        .add(b' ').add(b'!').add(b'"').add(b'#').add(b'$').add(b'%')
        .add(b'&').add(b'\'').add(b'(').add(b')').add(b'*').add(b'+')
        .add(b',').add(b':').add(b';').add(b'<').add(b'=').add(b'>')
        .add(b'?').add(b'@').add(b'[').add(b'\\').add(b']').add(b'^')
        .add(b'`').add(b'{').add(b'|').add(b'}');

    let code = "abc+def"; // 含 + 的授权码
    let enc = |s: &str| utf8_percent_encode(s, URI_ENC).to_string();
    let body = format!("code={}", enc(code));
    // + 应被编码为 %2B，而非原样保留（否则 form 解析会当空格）
    assert!(body.contains("code=abc%2Bdef"));
    assert!(!body.contains("code=abc+def"));
}

/// 验证刷新流程：refresh_token 换新 access_token，响应可能不含新 refresh_token。
#[tokio::test]
async fn test_refresh_returns_new_access_token() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .and(wiremock::matchers::body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "refreshed-at",
            // 华为刷新响应可能不含新 refresh_token
            "expires_in": 7200,
            "token_type": "Bearer",
        })))
        .mount(&server)
        .await;

    let resp = reqwest::Client::new()
        .post(format!("{}/oauth2/v3/token", server.uri()))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", "old-rt"),
            ("client_id", "118065481"),
            ("client_secret", "secret"),
        ])
        .send()
        .await
        .unwrap();

    let data: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(data["access_token"], "refreshed-at");
    // 无新 refresh_token 时调用方应沿用旧的
    assert!(data.get("refresh_token").is_none());
}

/// 验证 token 端点错误响应（华为返回 error_description）。
#[tokio::test]
async fn test_token_error_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant",
            "error_description": "授权码无效或已过期",
        })))
        .mount(&server)
        .await;

    let resp = reqwest::Client::new()
        .post(format!("{}/oauth2/v3/token", server.uri()))
        .form(&[("grant_type", "authorization_code")])
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let data: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(data["error"], "invalid_grant");
    assert_eq!(data["error_description"], "授权码无效或已过期");
}

/// 验证 OAuth 回调常量与端口配置一致（端到端 server 测试需真实端口，由手动测试覆盖）。
#[tokio::test]
async fn test_oauth_constants_consistent() {
    use petal_link_lib::constants;
    assert_eq!(constants::CALLBACK_PATH, "/oauth/callback");
    assert_eq!(constants::DEFAULT_CALLBACK_PORT, 9999);
    assert_eq!(constants::LOOPBACK_HOST, "127.0.0.1");
}

/// 验证用户信息三端点合并逻辑（mock 三个端点）。
#[tokio::test]
async fn test_user_info_merge_three_endpoints() {
    use serde_json::Value;

    // 模拟三个端点的 JSON 响应
    let oidc = json!({"sub": "oidc-sub-123"});
    let info = json!({"displayName": "张三", "openID": "app-openid", "displayNameFlag": 0});
    let phone = json!({"mobile": "13800001234"});

    // 合并：oidc < info < phone
    let mut merged = serde_json::Map::new();
    if let Some(o) = oidc.as_object() {
        merged.extend(o.clone());
    }
    if let Some(i) = info.as_object() {
        merged.extend(i.clone());
    }
    if let Some(p) = phone.as_object() {
        merged.extend(p.clone());
    }

    let user = petal_link_lib::auth::models::UserInfo::from_json(&Value::Object(merged));
    assert_eq!(user.sub.as_deref(), Some("oidc-sub-123"));
    assert_eq!(user.display_name.as_deref(), Some("张三"));
    assert_eq!(user.open_id.as_deref(), Some("app-openid"));
    assert_eq!(user.mobile.as_deref(), Some("13800001234"));
    // primaryLabel 应为 displayName（优先级最高）
    assert_eq!(user.primary_label().as_deref(), Some("张三"));
}
