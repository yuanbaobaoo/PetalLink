import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/service/platform/launch_at_login.dart';
import 'package:petal_link/service/update/update_service.dart';

void main() {
  group('SemanticVersion（对齐 CMP SemanticVersion）', () {
    test('解析常规与 v 前缀/后缀变体', () {
      expect(SemanticVersion.parse('1.2.3')?.toString(), '1.2.3');
      expect(SemanticVersion.parse('v1.2.3')?.toString(), '1.2.3');
      expect(SemanticVersion.parse('1.2.3-beta.1')?.toString(), '1.2.3');
      expect(SemanticVersion.parse('1.2.3+build5')?.toString(), '1.2.3');
    });

    test('非法版本返回 null', () {
      expect(SemanticVersion.parse(''), isNull);
      expect(SemanticVersion.parse('1.2'), isNull);
      expect(SemanticVersion.parse('a.b.c'), isNull);
      expect(SemanticVersion.parse('1.2.3.4'), isNull);
    });

    test('isNewerThan 逐段比较', () {
      final base = SemanticVersion.parse('1.2.3')!;
      expect(SemanticVersion.parse('1.2.4')!.isNewerThan(base), isTrue);
      expect(SemanticVersion.parse('1.3.0')!.isNewerThan(base), isTrue);
      expect(SemanticVersion.parse('2.0.0')!.isNewerThan(base), isTrue);
      expect(SemanticVersion.parse('1.2.3')!.isNewerThan(base), isFalse);
      expect(SemanticVersion.parse('1.2.2')!.isNewerThan(base), isFalse);
      expect(SemanticVersion.parse('0.9.9')!.isNewerThan(base), isFalse);
    });
  });

  group('UpdateService.parseManifest', () {
    final sha = 'a' * 64;

    test('Tauri updater 平台映射格式', () {
      final manifest = UpdateService.parseManifest({
        'version': '1.2.0',
        'notes': '修复若干问题',
        'pub_date': '2026-07-20T00:00:00Z',
        'platforms': {
          'darwin-aarch64': {
            'url': 'https://example.com/PetalLink_arm64.dmg',
            'sha256': sha,
            'signature': 'sig-content',
          },
        },
      }, platformKey: 'darwin-aarch64');

      expect(manifest.version, '1.2.0');
      expect(manifest.notes, '修复若干问题');
      expect(manifest.url, 'https://example.com/PetalLink_arm64.dmg');
      expect(manifest.sha256, sha);
      expect(manifest.signature, 'sig-content');
      expect(manifest.pubDate, '2026-07-20T00:00:00Z');
    });

    test('darwin-universal 回退', () {
      final manifest = UpdateService.parseManifest({
        'version': '1.2.0',
        'platforms': {
          'darwin-universal': {
            'url': 'https://example.com/PetalLink_universal.dmg',
            'sha256': sha,
          },
        },
      }, platformKey: 'darwin-x86_64');
      expect(manifest.url, contains('universal'));
    });

    test('CMP 扁平格式', () {
      final manifest = UpdateService.parseManifest({
        'version': 'v1.3.0',
        'url': 'https://example.com/PetalLink.dmg',
        'sha256': sha.toUpperCase(),
        'notes': 'n',
      });
      expect(manifest.version, 'v1.3.0');
      expect(manifest.sha256, sha.toUpperCase());
    });

    test('缺少当前平台下载项 → 抛错', () {
      expect(
        () => UpdateService.parseManifest({
          'version': '1.2.0',
          'platforms': {
            'windows-x86_64': {'url': 'https://example.com/a.zip', 'sha256': sha},
          },
        }, platformKey: 'darwin-aarch64'),
        throwsA(isA<GenericError>()),
      );
    });

    test('版本号非法 → 抛错', () {
      expect(
        () => UpdateService.parseManifest({
          'version': 'not-a-version',
          'url': 'https://example.com/a.dmg',
          'sha256': sha,
        }),
        throwsA(isA<GenericError>()),
      );
    });

    test('url 非 https → 抛错（对齐 CMP validateManifest）', () {
      expect(
        () => UpdateService.parseManifest({
          'version': '1.2.0',
          'url': 'http://example.com/a.dmg',
          'sha256': sha,
        }),
        throwsA(isA<GenericError>()),
      );
    });

    test('sha256 格式非法 → 抛错', () {
      expect(
        () => UpdateService.parseManifest({
          'version': '1.2.0',
          'url': 'https://example.com/a.dmg',
          'sha256': 'xyz',
        }),
        throwsA(isA<GenericError>()),
      );
    });

    test('sha256 缺失 → 允许解析为 null（对齐 Tauri 标准清单仅有 signature）',
        () {
      final manifest = UpdateService.parseManifest({
        'version': '1.2.0',
        'url': 'https://example.com/a.dmg',
        'signature': 'minisign-content',
      });
      expect(manifest.sha256, isNull);
      expect(manifest.signature, 'minisign-content');
    });
  });

  group('UpdateService.check', () {
    final sha = 'b' * 64;

    UpdateService serviceWith(http.Client client, {String version = '1.0.0'}) {
      return UpdateService(
        httpClient: client,
        currentVersion: version,
        endpoint: 'https://example.com/update.json',
        enableUpdateCheck: true,
      );
    }

    test('有更新 → 返回清单', () async {
      final client = MockClient((request) async {
        return http.Response.bytes(
          utf8.encode(jsonEncode({
            'version': '1.1.0',
            'url': 'https://example.com/PetalLink.dmg',
            'sha256': sha,
            'notes': 'changelog',
          })),
          200,
        );
      });

      final result = await serviceWith(client).check();
      expect(result.isOk, isTrue);
      final manifest = (result as Ok<UpdateManifest?>).value;
      expect(manifest, isNotNull);
      expect(manifest!.version, '1.1.0');
    });

    test('已是最新 → 返回 null', () async {
      final client = MockClient((request) async {
        return http.Response.bytes(
          utf8.encode(jsonEncode({
            'version': '1.0.0',
            'url': 'https://example.com/PetalLink.dmg',
            'sha256': sha,
          })),
          200,
        );
      });

      final result = await serviceWith(client).check();
      expect((result as Ok<UpdateManifest?>).value, isNull);
    });

      test('HTTP 错误 → Err', () async {
      final client = MockClient((request) async => http.Response('nf', 404));
      final result = await serviceWith(client).check();
      expect(result.isErr, isTrue);
    });

    test('清单校验失败 → Err（不崩溃）', () async {
      final client = MockClient((request) async {
        return http.Response.bytes(
            utf8.encode(jsonEncode({'version': '9.9.9'})), 200);
      });
      final result = await serviceWith(client).check();
      expect(result.isErr, isTrue);
    });

    test('端点非 https → Err', () async {
      final client = MockClient((request) async => http.Response('{}', 200));
      final result = await UpdateService(
        httpClient: client,
        endpoint: 'http://example.com/update.json',
        enableUpdateCheck: true,
      ).check();
      expect(result.isErr, isTrue);
    });

    test('dev 构建（enableUpdateCheck=false）→ 跳过检查返回 null', () async {
      // 不应发起任何网络请求（client 为空 mock 也行）
      final client = MockClient((request) async =>
          http.Response('should-not-be-called', 200));
      final result = await UpdateService(
        httpClient: client,
        endpoint: 'https://example.com/update.json',
        enableUpdateCheck: false,
      ).check();
      expect(result.isOk, isTrue);
      expect((result as Ok<UpdateManifest?>).value, isNull);
    });
  });

  group('UpdateService.downloadAndStage', () {
    late Directory tempDir;

    setUp(() {
      tempDir = Directory.systemTemp.createTempSync('update_service_test');
    });

    tearDown(() {
      if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
    });

    test('下载成功 + SHA-256 匹配 + 进度回调', () async {
      final payload = List<int>.generate(4096, (i) => i % 251);
      final digest = sha256.convert(payload).toString();
      final client = MockClient((request) async {
        return http.Response.bytes(payload, 200);
      });
      final service = UpdateService(
        httpClient: client,
        updatesDir: tempDir.path,
      );
      final manifest = UpdateManifest(
        version: '1.1.0',
        url: 'https://example.com/PetalLink.dmg',
        sha256: digest,
      );

      final progress = <(int, int?)>[];
      final result = await service.downloadAndStage(
        manifest,
        onProgress: (received, total) => progress.add((received, total)),
      );

      expect(result.isOk, isTrue);
      final path = (result as Ok<String>).value;
      expect(path, endsWith('PetalLink.dmg'));
      expect(await File(path).readAsBytes(), payload);
      expect(progress, isNotEmpty);
      // .part 临时文件不残留
      expect(await File('$path.part').exists(), isFalse);
    });

    test('SHA-256 不匹配 → 删包并报错', () async {
      final payload = utf8.encode('dmg-bytes');
      final client = MockClient((request) async {
        return http.Response.bytes(payload, 200);
      });
      final service = UpdateService(
        httpClient: client,
        updatesDir: tempDir.path,
      );
      final manifest = UpdateManifest(
        version: '1.1.0',
        url: 'https://example.com/PetalLink.dmg',
        sha256: '0' * 64,
      );

      final result = await service.downloadAndStage(manifest);
      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('SHA-256'));
      // 版本目录下无残留 dmg
      final dir = Directory('${tempDir.path}/1.1.0');
      final remains = dir.existsSync()
          ? dir.listSync().where((f) => f.path.endsWith('.dmg')).toList()
          : [];
      expect(remains, isEmpty);
    });

    test('HTTP 错误 → Err', () async {
      final client = MockClient((request) async => http.Response('x', 500));
      final service = UpdateService(
        httpClient: client,
        updatesDir: tempDir.path,
      );
      final result = await service.downloadAndStage(UpdateManifest(
        version: '1.1.0',
        url: 'https://example.com/PetalLink.dmg',
        sha256: 'a' * 64,
      ));
      expect(result.isErr, isTrue);
    });

    test('sha256 缺失 → 跳过哈希校验直接落盘（安装期由签名校验兜底）',
        () async {
      final payload = utf8.encode('dmg-bytes');
      final client = MockClient((request) async {
        return http.Response.bytes(payload, 200);
      });
      final service = UpdateService(
        httpClient: client,
        updatesDir: tempDir.path,
      );
      final result = await service.downloadAndStage(const UpdateManifest(
        version: '1.1.0',
        url: 'https://example.com/PetalLink.dmg',
      ));
      expect(result.isOk, isTrue);
      expect(await File((result as Ok<String>).value).readAsBytes(), payload);
    });
  });

  group('UpdateService.installAndRelaunch minisign 验签（对齐 Tauri updater）', () {
    late Directory tempDir;
    late String executable;
    late String archivePath;

    /// 测试用 minisign 公钥（由 minisign 0.12 真实生成）
    const testPublicKey = 'RWQumH1FCnRspPE49wySomZGx2w80Y4Yrt785hrQR5duOAcdhgVVb1Pu';

    /// 测试用签名（对 "hello petallink update package\n" 的 minisign prehashed 签名）
    const testSigText = '''untrusted comment: signature from minisign secret key
RUQumH1FCnRspNU2xY6zOPq5vZK16g5cmp0e3GzqTIkSkUQyH+zRTPuLJ/k/AkWByliYulOrRJzsPAsSaYubxLDtOxuWgXryDQM=
trusted comment: timestamp:1784872454\tfile:test_payload.bin\thashed
x7N6m+v81toEeyF+y5tuYgmgE3WGczy15xzF1EtRw4J/ln//l4eVGEqkRbaj5SlTaOtbuD6mTabudP4R6Y2/Dg==''';

    /// 被签名的文件内容（与签名匹配）
    final signedContent =
        'hello petallink update package\n'.codeUnits;

    setUp(() async {
      tempDir = Directory.systemTemp.createTempSync('update_install_test');
      executable = '${tempDir.path}/My.app/Contents/MacOS/My';
      await Directory('${tempDir.path}/My.app/Contents/MacOS')
          .create(recursive: true);
      await File(executable).writeAsString('bin');
      archivePath = '${tempDir.path}/pkg/PetalLink.app.tar.gz';
      await Directory('${tempDir.path}/pkg').create(recursive: true);
    });

    tearDown(() {
      if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
    });

    UpdateService serviceWith({required Future<ProcResult> Function(String, List<String>) runner}) {
      return UpdateService(
        updatesDir: tempDir.path,
        runner: runner,
        currentExecutable: executable,
        currentPid: 999999,
        publicKey: testPublicKey,
      );
    }

    test('未携带签名 → 拒绝安装', () async {
      // 造一个假 tar.gz（内容无关，验签前就因签名缺失被拒）
      await File(archivePath).writeAsBytes(signedContent);
      final service = serviceWith(
          runner: (exe, args) async => const ProcResult(exitCode: 0));
      final result =
          await service.installAndRelaunch(archivePath, signature: null);
      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('未携带签名'));
    });

    test('签名校验失败（内容被篡改）→ 拒绝安装', () async {
      // 写入与签名不匹配的内容
      await File(archivePath).writeAsBytes('tampered content'.codeUnits);
      final service = serviceWith(
          runner: (exe, args) async => const ProcResult(exitCode: 0));
      final result = await service.installAndRelaunch(
        archivePath,
        signature: testSigText,
      );
      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('签名校验失败'));
    });

    test('签名校验通过 → 进入解压阶段（tar 被调用）', () async {
      // 写入与签名匹配的内容
      await File(archivePath).writeAsBytes(signedContent);
      final recorded = <String>[];
      final service = serviceWith(runner: (exe, args) async {
        recorded.add('$exe ${args.join(' ')}');
        // tar 故意返回失败，避免走到 exit(0) 终止测试进程；
        // 能到达 tar 即证明 minisign 验签已通过
        if (exe.contains('tar')) {
          return const ProcResult(exitCode: 1, stderr: 'mock fail');
        }
        return const ProcResult(exitCode: 0);
      });
      final result = await service.installAndRelaunch(
        archivePath,
        signature: testSigText,
      );
      // tar 被调用（验签通过的证据），且因 mock 失败返回解压错误
      expect(recorded.any((c) => c.contains('tar')), isTrue);
      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('解压'));
    });
  });
}
