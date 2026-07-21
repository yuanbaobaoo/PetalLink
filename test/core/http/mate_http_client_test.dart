import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';

import '../../service/auth/fake_http.dart';

/// MateHttpClient Bearer 注入与 401 刷新失败语义测试。
///
/// 对齐 Rust `src/drive/client.rs::build_authed` / `execute_with_retry`：
/// - token 获取失败/为空 → 请求直接失败，不发出无认证请求
/// - 刷新失败 → TokenError 向上传播，HTTP 层不自动登出
void main() {
  group('Bearer 注入（对齐 Rust build_authed）', () {
    test('tokenProvider 抛异常 → 请求被拒绝，不发出无认证请求', () async {
      final adapter = FakeHttpAdapter(
        (req) => ResponseBody.fromString('{}', 200),
      );
      final client = MateHttpClient(
        baseUrl: '',
        tokenProvider: () async => throw AppError.tokenNotLoggedIn(),
        refreshTokenProvider: () async => null,
        dio: adapter.createDio(),
      );

      final result = await client.get('https://api.example.com/x');

      expect(result.isErr, isTrue);
      expect(adapter.requests, isEmpty, reason: '不得发出无认证请求');
    });

    test('tokenProvider 返回空串 → 请求被拒绝（未登录不发请求）', () async {
      final adapter = FakeHttpAdapter(
        (req) => ResponseBody.fromString('{}', 200),
      );
      final client = MateHttpClient(
        baseUrl: '',
        tokenProvider: () async => '',
        refreshTokenProvider: () async => null,
        dio: adapter.createDio(),
      );

      final result = await client.get('https://api.example.com/x');

      expect(result.isErr, isTrue);
      expect(adapter.requests, isEmpty);
    });

    test('token 正常 → 注入 Bearer 并放行', () async {
      final adapter = FakeHttpAdapter(
        (req) => ResponseBody.fromString('{"ok":true}', 200),
      );
      final client = MateHttpClient(
        baseUrl: '',
        tokenProvider: () async => 'tk',
        refreshTokenProvider: () async => null,
        dio: adapter.createDio(),
      );

      final result = await client.get('https://api.example.com/x');

      expect(result.isOk, isTrue);
      expect(adapter.requests.single.headers['Authorization'], 'Bearer tk');
    });
  });

  group('401 刷新失败（对齐 Rust：仅向上传播，不自动登出）', () {
    test('刷新返回空 → 以 TokenError 拒绝', () async {
      final adapter = FakeHttpAdapter(
        (req) => ResponseBody.fromString('{"error":"unauthorized"}', 401),
      );
      // 生产 Dio 默认 validateStatus（<400），401 才进入 onError 拦截链
      final dio = adapter.createDio()
        ..options.validateStatus = (s) => s != null && s < 400;
      final client = MateHttpClient(
        baseUrl: '',
        tokenProvider: () async => 'expired',
        refreshTokenProvider: () async => null,
        dio: dio,
      );

      final result = await client.get('https://api.example.com/x');

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<TokenError>());
    });
  });
}
