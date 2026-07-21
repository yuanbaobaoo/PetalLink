import 'dart:convert';
import 'dart:io';

import 'package:dio/dio.dart';

/// 统一异常类型 —— Service 层抛出这些类型，UI 层据此渲染友好提示。
///
/// 严格对齐 Rust 原版 `src/error.rs` 的 [AppError]：
/// kinds = Auth / Token / DriveApi / Config / quotaExceeded / Generic。
///
/// 安全：所有序列化/toString 输出均不泄露 token，错误消息只包含用户可读的中文描述。
sealed class AppError implements Exception {
  /// 用户可读的错误消息（中文）
  final String message;

  const AppError({required this.message});

  /// 错误类别名（与 Rust 序列化的 `kind` 字段一致，供 UI 按类别渲染）
  String get kind;

  /// 错误子码（snake_case，与 Rust 序列化的 `code` 字段一致；无子码时为 null）
  String? get code => null;

  /// HTTP 状态码（仅 [DriveApiError] 可能携带）
  int? get statusCode => null;

  /// 华为业务错误码（从错误响应体解析，仅 [DriveApiError] 可能携带）
  String? get errorCode => null;

  /// 仅从结构化 Drive 元数据读取 HTTP 状态；绝不解析用户可读消息。
  int? get driveStatus => null;

  /// 序列化为扁平结构（对齐 Rust 自定义 Serialize）：
  /// `{kind, code, message, status_code, error_code}`。
  Map<String, dynamic> toJson() {
    return {
      'kind': kind,
      'code': code,
      'message': message,
      'status_code': statusCode,
      'error_code': errorCode,
    };
  }

  @override
  String toString() {
    final buf = StringBuffer('$kind(');
    buf.write('message: $message');
    final c = code;
    if (c != null) buf.write(', code: $c');
    final sc = statusCode;
    if (sc != null) buf.write(', statusCode: $sc');
    final ec = errorCode;
    if (ec != null) buf.write(', errorCode: $ec');
    buf.write(')');
    return buf.toString();
  }

  // ===== Auth 工厂 =====

  /// 用户主动取消授权（非错误，UI 不应显示为失败）
  static AppError authCancelled() =>
      const AuthError(authCode: AuthErrorCode.cancelled, message: '用户取消授权');

  /// state 不匹配（防 CSRF）
  static AppError authStateMismatch() => const AuthError(
      authCode: AuthErrorCode.stateMismatch, message: '授权回调 state 校验失败，请重试');

  /// 回调超时
  static AppError authTimeout() =>
      const AuthError(authCode: AuthErrorCode.timeout, message: '登录超时，请重新登录');

  /// 华为返回 error 参数
  static AppError authDenied(String? errorDescription) => AuthError(
        authCode: AuthErrorCode.denied,
        message: errorDescription != null ? '授权失败：$errorDescription' : '授权被拒绝',
      );

  /// 浏览器无法打开
  static AppError authBrowserLaunchFailed() => const AuthError(
      authCode: AuthErrorCode.browserLaunchFailed, message: '无法打开浏览器，请检查系统设置');

  /// 未收到授权码
  static AppError authInvalidCode() =>
      const AuthError(authCode: AuthErrorCode.invalidCode, message: '未收到授权码');

  /// token 响应格式异常
  static AppError authTokenResponseInvalid() => const AuthError(
      authCode: AuthErrorCode.tokenResponseInvalid, message: 'token 响应格式异常');

  // ===== Token 工厂 =====

  /// 尚未登录
  static AppError tokenNotLoggedIn() =>
      const TokenError(tokenCode: TokenErrorCode.notLoggedIn, message: '尚未登录');

  /// Token 刷新失败
  static AppError tokenRefreshFailed([String? cause]) => TokenError(
        tokenCode: TokenErrorCode.refreshFailed,
        message: cause != null ? 'Token 刷新失败：$cause' : 'Token 刷新失败，请重新登录',
      );

  // ===== DriveApi 工厂 =====

  /// 从 HTTP 状态码构造（华为 4xx 错误体在 body 里携带 code/description）
  static AppError driveFromStatus(int statusCode, String body) =>
      driveFromResponse(statusCode, body);

  /// 从服务端错误响应构造，保留恢复策略需要的结构化元数据。
  static AppError driveFromResponse(
    int statusCode,
    String body, {
    RetryAfter? retryAfter,
    RequestSemantics semantics = RequestSemantics.read,
    bool authAlreadyReplayed = false,
  }) {
    return DriveApiError(
      driveCode: DriveApiErrorCode.fromStatus,
      message: '云端请求失败 ($statusCode)',
      statusCode: statusCode,
      errorCode: parseHuaweiErrorCode(body),
      retryAfter: retryAfter,
      requestMayHaveReachedServer: semantics.isWrite,
      authAlreadyReplayed: authAlreadyReplayed,
    );
  }

