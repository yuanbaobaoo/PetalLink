/// OAuth 本地回调 HTTP Server（需求 F-AUTH-02 / F-AUTH-06）。
///
/// 严格对齐 Rust 原版 `src/auth/oauth_server.rs`：
/// - 绑定 127.0.0.1:port（不监听 0.0.0.0，满足安全要求）
/// - 监听 GET /oauth/callback，解析 code/state/error/sub_error
/// - 单次使用：拿到首个请求结果后自动关闭
library;

import 'dart:async';
import 'dart:io';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/auth/auth_constants.dart';

/// OAuth 回调结果（对齐 Rust `OauthCallbackResult`）。
class OauthCallbackResult {
  /// 授权码
  final String? code;

  /// 防 CSRF state
  final String? state;

  /// 华为 error 码（如 '1101'）
  final String? error;

  /// 华为 error_description（如 'invalid scope'）
  final String? errorDescription;

  /// 华为 sub_error（如 '20042' 表示 scope 未授权）
  final String? subError;

  const OauthCallbackResult({
    this.code,
    this.state,
    this.error,
    this.errorDescription,
    this.subError,
  });

  /// 是否成功（有 code 且无 error）
  bool get isSuccess => code != null && error == null;
}

/// 可跨调用方触发回调监听停止的句柄（对齐 Rust `OauthServerStopHandle`）。
class OauthServerStopHandle {
  final void Function() _stop;

  OauthServerStopHandle._(this._stop);

  /// 通知 OAuth server 停止等待回调。
  void stop() => _stop();
}

/// 本地 OAuth 回调服务器。
///
/// 使用 dart:io HttpServer 监听 127.0.0.1；首个到达的请求即为回调结果
/// （含无效路径，对齐 Rust「单次使用」语义）。
class OauthServer {
  final HttpServer _server;
  final StreamSubscription<HttpRequest> _subscription;
  final Completer<OauthCallbackResult> _resultCompleter =
      Completer<OauthCallbackResult>();
  late final OauthServerStopHandle _stopHandle;

  /// 是否已停止（幂等保护）
  bool _stopped = false;

  OauthServer._(this._server, this._subscription) {
    _stopHandle = OauthServerStopHandle._(() => unawaited(stop()));
    // 预挂错误处理：server 在无人等待时被 stop（如授权流程提前失败），
    // 避免 completeError 成为 unhandled async error
    unawaited(_resultCompleter.future.catchError((_) => _closedResult));
  }

  /// 预挂错误处理的占位结果（仅用于让派生 Future 正常完成）
  static const OauthCallbackResult _closedResult =
      OauthCallbackResult(error: '通道已关闭');

  /// 启动监听（绑定失败抛 [AppError]）。重复启动新实例即可，无全局状态。
  static Future<OauthServer> start(int port) async {
    final HttpServer server;
    try {
      AppLogger.i('启动 OAuth 回调监听：${AuthConstants.loopbackHost}:$port');
      // 仅绑定 loopback IPv4
      server = await HttpServer.bind(InternetAddress.loopbackIPv4, port);
    } catch (e) {
      throw AppError.generic('绑定回调端口失败：$e');
    }

    late OauthServer self;
    final subscription = server.listen(
      (request) => self._handleRequest(request),
      onError: (Object e) => AppLogger.w('OAuth 回调 accept 失败：$e'),
    );
    self = OauthServer._(server, subscription);
    return self;
  }

  /// 实际监听端口（port=0 时由系统分配，供测试使用）。
  int get port => _server.port;

  /// 获取停止句柄，供取消授权从外部关闭监听。
  OauthServerStopHandle get stopHandle => _stopHandle;

