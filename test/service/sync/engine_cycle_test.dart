import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/baseline_store.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/executor.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/service/sync/status_aggregator.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

import '../auth/fake_http.dart';
import '../drive/drive_test_util.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';

import '../mount/proc_inode.dart';
import '../mount/proc_xattr.dart';

/// 引擎周期测试（对齐 Rust engine/cycle.rs + cache.rs）：
/// 启动恢复（BFS + 占位）→ watcher/poll 触发 → 防重入合并 →
/// 未信 checkpoint 禁删 → 连续 300 次增量强制全量。

/// 可控云端 fake（list/getStartCursor/changes 计数与故障注入）。
class _CloudFake {
  /// parentId → 子项 JSON
  final Map<String, List<Map<String, dynamic>>> children = {};

  int listCalls = 0;
  int getStartCursorCalls = 0;
  int changesCalls = 0;
  String cursor = 'c0';
  List<Map<String, dynamic>> pendingChanges = [];
  bool failList = false;
  bool failChanges = false;
  Completer<void>? listGate;

  FakeHttpAdapter adapter() {
    return FakeHttpAdapter((request) async {
      final path = request.uri.path;
      if (path.endsWith('/changes/getStartCursor')) {
        getStartCursorCalls++;
        return jsonResponse(
            {'category': 'drive#startCursor', 'startCursor': cursor});
      }
      if (path.endsWith('/changes')) {
        changesCalls++;
        if (failChanges) return jsonResponse({'error': 'boom'}, status: 500);
        final next = '${cursor}n';
        final changes = pendingChanges;
        pendingChanges = [];
        cursor = next;
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': changes,
          'newStartCursor': next,
        });
      }
      if (path.endsWith('/files')) {
        listCalls++;
        if (failList) return jsonResponse({'error': 'boom'}, status: 500);
        final gate = listGate;
        if (gate != null) await gate.future;
        final qp = request.uri.queryParameters['queryParam'] ?? '';
        final match =
            RegExp("'([^']+)' in parentFolder").firstMatch(qp);
        final parent = match?.group(1) ?? 'root';
        return jsonResponse(fileListPageJson(children[parent] ?? const []));
      }
      if (path.contains('/files/') && request.method == 'PATCH') {
        // 云端 rename/move：更新 children 并返回更新后的 File
        final id = path.split('/files/').last;
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        final newName = body['fileName'] as String?;
        final newParent =
            (body['parentFolder'] as List?)?.cast<String>();
        for (final entry in children.entries) {
          final idx = entry.value.indexWhere((f) => f['id'] == id);
          if (idx < 0) continue;
          final f = Map<String, dynamic>.of(entry.value[idx]);
          entry.value.removeAt(idx);
          if (newName != null) f['fileName'] = newName;
          if (newParent != null) f['parentFolder'] = newParent;
          children
              .putIfAbsent((f['parentFolder'] as List).first as String,
                  () => [])
              .add(f);
          return jsonResponse(f);
        }
        return jsonResponse(
            {'error': {'code': 'notFound'}}, status: 404);
      }
      if (path.contains('/files/')) {
        // GET /files/{id}：存在返回 File，否则 404（verifyDeleted → true）
        final id = path.split('/files/').last;
        for (final entry in children.entries) {
          for (final f in entry.value) {
            if (f['id'] == id) return jsonResponse(f);
          }
        }
        return jsonResponse(
            {'error': {'code': 'notFound'}}, status: 404);
      }
      throw StateError('未处理请求: ${request.uri}');
    });
  }
}

class _FakeOps extends TaskOperations {
  int executeCalls = 0;

  @override
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    executeCalls++;
    if (task.operation == TransferOperation.download ||
        task.operation == TransferOperation.downloadUpdate) {
      // 模拟下载落盘（供基线结算 stat）
      await File(task.localPath!).writeAsString('cloud');
    }
    return TaskExecutionOutcome(
      cloudFile: DriveFile(
        id: task.fileId ?? 'cloud-${task.name}',
        name: task.name,
        size: task.sourceSize ?? task.totalSize,
        parentFolder:
            task.parentFileId != null ? [task.parentFileId!] : null,
        editedTime:
            DateTime.fromMillisecondsSinceEpoch(1700000000000, isUtc: true),
      ),
    );
  }
}

