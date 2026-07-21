import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/service/mount/free_up.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

import 'proc_xattr.dart';

/// 测试用云端核验门面。
class FakeGate implements FreeUpRemoteGate {
  bool trusted = true;
  final Map<String, String> cloud = {};
  final Map<String, FreeUpRemoteSnapshot> remotes = {};
  final Set<String> deleted = {};
  int leaseCount = 0;

  /// 远端核验时注入的副作用（模拟并发改写）。
  Future<void> Function()? onFetchRemote;

  @override
  bool get cloudTreeTrusted => trusted;

  @override
  String? cloudFileIdAt(String relPath) => cloud[relPath];

  @override
  Future<FreeUpRemoteSnapshot> fetchRemote(String fileId) async {
    await onFetchRemote?.call();
    final remote = remotes[fileId];
    if (remote == null) {
      throw AppError.generic('远端文件不存在');
    }
    return remote;
  }

  @override
  Future<bool> verifyDeleted(String fileId) async => deleted.contains(fileId);

  @override
  FreeUpPathLease beginExclusivePathActivity(String relPath) {
    leaseCount++;
    return FakeLease();
  }
}

class FakeLease implements FreeUpPathLease {
  bool closed = false;

  @override
  void close() {
    closed = true;
  }
}

/// 带写钩子的 xattr（在占位创建期间注入并发改写）。
class HookXattr extends ProcXattrService {
  Future<void> Function(String path, String name)? onSet;

  @override
  Future<void> set(String path, String name, String value) async {
    await super.set(path, name, value);
    await onSet?.call(path, name);
  }
}

