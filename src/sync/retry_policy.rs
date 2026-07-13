//! 将结构化 Drive/本地错误映射为确定性的恢复决策。

use crate::error::{AppError, DriveApiErrorCode, DriveTransportKind, RetryAfter, TokenErrorCode};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation};

const fn operation_modifies_remote(operation: TransferOperation) -> bool {
    matches!(
        operation,
        TransferOperation::Create
            | TransferOperation::Update
            | TransferOperation::Delete
            | TransferOperation::Move
            | TransferOperation::Rename
            | TransferOperation::CreateFolder
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryContext {
    pub operation: TransferOperation,
    pub attempt_count: u32,
    pub now_ms: i64,
    pub jitter_ms: u64,
    pub auth_already_replayed: bool,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDecision {
    WaitForNetwork,
    Backoff { next_retry_at: i64 },
    RefreshAuth,
    VerifyRemote,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassifiedRecovery {
    pub kind: TransferErrorKind,
    pub decision: RecoveryDecision,
    pub consumes_retry_budget: bool,
}

pub fn classify_transfer_error(error: &AppError, context: RecoveryContext) -> ClassifiedRecovery {
    match error {
        AppError::QuotaExceeded { .. } => permanent(TransferErrorKind::Quota),
        AppError::Token {
            code: TokenErrorCode::NotLoggedIn | TokenErrorCode::RefreshFailed,
            ..
        }
        | AppError::Auth { .. } => permanent(TransferErrorKind::Auth),
        AppError::DriveApi {
            code: DriveApiErrorCode::QuotaExceeded,
            ..
        } => permanent(TransferErrorKind::Quota),
        AppError::DriveApi {
            error_code: Some(error_code),
            ..
        } if error_code == "upload_session_expired" => ClassifiedRecovery {
            kind: TransferErrorKind::SessionExpired,
            decision: RecoveryDecision::VerifyRemote,
            consumes_retry_budget: false,
        },
        AppError::DriveApi {
            status_code,
            retry_after,
            transport_kind,
            request_may_have_reached_server,
            auth_already_replayed,
            ..
        } => {
            if let Some(transport_kind) = transport_kind {
                return classify_transport(
                    *transport_kind,
                    *request_may_have_reached_server,
                    context,
                );
            }
            classify_status(
                *status_code,
                *retry_after,
                *auth_already_replayed || context.auth_already_replayed,
                *request_may_have_reached_server,
                context,
            )
        }
        AppError::Config { .. } | AppError::Generic { .. } => permanent(TransferErrorKind::Unknown),
    }
}

fn classify_transport(
    transport_kind: DriveTransportKind,
    request_may_have_reached_server: bool,
    context: RecoveryContext,
) -> ClassifiedRecovery {
    match transport_kind {
        DriveTransportKind::Connect => ClassifiedRecovery {
            kind: TransferErrorKind::Network,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        },
        _ if operation_modifies_remote(context.operation) && request_may_have_reached_server => {
            verify_remote()
        }
        DriveTransportKind::Network => ClassifiedRecovery {
            kind: TransferErrorKind::Network,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        },
        DriveTransportKind::Timeout => ClassifiedRecovery {
            kind: TransferErrorKind::Timeout,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        },
        DriveTransportKind::ResponseBody => ClassifiedRecovery {
            kind: TransferErrorKind::Network,
            decision: RecoveryDecision::WaitForNetwork,
            consumes_retry_budget: false,
        },
        DriveTransportKind::Decode if budget_exhausted(context) => {
            permanent(TransferErrorKind::Server)
        }
        DriveTransportKind::Decode => ClassifiedRecovery {
            kind: TransferErrorKind::Server,
            decision: RecoveryDecision::Backoff {
                next_retry_at: exponential_backoff_at(context),
            },
            consumes_retry_budget: true,
        },
        DriveTransportKind::Request | DriveTransportKind::Other => {
            permanent(TransferErrorKind::Unknown)
        }
    }
}

fn classify_status(
    status_code: Option<u16>,
    retry_after: Option<RetryAfter>,
    auth_already_replayed: bool,
    request_may_have_reached_server: bool,
    context: RecoveryContext,
) -> ClassifiedRecovery {
    match status_code {
        Some(401) if !auth_already_replayed => ClassifiedRecovery {
            kind: TransferErrorKind::Auth,
            decision: RecoveryDecision::RefreshAuth,
            consumes_retry_budget: false,
        },
        Some(401) => permanent(TransferErrorKind::Auth),
        Some(429) if budget_exhausted(context) => permanent(TransferErrorKind::RateLimit),
        Some(429) => ClassifiedRecovery {
            kind: TransferErrorKind::RateLimit,
            decision: RecoveryDecision::Backoff {
                next_retry_at: retry_after
                    .map(|retry_after| retry_after.next_retry_at(context.now_ms))
                    .unwrap_or_else(|| exponential_backoff_at(context)),
            },
            consumes_retry_budget: true,
        },
        Some(500 | 502 | 503 | 504) if budget_exhausted(context) => {
            if operation_modifies_remote(context.operation) && request_may_have_reached_server {
                ClassifiedRecovery {
                    kind: TransferErrorKind::Server,
                    decision: RecoveryDecision::VerifyRemote,
                    consumes_retry_budget: false,
                }
            } else {
                permanent(TransferErrorKind::Server)
            }
        }
        Some(500 | 502 | 503 | 504) => ClassifiedRecovery {
            kind: TransferErrorKind::Server,
            decision: RecoveryDecision::Backoff {
                next_retry_at: exponential_backoff_at(context),
            },
            consumes_retry_budget: true,
        },
        Some(400 | 409 | 422) => permanent(TransferErrorKind::Validation),
        Some(403) => permanent(TransferErrorKind::Permission),
        _ => permanent(TransferErrorKind::Unknown),
    }
}

fn budget_exhausted(context: RecoveryContext) -> bool {
    context.attempt_count >= context.max_attempts
}

fn exponential_backoff_at(context: RecoveryContext) -> i64 {
    let exponent = context.attempt_count.min(63);
    let seconds = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX).min(300);
    let delay_ms = seconds
        .saturating_mul(1_000)
        .saturating_add(context.jitter_ms)
        .min(300_000) as i64;
    context.now_ms.saturating_add(delay_ms)
}

fn permanent(kind: TransferErrorKind) -> ClassifiedRecovery {
    ClassifiedRecovery {
        kind,
        decision: RecoveryDecision::Fail,
        consumes_retry_budget: false,
    }
}

fn verify_remote() -> ClassifiedRecovery {
    ClassifiedRecovery {
        kind: TransferErrorKind::RemoteAmbiguous,
        decision: RecoveryDecision::VerifyRemote,
        consumes_retry_budget: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{
        AppError, DriveTransportKind, RequestSemantics, RetryAfter, TokenErrorCode,
    };

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

    #[test]
    fn policy_consumes_and_produces_persistent_transfer_enums() {
        let operation = crate::sync::transfer_state::TransferOperation::Create;
        let recovery_context = RecoveryContext {
            operation,
            attempt_count: 0,
            now_ms: 0,
            jitter_ms: 0,
            auth_already_replayed: false,
            max_attempts: 1,
        };
        let error = AppError::generic("do not replay");

        let kind: crate::sync::transfer_state::TransferErrorKind =
            classify_transfer_error(&error, recovery_context).kind;

        assert_eq!(
            kind,
            crate::sync::transfer_state::TransferErrorKind::Unknown
        );
    }

    #[test]
    fn persistent_operation_remote_write_mapping_is_complete() {
        use crate::sync::transfer_state::TransferOperation::*;

        for (operation, expected) in [
            (Create, true),
            (Update, true),
            (Download, false),
            (DownloadUpdate, false),
            (Delete, true),
            (Move, true),
            (Rename, true),
            (CreateFolder, true),
        ] {
            assert_eq!(operation_modifies_remote(operation), expected);
        }
    }

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

    #[test]
    fn exhausted_server_budget_verifies_writes_but_fails_reads() {
        let write_error =
            AppError::drive_from_response(503, "{}", None, RequestSemantics::Write, false);
        let read_error =
            AppError::drive_from_response(503, "{}", None, RequestSemantics::Read, false);
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
}