class _Fixture {
  late Directory mountDir;
  late Directory supportRoot;
  late _CloudFake cloud;
  late FilesService filesService;
  late ChangesService changesService;
  late TaskRunner runner;
  late _FakeOps ops;
  late SyncEngine engine;
  late bool online;
  late int nowMs;

  Future<void> tearDown() async {
    await engine.shutdownSync();
    await runner.dispose();
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    AppPaths.debugSupportRoot = null;
    if (mountDir.existsSync()) mountDir.deleteSync(recursive: true);
    if (supportRoot.existsSync()) supportRoot.deleteSync(recursive: true);
  }
}

Future<_Fixture> _buildEngine({
  Duration pollInterval = Duration.zero,
}) async {
  final f = _Fixture();
  f.mountDir = Directory.systemTemp.createTempSync('engine_cycle_mount');
  f.supportRoot = Directory.systemTemp.createTempSync('engine_cycle_support');
  AppPaths.debugSupportRoot = f.supportRoot.path;
  DatabaseService.debugDatabasePath = '${f.supportRoot.path}/petal_link.db';
  f.cloud = _CloudFake();
  final client = buildTestClient(f.cloud.adapter());
  f.filesService = FilesService(client);
  f.changesService = ChangesService(client);
  f.online = true;
  f.nowMs = 1700000000000;
  f.ops = _FakeOps();
  final transferService = TransferService(DatabaseService.instance);
  f.runner = TaskRunner(
    transferService: transferService,
    operations: f.ops,
    isOnline: () => f.online,
    nowMs: () => f.nowMs,
    mountRootProvider: () => f.mountDir.path,
  );
  final mount = MountManager(
    f.mountDir.path,
    xattr: ProcXattrService(),
    db: DatabaseService.instance,
    inodeBatchProvider: procInodeBatch,
  );
  final baselineStore = SyncBaselineStore(
    db: DatabaseService.instance,
    mountProvider: () => mount,
    nowMs: () => f.nowMs,
  );
  f.runner.setSyncHooks(baselineStore);
  f.engine = SyncEngine(
    filesApi: f.filesService,
    changesApi: f.changesService,
    db: DatabaseService.instance,
    statusAggregator: StatusAggregator.independent(),
    baselineStore: baselineStore,
    debounce: const Duration(milliseconds: 50),
    pollInterval: pollInterval,
    onlineCheck: () => f.online,
    nowMs: () => f.nowMs,
  );
  f.engine.setMount(mount);
  f.engine.setExecutor(
    SyncExecutor(
      filesApi: f.filesService,
      uploadApi: UploadService(client),
      downloadApi: DownloadService(client),
      db: DatabaseService.instance,
      mount: mount,
      taskRunner: f.runner,
      concurrencyProvider: () async => 4,
      beginActivity: f.engine.activity.begin,
    ),
    f.runner,
  );
  await f.runner.start();
  return f;
}

Future<List<({String path, SyncItemStatus status})>> items() async {
  final db = await DatabaseService.instance.database;
  final rows = await db.query('sync_items');
  return [
    for (final r in rows)
      (
        path: r['local_path'] as String,
        status: SyncItemStatus.fromCode(r['status'] as int? ?? 0)!,
      ),
  ];
}