  /// 断点上传会话已失效，但创建新会话前仍须复核目标写入是否到达远端。
  static AppError driveUploadSessionExpired(
    int statusCode, {
    bool authAlreadyReplayed = false,
  }) {
    return DriveApiError(
      driveCode: DriveApiErrorCode.fromStatus,
      message: '断点上传会话已失效 ($statusCode)',
      statusCode: statusCode,
      errorCode: 'upload_session_expired',
      // 失效会话可能已接收早先分片或最终写入；丢弃持久化会话身份前必须复核目标。
      requestMayHaveReachedServer: true,
      authAlreadyReplayed: authAlreadyReplayed,
    );
  }

  /// 配额不足
  static AppError driveQuotaExceeded() => const DriveApiError(
        driveCode: DriveApiErrorCode.quotaExceeded,
        message: '云盘空间不足',
        errorCode: 'quota_exceeded',
      );

  /// 网络连接失败
  static AppError driveNetwork([String? cause]) => driveTransport(
        DriveTransportKind.network,
        cause: cause,
      );

  /// 从传输失败构造，供 [MateHttpClient] 及直接上传/下载请求复用。
  ///
  /// 写请求（非 connect 阶段失败）保守标记「可能已到达服务端」。
  static AppError driveTransport(
    DriveTransportKind transportKind, {
    RequestSemantics semantics = RequestSemantics.read,
    bool authAlreadyReplayed = false,
    String? cause,
  }) {
    return driveTransportWithSubmission(
      transportKind,
      requestMayHaveReachedServer:
          semantics.isWrite && transportKind != DriveTransportKind.connect,
      authAlreadyReplayed: authAlreadyReplayed,
      cause: cause,
    );
  }

  /// 从已知提交阶段的传输失败构造；直接流式请求可显式保留提交不确定性。
  static AppError driveTransportWithSubmission(
    DriveTransportKind transportKind, {
    required bool requestMayHaveReachedServer,
    bool authAlreadyReplayed = false,
    String? cause,
  }) {
    return DriveApiError(
      driveCode: DriveApiErrorCode.network,
      message: transportKind == DriveTransportKind.decode ? '云端响应异常' : '网络连接失败，请检查网络',
      transportKind: transportKind,
      requestMayHaveReachedServer: requestMayHaveReachedServer,
      authAlreadyReplayed: authAlreadyReplayed,
    );
  }

  // ===== Config / Quota / Generic 工厂 =====

  /// 构造配置读写或校验错误。
  static AppError config(String message) => ConfigError(message: message);

  /// 构造包含所需与剩余字节数的配额不足错误。
  static AppError quotaExceeded(int required, int remaining) => QuotaExceededError(
        required: required,
        remaining: remaining,
        message: '空间不足：需要 $required 字节，剩余 $remaining 字节',
      );

  /// 构造文件系统、解析等通用错误。
  static AppError generic(String message) => GenericError(message: message);

  // ===== Dio 适配 =====

  /// 从 [DioException] 构造 [AppError]。
  ///
  /// 对齐 Rust `classify_transport_error` + `handle_error_response_with_metadata`：
  /// - 有响应 → 按状态码构造 DriveApi（解析 Retry-After 与华为错误码）
  /// - 无响应 → 按 Dio 异常类型分类传输阶段（connect/timeout/...），
  ///   写请求保守标记「可能已到达服务端」
  factory AppError.fromDioException(
    DioException e, {
    RequestSemantics semantics = RequestSemantics.read,
    bool authAlreadyReplayed = false,
  }) {
    // 请求取消：不属于传输失败，归为通用错误
    if (e.type == DioExceptionType.cancel) {
      return GenericError(message: e.message ?? '请求已取消');
    }

    // 已携带 AppError（如 401 刷新失败由拦截器注入）
    final inner = e.error;
    if (inner is AppError) return inner;

    // 有响应：按服务端错误处理
    final response = e.response;
    if (response != null) {
      final statusCode = response.statusCode ?? 0;
      return driveFromResponse(
        statusCode,
        responseBodyString(response),
        retryAfter: RetryAfter.tryParse(
            response.headers.value(HttpHeaders.retryAfterHeader)),
        semantics: semantics,
        authAlreadyReplayed: authAlreadyReplayed,
      );
    }

    // 无响应：传输阶段分类
    final kind = switch (e.type) {
      DioExceptionType.connectionError => DriveTransportKind.connect,
      DioExceptionType.connectionTimeout ||
      DioExceptionType.sendTimeout ||
      DioExceptionType.receiveTimeout ||
      DioExceptionType.transformTimeout =>
        DriveTransportKind.timeout,
      DioExceptionType.badCertificate => DriveTransportKind.connect,
      DioExceptionType.badResponse => DriveTransportKind.responseBody,
      DioExceptionType.unknown => inner is SocketException
          ? DriveTransportKind.connect
          : DriveTransportKind.other,
      DioExceptionType.cancel => DriveTransportKind.other,
    };
    return driveTransport(
      kind,
      semantics: semantics,
      authAlreadyReplayed: authAlreadyReplayed,
      cause: e.message,
    );
  }

