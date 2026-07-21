import 'dart:convert';
import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/config_entry.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

void main() {
  late Directory tempDir;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('config_service_test');
    DatabaseService.debugDatabasePath = '${tempDir.path}/petal_link.db';
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
  });

  ConfigService newService({
    Future<void> Function()? onEngineConfigChanged,
    Future<void> Function(String?, String?)? onMountDirChanged,
  }) {
    return ConfigService(
      DatabaseService.instance,
      const FlutterSecureStorage(),
      onEngineConfigChanged: onEngineConfigChanged,
      onMountDirChanged: onMountDirChanged,
    );
  }

  AppConfig configWithMount(String dir) => AppConfig(
        mountDir: dir,
        mountConfigured: true,
      );

  group('configLoad（对齐 config_load）', () {
    test('空存储 → 全默认值（对齐 serde default）', () async {
      final config = await newService().configLoad();
      expect(config.mountDir, '');
      expect(config.mountConfigured, isFalse);
      expect(config.concurrency, 6);
      expect(config.pollIntervalSec, 60);
      expect(config.debounceSec, 3);
      expect(config.skipPatterns, AppConfig.defaultSkipPatterns);
      expect(config.oauthCallbackPort, 9999);
    });
  });

  group('configSave（对齐 config_save）', () {
    test('保存后可重新加载（往返一致）', () async {
      final service = newService();
      final dir = '${tempDir.path}/mount';
      await service.configSave(AppConfig(
        mountDir: dir,
        mountConfigured: true,
        concurrency: 10,
        pollIntervalSec: 120,
        debounceSec: 5,
        skipPatterns: ['.git'],
        sortField: SortField.Size,
        sortOrder: SortOrder.Descending,
      ));

      final loaded = await service.configLoad();
      expect(loaded.mountDir, dir);
      expect(loaded.mountConfigured, isTrue);
      expect(loaded.concurrency, 10);
      expect(loaded.pollIntervalSec, 120);
      expect(loaded.debounceSec, 5);
      expect(loaded.skipPatterns, ['.git']);
      expect(loaded.sortField, SortField.Size);
      expect(loaded.sortOrder, SortOrder.Descending);
    });

    test('校验失败 → ConfigError（并发数越界）', () async {
      expect(
        () => newService().configSave(const AppConfig(concurrency: 0)),
        throwsA(isA<ConfigError>()),
      );
    });

    test('挂载目录不可写 → ConfigError（对齐目录可写探测）', () async {
      // 用「文件」当目录：create(recursive) 必然失败
      final file = File('${tempDir.path}/a-file');
      await file.writeAsString('x');
      expect(
        () => newService().configSave(configWithMount(file.path)),
        throwsA(isA<ConfigError>()),
      );
    });

    test('首次配置 → 触发引擎启动回调（分支2）', () async {
      var engineCalls = 0;
      final dirCalls = <(String?, String?)>[];
      final service = newService(
        onEngineConfigChanged: () async => engineCalls++,
        onMountDirChanged: (o, n) async => dirCalls.add((o, n)),
      );
      await service.configSave(configWithMount('${tempDir.path}/mount'));

      expect(engineCalls, 1);
      expect(dirCalls, isEmpty);
    });

    test('常规修改（目录不变）→ 引擎刷新回调（分支3）', () async {
      var engineCalls = 0;
      final dirCalls = <(String?, String?)>[];
      final service = newService(
        onEngineConfigChanged: () async => engineCalls++,
        onMountDirChanged: (o, n) async => dirCalls.add((o, n)),
      );
      final dir = '${tempDir.path}/mount';
      await service.configSave(configWithMount(dir));
      await service
          .configSave(configWithMount(dir).copyWith(concurrency: 12));

      expect(engineCalls, 2);
      expect(dirCalls, isEmpty);
    });

    test('目录变更 → 清运行时+缓存+重启回调，携带新旧绝对路径（分支1）', () async {
      var engineCalls = 0;
      final dirCalls = <(String?, String?)>[];
      final service = newService(
        onEngineConfigChanged: () async => engineCalls++,
        onMountDirChanged: (o, n) async => dirCalls.add((o, n)),
      );
      final dirA = '${tempDir.path}/mount-a';
      final dirB = '${tempDir.path}/mount-b';
      await service.configSave(configWithMount(dirA));
      await service.configSave(configWithMount(dirB));

      expect(dirCalls, [(dirA, dirB)]);
      // 分支1 不再触发引擎回调
      expect(engineCalls, 1);
    });

    test('取消配置 → 目录变更回调（newAbs 为 null）', () async {
      final dirCalls = <(String?, String?)>[];
      final service = newService(
        onMountDirChanged: (o, n) async => dirCalls.add((o, n)),
      );
      final dirA = '${tempDir.path}/mount-a';
      await service.configSave(configWithMount(dirA));
      await service.configSave(const AppConfig());

      expect(dirCalls, [(dirA, null)]);
      expect((await service.configLoad()).mountConfigured, isFalse);
    });
  });

  group('configExportJson / configImportJson（对齐 config.rs）', () {
    test('导出为 camelCase 键 JSON（不含 token）', () async {
      final service = newService();
      await service.configSave(AppConfig(
        mountDir: '${tempDir.path}/mount',
        mountConfigured: true,
        concurrency: 8,
      ));

      final jsonStr = await service.configExportJson();
      final map = jsonDecode(jsonStr) as Map<String, dynamic>;
      expect(map['mountDir'], '${tempDir.path}/mount');
      expect(map['mountConfigured'], isTrue);
      expect(map['concurrency'], 8);
      expect(map['pollIntervalSec'], 60);
      expect(map['debounceSec'], 3);
      expect(map.containsKey('mount_dir'), isFalse);
      expect(map.containsKey('token'), isFalse);
    });

    test('导入：合法 JSON → 返回配置，且不回写存储（仅校验）', () async {
      final service = newService();
      final dir = '${tempDir.path}/import-mount';
      final imported = service.configImportJson(jsonEncode({
        'mountDir': dir,
        'mountConfigured': true,
        'concurrency': 15,
        'pollIntervalSec': 300,
      }));

      expect(imported.mountDir, dir);
      expect(imported.concurrency, 15);
      expect(imported.pollIntervalSec, 300);

      // 仅校验不回写：存储仍为空（默认）
      final loaded = await service.configLoad();
      expect(loaded.mountConfigured, isFalse);
      expect(loaded.concurrency, 6);
    });

    test('导入：非法 JSON → ConfigError', () {
      expect(
        () => newService().configImportJson('not-json{'),
        throwsA(isA<ConfigError>()),
      );
    });

    test('导入：非对象 JSON → ConfigError', () {
      expect(
        () => newService().configImportJson('[1,2,3]'),
        throwsA(isA<ConfigError>()),
      );
    });

    test('导入：字段值越界 → ConfigError（对齐 validate）', () {
      expect(
        () => newService().configImportJson(jsonEncode({'concurrency': 99})),
        throwsA(isA<ConfigError>()),
      );
      expect(
        () => newService()
            .configImportJson(jsonEncode({'oauthCallbackPort': 0})),
        throwsA(isA<ConfigError>()),
      );
      // pollIntervalSec 非 0 且 <60
      expect(
        () =>
            newService().configImportJson(jsonEncode({'pollIntervalSec': 30})),
        throwsA(isA<ConfigError>()),
      );
    });
  });
}
