import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// 创建 Rust v1 结构的数据库（对齐 Rust migrations.rs 的 v1 onCreate 终态）。
Future<void> _createRustV1Database(String path) async {
  final db = await databaseFactoryFfi.openDatabase(
    path,
    options: OpenDatabaseOptions(
      version: 1,
      onCreate: (db, version) async {
        // Rust v1 sync_items（无 local_size）
        await db.execute('''
          CREATE TABLE sync_items (
              file_id           TEXT    NOT NULL,
              local_path        TEXT    NOT NULL,
              parent_folder_id  TEXT,
              name              TEXT    NOT NULL,
              is_folder         INTEGER NOT NULL DEFAULT 0,
              size              INTEGER NOT NULL DEFAULT 0,
              sha256            TEXT,
              local_mtime       INTEGER,
              cloud_edited_time INTEGER,
              last_sync_time    INTEGER,
              status            INTEGER NOT NULL DEFAULT 0,
              error_message     TEXT,
              PRIMARY KEY (file_id, local_path)
          )
        ''');
        // Rust v1 transfer_queue（无 v2-v5 新增列）
        await db.execute('''
          CREATE TABLE transfer_queue (
              id            INTEGER PRIMARY KEY AUTOINCREMENT,
              direction     INTEGER NOT NULL,
              file_id       TEXT,
              local_path    TEXT,
              name          TEXT    NOT NULL,
              total_size    INTEGER NOT NULL DEFAULT 0,
              transferred   INTEGER NOT NULL DEFAULT 0,
              state         INTEGER NOT NULL DEFAULT 0,
              error_message TEXT,
              created_at    INTEGER NOT NULL,
              finished_at   INTEGER
          )
        ''');
        await db.execute(
            'CREATE INDEX idx_sync_items_file_id ON sync_items(file_id)');
        await db.execute(
            'CREATE INDEX idx_sync_items_status ON sync_items(status)');
        await db.execute(
            'CREATE INDEX idx_transfer_state ON transfer_queue(state)');
      },
    ),
  );
  await db.close();
}

