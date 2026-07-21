import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/service/auth/oauth_server.dart';

void main() {
  late HttpClient http;

  setUp(() {
    http = HttpClient();
  });

  tearDown(() {
    http.close(force: true);
  });

  /// 向 server 发起 GET 并读取响应文本。
  Future<String> getPage(int port, String path) async {
    final request = await http.get('127.0.0.1', port, path);
    final response = await request.close();
    return response.transform(const SystemEncoding().decoder).join();
  }

  group('OauthServer', () {
    test('回调成功：解析 code/state，回写授权成功页，单次使用后关闭', () async {
      final server = await OauthServer.start(0);
      final port = server.port;
      final waitFuture = server.waitForCallback();

      final page = await getPage(
        port,
        '/oauth/callback?code=AUTH_CODE_123&state=STATE_XYZ',
      );

      final result = await waitFuture;
      expect(result.code, 'AUTH_CODE_123');
      expect(result.state, 'STATE_XYZ');
      expect(result.error, isNull);
      expect(result.isSuccess, isTrue);
      expect(page, contains('授权成功'));

      // 单次使用：server 已关闭，再次连接失败
      expect(
        () => getPage(port, '/oauth/callback?code=x'),
        throwsA(isA<SocketException>()),
      );
    });

    test('华为错误回调：解析 error/error_description/sub_error，回写失败页',
        () async {
      final server = await OauthServer.start(0);
      final waitFuture = server.waitForCallback();

      final page = await getPage(
        server.port,
        '/oauth/callback?error=1101&error_description=invalid%20scope&sub_error=20042',
      );

      final result = await waitFuture;
      expect(result.code, isNull);
      expect(result.error, '1101');
      expect(result.errorDescription, 'invalid scope');
      expect(result.subError, '20042');
      expect(result.isSuccess, isFalse);
      expect(page, contains('授权失败'));
    });

    test("'+' 在 query 中按 form-urlencoded 解码为空格（对齐 Rust url_decode）",
        () async {
      final server = await OauthServer.start(0);
      final waitFuture = server.waitForCallback();

      await getPage(server.port, '/oauth/callback?error_description=a+b');

      final result = await waitFuture;
      expect(result.errorDescription, 'a b');
    });

    test('非回调路径返回错误结果（对齐 Rust 无效回调路径）', () async {
      final server = await OauthServer.start(0);
      final waitFuture = server.waitForCallback();

      await getPage(server.port, '/favicon.ico');

      final result = await waitFuture;
      expect(result.isSuccess, isFalse);
      expect(result.error, '无效回调路径');
    });

    test('stopHandle.stop()：等待方收到通道关闭错误（取消语义）', () async {
      final server = await OauthServer.start(0);
      final waitFuture = server.waitForCallback();

      await Future<void>.delayed(const Duration(milliseconds: 50));
      server.stopHandle.stop();

      await expectLater(
        waitFuture,
        throwsA(
          isA<AppError>().having((e) => e.message, 'message', contains('通道关闭')),
        ),
      );
    });

    test('等待超时抛 authTimeout', () async {
      final server = await OauthServer.start(0);
      await expectLater(
        server.waitForCallback(timeout: const Duration(milliseconds: 100)),
        throwsA(
          isA<AuthError>().having(
              (e) => e.authCode, 'authCode', AuthErrorCode.timeout),
        ),
      );
    });
  });
}
