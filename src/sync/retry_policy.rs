//! 将结构化 Drive/本地错误映射为确定性的恢复决策。

use crate::error::{AppError, DriveApiErrorCode, DriveTransportKind, RetryAfter, TokenErrorCode};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation};

/// 判断传输操作是否可能改变云端状态。
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
/// 恢复策略分类所需的持久上下文。
pub struct RecoveryContext {
    pub operation: TransferOperation,
    pub attempt_count: u32,
    pub now_ms: i64,
    pub jitter_ms: u64,
    pub auth_already_replayed: bool,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 传输失败后应执行的下一步动作。
pub enum RecoveryDecision {
    WaitForNetwork,
    Backoff { next_retry_at: i64 },
    RefreshAuth,
    VerifyRemote,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 结构化错误类型及其恢复决策。
pub struct ClassifiedRecovery {
    pub kind: TransferErrorKind,
    pub decision: RecoveryDecision,
    pub consumes_retry_budget: bool,
}

/// 将运行时错误映射为可持久的恢复决策。
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

/// 根据传输阶段与请求送达可能性分类网络错误。
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

/// 根据 HTTP 状态、重试预算与写入语义选择恢复方式。
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

/// 判断当前传输是否已用尽自动重试预算。
fn budget_exhausted(context: RecoveryContext) -> bool {
    context.attempt_count >= context.max_attempts
}

/// 计算包含抖动且不超过上限的下次重试时间。
fn exponential_backoff_at(context: RecoveryContext) -> i64 {
    let exponent = context.attempt_count.min(63);
    let seconds = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX).min(300);
    let delay_ms = seconds
        .saturating_mul(1_000)
        .saturating_add(context.jitter_ms)
        .min(300_000) as i64;
    context.now_ms.saturating_add(delay_ms)
}

/// 构造不再自动重试的永久失败决策。
fn permanent(kind: TransferErrorKind) -> ClassifiedRecovery {
    ClassifiedRecovery {
        kind,
        decision: RecoveryDecision::Fail,
        consumes_retry_budget: false,
    }
}

/// 构造需向云端核实写入结果的歧义决策。
fn verify_remote() -> ClassifiedRecovery {
    ClassifiedRecovery {
        kind: TransferErrorKind::RemoteAmbiguous,
        decision: RecoveryDecision::VerifyRemote,
        consumes_retry_budget: false,
    }
}
