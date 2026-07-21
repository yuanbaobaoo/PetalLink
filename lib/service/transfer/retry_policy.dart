import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/types/enums.dart';

/// 将结构化 Drive/本地错误映射为确定性的恢复决策。
///
/// 严格对齐 Rust 原版 `src/sync/retry_policy.rs`。

/// 判断传输操作是否可能改变云端状态（对齐 Rust `operation_modifies_remote`）。
bool operationModifiesRemote(TransferOperation operation) {
  return switch (operation) {
    TransferOperation.Create ||
    TransferOperation.Update ||
    TransferOperation.Delete ||
    TransferOperation.Move ||
    TransferOperation.Rename ||
    TransferOperation.CreateFolder =>
      true,
    TransferOperation.Download || TransferOperation.DownloadUpdate => false,
  };
}

/// 恢复策略分类所需的持久上下文（对齐 Rust `RecoveryContext`）。
class RecoveryContext {
  /// 任务操作类型
  final TransferOperation operation;

  /// 已消耗的持久化尝试次数
  final int attemptCount;

  /// 当前时间戳（毫秒 epoch）
  final int nowMs;

  /// 抖动毫秒数（TaskRunner 对齐 Rust 传 0）
  final int jitterMs;

  /// 本次执行是否已发生过 401 刷新重放
  final bool authAlreadyReplayed;

  /// 单个任务允许的最大自动重试次数
  final int maxAttempts;

  const RecoveryContext({
    required this.operation,
    required this.attemptCount,
    required this.nowMs,
    this.jitterMs = 0,
    this.authAlreadyReplayed = false,
    required this.maxAttempts,
  });
}

/// 传输失败后应执行的下一步动作（对齐 Rust `RecoveryDecision`）。
sealed class RecoveryDecision {
  const RecoveryDecision();
}

/// 等待网络恢复（不消耗重试预算）
final class WaitForNetworkDecision extends RecoveryDecision {
  const WaitForNetworkDecision();
}

/// 指数/服务端提示退避（消耗重试预算）
final class BackoffDecision extends RecoveryDecision {
  /// 下一次允许重试的时间戳（毫秒 epoch）
  final int nextRetryAt;

  const BackoffDecision(this.nextRetryAt);
}

/// 刷新认证（401 首次；TaskRunner 对齐 Rust 不盲目重放，按失败处理）
final class RefreshAuthDecision extends RecoveryDecision {
  const RefreshAuthDecision();
}

/// 远端写入结果不确定，进入核验子状态
final class VerifyRemoteDecision extends RecoveryDecision {
  const VerifyRemoteDecision();
}

/// 永久失败
final class FailDecision extends RecoveryDecision {
  const FailDecision();
}

/// 结构化错误类型及其恢复决策（对齐 Rust `ClassifiedRecovery`）。
class ClassifiedRecovery {
  /// 可持久错误分类
  final TransferErrorKind kind;

  /// 恢复决策
  final RecoveryDecision decision;

  /// 是否消耗自动重试预算（attempt_count +1）
  final bool consumesRetryBudget;

  const ClassifiedRecovery({
    required this.kind,
    required this.decision,
    required this.consumesRetryBudget,
  });
}

/// 将运行时错误映射为可持久的恢复决策（对齐 Rust `classify_transfer_error`）。
ClassifiedRecovery classifyTransferError(AppError error, RecoveryContext context) {
  return switch (error) {
    QuotaExceededError() => _permanent(TransferErrorKind.Quota),
    TokenError(tokenCode: TokenErrorCode.notLoggedIn) ||
    TokenError(tokenCode: TokenErrorCode.refreshFailed) ||
    AuthError() =>
      _permanent(TransferErrorKind.Auth),
    DriveApiError(driveCode: DriveApiErrorCode.quotaExceeded) =>
      _permanent(TransferErrorKind.Quota),
    DriveApiError(errorCode: 'upload_session_expired') => const ClassifiedRecovery(
        kind: TransferErrorKind.SessionExpired,
        decision: VerifyRemoteDecision(),
        consumesRetryBudget: false,
      ),
    DriveApiError() => _classifyDriveApi(error, context),
    ConfigError() || GenericError() => _permanent(TransferErrorKind.Unknown),
  };
}

/// DriveApi 错误分发：传输阶段错误优先，否则按 HTTP 状态分类。
ClassifiedRecovery _classifyDriveApi(DriveApiError error, RecoveryContext context) {
  final transportKind = error.transportKind;
  if (transportKind != null) {
    return _classifyTransport(
      transportKind,
      error.requestMayHaveReachedServer,
      context,
    );
  }
  return _classifyStatus(
    error.statusCode,
    error.retryAfter,
    error.authAlreadyReplayed || context.authAlreadyReplayed,
    error.requestMayHaveReachedServer,
    context,
  );
}

