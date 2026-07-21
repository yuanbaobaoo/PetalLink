import 'dart:async';
import 'dart:io';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/auth_constants.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/service/auth/oauth_server.dart';
import 'package:petal_link/service/auth/pkce.dart';
import 'package:petal_link/service/auth/token_store.dart';

import 'fake_http.dart';

/// 内存 TokenStore（测试用）。
class InMemoryTokenStore implements TokenStore {
  TokenPair? stored;
  int clearCount = 0;

  @override
  Future<TokenPair?> load() async => stored;

  @override
  Future<void> save(TokenPair token) async {
    stored = token;
  }

  @override
  Future<void> clear() async {
    clearCount++;
    stored = null;
  }
}

void main() {
  const secrets = AuthSecrets(clientId: 'test-id', clientSecret: 'test-secret');

  TokenPair freshToken() => TokenPair(
        accessToken: 'fresh-access',
        refreshToken: 'fresh-refresh',
        expiresAt: DateTime.now()
            .add(const Duration(hours: 1))
            .millisecondsSinceEpoch,
      );

  TokenPair expiringToken() => TokenPair(
        accessToken: 'expiring-access',
        refreshToken: 'expiring-refresh',
        expiresAt: DateTime.now()
            .add(const Duration(seconds: 10))
            .millisecondsSinceEpoch,
      );

  /// 标准 token 端点成功响应。
  Map<String, dynamic> tokenEndpointJson() => {
        'access_token': 'new-access',
        'refresh_token': 'new-refresh',
        'expires_in': 3600,
        'token_type': 'Bearer',
        'scope': 'openid profile',
      };

  /// 路由华为三端点 + token 端点的假处理器。
  ResponseBody huaweiHandler(FakeRequest req) {
    final host = req.uri.host;
    if (FakeHttpAdapter.match(
        req, 'POST', 'oauth-login.cloud.huawei.com', '/oauth2/v3/token')) {
      return jsonResponse(tokenEndpointJson());
    }
    if (FakeHttpAdapter.match(
        req, 'POST', 'account.cloud.huawei.com', '/rest.php')) {
      final svc = req.uri.queryParameters['nsp_svc'];
      if (svc == 'GOpen.User.getInfo') {
        return jsonResponse({
          'displayName': '花瓣用户',
          'openID': 'openid-1',
          'headPictureURL': 'https://example.com/avatar.png',
        });
      }
      if (svc == 'GOpen.User.getPhone') {
        return textResponse('13800001111');
      }
      return jsonResponse({});
    }
    if (FakeHttpAdapter.match(
        req, 'GET', 'oauth-login.cloud.huawei.com', '/oauth2/v3/userinfo')) {
      return jsonResponse({'sub': 'sub-1'});
    }
    throw StateError('未路由请求：${req.method} $host${req.uri.path}');
  }

  group('buildAuthorizeUrl', () {
    test('scope 空格 → %20 且 / 不编码；其余参数 encodeComponent 编码', () {
      final url = buildAuthorizeUrl(
        'my-client',
        'http://127.0.0.1:9999/oauth/callback',
        'state-abc',
        const PkcePair(codeVerifier: 'v', codeChallenge: 'challenge-xyz'),
      );

      expect(
        url,
        startsWith('https://oauth-login.cloud.huawei.com/oauth2/v3/authorize?'),
      );
      // scope：空格 %20，斜杠保留（对齐 Rust build_authorize_url）
      expect(
        url,
        contains(
            'scope=openid%20profile%20https://www.huawei.com/auth/drive'),
      );
      expect(url, contains('response_type=code'));
      expect(url, contains('client_id=my-client'));
      expect(
        url,
        contains('redirect_uri=${Uri.encodeComponent('http://127.0.0.1:9999/oauth/callback')}'),
      );
      expect(url, contains('state=state-abc'));
      expect(url, contains('access_type=offline'));
      expect(url, contains('code_challenge=challenge-xyz'));
      expect(url, contains('code_challenge_method=S256'));
    });
  });

  group('AuthService.authorize（端到端回环流程）', () {
    late HttpClient http;

    /// 进行中的模拟回调任务（tearDown 前必须全部收尾，避免客户端被强杀）
    final callbackJobs = <Future<void>>[];

    setUp(() {
      http = HttpClient();
    });

    tearDown(() async {
      await Future.wait(callbackJobs);
      http.close(force: true);
    });

    /// 从捕获的授权 URL 模拟浏览器回调。
    Future<void> simulateCallback(
      String authUrl,
      Map<String, String> params,
    ) async {
      final redirect = Uri.parse(authUrl).queryParameters['redirect_uri']!;
      final query = params.entries
          .map((e) => '${e.key}=${Uri.encodeComponent(e.value)}')
          .join('&');
      final callback = Uri.parse('$redirect?$query');
      final request =
          await http.get(callback.host, callback.port, callback.path + (callback.hasQuery ? '?${callback.query}' : ''));
      final response = await request.close();
      await response.drain<void>();
    }

    /// 延迟触发模拟回调（浏览器回调稍后到达）。
    void scheduleCallback(
      String authUrl,
      Map<String, String> params, {
      Duration delay = const Duration(milliseconds: 50),
    }) {
      callbackJobs.add(
        Future<void>.delayed(delay, () => simulateCallback(authUrl, params)),
      );
    }

    /// 装配 AuthService：真实回环 server（随机端口）+ 假 HTTP + 捕获授权 URL。
    Future<({AuthService service, InMemoryTokenStore store, FakeHttpAdapter adapter, OauthServer server})>
        setupFlow({
      required Future<bool> Function(String url) onLaunch,
      ResponseBody Function(FakeRequest)? handler,
      AuthSecrets authSecrets = secrets,
    }) async {
      final store = InMemoryTokenStore();
      final adapter = FakeHttpAdapter(handler ?? huaweiHandler);
      final server = await OauthServer.start(0);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: authSecrets,
        browserLauncher: onLaunch,
        oauthServerFactory: (_) async => server,
      );
      return (service: service, store: store, adapter: adapter, server: server);
    }

    test('完整登录：回调 → 换 token → 存 store → 合并 UserInfo', () async {
      String? capturedUrl;

      final env = await setupFlow(
        onLaunch: (url) async {
          capturedUrl = url;
          // 浏览器回调稍后到达
          scheduleCallback(url, {
            'code': 'CODE+WITH/PLUS=',
            'state': Uri.parse(url).queryParameters['state']!,
          });
          return true;
        },
      );

      final token = await env.service.authorize(port: env.server.port);

      // token 已返回并持久化
      expect(token.accessToken, 'new-access');
      expect(env.store.stored?.accessToken, 'new-access');
      expect(env.store.stored?.refreshToken, 'new-refresh');

      // 授权 URL 指向华为 v3 端点且带 PKCE
      expect(capturedUrl, contains('oauth2/v3/authorize'));
      expect(capturedUrl, contains('code_challenge_method=S256'));

      // 换 token 请求：form 编码怪癖 —— code 中 '+' 必须 %2B
      final tokenReq = env.adapter.requests.firstWhere(
        (r) => r.uri.path == '/oauth2/v3/token',
      );
      expect(tokenReq.body, contains('grant_type=authorization_code'));
      expect(tokenReq.body, contains('code=CODE%2BWITH%2FPLUS%3D'));
      expect(tokenReq.body, contains('client_id=test-id'));
      expect(tokenReq.body, contains('client_secret=test-secret'));
      expect(tokenReq.body, contains('code_verifier='));

      // UserInfo 三端点合并：oidc sub + info displayName + phone mobile
      final userInfo = env.service.currentUserInfo;
      expect(userInfo, isNotNull);
      expect(userInfo!.sub, 'sub-1');
      expect(userInfo.displayName, '花瓣用户');
      expect(userInfo.mobile, '13800001111');
      expect(userInfo.avatarUrl, 'https://example.com/avatar.png');
      expect(userInfo.primaryLabel, '花瓣用户');
    });

    test('凭据未配置 → ConfigError，不开 server 不发请求', () async {
      final env = await setupFlow(
        authSecrets: const AuthSecrets(),
        onLaunch: (_) async => fail('不应打开浏览器'),
      );

      await expectLater(
        env.service.authorize(port: env.server.port),
        throwsA(isA<ConfigError>()),
      );
      expect(env.adapter.requests, isEmpty);
      await env.server.stop();
    });

    test('用户取消 → AuthError(cancelled)', () async {
      late final AuthService service;
      final env = await setupFlow(
        onLaunch: (url) async {
          unawaited(Future<void>.delayed(const Duration(milliseconds: 50),
              service.cancelAuthorize));
          return true;
        },
      );
      service = env.service;

      await expectLater(
        service.authorize(port: env.server.port),
        throwsA(isA<AuthError>().having(
            (e) => e.authCode, 'authCode', AuthErrorCode.cancelled)),
      );
    });

    test('state 不匹配 → AuthError(stateMismatch)', () async {
      final env = await setupFlow(
        onLaunch: (url) async {
          scheduleCallback(url, {
            'code': 'CODE',
            'state': 'WRONG-STATE',
          });
          return true;
        },
      );

      await expectLater(
        env.service.authorize(port: env.server.port),
        throwsA(isA<AuthError>().having(
            (e) => e.authCode, 'authCode', AuthErrorCode.stateMismatch)),
      );
    });

    test('华为 1101 + invalid scope → AuthError(denied) 含 AGC 指引', () async {
      final env = await setupFlow(
        onLaunch: (url) async {
          scheduleCallback(url, {
            'error': '1101',
            'error_description': 'invalid scope',
            'sub_error': '20042',
          });
          return true;
        },
      );

      await expectLater(
        env.service.authorize(port: env.server.port),
        throwsA(isA<AuthError>()
            .having((e) => e.authCode, 'authCode', AuthErrorCode.denied)
            .having((e) => e.message, 'message',
                contains('scope 未在 AppGallery Connect 后台授权'))),
      );
    });

    test('华为拒绝授权 → AuthError(denied) 携带描述', () async {
      final env = await setupFlow(
        onLaunch: (url) async {
          scheduleCallback(url, {
            'error': 'access_denied',
            'error_description': 'user denied',
          });
          return true;
        },
      );

      await expectLater(
        env.service.authorize(port: env.server.port),
        throwsA(isA<AuthError>()
            .having((e) => e.authCode, 'authCode', AuthErrorCode.denied)
            .having((e) => e.message, 'message', contains('user denied'))),
      );
    });

    test('UserInfo 拉取失败不阻塞登录（尽力而为）', () async {
      final env = await setupFlow(
        handler: (req) {
          if (req.uri.path == '/oauth2/v3/token') {
            return jsonResponse(tokenEndpointJson());
          }
          throw DioException(
            requestOptions: RequestOptions(),
            type: DioExceptionType.connectionError,
          );
        },
        onLaunch: (url) async {
          scheduleCallback(url, {
            'code': 'CODE',
            'state': Uri.parse(url).queryParameters['state']!,
          });
          return true;
        },
      );

      final token = await env.service.authorize(port: env.server.port);
      expect(token.accessToken, 'new-access');
      // 三端点全失败 → 合并为空 UserInfo（不阻塞登录，对齐 Rust 尽力而为语义）
      final userInfo = env.service.currentUserInfo;
      expect(userInfo, isNotNull);
      expect(userInfo!.primaryLabel, isNull);
    });
  });

  group('AuthService.restore', () {
    test('无 token → loggedIn=false，携带凭据/端口快照', () async {
      final store = InMemoryTokenStore();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      final snapshot = await service.restore();
      expect(snapshot.loggedIn, isFalse);
      expect(snapshot.secretConfigured, isTrue);
      expect(snapshot.callbackPort, AuthConstants.defaultCallbackPort);
      expect(adapter.requests, isEmpty);
    });

    test('token 新鲜 → loggedIn=true，不发刷新请求', () async {
      final store = InMemoryTokenStore()..stored = freshToken();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      final snapshot = await service.restore();
      expect(snapshot.loggedIn, isTrue);
      expect(adapter.requests, isEmpty);
    });

    test('token 临期 → 主动刷新后 loggedIn=true', () async {
      final store = InMemoryTokenStore()..stored = expiringToken();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      final snapshot = await service.restore();
      expect(snapshot.loggedIn, isTrue);
      expect(store.stored?.accessToken, 'new-access');
      expect(
        adapter.requests.where((r) => r.uri.path == '/oauth2/v3/token'),
        hasLength(1),
      );
    });

    test('临期刷新被拒（TokenError）→ 登出清理，loggedIn=false', () async {
      final store = InMemoryTokenStore()..stored = expiringToken();
      final adapter = FakeHttpAdapter(
        (req) => jsonResponse(
          {'error': 'invalid_grant', 'error_description': '已过期'},
          status: 400,
        ),
      );
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      final snapshot = await service.restore();
      expect(snapshot.loggedIn, isFalse);
      // 已登出：token 清除
      expect(store.clearCount, 1);
      expect(store.stored, isNull);
    });

    test('临期刷新遇网络故障 → 保留 token 并向上抛错（对齐 Rust）', () async {
      final store = InMemoryTokenStore()..stored = expiringToken();
      final adapter = FakeHttpAdapter((req) {
        throw DioException(
          requestOptions: RequestOptions(),
          type: DioExceptionType.connectionError,
          error: const SocketException('offline'),
        );
      });
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      await expectLater(
        service.restore(),
        throwsA(isA<DriveApiError>().having(
            (e) => e.driveCode, 'driveCode', DriveApiErrorCode.network)),
      );
      // 网络故障不登出：token 保留
      expect(store.clearCount, 0);
      expect(store.stored, isNotNull);
    });
  });

  group('AuthService.ensureValidAccessToken', () {
    test('新鲜 token 直接返回，临期触发刷新', () async {
      final store = InMemoryTokenStore()..stored = freshToken();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      expect(await service.ensureValidAccessToken(), 'fresh-access');
      expect(adapter.requests, isEmpty);

      store.stored = expiringToken();
      service.refresher.clearCurrent();
      expect(await service.ensureValidAccessToken(), 'new-access');
    });

    test('未登录 → TokenError(notLoggedIn)', () async {
      final service = AuthService(
        tokenStore: InMemoryTokenStore(),
        http: FakeHttpAdapter(huaweiHandler).createDio(),
        secrets: secrets,
      );
      await expectLater(
        service.ensureValidAccessToken(),
        throwsA(isA<TokenError>().having((e) => e.tokenCode, 'tokenCode',
            TokenErrorCode.notLoggedIn)),
      );
    });
  });

  group('AuthService.isLoggedIn / logout / getUserInfo', () {
    test('isLoggedIn 以 token store 记录为准', () async {
      final store = InMemoryTokenStore();
      final service = AuthService(
        tokenStore: store,
        http: FakeHttpAdapter(huaweiHandler).createDio(),
        secrets: secrets,
      );
      expect(await service.isLoggedIn(), isFalse);
      store.stored = freshToken();
      expect(await service.isLoggedIn(), isTrue);
    });

    test('logout 清空存储 + 内存 + UserInfo 缓存', () async {
      final store = InMemoryTokenStore()..stored = freshToken();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      await service.getUserInfo(); // 建立缓存
      expect(service.currentUserInfo, isNotNull);

      await service.logout();
      expect(store.stored, isNull);
      expect(service.currentUserInfo, isNull);
      expect(await service.isLoggedIn(), isFalse);
    });

    test('getUserInfo 合并三端点并缓存', () async {
      final store = InMemoryTokenStore()..stored = freshToken();
      final adapter = FakeHttpAdapter(huaweiHandler);
      final service = AuthService(
        tokenStore: store,
        http: adapter.createDio(),
        secrets: secrets,
      );

      final userInfo = await service.getUserInfo();
      expect(userInfo.sub, 'sub-1');
      expect(userInfo.displayName, '花瓣用户');
      expect(userInfo.mobile, '13800001111');
      expect(service.currentUserInfo?.displayName, '花瓣用户');

      // rest.php 请求为 form 编码并携带 access_token
      final infoReq = adapter.requests.firstWhere(
        (r) => r.uri.queryParameters['nsp_svc'] == 'GOpen.User.getInfo',
      );
      expect(infoReq.body, contains('access_token=fresh-access'));
      expect(infoReq.body, contains('getNickName=1'));
    });
  });

  group('AuthService.exchangeCodeForToken 失败路径', () {
    test('响应无 access_token → GenericError 携带华为描述', () async {
      final adapter = FakeHttpAdapter(
        (req) => jsonResponse(
          {'error': 'invalid_grant', 'error_description': 'code 无效'},
          status: 400,
        ),
      );
      final service = AuthService(
        tokenStore: InMemoryTokenStore(),
        http: adapter.createDio(),
        secrets: secrets,
      );

      await expectLater(
        service.exchangeCodeForToken(
          secrets: secrets,
          code: 'bad-code',
          redirectUri: 'http://127.0.0.1:9999/oauth/callback',
          codeVerifier: 'v',
        ),
        throwsA(isA<GenericError>()
            .having((e) => e.message, 'message', contains('code 无效'))),
      );
    });

    test('响应非 JSON → authTokenResponseInvalid', () async {
      final adapter =
          FakeHttpAdapter((req) => textResponse('<html>error</html>'));
      final service = AuthService(
        tokenStore: InMemoryTokenStore(),
        http: adapter.createDio(),
        secrets: secrets,
      );

      await expectLater(
        service.exchangeCodeForToken(
          secrets: secrets,
          code: 'c',
          redirectUri: 'http://127.0.0.1:9999/oauth/callback',
        ),
        throwsA(isA<AuthError>().having((e) => e.authCode, 'authCode',
            AuthErrorCode.tokenResponseInvalid)),
      );
    });
  });

  group('AuthService.validateCallback', () {
    test('无 code 无 error → authInvalidCode', () {
      expect(
        () => AuthService.validateCallback(
          const OauthCallbackResult(state: 's'),
          's',
        ),
        throwsA(isA<AuthError>().having((e) => e.authCode, 'authCode',
            AuthErrorCode.invalidCode)),
      );
    });

    test('合法回调通过', () {
      expect(
        () => AuthService.validateCallback(
          const OauthCallbackResult(code: 'c', state: 's'),
          's',
        ),
        returnsNormally,
      );
    });
  });
}
