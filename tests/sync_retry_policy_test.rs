//! 传输重试策略公开合同测试。

use petal_link_lib::error::{
    AppError, DriveTransportKind, RequestSemantics, RetryAfter, TokenErrorCode,
};
use petal_link_lib::sync::retry_policy::{
    classify_transfer_error, ClassifiedRecovery, RecoveryContext, RecoveryDecision,
};
use petal_link_lib::sync::transfer_state::{TransferErrorKind, TransferOperation};

/// 构造指定传输操作的默认恢复上下文。
fn context(operation: TransferOperation) -> RecoveryContext {
    RecoveryContext {
        operation,
        attempt_count: 0,
        now_ms: 10_000,
        jitter_ms: 0,
        auth_already_replayed: false,
        max_attempts: 5,
    }
}

/// 验证恢复策略输入输出使用持久化传输枚举。
#[test]
fn policy_consumes_and_produces_persistent_transfer_enums() {
    let operation = TransferOperation::Create;
    let recovery_context = RecoveryContext {
        operation,
        attempt_count: 0,
        now_ms: 0,
        jitter_ms: 0,
        auth_already_replayed: false,
        max_attempts: 1,
    };
    let error = AppError::generic("do not replay");

    let kind: TransferErrorKind = classify_transfer_error(&error, recovery_context).kind;

    assert_eq!(kind, TransferErrorKind::Unknown);
}

/// 验证即时恢复与永久失败的基础分类。
#[test]
fn classifies_immediate_recovery_and_permanent_failures() {
    let cases = [
        (
            AppError::drive_transport(
                DriveTransportKind::Connect,
                RequestSemantics::Write,
                false,
                Some("connect"),
            ),
            context(TransferOperation::Create),
            TransferErrorKind::Network,
            RecoveryDecision::WaitForNetwork,
            false,
        ),
        (
            AppError::drive_transport(
                DriveTransportKind::Timeout,
                RequestSemantics::Read,
                false,
                Some("timeout"),
            ),
            context(TransferOperation::Download),
            TransferErrorKind::Timeout,
            RecoveryDecision::WaitForNetwork,
            false,
        ),
        (
            AppError::drive_from_response(401, "{}", None, RequestSemantics::Read, false),
            context(TransferOperation::Download),
            TransferErrorKind::Auth,
            RecoveryDecision::RefreshAuth,
            false,
        ),
        (
            AppError::quota_exceeded(10, 1),
            context(TransferOperation::Create),
            TransferErrorKind::Quota,
            RecoveryDecision::Fail,
            false,
        ),
        (
            AppError::drive_from_response(403, "{}", None, RequestSemantics::Read, false),
            context(TransferOperation::Download),
            TransferErrorKind::Permission,
            RecoveryDecision::Fail,
            false,
        ),
        (
            AppError::drive_from_response(
                400,
                r#"{"errorCode":"validation"}"#,
                None,
                RequestSemantics::Write,
                false,
            ),
            context(TransferOperation::CreateFolder),
            TransferErrorKind::Validation,
            RecoveryDecision::Fail,
            false,
        ),
        (
            AppError::generic("503 timeout after write"),
            context(TransferOperation::Create),
            TransferErrorKind::Unknown,
            RecoveryDecision::Fail,
            false,
        ),
    ];

    for (error, recovery_context, kind, decision, consumes_retry_budget) in cases {
        assert_eq!(
            classify_transfer_error(&error, recovery_context),
            ClassifiedRecovery {
                kind,
                decision,
                consumes_retry_budget,
            },
            "error={error:?}"
        );
    }
}

/// 验证上下文已记录重放时第二次 401 直接认证失败。
#[test]
fn second_401_fails_auth_even_when_only_context_records_replay() {
    let error = AppError::drive_from_response(401, "{}", None, RequestSemantics::Read, false);
    let mut recovery_context = context(TransferOperation::Download);
    recovery_context.auth_already_replayed = true;

    assert_eq!(
        classify_transfer_error(&error, recovery_context),
        ClassifiedRecovery {
            kind: TransferErrorKind::Auth,
            decision: RecoveryDecision::Fail,
            consumes_retry_budget: false,
        }
    );
}

/// 验证令牌错误被分类为永久认证失败。
#[test]
fn token_errors_are_permanent_auth_failures() {
    let error = AppError::Token {
        code: TokenErrorCode::RefreshFailed,
        message: "refresh failed".into(),
    };

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::Download)),
        ClassifiedRecovery {
            kind: TransferErrorKind::Auth,
            decision: RecoveryDecision::Fail,
            consumes_retry_budget: false,
        }
    );
}

/// 验证不确定写超时要求远端核验。
#[test]
fn ambiguous_write_timeout_requires_remote_verification() {
    let error = AppError::drive_transport(
        DriveTransportKind::Timeout,
        RequestSemantics::Write,
        false,
        Some("timeout after submit"),
    );

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::Update)),
        ClassifiedRecovery {
            kind: TransferErrorKind::RemoteAmbiguous,
            decision: RecoveryDecision::VerifyRemote,
            consumes_retry_budget: false,
        }
    );
}

/// 验证已提交的旧式网络写错误要求远端核验。
#[test]
fn submitted_legacy_network_write_requires_remote_verification() {
    let error = AppError::drive_transport(
        DriveTransportKind::Network,
        RequestSemantics::Write,
        false,
        Some("connection lost after submit"),
    );

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::Update)),
        ClassifiedRecovery {
            kind: TransferErrorKind::RemoteAmbiguous,
            decision: RecoveryDecision::VerifyRemote,
            consumes_retry_budget: false,
        }
    );
}

