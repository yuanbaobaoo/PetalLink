import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/storage/app_paths.dart';

/// AppPaths 数据目录测试。
///
/// 回归背景：2026-07-21 发现 flutter run（debug 构建）的数据库写到了
/// 正式版数据目录（io.github.yuanbaobaoo.PetalLink），应为
/// PetalLink-dev（对齐 tauri.dev.conf.json 与 xcconfig 的 dev/release 隔离）。
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const channel =
      MethodChannel('plugins.flutter.io/path_provider');

  setUp(() {
    AppPaths.debugSupportRoot = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
      if (call.method == 'getApplicationSupportDirectory') {
        // path_provider 返回 <root>/<bundle id>（debug 构建为 -dev）
        return '/Users/test/Library/Application Support/'
            'io.github.yuanbaobaoo.PetalLink-dev';
      }
      return null;
    });
  });

  tearDown(() {
    AppPaths.debugSupportRoot = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  test('debug 构建（kDebugMode）→ 数据目录带 -dev 后缀', () async {
    final dir = await AppPaths.supportDir();
    expect(dir.path, contains('PetalLink-dev'));
    expect(dir.path, isNot(contains('PetalLink/')));

    final dbPath = await AppPaths.databasePath();
    expect(dbPath, contains('PetalLink-dev'));
  });

  test('debugSupportRoot 注入时按注入根 + bundle 名拼接（测试语义不变）',
      () async {
    AppPaths.debugSupportRoot = '/tmp/test-root';
    final dir = await AppPaths.supportDir();
    expect(dir.path,
        '/tmp/test-root/${AppPaths.bundleIdentifier}');
  });
}