  /// 从 HTTP 状态码和响应体构造 [AppError]（读语义）。
  factory AppError.fromStatusCode(int statusCode, String body) =>
      driveFromResponse(statusCode, body);

  /// 从 Dio Response 中提取响应体字符串（JSON body 重新编码，避免 `[object Object]` 式丢失）
  static String responseBodyString(Response<dynamic>? response) {
    final data = response?.data;
    if (data == null) return '';
    if (data is String) return data;
    if (data is Map || data is List) {
      try {
        return jsonEncode(data);
      } catch (_) {
        return data.toString();
      }
    }
    return data.toString();
  }

  /// 从华为错误响应的常见结构中提取错误码。
  ///
  /// 支持 `{"errorCode": "xxx"}` 与 `{"error": {"errorCode": xxx}}` 两种结构，
  /// 错误码为数字时转成字符串。
  static String? parseHuaweiErrorCode(String body) {
    final Object? value;
    try {
      value = jsonDecode(body);
    } catch (_) {
      return null;
    }
    if (value is! Map) return null;
    final errorCode = value['errorCode'] ??
        (value['error'] is Map ? (value['error'] as Map)['errorCode'] : null);
    if (errorCode is String) return errorCode;
    if (errorCode is num) return errorCode.toString();
    return null;
  }
}

/// OAuth 流程相关（取消 / state 不匹配 / 超时 / 被拒绝 / 浏览器打不开）
final class AuthError extends AppError {
  /// OAuth 错误子码
  final AuthErrorCode authCode;

  const AuthError({required this.authCode, required super.message});

  @override
  String get kind => 'Auth';

  @override
  String get code => authCode.wireName;
}

/// Token 相关（未登录 / 刷新失败）
final class TokenError extends AppError {
  /// Token 错误子码
  final TokenErrorCode tokenCode;

  const TokenError({required this.tokenCode, required super.message});

  @override
  String get kind => 'Token';

  @override
  String get code => tokenCode.wireName;
}

/// Drive API 调用异常（状态码 / 华为错误码 / 网络）
final class DriveApiError extends AppError {
  /// Drive API 错误子码
  final DriveApiErrorCode driveCode;

  @override
  final int? statusCode;

  @override
  final String? errorCode;

  /// 已解析的 HTTP `Retry-After`（服务端限流提示）
  final RetryAfter? retryAfter;

  /// 传输失败发生的阶段（仅传输类错误携带）
  final DriveTransportKind? transportKind;

  /// 失败时写入是否可能已到达服务端（恢复策略据此保守处理）
  final bool requestMayHaveReachedServer;

  /// 是否已经是 401 刷新后的重放请求
  final bool authAlreadyReplayed;

  const DriveApiError({
    required this.driveCode,
    required super.message,
    this.statusCode,
    this.errorCode,
    this.retryAfter,
    this.transportKind,
    this.requestMayHaveReachedServer = false,
    this.authAlreadyReplayed = false,
  });

  @override
  String get kind => 'DriveApi';

  @override
  String get code => driveCode.wireName;

  @override
  int? get driveStatus => statusCode;
}

/// 配置相关
final class ConfigError extends AppError {
  const ConfigError({required super.message});

  @override
  String get kind => 'Config';
}

/// 配额不足（上传前校验）
final class QuotaExceededError extends AppError {
  /// 需要的字节数
  final int required;

  /// 剩余的字节数
  final int remaining;

  const QuotaExceededError({
    required this.required,
    required this.remaining,
    required super.message,
  });

  @override
  String get kind => 'QuotaExceeded';
}

/// 通用错误（文件系统、序列化等）
final class GenericError extends AppError {
  const GenericError({required super.message});

  @override
  String get kind => 'Generic';
}