  /// 等待授权码（带超时）。
  ///
  /// - 正常：返回回调结果并关闭 server
  /// - 超时：抛 `AppError.authTimeout()`
  /// - 外部 stop（用户取消）：抛 `AppError.generic('OAuth 回调通道关闭')`
  ///
  /// 对齐 Rust `OauthServer.wait_for_callback`。
  Future<OauthCallbackResult> waitForCallback({
    Duration timeout = AuthConstants.oauthTimeout,
  }) async {
    try {
      final result = await _resultCompleter.future.timeout(timeout);
      await stop();
      return result;
    } on TimeoutException {
      await stop();
      AppLogger.w('OAuth 回调等待超时');
      throw AppError.authTimeout();
    } catch (_) {
      await stop();
      rethrow;
    }
  }

  /// 关闭 server，释放端口（幂等）。
  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    // 先释放等待方（取消语义即时生效）：通道关闭语义（对齐 Rust oneshot 关闭）
    if (!_resultCompleter.isCompleted) {
      _resultCompleter.completeError(AppError.generic('OAuth 回调通道关闭'));
    }
    await _subscription.cancel();
    try {
      // 非强制关闭：等在途响应发完；超时兜底强制关闭（防御浏览器预连接挂起）
      await _server.close().timeout(
        const Duration(seconds: 2),
        onTimeout: () => _server.close(force: true),
      );
    } catch (_) {
      // 关闭异常不影响结果
    }
    AppLogger.i('OAuth 回调 server 已关闭');
  }

  /// 处理首个 HTTP 请求：解析回调参数 → 回写响应页 → 完成结果并关闭。
  Future<void> _handleRequest(HttpRequest request) async {
    final result = _parseRequest(request);
    try {
      request.response
        ..statusCode = HttpStatus.ok
        ..headers.contentType =
            ContentType('text', 'html', charset: 'utf-8')
        ..headers.set(HttpHeaders.connectionHeader, 'close')
        ..write(buildResponsePage(result));
      await request.response.close();
    } catch (e) {
      AppLogger.w('OAuth 回写响应页失败', e);
    }
    if (!_resultCompleter.isCompleted) {
      _resultCompleter.complete(result);
    }
    // 单次使用：拿到结果后停止监听
    await stop();
  }

  /// 解析请求，提取回调参数（对齐 Rust `handle_request` + `parse_query`）。
  ///
  /// Dart `Uri.queryParameters` 按 form-urlencoded 解码（'+' 视为空格），
  /// 与 Rust `url_decode` 语义一致。
  OauthCallbackResult _parseRequest(HttpRequest request) {
    if (request.uri.path != AuthConstants.callbackPath) {
      return const OauthCallbackResult(error: '无效回调路径');
    }
    final q = request.uri.queryParameters;
    String? pick(String key) {
      final v = q[key];
      return v == null || v.isEmpty ? null : v;
    }

    return OauthCallbackResult(
      code: pick('code'),
      state: pick('state'),
      error: pick('error'),
      errorDescription: pick('error_description'),
      subError: pick('sub_error'),
    );
  }
}

/// 构建回写浏览器的友好页面（对齐 Rust `build_response_page`）。
String buildResponsePage(OauthCallbackResult result) {
  if (result.isSuccess) {
    return '<!DOCTYPE html>\n'
        '<html><head><meta charset="utf-8"><title>授权成功</title>\n'
        '<style>body{font-family:-apple-system,sans-serif;text-align:center;margin-top:80px;color:#333}\n'
        'h1{color:#1a7f37}</style></head>\n'
        '<body><h1>✅ 授权成功</h1>\n'
        '<p>已成功登录华为云盘，现在可以关闭此页面并返回 App。</p></body></html>';
  }
  final reason = result.error ?? '未知错误';
  return '<!DOCTYPE html>\n'
      '<html><head><meta charset="utf-8"><title>授权失败</title>\n'
      '<style>body{font-family:-apple-system,sans-serif;text-align:center;margin-top:80px;color:#333}\n'
      'h1{color:#d73a49}</style></head>\n'
      '<body><h1>❌ 授权失败</h1>\n'
      '<p>$reason</p>\n'
      '<p>请返回 App 重新登录。</p></body></html>';
}
