//! OAuth 与认证模块公开合同测试。
//!
//! 覆盖 token 模型、用户信息、PKCE、回调服务、授权 URL 与配置常量。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use petal_link_lib::auth::models::{TokenPair, UserInfo};
use petal_link_lib::auth::oauth_server::OauthServer;
use petal_link_lib::auth::pkce::{generate_pkce, generate_state};
use petal_link_lib::auth::service::build_authorize_url;
use petal_link_lib::constants;
use serde_json::json;
use sha2::{Digest, Sha256};

/// 验证 OAuth 回调常量与端口配置一致（端到端 server 测试需真实端口，由手动测试覆盖）。
#[tokio::test]
async fn test_oauth_constants_consistent() {
    assert_eq!(constants::CALLBACK_PATH, "/oauth/callback");
    assert_eq!(constants::DEFAULT_CALLBACK_PORT, 9999);
    assert_eq!(constants::LOOPBACK_HOST, "127.0.0.1");
}

/// 验证缺省过期时间仍生成有效 token。
#[test]
fn test_token_defaults_expires_in() {
    let json = json!({"access_token": "at", "refresh_token": "rt"});
    let token = TokenPair::from_token_response(&json).unwrap();
    assert!(!token.is_expired());
}

/// 验证 token 过期窗口按缓冲时间判断。
#[test]
fn test_will_expire_within() {
    let json = json!({
        "access_token": "at",
        "refresh_token": "rt",
        "expires_in": 10,
    });
    let token = TokenPair::from_token_response(&json).unwrap();
    assert!(token.will_expire_within(60));
    assert!(!token.will_expire_within(0));
}

/// 验证用户主标签按公开字段优先级选择。
#[test]
fn test_userinfo_primary_label_priority() {
    let u = UserInfo {
        display_name: Some("张三".into()),
        mobile: Some("13800000000".into()),
        ..Default::default()
    };
    assert_eq!(u.primary_label().as_deref(), Some("张三"));

    let u = UserInfo {
        mobile: Some("13800000000".into()),
        name: Some("name".into()),
        ..Default::default()
    };
    assert_eq!(u.primary_label().as_deref(), Some("13800000000"));

    let u = UserInfo {
        name: Some("oidc-name".into()),
        open_id: Some("opid".into()),
        ..Default::default()
    };
    assert_eq!(u.primary_label().as_deref(), Some("oidc-name"));
}

/// 验证华为用户信息字段别名正确解析。
#[test]
fn test_userinfo_from_json_field_aliases() {
    let json = json!({
        "openID": "opid",
        "displayName": "昵称",
        "displayNameFlag": 1,
    });
    let u = UserInfo::from_json(&json);
    assert_eq!(u.open_id.as_deref(), Some("opid"));
    assert_eq!(u.display_name.as_deref(), Some("昵称"));
    assert!(u.is_anonymized);
}

/// 验证匿名昵称回退为手机号。
#[test]
fn test_userinfo_anonymous_resolves_to_mobile() {
    let u = UserInfo {
        display_name: Some("182****1234".into()),
        mobile: Some("18200001234".into()),
        is_anonymized: true,
        ..Default::default()
    };
    let resolved = u.resolve_anonymous_as_mobile();
    assert!(resolved.display_name.is_none());
    assert_eq!(resolved.primary_label().as_deref(), Some("18200001234"));
    assert_eq!(resolved.secondary_label().as_deref(), Some("匿名账号"));
}

/// 验证中文昵称头像缩写取首字符。
#[test]
fn test_userinfo_initial_cjk() {
    let u = UserInfo {
        display_name: Some("张三".into()),
        ..Default::default()
    };
    assert_eq!(u.initial().as_deref(), Some("张"));
}

/// 验证 OAuth state 为 64 位十六进制串。
#[test]
fn test_state_is_64_hex_chars() {
    let state = generate_state();
    assert_eq!(state.len(), 64);
    assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
}

/// 验证 OAuth state 每次随机生成。
#[test]
fn test_state_is_random() {
    let s1 = generate_state();
    let s2 = generate_state();
    assert_ne!(s1, s2);
}