/// 验证明确未提交的写超时等待网络恢复。
#[test]
fn write_timeout_known_pre_submit_waits_for_network() {
    let error = AppError::drive_transport_with_submission(
        DriveTransportKind::Timeout,
        false,
        false,
        Some("timeout before submit"),
    );

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::Update)),
        ClassifiedRecovery {
            kind: TransferErrorKind::Timeout,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        }
    );
}

/// 验证不确定写响应解码失败要求远端核验。
#[test]
fn ambiguous_write_decode_requires_remote_verification() {
    let error = AppError::drive_transport(
        DriveTransportKind::Decode,
        RequestSemantics::Write,
        false,
        Some("response decode failed"),
    );

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::CreateFolder)),
        ClassifiedRecovery {
            kind: TransferErrorKind::RemoteAmbiguous,
            decision: RecoveryDecision::VerifyRemote,
            consumes_retry_budget: false,
        }
    );
}

/// 验证读响应解码失败使用有预算的服务端退避。
#[test]
fn read_decode_uses_budgeted_server_backoff_instead_of_waiting_for_network() {
    let error = AppError::drive_transport(
        DriveTransportKind::Decode,
        RequestSemantics::Read,
        false,
        Some("malformed 2xx"),
    );
    let mut recovery_context = context(TransferOperation::Download);
    recovery_context.attempt_count = 2;

    assert_eq!(
        classify_transfer_error(&error, recovery_context),
        ClassifiedRecovery {
            kind: TransferErrorKind::Server,
            decision: RecoveryDecision::Backoff {
                next_retry_at: 14_000,
            },
            consumes_retry_budget: true,
        }
    );

    recovery_context.attempt_count = recovery_context.max_attempts;
    assert_eq!(
        classify_transfer_error(&error, recovery_context),
        ClassifiedRecovery {
            kind: TransferErrorKind::Server,
            decision: RecoveryDecision::Fail,
            consumes_retry_budget: false,
        }
    );
}

/// 验证读请求错误与响应体错误采用不同恢复决策。
#[test]
fn read_request_error_fails_unknown_but_response_body_waits_for_network() {
    let request = AppError::drive_transport(
        DriveTransportKind::Request,
        RequestSemantics::Read,
        false,
        Some("request construction"),
    );
    let response_body = AppError::drive_transport(
        DriveTransportKind::ResponseBody,
        RequestSemantics::Read,
        false,
        Some("stream interrupted"),
    );

    assert_eq!(
        classify_transfer_error(&request, context(TransferOperation::Download)),
        ClassifiedRecovery {
            kind: TransferErrorKind::Unknown,
            decision: RecoveryDecision::Fail,
            consumes_retry_budget: false,
        }
    );
    assert_eq!(
        classify_transfer_error(&response_body, context(TransferOperation::Download)),
        ClassifiedRecovery {
            kind: TransferErrorKind::Network,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        }
    );
}

/// 验证限流遵守 Retry-After 并消耗重试预算。
#[test]
fn rate_limit_honors_retry_after_and_consumes_retry_budget() {
    let error = AppError::drive_from_response(
        429,
        "{}",
        Some(RetryAfter::DelaySeconds(17)),
        RequestSemantics::Read,
        false,
    );

    assert_eq!(
        classify_transfer_error(&error, context(TransferOperation::Download)),
        ClassifiedRecovery {
            kind: TransferErrorKind::RateLimit,
            decision: RecoveryDecision::Backoff {
                next_retry_at: 27_000,
            },
            consumes_retry_budget: true,
        }
    );
}

/// 验证服务端退避确定性指数增长且受上限约束。
#[test]
fn server_backoff_is_exponential_deterministic_and_capped() {
    let error = AppError::drive_from_response(503, "{}", None, RequestSemantics::Read, false);
    let mut recovery_context = context(TransferOperation::Download);
    recovery_context.attempt_count = 3;
    recovery_context.jitter_ms = 250;

    assert_eq!(
        classify_transfer_error(&error, recovery_context),
        ClassifiedRecovery {
            kind: TransferErrorKind::Server,
            decision: RecoveryDecision::Backoff {
                next_retry_at: 18_250,
            },
            consumes_retry_budget: true,
        }
    );

    recovery_context.attempt_count = 20;
    recovery_context.max_attempts = 21;
    assert_eq!(
        classify_transfer_error(&error, recovery_context).decision,
        RecoveryDecision::Backoff {
            next_retry_at: 310_000,
        }
    );
}

/// 验证服务端预算耗尽后写入转核验而读取失败。
#[test]
fn exhausted_server_budget_verifies_writes_but_fails_reads() {
    let write_error =
        AppError::drive_from_response(503, "{}", None, RequestSemantics::Write, false);
    let read_error = AppError::drive_from_response(503, "{}", None, RequestSemantics::Read, false);
    let pre_submit_write_error =
        AppError::drive_from_response_with_submission(503, "{}", None, false, false);
    let mut write_context = context(TransferOperation::Update);
    write_context.attempt_count = write_context.max_attempts;
    let mut read_context = context(TransferOperation::Download);
    read_context.attempt_count = read_context.max_attempts;

    assert_eq!(
        classify_transfer_error(&write_error, write_context).decision,
        RecoveryDecision::VerifyRemote
    );
    assert_eq!(
        classify_transfer_error(&read_error, read_context).decision,
        RecoveryDecision::Fail
    );
    assert_eq!(
        classify_transfer_error(&pre_submit_write_error, write_context).decision,
        RecoveryDecision::Fail
    );
}
