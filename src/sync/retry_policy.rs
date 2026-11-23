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
            // 预算耗尽后不再转入远端核验：核验后确认未提交的重放路径会重新执行上传，
            // 不消耗预算的核验决策会让「失败→核验→未提交→重放」形成无限循环
            // （2026-09-15 五个大文件任务各循环 2,200+ 次）。
            if budget_exhausted(context) {
                permanent(TransferErrorKind::RemoteAmbiguous)
            } else {
                verify_remote()
            }
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
            // 预算耗尽即终态失败，不再做最后一次核验：写已落云的场景由人工重试的
            // 同名碰撞检查 → 远端核验收敛（见 transfer_operations 执行前碰撞检查）。
            // 可能已提交的写保留歧义分类，读与未提交的写维持服务端分类。
            if operation_modifies_remote(context.operation) && request_may_have_reached_server {
                permanent(TransferErrorKind::RemoteAmbiguous)
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

/// 单任务自动重试的退避阶梯（秒）：30s / 2m / 10m / 30m / 1h。
///
/// 旧实现按 1/2/4/8/16 秒递增、5 次预算合计约 31 秒，秒级抖动就会把任务打到终态；
/// 这条阶梯把窗口拉到约 1.7 小时，同时保留「预算耗尽即终态」的上限，
/// 不会因为放宽曲线重新引入无限重试。
const RETRY_BACKOFF_LADDER_SECS: [u64; 5] = [30, 120, 600, 1_800, 3_600];

/// 本轮退避时长（毫秒）。超出阶梯长度时取最后一档。
pub fn retry_backoff_ms(attempt_count: u32) -> u64 {
    // 阶梯索引用饱和转换，避免大 attempt_count 溢出。
    let index = usize::try_from(attempt_count)
        .unwrap_or(usize::MAX)
        .min(RETRY_BACKOFF_LADDER_SECS.len() - 1);
    RETRY_BACKOFF_LADDER_SECS[index].saturating_mul(1_000)
}

/// 计算 0～20% 的确定性抖动（毫秒）。
///
/// 同一任务同一轮次得到稳定值，便于复现与断言；不同任务错开，
/// 避免一批任务在同一秒齐刷刷重试再次压垮服务端。
pub fn retry_jitter_ms(attempt_count: u32, task_id: i64) -> u64 {
    // 混合任务 ID 与轮次，得到一个稳定的伪随机种子。
    let seed = (task_id.unsigned_abs())
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(attempt_count).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    // 抖动上限为本轮退避时长的 20%。
    let ceiling = retry_backoff_ms(attempt_count) / 5;
    if ceiling == 0 {
        return 0;
    }
    (seed >> 33) % ceiling
}

/// 有上限的指数退避秒数：1, 2, 4, 8, ... 直到 cap（指数溢出按最大值处理）。
pub(crate) fn capped_backoff_secs(exponent: u32, cap_secs: u64) -> u64 {
    1_u64
        .checked_shl(exponent.min(63))
        .unwrap_or(u64::MAX)
        .min(cap_secs)
}

/// 按退避阶梯与抖动计算下次重试时间。
///
/// 抖动由调用方按任务 ID 生成（见 `retry_jitter_ms`），这里不再截断到 5 分钟——
/// 阶梯最后一档本身就是 1 小时。
fn exponential_backoff_at(context: RecoveryContext) -> i64 {
    let delay_ms = retry_backoff_ms(context.attempt_count).saturating_add(context.jitter_ms);
    context
        .now_ms
        .saturating_add(i64::try_from(delay_ms).unwrap_or(i64::MAX))
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
/// 消耗重试预算：远端写入歧义说明一次完整上传尝试已失败，
/// 不消耗预算会让核验-重放环路绕过 MAX_AUTOMATIC_ATTEMPTS 永不终止。
fn verify_remote() -> ClassifiedRecovery {
    ClassifiedRecovery {
        kind: TransferErrorKind::RemoteAmbiguous,
        decision: RecoveryDecision::VerifyRemote,
        consumes_retry_budget: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 退避阶梯必须覆盖 30s / 2m / 10m / 30m / 1h，且超出长度取末档。
    #[test]
    fn retry_backoff_follows_configured_ladder() {
        assert_eq!(retry_backoff_ms(0), 30_000);
        assert_eq!(retry_backoff_ms(1), 120_000);
        assert_eq!(retry_backoff_ms(2), 600_000);
        assert_eq!(retry_backoff_ms(3), 1_800_000);
        assert_eq!(retry_backoff_ms(4), 3_600_000);
        // 超出阶梯长度与超大轮次都落回末档，不溢出。
        assert_eq!(retry_backoff_ms(9), 3_600_000);
        assert_eq!(retry_backoff_ms(u32::MAX), 3_600_000);
    }

    /// 抖动必须落在 [0, 20% 退避时长) 内，且同一任务同一轮次稳定可复现。
    #[test]
    fn retry_jitter_is_bounded_and_deterministic() {
        for attempt in 0..6 {
            let ceiling = retry_backoff_ms(attempt) / 5;
            let jitter = retry_jitter_ms(attempt, 42);
            assert!(jitter < ceiling, "抖动越界：{jitter} >= {ceiling}");
            assert_eq!(jitter, retry_jitter_ms(attempt, 42), "同输入必须稳定");
        }
        // 不同任务应当错开，避免同一秒集体重试。
        assert_ne!(retry_jitter_ms(3, 1), retry_jitter_ms(3, 2));
        assert_ne!(retry_jitter_ms(3, 2), retry_jitter_ms(3, 3));
    }
}
