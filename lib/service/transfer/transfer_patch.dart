import 'package:petal_link/types/enums.dart';

/// 可空列三态补丁（对齐 Rust `ColumnPatch<T>`）：保留 / 设值 / 清空。
///
/// 用于状态迁移时精确表达「不改此列」「写入新值」「写入 NULL」三种意图，
/// 避免用可空参数传 null 时无法区分「不传」与「显式清空」。
sealed class ColumnPatch<T> {
  const ColumnPatch._();

  /// 保留当前数据库值
  const factory ColumnPatch.keep() = KeepPatch<T>;

  /// 替换当前值
  const factory ColumnPatch.set(T value) = SetPatch<T>;

  /// 写入 SQL NULL
  const factory ColumnPatch.clear() = ClearPatch<T>;
}

/// 保留当前数据库值
final class KeepPatch<T> extends ColumnPatch<T> {
  const KeepPatch() : super._();
}

/// 替换当前值
final class SetPatch<T> extends ColumnPatch<T> {
  /// 新值
  final T value;

  const SetPatch(this.value) : super._();
}

/// 写入 SQL NULL
final class ClearPatch<T> extends ColumnPatch<T> {
  const ClearPatch() : super._();
}

/// 一次状态迁移附带的可变字段更新（对齐 Rust `TransferPatch`）。
///
/// 所有 [ColumnPatch] 字段默认 [ColumnPatch.keep]（不动该列）；
/// 计数字段为 null 时保留原值。
class TransferPatch {
  /// 结构化错误类型
  final ColumnPatch<TransferErrorKind> errorKind;

  /// 错误消息
  final ColumnPatch<String> errorMessage;

  /// 下一次允许重试的时间戳（毫秒 epoch）
  final ColumnPatch<int> nextRetryAt;

  /// 完成时间（毫秒 epoch）
  final ColumnPatch<int> finishedAt;

  /// 远端结果复核确认的资源 fileId
  final ColumnPatch<String> remoteResultFileId;

  /// 华为 resume 上传会话 URL
  final ColumnPatch<String> sessionUrl;

  /// 已传输字节数（null 保留原值）
  final int? transferred;

  /// 断点偏移（null 保留原值）
  final int? resumeOffset;

  /// 已消耗的持久化尝试次数（null 保留原值）
  final int? attemptCount;

  /// 原子失效全部持久化断点上传身份（对齐 Rust
  /// `transition_transfer_clearing_upload_session`）：
  /// session_url=NULL、server_id=NULL、upload_id=NULL、
  /// transferred=0、resume_offset=0。
  /// 仅当远端复核确认目标写入不存在后方可置位。
  final bool clearUploadSession;

  const TransferPatch({
    this.errorKind = const KeepPatch(),
    this.errorMessage = const KeepPatch(),
    this.nextRetryAt = const KeepPatch(),
    this.finishedAt = const KeepPatch(),
    this.remoteResultFileId = const KeepPatch(),
    this.sessionUrl = const KeepPatch(),
    this.transferred,
    this.resumeOffset,
    this.attemptCount,
    this.clearUploadSession = false,
  });

  /// 清错误补丁：error_kind/error_message/next_retry_at/finished_at 全部清空
  /// （对齐 Rust 多处 `error_kind: Clear, error_message: Clear,
  /// next_retry_at: Clear, finished_at: Clear` 组合）。
  const TransferPatch.clearingError({
    this.errorKind = const ClearPatch(),
    this.errorMessage = const ClearPatch(),
    this.nextRetryAt = const ClearPatch(),
    this.finishedAt = const ClearPatch(),
    this.remoteResultFileId = const KeepPatch(),
    this.sessionUrl = const KeepPatch(),
    this.transferred,
    this.resumeOffset,
    this.attemptCount,
    this.clearUploadSession = false,
  });
}
