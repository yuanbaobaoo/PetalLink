import 'dart:async';
import 'dart:io';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/token_refresher.dart';
import 'package:petal_link/service/auth/token_store.dart';

import 'fake_http.dart';

/// 内存 TokenStore（测试用）。
class InMemoryTokenStore implements TokenStore {
  TokenPair? stored;
  int saveCount = 0;

  @override
  Future<TokenPair?> load() async => stored;

  @override
  Future<void> save(TokenPair token) async {
    saveCount++;
    stored = token;
  }

  @override
  Future<void> clear() async {
    stored = null;
  }
}

void main() {
  const secrets = AuthSecrets(clientId: 'test-id', clientSecret: 'test-secret');
  Future<AuthSecrets> secretsProvider() async => secrets;

  TokenPair currentToken({int? expiresAt}) => TokenPair(
        accessToken: 'old-access',
        refreshToken: 'old-refresh',
        expiresAt: expiresAt ??
            DateTime.now()
                .add(const Duration(hours: 1))
                .millisecondsSinceEpoch,
        scope: 'openid',
      );

  Map<String, dynamic> refreshResponse({
    String accessToken = 'new-access',
    String? refreshToken = 'new-refresh',
    int expiresIn = 3600,
  }) {
    return {
      'access_token': accessToken,
      'refresh_token': ?refreshToken,
      'expires_in': expiresIn,
      'token_type': 'Bearer',
    };
  }

  group('TokenRefresher.refresh', () {
    test('刷新成功：返回新 token 并持久化 + 更新内存', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter((req) {
        expect(req.uri.path, '/oauth2/v3/token');
        return jsonResponse(refreshResponse());
      });
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      final token = await refresher.refresh();

      expect(token.accessToken, 'new-access');
      expect(token.refreshToken, 'new-refresh');
      expect(store.stored?.accessToken, 'new-access');
      expect(store.saveCount, 1);
      expect(await refresher.currentToken(), isNotNull);
      expect((await refresher.currentToken())!.accessToken, 'new-access');
    });

    test('刷新请求为 form-urlencoded，携带 grant_type/refresh_token/client 凭据',
        () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter((req) => jsonResponse(refreshResponse()));
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      await refresher.refresh();

      final req = adapter.requests.single;
      expect(req.method, 'POST');
      expect(req.headers['content-type'],
          contains('application/x-www-form-urlencoded'));
      expect(req.body, contains('grant_type=refresh_token'));
      expect(req.body, contains('refresh_token=old-refresh'));
      expect(req.body, contains('client_id=test-id'));
      expect(req.body, contains('client_secret=test-secret'));
    });

    test('响应不含新 refresh_token 时沿用旧的（对齐 Rust）', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter(
        (req) => jsonResponse(refreshResponse(refreshToken: null)),
      );
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      final token = await refresher.refresh();
      expect(token.refreshToken, 'old-refresh');
    });

    test('响应不含 access_token → tokenRefreshFailed（携带 error_description）',
        () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter(
        (req) => jsonResponse(
          {'error': 'invalid_grant', 'error_description': 'refresh 已失效'},
          status: 400,
        ),
      );
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      await expectLater(
        refresher.refresh(),
        throwsA(
          isA<TokenError>()
              .having((e) => e.tokenCode, 'tokenCode',
                  TokenErrorCode.refreshFailed)
              .having((e) => e.message, 'message', contains('refresh 已失效')),
        ),
      );
      // 失败不落盘
      expect(store.saveCount, 0);
    });

    test('无当前 token → tokenNotLoggedIn', () async {
      final store = InMemoryTokenStore();
      final adapter = FakeHttpAdapter((req) => jsonResponse(refreshResponse()));
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      await expectLater(
        refresher.refresh(),
        throwsA(isA<TokenError>().having((e) => e.tokenCode, 'tokenCode',
            TokenErrorCode.notLoggedIn)),
      );
      expect(adapter.requests, isEmpty);
    });

    test('singleflight：并发刷新只发一次 HTTP 请求，共享同一结果', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final gate = Completer<void>();
      final adapter = FakeHttpAdapter((req) async {
        await gate.future; // 阻塞直到放行，保证并发窗口
        return jsonResponse(refreshResponse());
      });
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      final futures = [for (var i = 0; i < 5; i++) refresher.refresh()];
      // 全部进入 flight 后放行
      await Future<void>.delayed(Duration.zero);
      gate.complete();
      final results = await Future.wait(futures);

      expect(adapter.requests, hasLength(1));
      for (final token in results) {
        expect(token.accessToken, 'new-access');
      }
      expect(store.saveCount, 1);
    });

    test('singleflight：并发共享失败结果；flight 结束后可重试', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      var calls = 0;
      final gate = Completer<void>();
      final adapter = FakeHttpAdapter((req) async {
        calls++;
        await gate.future;
        return jsonResponse({'error': 'invalid_grant'}, status: 400);
      });
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      final futures = [refresher.refresh(), refresher.refresh()];
      await Future<void>.delayed(Duration.zero);
      gate.complete();
      for (final f in futures) {
        await expectLater(f, throwsA(isA<TokenError>()));
      }
      expect(calls, 1);

      // flight 已清理：再次 refresh 触发新请求
      await expectLater(refresher.refresh(), throwsA(isA<TokenError>()));
      expect(calls, 2);
    });

    test('连接失败 → DriveApiError(network)，恢复时保留 token（对齐 Rust 分类）',
        () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter((req) {
        throw DioException(
          requestOptions: RequestOptions(),
          type: DioExceptionType.connectionError,
          error: const SocketException('connection refused'),
        );
      });
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      await expectLater(
        refresher.refresh(),
        throwsA(isA<DriveApiError>().having(
            (e) => e.driveCode, 'driveCode', DriveApiErrorCode.network)),
      );
    });

    test('超时 → DriveApiError(network, timeout)', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter((req) {
        throw DioException(
          requestOptions: RequestOptions(),
          type: DioExceptionType.receiveTimeout,
        );
      });
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      await expectLater(
        refresher.refresh(),
        throwsA(isA<DriveApiError>()
            .having((e) => e.driveCode, 'driveCode', DriveApiErrorCode.network)
            .having((e) => e.transportKind, 'transportKind',
                DriveTransportKind.timeout)),
      );
    });
  });

  group('TokenRefresher 内存缓存', () {
    test('currentToken 优先内存，回退存储；setCurrent/clearCurrent 生效', () async {
      final store = InMemoryTokenStore()..stored = currentToken();
      final adapter = FakeHttpAdapter((req) => jsonResponse(refreshResponse()));
      final refresher = TokenRefresher(
        tokenStore: store,
        secretsProvider: secretsProvider,
        http: adapter.createDio(),
      );

      // 回退存储
      expect((await refresher.currentToken())!.accessToken, 'old-access');

      // 内存优先
      final mem = currentToken().copyWith(accessToken: 'mem-access');
      refresher.setCurrent(mem);
      expect((await refresher.currentToken())!.accessToken, 'mem-access');

      // 清空后回退存储
      refresher.clearCurrent();
      expect((await refresher.currentToken())!.accessToken, 'old-access');
    });
  });

  group('临期判断（TokenPair.willExpireWithin 语义）', () {
    test('60 秒内到期视为临期，需触发刷新', () {
      final soon = DateTime.now()
          .add(const Duration(seconds: 30))
          .millisecondsSinceEpoch;
      expect(currentToken(expiresAt: soon).willExpireWithin(), isTrue);

      final later = DateTime.now()
          .add(const Duration(minutes: 10))
          .millisecondsSinceEpoch;
      expect(currentToken(expiresAt: later).willExpireWithin(), isFalse);
    });
  });
}
