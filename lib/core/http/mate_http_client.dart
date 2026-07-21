import 'dart:convert';

import 'package:dio/dio.dart';

// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';

/// HTTP 客户端封装（严格对齐 Rust 原版 `src/drive/client.rs` 的 DriveClient）。
///
/// 职责：
/// - 连接 15s / 发送 60s / 接收 60s 超时
/// - Bearer token 自动注入（通过 [tokenProvider] 回调获取；
///   `oauth2/v3/token` 端点不注入）
/// - 401 → 强制刷新 token（singleflight：并发 401 只刷新一次）后单次重放
/// - 非 2xx 响应统一转为 [AppError]（解析 Retry-After 与华为错误码）
/// - 传输错误按阶段分类（connect/timeout/...），按读/写语义保守标记
///   「写请求是否可能已到达服务端」
/// - 所有 HTTP 方法返回 [AppResult]
///
/// 用法：
/// ```dart
/// final client = MateHttpClient(
///   baseUrl: 'https://driveapis.cloud.huawei.com.cn/drive/v1',
///   tokenProvider: () async => authService.accessToken,
///   refreshTokenProvider: () async => authService.refresh(),
///   onAuthExpired: () => authService.logout(),
/// );
/// final result = await client.get('/files');
/// ```
class MateHttpClient {
  final String _baseUrl;
  final Future<String> Function() _tokenProvider;
  final Future<String?> Function() _refreshTokenProvider;
  final void Function() _onAuthExpired;

  late final Dio _client;

  /// 401 刷新 singleflight 锁：非 null 表示正在刷新，并发 401 等待同一个 Future
  Future<String?>? _refreshFuture;

  /// 标记是否已触发 onAuthExpired，避免重复调用
  bool _authExpiredCalled = false;

  /// 连接超时（对齐 Rust connect_timeout 15s）
  static const Duration connectTimeout = Duration(seconds: 15);

  /// 发送超时（对齐 Rust send 60s）
  static const Duration sendTimeout = Duration(seconds: 60);

  /// 接收超时（对齐 Rust receive 60s）
  static const Duration receiveTimeout = Duration(seconds: 60);

  /// 重放标记：写入 RequestOptions.extra，标识该请求已是 401 刷新后的重放
  static const String _authReplayedKey = '_authReplayed';

  MateHttpClient({
    required String baseUrl,
    required Future<String> Function() tokenProvider,
    required Future<String?> Function() refreshTokenProvider,
    required void Function() onAuthExpired,
    Dio? dio,
  })  : _baseUrl = baseUrl,
        _tokenProvider = tokenProvider,
        _refreshTokenProvider = refreshTokenProvider,
        _onAuthExpired = onAuthExpired {
    _client = dio ?? _createDio();
    if (dio != null) {
      // 注入的 Dio（测试 seam）同样挂 Bearer 注入拦截器
      dio.interceptors.add(_authInterceptor());
    }
  }

  Dio _createDio() {
    final dio = Dio(
      BaseOptions(
        baseUrl: _baseUrl,
        connectTimeout: connectTimeout,
        sendTimeout: sendTimeout,
        receiveTimeout: receiveTimeout,
        headers: {
          'Content-Type': 'application/json',
        },
      ),
    );

    // Bearer 注入 + 401 刷新重放拦截器
    dio.interceptors.add(_authInterceptor());

    return dio;
  }

  /// 构造 Bearer 注入 + 401 刷新重放拦截器
  _MateAuthInterceptor _authInterceptor() {
    return _MateAuthInterceptor(
      tokenProvider: _tokenProvider,
      onUnauthorized: _handle401,
    );
  }

  // ============================================================
  // 401 处理：singleflight 刷新
  // ============================================================

  /// 处理 401：singleflight 刷新 token。
  ///
  /// 返回非 null token 表示刷新成功；返回 null 表示刷新失败（已触发登出回调）。
  Future<String?> _handle401() {
    // singleflight：如果已有刷新在进行，等待它
    final pending = _refreshFuture;
    if (pending != null) return pending;

    final future = _doRefresh();
    _refreshFuture = future;
    return future.whenComplete(() => _refreshFuture = null);
  }