/// OAuth 错误子码
enum AuthErrorCode {
  /// 用户取消授权
  cancelled,

  /// state 校验失败
  stateMismatch,

  /// 回调超时
  timeout,

  /// 授权被拒绝
  denied,

  /// 浏览器打不开
  browserLaunchFailed,

  /// 未收到授权码
  invalidCode,

  /// token 响应格式异常
  tokenResponseInvalid,

  /// scope 无效
  scopeInvalid;

  /// 序列化用 snake_case 名（对齐 Rust serde）
  String get wireName => switch (this) {
        AuthErrorCode.cancelled => 'cancelled',
        AuthErrorCode.stateMismatch => 'state_mismatch',
        AuthErrorCode.timeout => 'timeout',
        AuthErrorCode.denied => 'denied',
        AuthErrorCode.browserLaunchFailed => 'browser_launch_failed',
        AuthErrorCode.invalidCode => 'invalid_code',
        AuthErrorCode.tokenResponseInvalid => 'token_response_invalid',
        AuthErrorCode.scopeInvalid => 'scope_invalid',
      };
}

/// Token 错误子码
enum TokenErrorCode {
  /// 尚未登录
  notLoggedIn,

  /// 刷新失败
  refreshFailed;

  /// 序列化用 snake_case 名（对齐 Rust serde）
  String get wireName => switch (this) {
        TokenErrorCode.notLoggedIn => 'not_logged_in',
        TokenErrorCode.refreshFailed => 'refresh_failed',
      };
}

/// Drive API 错误子码
enum DriveApiErrorCode {
  /// 通用 HTTP 状态码错误
  fromStatus,

  /// 配额不足
  quotaExceeded,

  /// 网络连接失败
  network;

  /// 序列化用 snake_case 名（对齐 Rust serde）
  String get wireName => switch (this) {
        DriveApiErrorCode.fromStatus => 'from_status',
        DriveApiErrorCode.quotaExceeded => 'quota_exceeded',
        DriveApiErrorCode.network => 'network',
      };
}

/// 请求的副作用语义。传输层据此保守记录失败时写入是否可能已到达服务端。
enum RequestSemantics {
  /// 只读请求（GET/HEAD/OPTIONS）
  read,

  /// 写请求（POST/PUT/PATCH/DELETE）
  write;

  /// 判断请求是否可能对服务端状态产生写入副作用。
  bool get isWrite => this == RequestSemantics.write;
}

/// HTTP 传输失败发生的阶段。
enum DriveTransportKind {
  /// 未分类网络错误
  network,

  /// 连接建立阶段失败（写请求此阶段失败可安全重试）
  connect,

  /// 超时
  timeout,

  /// 请求构造/发送阶段失败
  request,

  /// 响应体读取失败
  responseBody,

  /// 响应解码失败
  decode,

  /// 其他
  other,
}

/// 已解析的 HTTP `Retry-After`。
sealed class RetryAfter {
  const RetryAfter();

  /// 将服务端重试提示换算为不早于当前时刻的毫秒时间戳。
  int nextRetryAt(int nowMs);

  /// 解析 `Retry-After` 的 delta-seconds 或 IMF-fixdate 形式。
  static RetryAfter? tryParse(String? value) {
    if (value == null) return null;
    final trimmed = value.trim();
    final seconds = int.tryParse(trimmed);
    if (seconds != null && seconds >= 0) {
      return RetryAfterDelay(seconds);
    }
    try {
      // HttpDate.parse 支持 IMF-fixdate（RFC 1123）
      return RetryAfterAt(HttpDate.parse(trimmed).millisecondsSinceEpoch);
    } catch (_) {
      return null;
    }
  }
}

/// delta-seconds 形式：相对当前时刻的秒数
final class RetryAfterDelay extends RetryAfter {
  /// 延迟秒数
  final int seconds;

  const RetryAfterDelay(this.seconds);

  @override
  int nextRetryAt(int nowMs) {
    // 防溢出（对齐 Rust saturating 语义）
    final safeSeconds = seconds > _maxMillis ~/ 1000 ? _maxMillis ~/ 1000 : seconds;
    return nowMs + safeSeconds * 1000;
  }

  /// int64 最大毫秒数
  static const int _maxMillis = 0x7FFFFFFFFFFFFFFF;
}

/// IMF-fixdate 形式：绝对毫秒时间戳
final class RetryAfterAt extends RetryAfter {
  /// 绝对重试时刻（epoch 毫秒）
  final int unixMs;

  const RetryAfterAt(this.unixMs);

  @override
  int nextRetryAt(int nowMs) => unixMs > nowMs ? unixMs : nowMs;
}