void main() {
  late Directory tempDir;
  late String dbPath;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_db_test');
    dbPath = '${tempDir.path}/petal_link.db';
    DatabaseService.debugDatabasePath = dbPath;
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  /// 读取表的全部列名。
  Future<Set<String>> columnNames(String table) async {
    final db = await DatabaseService.instance.database;
    final rows = await db.rawQuery('PRAGMA table_info($table)');
    return rows.map((r) => r['name'] as String).toSet();
  }

  group('DatabaseService 新库建表（schema v5 终态）', () {
    test('user_version 为 7', () async {
      final db = await DatabaseService.instance.database;
      final rows = await db.rawQuery('PRAGMA user_version');
      expect(rows.first.values.first, 7);
    });

    test('v6 新增 local_inode_map 与 free_up_staging 表（inode 方案）', () async {
      final db = await DatabaseService.instance.database;
      final tables = (await db.rawQuery(
              "SELECT name FROM sqlite_master WHERE type='table'"))
          .map((r) => r['name'] as String)
          .toSet();
      expect(tables, contains('local_inode_map'));
      expect(tables, contains('free_up_staging'));
      // local_inode_map：inode 单列主键 + 两个二级索引
      final indexes = (await db.rawQuery(
              "SELECT name FROM sqlite_master WHERE type='index'"))
          .map((r) => r['name'] as String)
          .toSet();
      expect(indexes, contains('idx_inode_map_path'));
      expect(indexes, contains('idx_inode_map_fid'));
    });

    test('sync_items 列逐字段对齐 Rust', () async {
      final columns = await columnNames('sync_items');
      expect(columns, {
        'file_id',
        'local_path',
        'parent_folder_id',
        'name',
        'is_folder',
        'size',
        'local_size',
        'sha256',
        'local_mtime',
        'cloud_edited_time',
        'last_sync_time',
        'status',
        'error_message',
      });
    });

    test('transfer_queue 列逐字段对齐 Rust（9 态状态机）', () async {
      final columns = await columnNames('transfer_queue');
      expect(columns, {
        'id',
        'direction',
        'file_id',
        'local_path',
        'name',
        'total_size',
        'transferred',
        'state',
        'error_message',
        'created_at',
        'finished_at',
        'server_id',
        'upload_id',
        'resume_offset',
        'session_url',
        'relative_path',
        'parent_file_id',
        'operation',
        'source_mtime',
        'source_size',
        'expected_cloud_edited_time',
        'attempt_count',
        'next_retry_at',
        'error_kind',
        'remote_result_file_id',
        'state_revision',
      });
    });

    test('sync_items 复合主键 (file_id, local_path)', () async {
      final db = await DatabaseService.instance.database;
      await db.insert('sync_items', {
        'file_id': 'f1',
        'local_path': '/a/b',
        'name': 'b',
      });
      // 相同 file_id + 不同 local_path：允许
      await db.insert('sync_items', {
        'file_id': 'f1',
        'local_path': '/a/c',
        'name': 'c',
      });
      // 完全相同的复合主键：冲突
      expect(
        () => db.insert('sync_items', {
          'file_id': 'f1',
          'local_path': '/a/b',
          'name': 'b2',
        }),
        throwsA(anything),
      );
    });

    test('全部索引已创建', () async {
      final db = await DatabaseService.instance.database;
      final rows = await db.rawQuery(
        "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
      );
      final names = rows.map((r) => r['name'] as String).toSet();
      expect(names, containsAll([
        'idx_sync_items_file_id',
        'idx_sync_items_status',
        'idx_transfer_state',
        'idx_transfer_state_retry',
        'idx_transfer_relative_state',
      ]));
    });

    test('幻影 v6 旧库（sync_cursor 无 config/inode）→ v7 自愈补齐',
        () async {
      // 开发期另一套 v6 schema：与 inode v6 版本号相撞，
      // 不触发 onUpgrade 时永远缺 config/inode 表（2026-07-21 启动事故）
      final old = await databaseFactoryFfi.openDatabase(dbPath,
          options: OpenDatabaseOptions(version: 6, onCreate: (db, v) async {
        await db.execute(
            'CREATE TABLE sync_items (file_id TEXT NOT NULL, local_path TEXT NOT NULL, name TEXT NOT NULL DEFAULT "", PRIMARY KEY (file_id, local_path))');
        await db.execute(
            'CREATE TABLE transfer_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)');
        await db.execute(
            'CREATE TABLE sync_cursor (id INTEGER PRIMARY KEY, cursor TEXT)');
      }));
      await old.close();

      final db = await DatabaseService.instance.database;

      final version = await db.rawQuery('PRAGMA user_version');
      expect(version.first.values.first, 7);
      final tables = (await db.rawQuery(
              "SELECT name FROM sqlite_master WHERE type='table'"))
          .map((r) => r['name'] as String)
          .toSet();
      expect(tables, containsAll(['config', 'local_inode_map', 'free_up_staging']));
      // 旧数据保留
      expect(tables, contains('sync_cursor'));
    });

    test('config 键值表可用（对应 Rust config.json）', () async {
      final db = await DatabaseService.instance.database;
      await db.insert('config', {'key': 'k1', 'value': 'v1'});
      final rows = await db
          .query('config', where: 'key = ?', whereArgs: ['k1']);
      expect(rows.single['value'], 'v1');
    });

    test('重复打开返回同一实例', () async {
      final db1 = await DatabaseService.instance.database;
      final db2 = await DatabaseService.instance.database;
      expect(identical(db1, db2), isTrue);
    });
  });

  group('DatabaseService 迁移（Rust v1 → v6）', () {
    test('分步升级补齐全部列与索引', () async {
      await _createRustV1Database(dbPath);

      // 触发懒初始化（onUpgrade 1 → 7）
      final db = await DatabaseService.instance.database;

      final version = await db.rawQuery('PRAGMA user_version');
      expect(version.first.values.first, 7);

      // v6 两张 inode 表随升级补齐
      final tables = (await db.rawQuery(
              "SELECT name FROM sqlite_master WHERE type='table'"))
          .map((r) => r['name'] as String)
          .toSet();
      expect(tables, contains('local_inode_map'));
      expect(tables, contains('free_up_staging'));

      final tq = await columnNames('transfer_queue');
      expect(
        tq,
        containsAll([
          'server_id',
          'upload_id',
          'resume_offset',
          'session_url',
          'relative_path',
          'operation',
          'attempt_count',
          'next_retry_at',
          'error_kind',
          'state_revision',
        ]),
      );

      final si = await columnNames('sync_items');
      expect(si, contains('local_size'));

      final indexes = await db.rawQuery(
        "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
      );
      final names = indexes.map((r) => r['name'] as String).toSet();
      expect(names, containsAll([
        'idx_transfer_state_retry',
        'idx_transfer_relative_state',
      ]));
    });

    test('旧状态值归一化到新 9 态数值', () async {
      await _createRustV1Database(dbPath);

      // 插入旧状态任务：0 PENDING / 1 RUNNING / 3 COMPLETED / 4 FAILED / 5 CANCELED
      // 注意：v1 结构插入，created_at 必填
      final raw = await databaseFactoryFfi.openDatabase(dbPath);
      for (final state in [0, 1, 3, 4, 5]) {
        await raw.insert('transfer_queue', {
          'direction': 0,
          'name': 'task_state_$state',
          'state': state,
          'created_at': 1000,
        });
      }
      await raw.close();

      final db = await DatabaseService.instance.database;
      final rows = await db.query('transfer_queue', orderBy: 'id ASC');
      expect(rows, hasLength(5));

      // 旧 0/1（活动任务）：mount_root 缺失无法恢复相对路径 → Failed(7) + Validation(7)
      expect(rows[0]['state'], 7);
      expect(rows[0]['error_kind'], 7);
      expect(rows[0]['error_message'], contains('无法安全恢复'));
      expect(rows[1]['state'], 7);
      expect(rows[1]['error_kind'], 7);

      // 旧 3 → Completed(6)；旧 4 → Failed(7) + Unknown(11)；旧 5 → Canceled(8)
      expect(rows[2]['state'], 6);
      expect(rows[3]['state'], 7);
      expect(rows[3]['error_kind'], 11);
      expect(rows[4]['state'], 8);
    });
  });
}