  /// 执行 token 刷新（对齐 Rust refresher().refresh()）
  Future<String?> _doRefresh() async {
    try {
      AppLogger.i('收到 401，刷新 token 后重放');
      final newToken = await _refreshTokenProvider();
      if (newToken != null && newToken.isNotEmpty) {
        AppLogger.i('Token 刷新成功');
        return newToken;
      }
      AppLogger.e('Token 刷新失败：返回空 token');
      _fireAuthExpired();
      return null;
    } catch (e, st) {
      AppLogger.e('Token 刷新异常', e, st);
      _fireAuthExpired();
      return null;
    }
  }

  /// 触发 onAuthExpired（最多一次）
  void _fireAuthExpired() {
    if (!_authExpiredCalled) {
      _authExpiredCalled = true;
      _onAuthExpired();
    }
  }

  /// 触发 singleflight 强制刷新（供裸请求 401 手动重放使用）。
  ///
  /// 返回非 null token 表示刷新成功；返回 null 表示刷新失败（已触发登出回调）。
  /// 对齐 Rust `refresher().refresh()`。
  Future<String?> Function() get forceRefreshToken => _handle401;

  // ============================================================
  // 公开 HTTP 方法（均返回 AppResult）
  // ============================================================

  /// GET 请求
  Future<AppResult<T>> get<T>(
    String path, {
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'GET',
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// POST 请求
  Future<AppResult<T>> post<T>(
    String path, {
    dynamic data,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'POST',
      data: data,
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// PUT 请求
  Future<AppResult<T>> put<T>(
    String path, {
    dynamic data,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'PUT',
      data: data,
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// PATCH 请求；传输失败会保留写请求是否可能已提交的语义。
  Future<AppResult<T>> patch<T>(
    String path, {
    dynamic data,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'PATCH',
      data: data,
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// DELETE 请求
  Future<AppResult<T>> delete<T>(
    String path, {
    dynamic data,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'DELETE',
      data: data,
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// POST 表单（multipart/form-data）
  Future<AppResult<T>> postForm<T>(
    String path, {
    required FormData formData,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _request<T>(
      path,
      method: 'POST',
      data: formData,
      queryParameters: queryParameters,
      headers: headers,
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  // ============================================================
  // 裸请求（上传分片 / 下载内容 / 写操作状态核验等需要完整 Response 的场景）
  // ============================================================

  /// 发送裸请求并返回完整 [Response]（含状态码与响应头）。
  ///
  /// - `validateStatus` 恒真：任意状态码都正常返回，由调用方自行处理
  ///   （上传 308、删除核验 404、写操作严格 200 合同等）
  /// - Bearer 仍由拦截器自动注入；401 不会自动重放（由 [requestRawAuthed]
  ///   或调用方处理）
  /// - 传输失败抛 [DioException]（不归一化为 [AppError]，由调用方按语义分类）
  Future<Response<T>> requestRaw<T>(
    String method,
    String url, {
    Object? data,
    Map<String, dynamic>? headers,
    ResponseType? responseType,
    Duration? sendTimeout,
    Duration? receiveTimeout,
    bool followRedirects = false,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) {
    return _client.request<T>(
      url,
      data: data,
      options: Options(
        method: method,
        headers: headers,
        responseType: responseType,
        sendTimeout: sendTimeout,
        receiveTimeout: receiveTimeout,
        validateStatus: (_) => true,
        followRedirects: followRedirects,
      ),
      cancelToken: cancelToken,
      onSendProgress: onSendProgress,
      onReceiveProgress: onReceiveProgress,
    );
  }

  /// 发送裸请求；401 时 singleflight 刷新 token 后原样重放一次。
  ///
  /// 对齐 Rust `send_content_request` / 上传分片的 401 手动重放：
  /// - 首次请求 401 → 刷新（singleflight）→ 同一 URL/body/header 重放一次；
  ///   重放时 Bearer 由拦截器注入刷新后的 token
  /// - 刷新失败抛 [TokenError]（已触发登出回调）
  /// - 传输失败抛 [AppError]（按 [semantics] 与是否已重放分类）
  /// - 其他状态码原样返回，由调用方解释
  Future<({Response<T> response, bool authReplayed})> requestRawAuthed<T>(
    String method,
    String url, {
    Object? data,
    Map<String, dynamic>? headers,
    ResponseType? responseType,
    RequestSemantics semantics = RequestSemantics.read,
    Duration? sendTimeout,
    Duration? receiveTimeout,
    bool followRedirects = false,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) async {
    final Response<T> first;
    try {
      first = await requestRaw<T>(
        method,
        url,
        data: data,
        headers: headers,
        responseType: responseType,
        sendTimeout: sendTimeout,
        receiveTimeout: receiveTimeout,
        followRedirects: followRedirects,
        cancelToken: cancelToken,
        onSendProgress: onSendProgress,
        onReceiveProgress: onReceiveProgress,
      );
    } on DioException catch (e) {
      throw AppError.fromDioException(e,
          semantics: semantics, authAlreadyReplayed: false);
    }
    if (first.statusCode != 401) {
      return (response: first, authReplayed: false);
    }

    // 401：丢弃未消费的首个响应体（流式响应必须排空后才能复用连接）
    final firstData = first.data;
    if (firstData is ResponseBody) {
      try {
        await firstData.stream.drain<void>();
      } catch (_) {
        // 尽力排空
      }
    }

    final newToken = await _handle401();
    if (newToken == null || newToken.isEmpty) {
      throw AppError.tokenRefreshFailed();
    }
    try {
      final replayed = await requestRaw<T>(
        method,
        url,
        data: data,
        headers: headers,
        responseType: responseType,
        sendTimeout: sendTimeout,
        receiveTimeout: receiveTimeout,
        followRedirects: followRedirects,
        cancelToken: cancelToken,
        onSendProgress: onSendProgress,
        onReceiveProgress: onReceiveProgress,
      );
      return (response: replayed, authReplayed: true);
    } on DioException catch (e) {
      throw AppError.fromDioException(e,
          semantics: semantics, authAlreadyReplayed: true);
    }
  }

  // ============================================================
  // 内部请求执行
  // ============================================================

  /// 执行请求并返回 [AppResult]。
  ///
  /// 对齐 Rust `execute_with_retry` + `ensure_success_response`：
  /// 仅放行最终 2xx 响应；其余状态读取错误体后返回结构化失败。
  Future<AppResult<T>> _request<T>(
    String path, {
    required String method,
    dynamic data,
    Map<String, dynamic>? queryParameters,
    Map<String, dynamic>? headers,
    CancelToken? cancelToken,
    ProgressCallback? onSendProgress,
    ProgressCallback? onReceiveProgress,
  }) async {
    final semantics = _requestSemantics(method);
    try {
      final response = await _client.request<T>(
        path,
        data: data,
        queryParameters: queryParameters,
        options: Options(method: method, headers: headers),
        cancelToken: cancelToken,
        onSendProgress: onSendProgress,
        onReceiveProgress: onReceiveProgress,
      );

      final statusCode = response.statusCode ?? 0;
      if (statusCode >= 200 && statusCode < 300) {
        return Ok<T>(_coerceData<T>(response.data));
      }

      // 非 2xx → 结构化 AppError（dio 默认会对非 2xx 抛异常，此处兜底）
      final error = _errorFromResponse(response, semantics);
      AppLogger.e('HTTP $statusCode: $method $path → $error');
      return Err<T>(error);
    } on DioException catch (e) {
      final replayed = e.requestOptions.extra[_authReplayedKey] == true;
      final error = AppError.fromDioException(
        e,
        semantics: semantics,
        authAlreadyReplayed: replayed,
      );
      AppLogger.e('请求异常: $method $path → $error', e);
      return Err<T>(error);
    } catch (e, st) {
      AppLogger.e('请求未知异常: $method $path', e, st);
      return Err<T>(GenericError(message: e.toString()));
    }
  }

  /// 从非 2xx 响应构造结构化错误（解析 Retry-After 与华为错误码）。
  AppError _errorFromResponse(
    Response<dynamic> response,
    RequestSemantics semantics,
  ) {
    final replayed = response.requestOptions.extra[_authReplayedKey] == true;
    return AppError.driveFromResponse(
      response.statusCode ?? 0,
      AppError.responseBodyString(response),
      retryAfter:
          RetryAfter.tryParse(response.headers.value('Retry-After')),
      semantics: semantics,
      authAlreadyReplayed: replayed,
    );
  }

  /// 将响应体适配到调用方声明的泛型。
  ///
  /// Service 层惯用 `get<String>` 再自行 jsonDecode；
  /// dio 默认会把 JSON 响应解码为 Map，这里重新编码以保证 `as T` 安全。
  T _coerceData<T>(dynamic data) {
    if (data is T) return data;
    if (T == String && data != null) {
      if (data is Map || data is List) return jsonEncode(data) as T;
      return data.toString() as T;
    }
    return data as T;
  }

  /// 按 HTTP 方法区分只读请求与可能已提交的写请求。
  static RequestSemantics _requestSemantics(String method) {
    return switch (method.toUpperCase()) {
      'GET' || 'HEAD' || 'OPTIONS' => RequestSemantics.read,
      _ => RequestSemantics.write,
    };
  }
}

// ============================================================
// 认证拦截器：Bearer 注入 + 401 刷新重放
// ============================================================

/// Bearer token 注入 + 401 自动刷新重放拦截器。
///
/// - [onRequest]：从 tokenProvider 获取 token 并注入 Authorization 头；
///   `oauth2/v3/token` 端点不注入（对齐 Rust build_authed）
/// - [onError]：捕获 401，通过 onUnauthorized 触发 singleflight 刷新并重放一次
class _MateAuthInterceptor extends Interceptor {
  final Future<String> Function() tokenProvider;
  final Future<String?> Function() onUnauthorized;

  _MateAuthInterceptor({
    required this.tokenProvider,
    required this.onUnauthorized,
  });

  @override
  void onRequest(
    RequestOptions options,
    RequestInterceptorHandler handler,
  ) async {
    // token 端点（含 oauth2/v3/token）不注入 auth
    if (options.path.contains('oauth2/v3/token')) {
      handler.next(options);
      return;
    }
    try {
      final token = await tokenProvider();
      if (token.isNotEmpty) {
        options.headers['Authorization'] = 'Bearer $token';
      }
    } catch (e) {
      // token 获取失败不阻塞请求（由服务端返回 401 触发刷新流程）
      AppLogger.e('获取 token 失败', e);
    }
    handler.next(options);
  }

  @override
  void onError(DioException err, ErrorInterceptorHandler handler) async {
    // 只处理 401
    if (err.response?.statusCode != 401) {
      handler.next(err);
      return;
    }

    // 避免死循环：重放后的请求再 401 直接放行（由上层归一化为结构化错误）
    if (err.requestOptions.extra[MateHttpClient._authReplayedKey] == true) {
      AppLogger.e('刷新后仍返回 401，中止重试');
      handler.next(err);
      return;
    }

    final String? newToken;
    try {
      // singleflight 刷新
      newToken = await onUnauthorized();
    } catch (_) {
      handler.next(err);
      return;
    }

    if (newToken == null || newToken.isEmpty) {
      // 刷新失败 → Token 错误（对齐 Rust refresher().refresh() 失败语义）
      handler.reject(
        DioException(
          requestOptions: err.requestOptions,
          response: err.response,
          error: AppError.tokenRefreshFailed(),
          type: DioExceptionType.unknown,
        ),
      );
      return;
    }

    // 用新 token 重放原请求（单次）
    final opts = err.requestOptions;
    opts.headers['Authorization'] = 'Bearer $newToken';
    opts.extra[MateHttpClient._authReplayedKey] = true;

    try {
      // 用独立 Dio 重放，避免拦截器递归；extra 标记随 requestOptions 传递
      final response = await Dio().fetch<void>(opts);
      handler.resolve(response);
    } on DioException catch (retryErr) {
      handler.next(retryErr);
    } catch (retryErr) {
      handler.next(
        DioException(
          requestOptions: opts,
          error: retryErr,
          type: DioExceptionType.unknown,
        ),
      );
    }
  }
}