/// 验证 PKCE verifier 长度符合 RFC 7636。
#[test]
fn test_pkce_verifier_length_in_rfc_range() {
    let pkce = generate_pkce();
    assert!(
        (43..=128).contains(&pkce.code_verifier.len()),
        "verifier 长度 {} 不在 RFC 范围",
        pkce.code_verifier.len()
    );
}

/// 验证 PKCE verifier 编码不带填充。
#[test]
fn test_pkce_verifier_no_padding() {
    let pkce = generate_pkce();
    assert!(!pkce.code_verifier.contains('='));
    assert!(!pkce.code_challenge.contains('='));
}

/// 验证 challenge 是 verifier 的 SHA-256 Base64URL。
#[test]
fn test_pkce_challenge_is_sha256_of_verifier() {
    let pkce = generate_pkce();
    let mut hasher = Sha256::new();
    hasher.update(pkce.code_verifier.as_bytes());
    let digest = hasher.finalize();
    let expected = URL_SAFE_NO_PAD.encode(digest);
    assert_eq!(pkce.code_challenge, expected);
}

/// 验证 PKCE 参数每次随机生成。
#[test]
fn test_pkce_is_random() {
    let p1 = generate_pkce();
    let p2 = generate_pkce();
    assert_ne!(p1.code_verifier, p2.code_verifier);
    assert_ne!(p1.code_challenge, p2.code_challenge);
}

/// 验证调试展示不会泄露 verifier。
#[test]
fn test_display_hides_verifier() {
    let pkce = generate_pkce();
    let s = format!("{pkce}");
    assert!(s.contains("<hidden>"));
    assert!(!s.contains(&pkce.code_verifier));
}

/// 验证停止句柄会结束等待中的回调任务。
#[tokio::test]
async fn test_stop_handle_closes_wait_for_callback() {
    let server = OauthServer::start(0).await.expect("启动 OAuth 测试 server");
    let stop = server.stop_handle();
    let waiter = tokio::spawn(server.wait_for_callback());

    stop.stop();

    let result = waiter.await.expect("等待任务应结束");
    assert!(
        result.is_err(),
        "stop 后 wait_for_callback 不应继续等到超时"
    );
}

/// 验证授权 scope 中的斜杠不被编码。
#[test]
fn test_build_authorize_url_scope_not_encoded() {
    let pkce = generate_pkce();
    let url = build_authorize_url("http://127.0.0.1:9999/oauth/callback", "mystate", &pkce);
    assert!(url.starts_with(constants::AUTHORIZE_URL));
    assert!(url.contains("scope=openid%20profile%20https://www.huawei.com/auth/drive"));
    assert!(!url.contains("drive%2F"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains(&format!("code_challenge={}", pkce.code_challenge)));
}

/// 验证授权 URL 携带全部必要参数。
#[test]
fn test_build_authorize_url_params() {
    let pkce = generate_pkce();
    let url = build_authorize_url("http://127.0.0.1:9999/oauth/callback", "st", &pkce);
    assert!(url.contains("response_type=code"));
    assert!(url.contains(&format!("client_id={}", constants::resolved_client_id())));
    assert!(url.contains("state=st"));
    assert!(url.contains("access_type=offline"));
}

/// 验证未注入密钥时使用编译期占位符。
#[test]
fn test_resolved_secret_uses_placeholder_when_unset() {
    if constants::BUILD_SECRET.is_empty() {
        assert_eq!(
            constants::resolved_client_secret(),
            constants::PLACEHOLDER_SECRET
        );
        assert!(!constants::client_secret_configured());
    }
}

/// 验证未注入客户端标识时解析为空。
#[test]
fn test_resolved_client_id_empty_when_unset() {
    if constants::BUILD_CLIENT_ID.is_empty() {
        assert_eq!(constants::resolved_client_id(), "");
        assert!(!constants::client_id_configured());
    }
}

/// 验证调试包标识使用开发后缀。
#[test]
fn test_bundle_identifier_is_github() {
    assert_eq!(
        constants::BUNDLE_IDENTIFIER,
        "io.github.yuanbaobaoo.PetalLink-dev"
    );
}

/// 验证 OAuth scope 使用完整云盘权限。
#[test]
fn test_scopes_use_full_drive() {
    assert!(constants::SCOPES.contains(&"https://www.huawei.com/auth/drive"));
    assert!(!constants::SCOPES.iter().any(|s| s.contains("drive.file")));
}
