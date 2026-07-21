import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/service/update/update_service.dart';

void main() {
  late Directory tempDir;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('update_controller_test');
  });

  tearDown(() {
    if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
  });

  const manifestJson = {
    'version': '1.1.0',
    'url': 'https://example.com/PetalLink.dmg',
    'sha256': 'will-be-patched-per-test',
    'notes': '更新日志',
  };

  /// 构造 (controller, 可变活跃传输标志)；manifest 校验值按 payload 实算
  (UpdateController, bool Function()) controllerWith({
    required Map<String, dynamic> manifest,
    List<int>? payload,
    bool hasActive = false,
    String currentVersion = '1.0.0',
  }) {
    final activeFlag = <String, bool>{'v': hasActive};
    final client = MockClient((request) async {
      if (request.url.path.endsWith('update.json')) {
        return http.Response.bytes(utf8.encode(jsonEncode(manifest)), 200);
      }
      return http.Response.bytes(payload ?? [], 200);
    });
    final service = UpdateService(
      httpClient: client,
      currentVersion: currentVersion,
      updatesDir: tempDir.path,
      endpoint: 'https://example.com/update.json',
    );
    final controller = UpdateController(
      updateService: service,
      hasActiveTransfersProvider: () async => activeFlag['v']!,
      waitPollInterval: const Duration(milliseconds: 20),
      waitTimeout: const Duration(milliseconds: 200),
    );
    return (controller, () => activeFlag['v']!);
  }

  Map<String, dynamic> manifestFor(List<int> payload) {
    return {
      ...manifestJson,
      'sha256': sha256.convert(payload).toString(),
    };
  }

  group('检查更新（对齐 Vue silentCheck/manualCheck）', () {
    test('手动检查发现更新 → available + 弹窗', () async {
      final (controller, _) = controllerWith(manifest: manifestFor([1]));
      final found = await controller.manualCheck();

      expect(found, isTrue);
      expect(controller.state.value.phase, UpdatePhase.available);
      expect(controller.state.value.manifest?.version, '1.1.0');
      expect(controller.state.value.dialogVisible, isTrue);
    });

    test('静默检查发现更新 → available 但不弹窗', () async {
      final (controller, _) = controllerWith(manifest: manifestFor([1]));
      await controller.silentCheck();

      expect(controller.state.value.phase, UpdatePhase.available);
      expect(controller.state.value.dialogVisible, isFalse);
    });

    test('手动检查已是最新 → upToDate', () async {
      final (controller, _) = controllerWith(
        manifest: manifestFor([1]),
        currentVersion: '9.9.9',
      );
      final found = await controller.manualCheck();

      expect(found, isFalse);
      expect(controller.state.value.phase, UpdatePhase.upToDate);
    });

    test('静默检查已是最新 → idle', () async {
      final (controller, _) = controllerWith(
        manifest: manifestFor([1]),
        currentVersion: '9.9.9',
      );
      await controller.silentCheck();
      expect(controller.state.value.phase, UpdatePhase.idle);
    });

    test('检查失败：静默 → idle，手动 → failed', () async {
      final client = MockClient((request) async => http.Response('x', 500));
      final service = UpdateService(
        httpClient: client,
        endpoint: 'https://example.com/update.json',
        updatesDir: tempDir.path,
      );
      final controller = UpdateController(
        updateService: service,
        hasActiveTransfersProvider: () async => false,
      );

      await controller.silentCheck();
      expect(controller.state.value.phase, UpdatePhase.idle);

      await controller.manualCheck();
      expect(controller.state.value.phase, UpdatePhase.failed);
      expect(controller.state.value.errorMessage, isNotNull);
    });

    test('聚焦检查 10 分钟节流', () async {
      final (controller, _) = controllerWith(manifest: manifestFor([1]));
      await controller.manualCheck();
      controller.dismiss();

      // 距上次检查 <10 分钟 → 节流不再检查（phase 保持 idle）
      await controller.checkOnFocus();
      expect(controller.state.value.phase, UpdatePhase.idle);
    });
  });

  group('下载与等待传输（对齐 Vue downloadAndInstall/waitForTransfers）', () {
    test('下载校验完成 → 无活跃传输 → ready', () async {
      final payload = List<int>.generate(2048, (i) => i % 253);
      final (controller, _) = controllerWith(
        manifest: manifestFor(payload),
        payload: payload,
      );
      await controller.manualCheck();
      await controller.downloadAndInstall();

      expect(controller.state.value.phase, UpdatePhase.ready);
      expect(controller.state.value.downloadProgress, 1.0);
      expect(controller.state.value.dialogVisible, isTrue);
    });

    test('SHA-256 不匹配 → failed', () async {
      final payload = utf8.encode('payload');
      final (controller, _) = controllerWith(
        manifest: {...manifestJson, 'sha256': 'f' * 64},
        payload: payload,
      );
      await controller.manualCheck();
      await controller.downloadAndInstall();

      expect(controller.state.value.phase, UpdatePhase.failed);
      expect(controller.state.value.errorMessage, contains('SHA-256'));
    });

    test('有活跃传输 → waitingTransfers；传输结束 → ready（门控）', () async {
      final payload = utf8.encode('dmg');
      final activeFlag = <String, bool>{'v': true};
      final client = MockClient((request) async {
        if (request.url.path.endsWith('update.json')) {
          return http.Response.bytes(
              utf8.encode(jsonEncode(manifestFor(payload))), 200);
        }
        return http.Response.bytes(payload, 200);
      });
      final service = UpdateService(
        httpClient: client,
        updatesDir: tempDir.path,
        endpoint: 'https://example.com/update.json',
      );
      final controller = UpdateController(
        updateService: service,
        hasActiveTransfersProvider: () async => activeFlag['v']!,
        waitPollInterval: const Duration(milliseconds: 20),
        waitTimeout: const Duration(seconds: 5),
      );

      await controller.manualCheck();
      // downloadAndInstall 内部会等待传输结束；先推进到 waitingTransfers
      final flow = controller.downloadAndInstall();
      await Future<void>.delayed(const Duration(milliseconds: 100));
      expect(controller.state.value.phase, UpdatePhase.waitingTransfers);
      expect(controller.state.value.hasActiveTransfers, isTrue);

      // 传输结束 → 轮询推进 ready
      activeFlag['v'] = false;
      await flow;
      expect(controller.state.value.phase, UpdatePhase.ready);
    });

    test('等待传输超时 → 回退 downloaded（可重试）', () async {
      final payload = utf8.encode('dmg');
      final (controller, _) = controllerWith(
        manifest: manifestFor(payload),
        payload: payload,
        hasActive: true,
      );
      // waitTimeout 200ms（构造注入），活跃传输一直存在 → 超时
      await controller.manualCheck();
      await controller.downloadAndInstall();

      expect(controller.state.value.phase, UpdatePhase.downloaded);
      expect(controller.state.value.dialogVisible, isTrue);
    });

    test('dismiss 回到 idle 并清理状态', () async {
      final (controller, _) = controllerWith(manifest: manifestFor([1]));
      await controller.manualCheck();
      controller.dismiss();

      expect(controller.state.value.phase, UpdatePhase.idle);
      expect(controller.state.value.manifest, isNull);
      expect(controller.state.value.dialogVisible, isFalse);
    });
  });
}
