import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// inode 身份映射存储测试（对标 CMP `InodeIdentityStore` 与
/// `docs/design/10` §3.1 的 local_inode_map 表）。
void main() {
  sqfliteFfiInit();
  databaseFactory = databaseFactoryFfi;

  group('SqfliteInodeIdentityStore', () {
    late SqfliteInodeIdentityStore store;
    late String dbPath;

    setUp(() async {
      final dir = Directory.systemTemp.createTempSync('inode_identity_test');
      dbPath = '${dir.path}/petal_link.db';
      DatabaseService.debugDatabasePath = dbPath;
      store = SqfliteInodeIdentityStore(DatabaseService.instance);
    });

    tearDown(() async {
      await DatabaseService.instance.close();
      DatabaseService.debugDatabasePath = null;
      final f = File(dbPath);
      if (f.existsSync()) f.deleteSync();
    });

    test('upsert + lookup：inode → (path, fileId)', () async {
      await store.upsert(1001, 'docs/a.txt', 'fid-a');
      final rec = await store.lookup(1001);
      expect(rec, isNotNull);
      expect(rec!.relativePath, 'docs/a.txt');
      expect(rec.fileId, 'fid-a');
      expect(rec.scannedAt, greaterThan(0));
    });

    test('lookup 未命中返回 null', () async {
      expect(await store.lookup(9999), isNull);
    });

    test('upsert 同 inode 覆盖（移动后路径更新）', () async {
      await store.upsert(1001, 'docs/a.txt', 'fid-a');
      await store.upsert(1001, 'docs/b.txt', 'fid-a');
      final rec = await store.lookup(1001);
      expect(rec!.relativePath, 'docs/b.txt');
    });

    test('purgeMissing 清理本轮未见的 inode', () async {
      await store.upsert(1001, 'a.txt', 'fid-a');
      await store.upsert(1002, 'b.txt', 'fid-b');
      await store.upsert(1003, 'c.txt', 'fid-c');

      await store.purgeMissing({1001, 1003});

      expect(await store.lookup(1001), isNotNull);
      expect(await store.lookup(1002), isNull);
      expect(await store.lookup(1003), isNotNull);
    });

    test('整表可安全清空重建（purgeMissing 空集）', () async {
      await store.upsert(1001, 'a.txt', 'fid-a');
      await store.purgeMissing(const {});
      expect(await store.lookup(1001), isNull);
      // 清空后可重新写入
      await store.upsert(2001, 'x.txt', 'fid-x');
      expect((await store.lookup(2001))!.fileId, 'fid-x');
    });
  });

  group('MemoryInodeIdentityStore（测试 fake，契约一致）', () {
    test('同一契约：upsert/lookup/purgeMissing', () async {
      final store = MemoryInodeIdentityStore();
      await store.upsert(1, 'a.txt', 'fid-a');
      await store.upsert(2, 'b.txt', 'fid-b');
      expect((await store.lookup(1))!.fileId, 'fid-a');
      await store.purgeMissing({2});
      expect(await store.lookup(1), isNull);
      expect(await store.lookup(2), isNotNull);
    });
  });
}