void main() {
  late Directory tempDir;
  late HookXattr xattr;
  late MountManager mount;
  late FakeGate gate;
  late FreeUpService service;
  late String dbPath;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_freeup_test');
    xattr = HookXattr();
    dbPath = '${tempDir.path}/petal_link.db';
    DatabaseService.debugDatabasePath = dbPath;
    mount = MountManager(
      tempDir.path,
      xattr: xattr,
      db: DatabaseService.instance,
    );
    gate = FakeGate();
    service = FreeUpService(
      mount: mount,
      db: DatabaseService.instance,
      gate: gate,
    );
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  String abs(String rel) => '${tempDir.path}/$rel';

  /// 创建内容与云端一致的已同步文件，并写入基线。
  /// 返回 (localMtime, size)。
  Future<(int, int)> seedSyncedFile({
    String relPath = 'a.bin',
    String fileId = 'fid1',
    int size = 100,
    int cloudEditedTime = 111,
  }) async {
    final file = File(abs(relPath));
    file.createSync(recursive: true);
    file.writeAsBytesSync(List<int>.generate(size, (i) => i % 256));
    final mtime = file.statSync().modified.millisecondsSinceEpoch;

    final db = await DatabaseService.instance.database;
    await db.insert('sync_items', {
      'file_id': fileId,
      'local_path': relPath,
      'name': relPath.split('/').last,
      'is_folder': 0,
      'size': size,
      'local_size': size,
      'local_mtime': mtime,
      'cloud_edited_time': cloudEditedTime,
      'status': SyncItemStatus.Synced.code,
    });

    gate.cloud[relPath] = fileId;
    gate.remotes[fileId] = FreeUpRemoteSnapshot(
      id: fileId,
      size: size,
      editedTimeMs: cloudEditedTime,
    );
    return (mtime, size);
  }

  Future<Map<String, Object?>?> baselineRow(String fileId) async {
    final db = await DatabaseService.instance.database;
    final rows =
        await db.query('sync_items', where: 'file_id = ?', whereArgs: [fileId]);
    return rows.isEmpty ? null : rows.first;
  }

  group('FreeUpService.freeUpSpace 校验分支', () {
    test('引擎未注入 → 拒绝', () async {
      final noGate =
          FreeUpService(mount: mount, db: DatabaseService.instance);
      expect(
        () => noGate.freeUpSpace(fileId: 'f', relPath: 'a.bin', size: 1),
        throwsA(isA<ConfigError>()),
      );
    });

    test('前端路径与 relPath 不一致 → 拒绝', () async {
      expect(
        () => service.freeUpSpace(
          fileId: 'fid1',
          relPath: 'a.bin',
          size: 100,
          localPath: abs('other.bin'),
        ),
        throwsA(predicate((e) => e is AppError && e.message.contains('路径不一致'))),
      );
    });

    test('云端索引未追平 → 拒绝', () async {
      await seedSyncedFile();
      gate.trusted = false;
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('云端索引尚未追平'))),
      );
    });

    test('目标是占位符 → 拒绝', () async {
      await seedSyncedFile();
      // 换成占位符
      File(abs('a.bin')).deleteSync();
      await mount.createPlaceholderStrict('a.bin', 'fid1', 100);
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('不是已下载的普通文件'))),
      );
    });

    test('本地大小与传入 size 不一致 → 拒绝', () async {
      await seedSyncedFile();
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 99),
        throwsA(predicate((e) => e is AppError && e.message.contains('大小已变化'))),
      );
    });

    test('无匹配基线 → 拒绝', () async {
      await seedSyncedFile();
      final db = await DatabaseService.instance.database;
      await db.delete('sync_items');
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('成功同步基线'))),
      );
    });

    test('基线 local_mtime 不一致 → 拒绝', () async {
      await seedSyncedFile();
      final db = await DatabaseService.instance.database;
      await db.update('sync_items', {'local_mtime': 1});
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('基线不一致'))),
      );
    });

    test('可信云树中 fileId 不同 → 拒绝', () async {
      await seedSyncedFile();
      gate.cloud['a.bin'] = 'other-id';
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('可信云树'))),
      );
    });

    test('远端版本与基线不一致 → 拒绝', () async {
      await seedSyncedFile();
      gate.remotes['fid1'] =
          const FreeUpRemoteSnapshot(id: 'fid1', size: 100, editedTimeMs: 222);
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('远端副本'))),
      );
    });

    test('远端已回收 → 拒绝', () async {
      await seedSyncedFile();
      gate.deleted.add('fid1');
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('远端副本'))),
      );
    });

    test('存在活动传输 → 拒绝', () async {
      await seedSyncedFile();
      final db = await DatabaseService.instance.database;
      await db.insert('transfer_queue', {
        'direction': TransferDirection.Upload.code,
        'file_id': 'fid1',
        'name': 'a.bin',
        'total_size': 100,
        'state': TransferState.Running.code,
        'created_at': 1,
        'relative_path': 'a.bin',
      });
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('活动传输'))),
      );
    });

    test('终态传输不阻塞释放', () async {
      await seedSyncedFile();
      final db = await DatabaseService.instance.database;
      await db.insert('transfer_queue', {
        'direction': TransferDirection.Upload.code,
        'file_id': 'fid1',
        'name': 'a.bin',
        'total_size': 100,
        'state': TransferState.Completed.code,
        'created_at': 1,
        'relative_path': 'a.bin',
      });
      final freed = await service.freeUpSpace(
          fileId: 'fid1', relPath: 'a.bin', size: 100);
      expect(freed, 100);
    });
  });

  group('FreeUpService.freeUpSpace 原子事务', () {
    test('成功路径：替换为占位 + DB CAS + 暂存清理', () async {
      await seedSyncedFile();
      final freed =
          await service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100);
      expect(freed, 100);

      // 原位置为占位符
      final file = File(abs('a.bin'));
      expect(file.lengthSync(), 0);
      expect(await xattr.get(abs('a.bin'), xattrFileId), 'fid1');
      expect(await xattr.get(abs('a.bin'), xattrState), statePlaceholder);
      expect(await xattr.get(abs('a.bin'), xattrSize), '100');

      // DB 已结算为 CloudOnly + local_size=0
      final row = await baselineRow('fid1');
      expect(row!['status'], SyncItemStatus.CloudOnly.code);
      expect(row['local_size'], 0);

      // 无暂存残留
      final leftovers = tempDir
          .listSync()
          .where((e) => e.path.contains('.hwcloud_freeup-'))
          .toList();
      expect(leftovers, isEmpty);
    });

    test('DB CAS 冲突 → 回滚恢复文件内容', () async {
      await seedSyncedFile();
      // 占位符创建期间（租约检查之后、CAS 之前）基线被并发删除 → CAS 必然失败
      xattr.onSet = (path, name) async {
        if (name == xattrState) {
          final db = await DatabaseService.instance.database;
          await db.delete('sync_items');
        }
      };
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('并发变化'))),
      );

      // 文件内容完整恢复
      final restored = File(abs('a.bin'));
      expect(restored.lengthSync(), 100);
      expect(await mount.isPlaceholderFile(abs('a.bin')), isFalse);
      // 恢复标记已清
      expect(await xattr.get(abs('a.bin'), xattrFreeUpRelativePath), isNull);
      // 无暂存残留
      final leftovers = tempDir
          .listSync()
          .where((e) => e.path.contains('.hwcloud_freeup-'))
          .toList();
      expect(leftovers, isEmpty);
    });

    test('基线在远端核验期间被删除 → 租约失效拒绝', () async {
      await seedSyncedFile();
      gate.onFetchRemote = () async {
        final db = await DatabaseService.instance.database;
        await db.delete('sync_items');
      };
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('租约已失效'))),
      );
      // 文件未被触碰
      expect(File(abs('a.bin')).lengthSync(), 100);
    });

    test('基线并发改写（非删除）→ 租约失效拒绝', () async {
      await seedSyncedFile();
      gate.onFetchRemote = () async {
        final db = await DatabaseService.instance.database;
        await db.update('sync_items', {'local_size': 1});
      };
      expect(
        () => service.freeUpSpace(fileId: 'fid1', relPath: 'a.bin', size: 100),
        throwsA(predicate((e) => e is AppError && e.message.contains('租约已失效'))),
      );
      // 文件未被触碰
      expect(File(abs('a.bin')).lengthSync(), 100);
    });
  });

  group('FreeUpService.checkSafeFreeUp', () {
    test('全部一致 → safe', () async {
      await seedSyncedFile();
      expect(await service.checkSafeFreeUp('a.bin', 'fid1'), 'safe');
    });

    test('云端不存在 → not_in_cloud', () async {
      await seedSyncedFile();
      gate.cloud.clear();
      expect(await service.checkSafeFreeUp('a.bin', 'fid1'), 'not_in_cloud');
    });

    test('云端索引未追平 → not_synced', () async {
      await seedSyncedFile();
      gate.trusted = false;
      expect(await service.checkSafeFreeUp('a.bin', 'fid1'), 'not_synced');
    });

    test('基线非 Synced → not_synced', () async {
      await seedSyncedFile();
      final db = await DatabaseService.instance.database;
      await db.update('sync_items', {'status': SyncItemStatus.Syncing.code});
      expect(await service.checkSafeFreeUp('a.bin', 'fid1'), 'not_synced');
    });

    test('本地文件缺失 → not_synced', () async {
      await seedSyncedFile();
      File(abs('a.bin')).deleteSync();
      expect(await service.checkSafeFreeUp('a.bin', 'fid1'), 'not_synced');
    });

    test('引擎未注入 → not_synced', () async {
      final noGate =
          FreeUpService(mount: mount, db: DatabaseService.instance);
      expect(await noGate.checkSafeFreeUp('a.bin', 'fid1'), 'not_synced');
    });
  });

  group('FreeUpService.listFreeableInFolder', () {
    Future<void> insertRow(String fileId, String localPath,
        {bool isFolder = false,
        SyncItemStatus status = SyncItemStatus.Synced,
        int localSize = 10}) async {
      final db = await DatabaseService.instance.database;
      await db.insert('sync_items', {
        'file_id': fileId,
        'local_path': localPath,
        'name': localPath.split('/').last,
        'is_folder': isFolder ? 1 : 0,
        'size': localSize,
        'local_size': localSize,
        'status': status.code,
      });
    }

    test('根目录枚举全部已同步非目录', () async {
      await insertRow('f1', 'a.txt');
      await insertRow('f2', 'docs/b.txt');
      await insertRow('f3', 'docs', isFolder: true);
      await insertRow('f4', 'c.txt', status: SyncItemStatus.CloudOnly);

      final items = await service.listFreeableInFolder('');
      expect(items.map((e) => e.fileId), containsAll(['f1', 'f2']));
      expect(items, hasLength(2));
    });

    test('子目录前缀带分隔符边界（docs 不误配 docs-backup）', () async {
      await insertRow('f1', 'docs/a.txt');
      await insertRow('f2', 'docs/sub/b.txt');
      await insertRow('f3', 'docs-backup/c.txt');
      await insertRow('f4', 'root.txt');

      final items = await service.listFreeableInFolder('docs');
      expect(items.map((e) => e.fileId), containsAll(['f1', 'f2']));
      expect(items, hasLength(2));
    });
  });

  group('FreeUpService.freeUpBatch', () {
    test('成功与跳过混合统计', () async {
      await seedSyncedFile();
      // 第二项无基线 → 跳过
      final result = await service.freeUpBatch([
        const FreeableItem(
            fileId: 'fid1', relPath: 'a.bin', name: 'a.bin', size: 100),
        const FreeableItem(
            fileId: 'ghost', relPath: 'g.bin', name: 'g.bin', size: 5),
      ]);
      expect(result.freedCount, 1);
      expect(result.skippedCount, 1);
      expect(result.freedBytes, 100);
      expect(result.errors, hasLength(1));
      expect(result.errors.single, contains('g.bin'));
    });

    test('全部成功', () async {
      await seedSyncedFile();
      await seedSyncedFile(
          relPath: 'b.bin', fileId: 'fid2', size: 50, cloudEditedTime: 222);
      final result = await service.freeUpBatch([
        const FreeableItem(
            fileId: 'fid1', relPath: 'a.bin', name: 'a.bin', size: 100),
        const FreeableItem(
            fileId: 'fid2', relPath: 'b.bin', name: 'b.bin', size: 50),
      ]);
      expect(result.freedCount, 2);
      expect(result.freedBytes, 150);
      expect(result.errors, isEmpty);
    });
  });
}