void main() {
  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  test('启动周期：无缓存全量 BFS → 云端新文件创建占位 + CLOUD_ONLY 基线', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = [
        folderJson(id: 'd1', name: 'docs', parentFolder: ['root']),
        fileJson(
            id: 'f1',
            name: 'a.txt',
            parentFolder: ['root'],
            editedTime: '2023-11-14T22:13:20.000Z'),
      ];
      f.cloud.children['d1'] = [
        fileJson(
            id: 'f2',
            name: 'b.txt',
            parentFolder: ['d1'],
            editedTime: '2023-11-14T22:13:20.000Z'),
      ];
      final phases = <String?>[];
      final sub = f.engine.stateReceiver().listen((s) {
        phases.add(s.syncPhase?.wireName);
      });
      await f.engine.start();
      await sub.cancel();

      // 云树可信 + checkpoint 已持久化
      expect(f.engine.cloudTreeIsTrusted(), isTrue);
      // 占位符已创建
      final aStat = await FileStat.stat(p.join(f.mountDir.path, 'a.txt'));
      expect(aStat.type, FileSystemEntityType.file);
      final bStat = await FileStat.stat(p.join(f.mountDir.path, 'docs', 'b.txt'));
      expect(bStat.type, FileSystemEntityType.file);
      // 基线：文件 CLOUD_ONLY（占位）；目录 SYNCED（CreateFolder 口径）
      final rows = await items();
      expect(rows.length, 3);
      final byPath = {for (final r in rows) r.path: r.status};
      expect(byPath['docs'], SyncItemStatus.synced);
      expect(byPath['a.txt'], SyncItemStatus.cloudOnly);
      expect(byPath['docs/b.txt'], SyncItemStatus.cloudOnly);
      // 经历了启动索引阶段
      expect(phases, contains('indexing-startup'));
    } finally {
      await f.tearDown();
    }
  });

  test('watcher 触发：本地新文件 → 上传并结算 SYNCED 基线', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = const [];
      await f.engine.start();
      // 本地新建文件，触发 local-watcher 周期
      await File(p.join(f.mountDir.path, 'new.txt')).writeAsString('hello');
      f.engine.requestCycleBackground('local-watcher');
      // 等待基线收敛
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (true) {
        final rows = await items();
        if (rows.any((r) =>
            r.path == 'new.txt' && r.status == SyncItemStatus.synced)) {
          break;
        }
        if (DateTime.now().isAfter(deadline)) {
          fail('new.txt 未在超时内同步');
        }
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      expect(f.ops.executeCalls, greaterThanOrEqualTo(1));
    } finally {
      await f.tearDown();
    }
  });

  test('poll 触发：定时云端刷新（增量 changes 被周期调用）', () async {
    final f = await _buildEngine(pollInterval: const Duration(milliseconds: 150));
    try {
      f.cloud.children['root'] = const [];
      await f.engine.start();
      final before = f.cloud.changesCalls;
      await Future<void>.delayed(const Duration(milliseconds: 500));
      expect(f.cloud.changesCalls, greaterThanOrEqualTo(before + 2));
    } finally {
      await f.tearDown();
    }
  });

  test('防重入：并发触发合并为单次全量 BFS', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = const [];
      await f.engine.start();
      final listBefore = f.cloud.listCalls;
      f.cloud.listGate = Completer<void>();
      // 两个触发源并发（manual-full + auto-incremental）
      final f1 = f.engine.runSyncCycle('manual-refresh');
      await Future<void>.delayed(const Duration(milliseconds: 50));
      final f2 = f.engine.runSyncCycle('auto-cloud-refresh');
      await Future<void>.delayed(const Duration(milliseconds: 50));
      f.cloud.listGate!.complete();
      f.cloud.listGate = null;
      await f1;
      await f2;
      // 合并后仅一次全量 BFS（root 列表调用 +1）
      expect(f.cloud.listCalls, listBefore + 1);
    } finally {
      await f.tearDown();
    }
  });

  test('未信 checkpoint 禁删：云端刷新失败撤销信任，本地文件不删不传', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = [
        fileJson(
            id: 'f1',
            name: 'keep.txt',
            parentFolder: ['root'],
            editedTime: '2023-11-14T22:13:20.000Z'),
      ];
      await f.engine.start();
      expect(f.engine.cloudTreeIsTrusted(), isTrue);
      // 云端删除 + 全部端点故障 → 增量失败回退全量也失败 → 不可信
      f.cloud.children['root'] = const [];
      f.cloud.failChanges = true;
      f.cloud.failList = true;
      await expectLater(
        f.engine.runSyncCycle('auto-cloud-refresh'),
        throwsA(isA<AppError>()),
      );
      expect(f.engine.cloudTreeIsTrusted(), isFalse);
      // 本地占位/基线必须在（未信禁删）
      final aStat =
          await FileStat.stat(p.join(f.mountDir.path, 'keep.txt'));
      expect(aStat.type, FileSystemEntityType.file);
      expect(await items(), isNotEmpty);
      // 恢复端点 → 增量回放 Removed → 可信后正常收敛（云端已删 → 删除本地占位）
      f.cloud.failChanges = false;
      f.cloud.failList = false;
      f.cloud.pendingChanges = [
        {
          'category': 'drive#change',
          'type': 'File',
          'fileId': 'f1',
          'deleted': true,
        },
      ];
      await f.engine.runSyncCycle('auto-cloud-refresh');
      expect(f.engine.cloudTreeIsTrusted(), isTrue);
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (true) {
        final type = await FileSystemEntity.type(
            p.join(f.mountDir.path, 'keep.txt'),
            followLinks: false);
        if (type == FileSystemEntityType.notFound) break;
        if (DateTime.now().isAfter(deadline)) {
          fail('可信后云端删除未收敛到本地');
        }
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    } finally {
      await f.tearDown();
    }
  });

  test('连续 300 次增量强制全量 BFS', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = const [];
      await f.engine.start();
      // 常规增量：getStartCursor 不再调用
      final startBefore = f.cloud.getStartCursorCalls;
      await f.engine.runSyncCycle('auto-cloud-refresh');
      expect(f.cloud.getStartCursorCalls, startBefore);
      expect(f.engine.cloudIndex.incrementalSinceFull, 1);
      // 达到阈值 → 强制全量
      f.engine.cloudIndex.incrementalSinceFull = 300;
      await f.engine.runSyncCycle('auto-cloud-refresh');
      expect(f.cloud.getStartCursorCalls, startBefore + 1);
      expect(f.engine.cloudIndex.incrementalSinceFull, 0);
    } finally {
      await f.tearDown();
    }
  });

  test('增量回放：Modified 新增云端文件 → 下一周期创建占位', () async {
    final f = await _buildEngine();
    try {
      f.cloud.children['root'] = const [];
      await f.engine.start();
      // 服务端新增文件（changes 增量 + list 结果同步更新供全量兜底）
      f.cloud.children['root'] = [
        fileJson(
            id: 'f9',
            name: 'inc.txt',
            parentFolder: ['root'],
            editedTime: '2023-11-14T22:13:20.000Z'),
      ];
      f.cloud.pendingChanges = [
        {
          'category': 'drive#change',
          'type': 'File',
          'fileId': 'f9',
          'deleted': false,
          'file': fileJson(
              id: 'f9',
              name: 'inc.txt',
              parentFolder: ['root'],
              editedTime: '2023-11-14T22:13:20.000Z'),
        },
      ];
      await f.engine.runSyncCycle('auto-cloud-refresh');
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (true) {
        final rows = await items();
        if (rows.any((r) =>
            r.path == 'inc.txt' && r.status == SyncItemStatus.cloudOnly)) {
          break;
        }
        if (DateTime.now().isAfter(deadline)) {
          fail('增量新增文件未创建占位');
        }
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    } finally {
      await f.tearDown();
    }
  });

  test('retryTransfer 成功后回插 live 云树（对齐 Rust retry_transfer）', () async {
    final f = await _buildEngine();
    try {
      await f.engine.start();
      // 造一个 Failed 上传任务（真实源文件满足静态校验）
      final rel = 'retry.bin';
      final file = File(p.join(f.mountDir.path, rel));
      await file.writeAsBytes(List<int>.filled(64, 7), flush: true);
      final stat = await file.stat();
      final transferService = TransferService(DatabaseService.instance);
      final enqueued = (await transferService.enqueue(TransferTask(
        direction: TransferDirection.upload,
        localPath: file.path,
        name: rel,
        totalSize: stat.size,
        relativePath: rel,
        operation: TransferOperation.create,
        sourceMtime: stat.modified.millisecondsSinceEpoch,
        sourceSize: stat.size,
        createdAt: 1,
      )))
          .unwrap();
      final db = await DatabaseService.instance.database;
      await db.update(
        'transfer_queue',
        {
          'state': TransferState.failed.code,
          'state_revision': enqueued.stateRevision + 1,
        },
        where: 'id = ?',
        whereArgs: [enqueued.id],
      );

      await f.engine.retryTransfer(enqueued.id);

      // 后台执行收敛到 Completed
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (true) {
        final rows = await db
            .query('transfer_queue', where: 'id = ?', whereArgs: [enqueued.id]);
        if (rows.first['state'] == TransferState.completed.code) break;
        if (DateTime.now().isAfter(deadline)) fail('重试任务未收敛到 Completed');
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      // 关键断言：成功的 cloudFile 必须回插 live 云树 + pathToId
      // （修复前 runner.retry fire-and-forget，云树无回插）
      expect(f.engine.cloudIndex.tree[rel], isNotNull);
      expect(f.engine.cloudIndex.pathToId[rel],
          f.engine.cloudIndex.tree[rel]!.id);
    } finally {
      await f.tearDown();
    }
  });

  test('scanLocal 后写入 inode 映射并清理陈旧记录（docs/design/10 阶段1）',
      () async {
    final f = await _buildEngine();
    try {
      final identity = MemoryInodeIdentityStore();
      f.engine.identity = identity;
      // 本地文件 + DB 基线
      final file = File(p.join(f.mountDir.path, 'a.txt'));
      await file.writeAsString('hello');
      final db = await DatabaseService.instance.database;
      await db.insert('sync_items', {
        'file_id': 'fid-a',
        'local_path': 'a.txt',
        'name': 'a.txt',
        'is_folder': 0,
        'size': 5,
        'status': 0,
      });

      await f.engine.scanLocal();

      // 有基线的文件 → 映射已写入（fake provider 由 fixture mount 注入）
      final all = identity.debugAll;
      expect(all, isNotEmpty);
      expect(all.values.any((r) => r.relativePath == 'a.txt' && r.fileId == 'fid-a'),
          isTrue);

      // 文件删除后重扫 → 陈旧记录被 purge
      await file.delete();
      await f.engine.scanLocal();
      expect(
          identity.debugAll.values.any((r) => r.relativePath == 'a.txt'),
          isFalse);
    } finally {
      await f.tearDown();
    }
  });

  test('本地 mv 已同步文件 → inode 检测为 MoveInCloud（非删+传）', () async {
    final f = await _buildEngine();
    try {
      f.engine.identity = MemoryInodeIdentityStore();
      f.cloud.children['root'] = [
        fileJson(
            id: 'f1',
            name: 'a.txt',
            parentFolder: ['root'],
            editedTime: '2023-11-14T22:13:20.000Z'),
      ];
      await f.engine.start();
      // 等占位创建完成（a.txt 落基线 CloudOnly）
      final db = await DatabaseService.instance.database;
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (true) {
        final rows = await items();
        if (rows.any(
            (r) => r.path == 'a.txt' && r.status == SyncItemStatus.cloudOnly)) {
          break;
        }
        if (DateTime.now().isAfter(deadline)) fail('占位 a.txt 未落基线');
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      // 填充 inode 映射（占位 a.txt inode → f1）
      await f.engine.scanLocal();

      // 用户 mv 占位符
      await File(p.join(f.mountDir.path, 'a.txt'))
          .rename(p.join(f.mountDir.path, 'b.txt'));
      await f.engine.runSyncCycle('local-watcher');

      // 基线重键到 b.txt（fileId 不变），旧路径行消失
      final rows = await items();
      expect(rows.any((r) => r.path == 'b.txt' && r.status != SyncItemStatus.failed),
          isTrue);
      expect(rows.any((r) => r.path == 'a.txt'), isFalse);
      // live 云树同步更新
      expect(f.engine.cloudIndex.tree['b.txt'], isNotNull);
      expect(f.engine.cloudIndex.tree['a.txt'], isNull);
      // 远端执行了 rename（PATCH），而不是上传
      expect(f.cloud.children['root']!.any((c) => c['fileName'] == 'b.txt'),
          isTrue);
      final uploads = await db.query('transfer_queue',
          where: 'direction = ?', whereArgs: [TransferDirection.upload.code]);
      expect(uploads, isEmpty);
    } finally {
      await f.tearDown();
    }
  });

  test('本地 cp 已同步文件 → 副本作为新文件上传（新 inode）', () async {
    final f = await _buildEngine();
    try {
      f.engine.identity = MemoryInodeIdentityStore();
      // 云端 + 本地 + 基线三方一致的已同步真实文件
      const edited = '2023-11-14T22:13:20.000Z';
      final editedMs = DateTime.parse(edited).toUtc().millisecondsSinceEpoch;
      f.cloud.children['root'] = [
        fileJson(
            id: 'f1',
            name: 'a.txt',
            size: 5,
            parentFolder: ['root'],
            editedTime: edited),
      ];
      final aFile = File(p.join(f.mountDir.path, 'a.txt'));
      await aFile.writeAsString('hello', flush: true);
      final aStat = await aFile.stat();
      final db = await DatabaseService.instance.database;
      await db.insert('sync_items', {
        'file_id': 'f1',
        'local_path': 'a.txt',
        'name': 'a.txt',
        'is_folder': 0,
        'size': 5,
        'local_size': 5,
        'local_mtime': aStat.modified.millisecondsSinceEpoch,
        'cloud_edited_time': editedMs,
        'status': 0,
      });
      await f.engine.start();
      await f.engine.scanLocal();

      // 用户 cp 真实文件（副本 xattr 跟随但 inode 是新的）
      await aFile.copy(p.join(f.mountDir.path, 'c.txt'));
      await f.engine.runSyncCycle('local-watcher');

      // 原件不受影响
      final rows = await items();
      expect(rows.any((r) => r.path == 'a.txt'), isTrue);
      // 副本进入上传队列（作为新文件），未被误判为移动
      final uploads = await db.query('transfer_queue',
          where: 'direction = ?', whereArgs: [TransferDirection.upload.code]);
      expect(uploads.any((u) => u['relative_path'] == 'c.txt'), isTrue);
      expect(f.engine.cloudIndex.tree['c.txt'], isNull);
    } finally {
      await f.tearDown();
    }
  });

  test('applyResults 内存发布优先 result.cloudFile（对齐 Rust results.rs）',
      () async {
    final f = await _buildEngine();
    try {
      await Directory(p.join(f.mountDir.path, 'newdir')).create(recursive: true);
      final resultFile = DriveFile(
        id: 'new-folder-id',
        name: 'newdir',
        size: 0,
        parentFolder: ['root'],
        editedTime:
            DateTime.fromMillisecondsSinceEpoch(1700000000000, isUtc: true),
      );
      final action = SyncAction(
        actionType: SyncActionType.createFolder,
        relativePath: 'newdir',
      );
      await f.engine.applyResults(
          [action], [ActionResult.ok(cloudFile: resultFile)]);

      // 修复前内存发布只看 cloudIndex.tree / action.cloudFile，
      // 丢失 result.cloudFile → live 云树缺条目
      expect(f.engine.cloudIndex.tree['newdir']?.id, 'new-folder-id');
      expect(f.engine.cloudIndex.pathToId['newdir'], 'new-folder-id');
    } finally {
      await f.tearDown();
    }
  });
}
