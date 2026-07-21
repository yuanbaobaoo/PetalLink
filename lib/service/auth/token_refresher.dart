// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'dart:convert';

import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/auth_constants.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/token_store.dart';

/// Token 刷新器（需求 F-AUTH-04）。
///
/// 严格对齐 Rust 原版 `src/auth/token_refresher.rs`：
/// - 用 refresh_token 换新的 access_token
/// - singleflight：并发调用共享同一个 in-flight 结果（成功与失败均共享）
/// - 华为刷新响应可能不含新 refresh_token → 沿用旧的
/// - 先原子替换持久化文件，成功后再更新内存副本
class TokenRefresher {
  final TokenStore _tokenStore;
  final Dio _http;
  final Future<AuthSecrets> Function() _secretsProvider;

  /// 当前持有的 token（内存缓存，避免每次刷新都读存储）
  TokenPair? _current;

  /// singleflight 锁：非 null 表示正在刷新，并发调用等待同一个 Future
  Future<TokenPair>? _refreshFlight;

  TokenRefresher({
    required TokenStore tokenStore,
    required Future<AuthSecrets> Function() secretsProvider,
    Dio? http,
  })  : _tokenStore = tokenStore,
        _secretsProvider = secretsProvider,
        _http = http ?? _createDefaultHttp();
  /// 默认 HTTP 客户端（30s 超时，对齐 Rust reqwest；接受任意状态码，
  /// 华为错误体在 body 里，由解析逻辑统一判定，对齐 Rust 不校验 status）。
  static Dio _createDefaultHttp() {
    return Dio(BaseOptions(
      connectTimeout: AuthConstants.httpTimeout,
      sendTimeout: AuthConstants.httpTimeout,
      receiveTimeout: AuthConstants.httpTimeout,
      validateStatus: (_) => true,
    ));
  }

  /// 更新内存中的 token 缓存。
  void setCurrent(TokenPair token) {
    _current = token;
  }

  /// 清空内存中的 token 缓存。
  void clearCurrent() {
    _current = null;
  }

  /// 获取当前 token（优先内存缓存，回退存储）。
  Future<TokenPair?> currentToken() async {
    final cached = _current;
    if (cached != null) return cached;
    return _tokenStore.load();
  }

  /// 刷新 token 并持久化。返回新 token。
  ///
  /// 并发调用共享同一次刷新结果（成功与失败均共享）。
  /// 对齐 Rust `TokenRefresher.refresh`。
  Future<TokenPair> refresh() {
    final pending = _refreshFlight;
    if (pending != null) return pending;
    final future = _performRefresh();
    _refreshFlight = future;
    return future.whenComplete(() => _refreshFlight = null);
  }

  /// 请求新 access token，先原子替换持久化文件，成功后再更新内存副本。
  Future<TokenPair> _performRefresh() async {
    final current = await currentToken();
    if (current == null) throw AppError.tokenNotLoggedIn();

    AppLogger.i('开始刷新 token...');
    final secrets = await _secretsProvider();

    // 刷新用 form-urlencoded（refresh_token 无特殊字符，对齐 Rust `.form()`）
    final body = [
      'grant_type=refresh_token',
      'refresh_token=${Uri.encodeQueryComponent(current.refreshToken)}',
      'client_id=${Uri.encodeQueryComponent(secrets.clientId)}',
      'client_secret=${Uri.encodeQueryComponent(secrets.clientSecret)}',
    ].join('&');

    final Response<String> resp;
    try {
      resp = await _http.post<String>(
        AuthConstants.tokenUrl,
        data: body,
        options: Options(
          contentType: Headers.formUrlEncodedContentType,
          responseType: ResponseType.plain,
        ),
      );
    } on DioException catch (e) {
      throw _classifyTransportError(e);
    }

    final text = resp.data ?? '';
    final Map<String, dynamic> data;
    try {
      final decoded = jsonDecode(text);
      if (decoded is! Map<String, dynamic>) {
        throw AppError.tokenRefreshFailed('响应非 JSON 对象');
      }
      data = decoded;
    } on AppError {
      rethrow;
    } catch (e) {
      throw AppError.tokenRefreshFailed(e.toString());
    }

    final accessToken = data['access_token'];
    if (accessToken is! String || accessToken.isEmpty) {
      final cause = data['error_description'] ?? data['error'];
      throw AppError.tokenRefreshFailed(cause?.toString());
    }

    // 华为刷新响应可能不含新 refresh_token → 沿用旧的
    final refreshToken = data['refresh_token'];
    final expiresIn = _tolerantInt(data['expires_in']) ?? 3600;
    final tokenType = data['token_type'];
    final scope = data['scope'];

    final newToken = TokenPair(
      accessToken: accessToken,
      refreshToken: refreshToken is String && refreshToken.isNotEmpty
          ? refreshToken
          : current.refreshToken,
      expiresAt: DateTime.now().millisecondsSinceEpoch + expiresIn * 1000,
      tokenType: tokenType is String && tokenType.isNotEmpty
          ? tokenType
          : 'Bearer',
      scope: scope is String ? scope : current.scope,
    );

    await _tokenStore.save(newToken);
    _current = newToken;
    AppLogger.i('token 刷新成功');
    return newToken;
  }

  /// 区分网络错误 vs token 刷新失败（对齐 Rust `classify_refresh_transport_flags`）：
  /// 连接/超时/响应体中断 → 网络连接失败（DriveApi network，恢复时保留 token）；
  /// 其余 → token 刷新失败（恢复时清理登录态）。
  static AppError _classifyTransportError(DioException e) {
    final kind = switch (e.type) {
      DioExceptionType.connectionError => DriveTransportKind.connect,
      DioExceptionType.connectionTimeout ||
      DioExceptionType.sendTimeout ||
      DioExceptionType.receiveTimeout =>
        DriveTransportKind.timeout,
      DioExceptionType.badResponse => DriveTransportKind.responseBody,
      _ => null,
    };
    if (kind == null) {
      return AppError.tokenRefreshFailed(e.message ?? e.toString());
    }
    return AppError.driveTransport(kind, cause: e.message);
  }

  /// 容忍解析 int：接受 int / num / String。
  static int? _tolerantInt(Object? v) {
    if (v is int) return v;
    if (v is num) return v.toInt();
    if (v is String) return int.tryParse(v.trim());
    return null;
  }
}
