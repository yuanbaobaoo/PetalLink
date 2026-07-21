import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/sync/status_aggregator.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// StatusAggregator 测试（对齐 Rust status_aggregator.rs：
/// 7 项计数、completed 口径、failed_items 上限 20、revision 单调）。

SyncItem item(
  String fileId,
  String path, {
  SyncItemStatus status = SyncItemStatus.synced,
  String? error,
}) {
  return SyncItem(
    fileId: fileId,
    localPath: path,
    name: path.split('/').last,
    status: status,
    errorMessage: error,
  );
}

TransferTask task({
  required int id,
  TransferState state = TransferState.running,
  TransferDirection direction = TransferDirection.upload,
}) {
  return TransferTask(
    id: id,
    direction: direction,
    name: 't$id',
    state: state,
    createdAt: id,
  );
}

void main() {
  late Directory dir;
  late StatusAggregator aggregator;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    dir = Directory.systemTemp.createTempSync('aggregator_test');
    DatabaseService.debugDatabasePath = '${dir.path}/petal_link.db';
    aggregator = StatusAggregator.independent();
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (dir.existsSync()) dir.deleteSync(recursive: true);
  });

  Future<void> seed({
    List<SyncItem> items = const [],
    List<TransferTask> tasks = const [],
  }) async {
    final db = await DatabaseService.instance.database;
    for (final i in items) {
      await db.insert('sync_items', i.toRow());
    }
    for (final t in tasks) {
      await db.insert('transfer_queue', t.toRow());
    }
  }

  test('空库 → 全零快照，revision 从 1 起单调', () async {
    final db = await DatabaseService.instance.database;
    final s1 = await aggregator.snapshot(db, RuntimeStatus());
    expect(s1.revision, 1);
    expect(s1.total, 0);
    expect(s1.progress, 1.0);
    final s2 = await aggregator.snapshot(db, RuntimeStatus());
    expect(s2.revision, 2);
  });

  test('7 项计数与 completed = total - failed - conflict', () async {
    await seed(
      items: [
        item('f1', 'a'),
        item('f2', 'b'),
        item('f3', 'c', status: SyncItemStatus.failed, error: 'boom'),
        item('f4', 'd', status: SyncItemStatus.conflict),
        item('f5', 'e', status: SyncItemStatus.cloudOnly),
      ],
      tasks: [
        task(id: 1, direction: TransferDirection.upload),
        task(id: 2, direction: TransferDirection.download),
        task(id: 3, direction: TransferDirection.downloadUpdate),
        task(id: 4, state: TransferState.waitingForNetwork),
        task(id: 5, state: TransferState.failed),
        task(id: 6, state: TransferState.completed),
      ],
    );
    final db = await DatabaseService.instance.database;
    final s = await aggregator.snapshot(db, RuntimeStatus());
    expect(s.total, 5);
    expect(s.failed, 1);
    expect(s.conflict, 1);
    expect(s.completed, 3); // 5 - 1 - 1（含 CLOUD_ONLY 计入完成口径）
    expect(s.uploading, 1);
    expect(s.downloading, 2); // Download + DownloadUpdate
    expect(s.waitingNetwork, 1);
    expect(s.transferFailed, 1);
    expect(s.failedItems.single.relativePath, 'c');
    expect(s.failedItems.single.errorMessage, 'boom');
  });

  test('failed_items 按路径字典序 LIMIT 20', () async {
    final items = [
      for (var i = 0; i < 25; i++)
        item('f$i', 'p${i.toString().padLeft(2, '0')}',
            status: SyncItemStatus.failed),
    ];
    // 乱序插入验证 ORDER BY
    await seed(items: items.reversed.toList());
    final db = await DatabaseService.instance.database;
    final s = await aggregator.snapshot(db, RuntimeStatus());
    expect(s.failed, 25);
    expect(s.failedItems.length, 20);
    expect(s.failedItems.first.relativePath, 'p00');
    expect(s.failedItems.last.relativePath, 'p19');
  });

  test('运行时字段注入（isIndexing/syncPhase/editing）', () async {
    final db = await DatabaseService.instance.database;
    final s = await aggregator.snapshot(
      db,
      RuntimeStatus(
        isRunning: true,
        isIndexing: true,
        indexingScannedFolders: 7,
        indexingDiscoveredItems: 42,
        editing: 2,
        syncPhase: SyncPhase.indexingStartup,
        lastSyncTime: 1700000000000,
        contentChanged: true,
      ),
    );
    expect(s.isRunning, isTrue);
    expect(s.isIndexing, isTrue);
    expect(s.indexingScannedFolders, 7);
    expect(s.indexingDiscoveredItems, 42);
    expect(s.editing, 2);
    expect(s.syncPhase, SyncPhase.indexingStartup);
    expect(s.contentChanged, isTrue);
    expect(s.lastSyncTime, 1700000000000);
  });

  test('发布锁串行化：并发快照 revision 严格递增无重复', () async {
    final db = await DatabaseService.instance.database;
    final revisions = await Future.wait([
      for (var i = 0; i < 10; i++)
        aggregator.lockPublication(() => aggregator.snapshot(db, RuntimeStatus())
            .then((s) => s.revision)),
    ]);
    final sorted = List<int>.of(revisions)..sort();
    for (var i = 1; i < sorted.length; i++) {
      expect(sorted[i], sorted[i - 1] + 1);
    }
  });
}
