//! 应用错误消息、序列化与 Drive 恢复元数据合同测试。

use petal_link_lib::error::{
    parse_retry_after, AppError, AuthErrorCode, DriveTransportKind, RequestSemantics, RetryAfter,
};

/// 验证取消授权保留错误码和文案。
#[test]
fn test_auth_cancelled_message() {
    let e = AppError::auth_cancelled();
    assert!(matches!(
        e,
        AppError::Auth {
            code: AuthErrorCode::Cancelled,
            ..
        }
    ));
    assert_eq!(e.to_string(), "用户取消授权");
}

/// 验证配额错误包含所需容量。
#[test]
fn test_quota_message() {
    let e = AppError::quota_exceeded(100, 50);
    assert!(matches!(e, AppError::QuotaExceeded { .. }));
    assert!(e.to_string().contains("需要 100"));
}

/// 验证 Drive 状态错误保留 HTTP 状态码。
#[test]
fn test_drive_from_status() {
    let e = AppError::drive_from_status(404, "not found body");
    match e {
        AppError::DriveApi { status_code, .. } => {
            assert_eq!(status_code, Some(404));
        }
        _ => panic!("应为 DriveApi"),
    }
}

/// 验证错误序列化结构保持扁平。
#[test]
fn test_serde_flat_structure() {
    let e = AppError::auth_denied(Some("用户拒绝"));
    let v: serde_json::Value = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], "Auth");
    assert_eq!(v["code"], "denied");
    assert_eq!(v["message"], "授权失败：用户拒绝");
    assert!(v["message"].is_string());
    assert!(v.get("status_code").is_some());
}

/// 验证网络错误与 token 刷新错误可区分。
#[test]
fn test_serde_network_vs_refresh_distinct() {
    let net = AppError::drive_network(Some("timeout"));
    let refresh = AppError::token_refresh_failed(Some("invalid_grant"));
    let nv: serde_json::Value = serde_json::to_value(&net).unwrap();
    let rv: serde_json::Value = serde_json::to_value(&refresh).unwrap();
    assert_eq!(nv["kind"], "DriveApi");
    assert_eq!(nv["code"], "network");
    assert_eq!(nv["message"], "网络连接失败，请检查网络");
    assert_eq!(rv["kind"], "Token");
    assert_eq!(rv["code"], "refresh_failed");
    assert!(rv["message"].as_str().unwrap().contains("Token 刷新失败"));
    assert_ne!(nv["kind"], rv["kind"]);
}

/// 验证 DriveApi 序列化携带状态码。
#[test]
fn test_serde_driveapi_carries_status_code() {
    let e = AppError::drive_from_status(404, "not found");
    let v: serde_json::Value = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], "DriveApi");
    assert_eq!(v["status_code"], 404);
    assert!(v["message"].is_string());
}

/// 验证数字华为错误码和重试信息被保留。
#[test]
fn drive_response_preserves_numeric_huawei_code_and_retry_after() {
    let error = AppError::drive_from_response(
        429,
        r#"{"errorCode":21004002,"errorDescription":"slow down"}"#,
        Some(RetryAfter::DelaySeconds(17)),
        RequestSemantics::Write,
        true,
    );

    match error {
        AppError::DriveApi {
            status_code,
            error_code,
            retry_after,
            transport_kind,
            request_may_have_reached_server,
            auth_already_replayed,
            ..
        } => {
            assert_eq!(status_code, Some(429));
            assert_eq!(error_code.as_deref(), Some("21004002"));
            assert_eq!(retry_after, Some(RetryAfter::DelaySeconds(17)));
            assert_eq!(transport_kind, None);
            assert!(request_may_have_reached_server);
            assert!(auth_already_replayed);
        }
        other => panic!("expected DriveApi, got {other:?}"),
    }
}

/// 验证字符串华为错误码被保留。
#[test]
fn drive_response_preserves_string_huawei_code() {
    let error = AppError::drive_from_response(
        400,
        r#"{"errorCode":"21004002"}"#,
        None,
        RequestSemantics::Read,
        false,
    );

    assert!(matches!(
        error,
        AppError::DriveApi {
            error_code: Some(ref code),
            ..
        } if code == "21004002"
    ));
}

/// 验证 Retry-After 支持秒数和 HTTP 日期。
#[test]
fn retry_after_parser_accepts_delta_seconds_and_http_date() {
    assert_eq!(
        parse_retry_after("120"),
        Some(RetryAfter::DelaySeconds(120))
    );
    assert_eq!(
        parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(RetryAfter::AtUnixMs(784_111_777_000))
    );
    assert_eq!(parse_retry_after("not-a-retry-after"), None);
}

/// 验证写请求结构化元数据区分连接与超时。
#[test]
fn write_transport_metadata_distinguishes_connect_from_timeout() {
    let connect = AppError::drive_transport(
        DriveTransportKind::Connect,
        RequestSemantics::Write,
        false,
        Some("connect failed"),
    );
    let timeout = AppError::drive_transport(
        DriveTransportKind::Timeout,
        RequestSemantics::Write,
        true,
        Some("timed out"),
    );

    assert!(matches!(
        connect,
        AppError::DriveApi {
            transport_kind: Some(DriveTransportKind::Connect),
            request_may_have_reached_server: false,
            auth_already_replayed: false,
            ..
        }
    ));
    assert!(matches!(
        timeout,
        AppError::DriveApi {
            transport_kind: Some(DriveTransportKind::Timeout),
            request_may_have_reached_server: true,
            auth_already_replayed: true,
            ..
        }
    ));
}

/// 验证内部 Drive 元数据不改变前端序列化结构。
#[test]
fn internal_drive_metadata_does_not_change_frontend_shape() {
    let error = AppError::drive_from_response(
        503,
        r#"{"errorCode":"busy"}"#,
        Some(RetryAfter::DelaySeconds(3)),
        RequestSemantics::Write,
        true,
    );
    let value = serde_json::to_value(error).unwrap();
    let object = value.as_object().unwrap();

    assert_eq!(object.len(), 5);
    assert!(object.get("retry_after").is_none());
    assert!(object.get("transport_kind").is_none());
    assert!(object.get("request_may_have_reached_server").is_none());
    assert!(object.get("auth_already_replayed").is_none());
}

/// 验证结构化状态匹配不依赖展示文案。
#[test]
fn structured_status_matching_never_reads_display_message() {
    let fake_status = AppError::generic("云端请求失败 (404), please retry 409");
    let structured = AppError::drive_from_status(404, "{}");

    assert_eq!(fake_status.drive_status(), None);
    assert_eq!(structured.drive_status(), Some(404));
}
