/// Drive 服务层共享的 HTTP 辅助（内部实现细节，非公开服务）。
library;

import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';

/// 从非 2xx 裸响应构造结构化错误。
///
/// 对齐 Rust `handle_error_response_with_metadata`：解析 Retry-After 与华为
/// 错误码，保留请求语义与 401 重放状态。
AppError httpErrorFromResponse(
  Response<dynamic> response,
  RequestSemantics semantics,
  bool authAlreadyReplayed,
) {
  return AppError.driveFromResponse(
    response.statusCode ?? 0,
    AppError.responseBodyString(response),
    retryAfter: RetryAfter.tryParse(response.headers.value('Retry-After')),
    semantics: semantics,
    authAlreadyReplayed: authAlreadyReplayed,
  );
}
