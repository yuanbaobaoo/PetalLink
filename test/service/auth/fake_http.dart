import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';

/// 记录的一次 HTTP 请求（供测试断言）。
class FakeRequest {
  /// HTTP 方法
  final String method;

  /// 完整请求 URI（含 query）
  final Uri uri;

  /// 请求体文本（form body / JSON 等）
  final String body;

  /// 请求头
  final Map<String, dynamic> headers;

  const FakeRequest({
    required this.method,
    required this.uri,
    required this.body,
    required this.headers,
  });
}

/// 假请求处理器：返回响应体或抛出异常（模拟传输失败）。
typedef FakeHttpHandler = FutureOr<ResponseBody> Function(FakeRequest request);

/// 测试用 Dio HttpClientAdapter：按 handler 路由，记录全部请求。
class FakeHttpAdapter implements HttpClientAdapter {
  final FakeHttpHandler _handler;

  /// 已接收的请求（按到达顺序）
  final List<FakeRequest> requests = [];

  FakeHttpAdapter(this._handler);

  /// 构造接入本 adapter 的 Dio（接受任意状态码，与生产配置一致）。
  Dio createDio() {
    final dio = Dio(BaseOptions(validateStatus: (_) => true));
    dio.httpClientAdapter = this;
    return dio;
  }

  /// 匹配请求（method + host + path，忽略 query）。
  static bool match(
    FakeRequest request,
    String method,
    String host,
    String path,
  ) {
    return request.method == method &&
        request.uri.host == host &&
        request.uri.path == path;
  }

  @override
  Future<ResponseBody> fetch(
    RequestOptions options,
    Stream<Uint8List>? requestStream,
    Future<void>? cancelFuture,
  ) async {
    final body = requestStream == null
        ? ''
        : await utf8.decoder.bind(requestStream).join();
    final request = FakeRequest(
      method: options.method,
      uri: options.uri,
      body: body,
      headers: options.headers,
    );
    requests.add(request);
    return _handler(request);
  }

  @override
  void close({bool force = false}) {}
}

/// 构造 JSON 响应体。
ResponseBody jsonResponse(Map<String, dynamic> json, {int status = 200}) {
  return ResponseBody.fromString(
    jsonEncode(json),
    status,
    headers: {
      Headers.contentTypeHeader: ['application/json'],
    },
  );
}

/// 构造纯文本响应体。
ResponseBody textResponse(String text, {int status = 200}) {
  return ResponseBody.fromString(
    text,
    status,
    headers: {
      Headers.contentTypeHeader: ['text/plain'],
    },
  );
}