/// 根据传输阶段与请求送达可能性分类网络错误（对齐 Rust `classify_transport`）。
ClassifiedRecovery _classifyTransport(
  DriveTransportKind transportKind,
  bool requestMayHaveReachedServer,
  RecoveryContext context,
) {
  switch (transportKind) {
    case DriveTransportKind.connect:
      return const ClassifiedRecovery(
        kind: TransferErrorKind.Network,
        decision: WaitForNetworkDecision(),
        consumesRetryBudget: false,
      );
    case DriveTransportKind.network:
    case DriveTransportKind.timeout:
    case DriveTransportKind.responseBody:
    case DriveTransportKind.decode:
    case DriveTransportKind.request:
    case DriveTransportKind.other:
      // 写操作非 connect 阶段失败且请求可能已送达 → 禁止盲目重放，先核验远端
      if (operationModifiesRemote(context.operation) &&
          requestMayHaveReachedServer) {
        return _verifyRemote();
      }
      switch (transportKind) {
        case DriveTransportKind.network:
        case DriveTransportKind.responseBody:
          return const ClassifiedRecovery(
            kind: TransferErrorKind.Network,
            decision: WaitForNetworkDecision(),
            consumesRetryBudget: false,
          );
        case DriveTransportKind.timeout:
          return const ClassifiedRecovery(
            kind: TransferErrorKind.Timeout,
            decision: WaitForNetworkDecision(),
            consumesRetryBudget: false,
          );
        case DriveTransportKind.decode:
          if (_budgetExhausted(context)) {
            return _permanent(TransferErrorKind.Server);
          }
          return ClassifiedRecovery(
            kind: TransferErrorKind.Server,
            decision: BackoffDecision(_exponentialBackoffAt(context)),
            consumesRetryBudget: true,
          );
        case DriveTransportKind.request:
        case DriveTransportKind.other:
          return _permanent(TransferErrorKind.Unknown);
        case DriveTransportKind.connect:
          // 已在上方提前返回
          return _permanent(TransferErrorKind.Unknown);
      }
  }
}

/// 根据 HTTP 状态、重试预算与写入语义选择恢复方式（对齐 Rust `classify_status`）。
ClassifiedRecovery _classifyStatus(
  int? statusCode,
  RetryAfter? retryAfter,
  bool authAlreadyReplayed,
  bool requestMayHaveReachedServer,
  RecoveryContext context,
) {
  switch (statusCode) {
    case 401:
      if (!authAlreadyReplayed) {
        return const ClassifiedRecovery(
          kind: TransferErrorKind.Auth,
          decision: RefreshAuthDecision(),
          consumesRetryBudget: false,
        );
      }
      return _permanent(TransferErrorKind.Auth);
    case 429:
      if (_budgetExhausted(context)) {
        return _permanent(TransferErrorKind.RateLimit);
      }
      return ClassifiedRecovery(
        kind: TransferErrorKind.RateLimit,
        decision: BackoffDecision(
          retryAfter?.nextRetryAt(context.nowMs) ??
              _exponentialBackoffAt(context),
        ),
        consumesRetryBudget: true,
      );
    case 500:
    case 502:
    case 503:
    case 504:
      if (_budgetExhausted(context)) {
        if (operationModifiesRemote(context.operation) &&
            requestMayHaveReachedServer) {
          return const ClassifiedRecovery(
            kind: TransferErrorKind.Server,
            decision: VerifyRemoteDecision(),
            consumesRetryBudget: false,
          );
        }
        return _permanent(TransferErrorKind.Server);
      }
      return ClassifiedRecovery(
        kind: TransferErrorKind.Server,
        decision: BackoffDecision(_exponentialBackoffAt(context)),
        consumesRetryBudget: true,
      );
    case 400:
    case 409:
    case 422:
      return _permanent(TransferErrorKind.Validation);
    case 403:
      return _permanent(TransferErrorKind.Permission);
    default:
      return _permanent(TransferErrorKind.Unknown);
  }
}

/// 判断当前传输是否已用尽自动重试预算。
bool _budgetExhausted(RecoveryContext context) {
  return context.attemptCount >= context.maxAttempts;
}

/// 计算包含抖动且不超过上限的下次重试时间（对齐 Rust `exponential_backoff_at`）。
int _exponentialBackoffAt(RecoveryContext context) {
  final exponent = context.attemptCount < 63 ? context.attemptCount : 63;
  // 2^9=512 已超 300s 上限；指数 ≥9 直接取上限，避免 Dart 有符号位移溢出为负
  // （对齐 Rust checked_shl + min(300) 语义；手动重试可累积 attempt_count 到 63+）。
  final seconds = exponent >= 9 ? 300 : 1 << exponent;
  var delayMs = seconds * 1000 + context.jitterMs;
  if (delayMs > 300000) delayMs = 300000;
  return context.nowMs + delayMs;
}

/// 构造不再自动重试的永久失败决策。
ClassifiedRecovery _permanent(TransferErrorKind kind) {
  return ClassifiedRecovery(
    kind: kind,
    decision: const FailDecision(),
    consumesRetryBudget: false,
  );
}

/// 构造需向云端核实写入结果的歧义决策。
ClassifiedRecovery _verifyRemote() {
  return const ClassifiedRecovery(
    kind: TransferErrorKind.RemoteAmbiguous,
    decision: VerifyRemoteDecision(),
    consumesRetryBudget: false,
  );
}
