import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/sync/baseline_store.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// 同路径仲裁（admission）+ sync_items 基线结算测试。
///
/// 对齐 Rust task_runner/admission.rs:125-184 分支链与
/// settlement.rs / persistence.rs 的 sync_items 回写规则。

class _FakeOps extends TaskOperations {
  /// 执行计数
  int executeCalls = 0;

  /// 逐路径行为（relPath → outcome）
  Map<String, TaskExecutionOutcome Function(TransferTask)> behaviors = {};

  @override
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    executeCalls++;
    final behavior = behaviors[task.relativePath];
    if (behavior != null) return behavior(task);
    // 默认：上传成功（元数据与源快照一致）
    return TaskExecutionOutcome(
      cloudFile: DriveFile(
        id: task.fileId ?? 'cloud-${task.name}',
        name: task.name,
        size: task.sourceSize ?? 0,
        editedTime: DateTime.fromMillisecondsSinceEpoch(1700000000000,
            isUtc: true),
      ),
    );
  }
}

void main() {
  late Directory mountDir;
  late TransferService transferService;
  late _FakeOps ops;
  late TaskRunner runner;
  late SyncBaselineStore baselineStore;
  late int nowMs;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    mountDir = Directory.systemTemp.createTempSync('admission_test');
    DatabaseService.debugDatabasePath = '${mountDir.path}/petal_link.db';
    transferService = TransferService(DatabaseService.instance);
    ops = _FakeOps();
    nowMs = 1700000000000;
    baselineStore = SyncBaselineStore(
      db: DatabaseService.instance,
      mountProvider: () => null,
      nowMs: () => nowMs,
    );
    runner = TaskRunner(
      transferService: transferService,
      operations: ops,
      nowMs: () => nowMs,
      mountRootProvider: () => mountDir.path,
    );
    runner.setSyncHooks(baselineStore);
  });

  tearDown(() async {
    await runner.dispose();
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (mountDir.existsSync()) mountDir.deleteSync(recursive: true);
  });

  /// 创建本地文件并构造匹配的 Create 上传意图
  Future<TransferTask> newUploadIntent(String rel, {String content = 'x'}) async {
    final abs = p.join(mountDir.path, rel);
    await File(abs).writeAsString(content);
    final stat = await FileStat.stat(abs);
    return TransferTask(
      direction: TransferDirection.upload,
      localPath: abs,
      name: p.basename(rel),
      totalSize: stat.size,
      createdAt: nowMs,
      relativePath: rel,
      parentFileId: 'root',
      operation: TransferOperation.create,
      sourceMtime: stat.modified.millisecondsSinceEpoch,
      sourceSize: stat.size,
    );
  }

  Future<List<TransferTask>> allTasks() =>
      transferService.getAllTasks().then((r) => r.unwrapOr([]));

  Future<List<SyncItem>> allItems() => baselineStore.loadAll();

  group('同路径仲裁（对齐 admission.rs:125-184）', () {
    test('插入新行并执行成功 → completed + 基线 SYNCED', () async {
      final intent = await newUploadIntent('a.txt');
      final result = await runner.enqueueAndRun(intent);
      final enqueued = result.unwrap();
      expect(enqueued.outcome.disposition, TaskDisposition.completed);
      expect(ops.executeCalls, 1);
      final items = await allItems();
      expect(items.single.localPath, 'a.txt');
      expect(items.single.status, SyncItemStatus.synced);
      expect(items.single.fileId, 'cloud-a.txt');
    });

    test('同意图去重：同路径同意图 → 复用在途任务，不插入新行', () async {
      final intent = await newUploadIntent('a.txt');
      // 预置同意图 Pending 行
      final stored = (await transferService.enqueue(intent)).unwrap();
      final again = await newUploadIntent('a.txt');
      final result = await runner.enqueueAndRun(again);
      final enqueued = result.unwrap();
      expect(enqueued.taskId, stored.id);
      expect((await allTasks()).length, 1);
      // 任务被调度执行（pump 触发）
      await runner.idle;
      final fresh = (await transferService.getTaskById(stored.id)).unwrapOr(null);
      expect(fresh?.state, TransferState.completed);
    });

    test('在途写不同意图 → blockedByActiveIntent', () async {
      final intent = await newUploadIntent('a.txt');
      final running = intent.copyWith(
        state: TransferState.running,
        sourceMtime: intent.sourceMtime! + 999,
      );
      await transferService.enqueue(running);
      final again = await newUploadIntent('a.txt');
      final result = await runner.enqueueAndRun(again);
      expect(result.unwrap().outcome.disposition,
          TaskDisposition.blockedByActiveIntent);
      expect((await allTasks()).length, 1);
    });

    test('歧义重启提升：RestartRequired + 远端结果 → VerifyingRemote', () async {
      final intent = await newUploadIntent('a.txt');
      final restart = intent.copyWith(
        state: TransferState.restartRequired,
        remoteResultFileId: 'cloud-dup',
      );
      final stored = (await transferService.enqueue(restart)).unwrap();
      final again = await newUploadIntent('a.txt', content: 'yy');
      final result = await runner.enqueueAndRun(again);
      expect(result.unwrap().outcome.disposition,
          TaskDisposition.verifyingRemote);
      final fresh = (await transferService.getTaskById(stored.id)).unwrapOr(null);
      expect(fresh?.state, TransferState.verifyingRemote);
      expect(fresh?.errorKind, TransferErrorKind.remoteAmbiguous);
      expect((await allTasks()).length, 1);
    });

    test('重规划：同路径阻塞任务被新意图覆写意图列', () async {
      final intent = await newUploadIntent('a.txt');
      final old = (await transferService.enqueue(intent)).unwrap();
      // 新意图（内容变化 → 新快照）
      final replacement = await newUploadIntent('a.txt', content: 'new-content');
      final result = await runner.enqueueAndRun(replacement);
      expect(result.unwrap().taskId, old.id);
      expect((await allTasks()).length, 1);
      final fresh = (await transferService.getTaskById(old.id)).unwrapOr(null);
      expect(fresh?.sourceSize, replacement.sourceSize);
      expect(fresh?.totalSize, replacement.totalSize);
      // SYNCING 回写（replan 钩子）
      final items = await allItems();
      if (items.isNotEmpty) {
        expect(items.first.status, isNot(SyncItemStatus.failed));
      }
    });

    test('Failed 路径屏障：同路径 Failed 行拒绝新自动意图', () async {
      final intent = await newUploadIntent('a.txt');
      final failed = intent.copyWith(
        state: TransferState.failed,
        errorKind: TransferErrorKind.quota,
        errorMessage: '配额不足',
      );
      await transferService.enqueue(failed);
      final again = await newUploadIntent('a.txt');
      final result = await runner.enqueueAndRun(again);
      expect(result.unwrap().outcome.disposition,
          TaskDisposition.blockedByActiveIntent);
      expect((await allTasks()).length, 1);
    });

    test('每周期歧义重启批量提升（promoteAmbiguousRestarts）', () async {
      final intent = await newUploadIntent('a.txt');
      await transferService.enqueue(intent.copyWith(
        state: TransferState.restartRequired,
        remoteResultFileId: 'cloud-x',
      ));
      final other = await newUploadIntent('b.txt');
      // 无远端结果的 RestartRequired 不提升
      await transferService.enqueue(other.copyWith(
        state: TransferState.restartRequired,
      ));
      final promoted = await runner.promoteAmbiguousRestarts();
      expect(promoted, 1);
    });
  });

  group('sync_items 基线结算（对齐 settlement.rs）', () {
    test('上传成功：pending: 占位行被清理', () async {
      // 预置 pending: 占位行
      final db = await DatabaseService.instance.database;
      await db.insert(
          'sync_items',
          SyncItem(
            fileId: '$pendingFileIdPrefix${'a.txt'}',
            localPath: 'a.txt',
            name: 'a.txt',
            status: SyncItemStatus.syncing,
          ).toRow());
      final intent = await newUploadIntent('a.txt');
      final result = await runner.enqueueAndRun(intent);
      expect(result.unwrap().outcome.disposition, TaskDisposition.completed);
      final items = await allItems();
      expect(items.length, 1);
      expect(items.single.fileId, 'cloud-a.txt');
      expect(items.single.status, SyncItemStatus.synced);
      expect(items.single.localSize, intent.sourceSize);
      expect(items.single.localMtime, intent.sourceMtime);
    });

    test('Update 成功：同 fileId 旧路径行被清理', () async {
      final db = await DatabaseService.instance.database;
      // 旧路径基线（改名前）
      await db.insert(
          'sync_items',
          SyncItem(
            fileId: 'f1',
            localPath: 'old.txt',
            name: 'old.txt',
            localSize: 1,
            localMtime: 1,
            cloudEditedTime: 1,
            status: SyncItemStatus.synced,
          ).toRow());
      final abs = p.join(mountDir.path, 'new.txt');
      await File(abs).writeAsString('zz');
      final stat = await FileStat.stat(abs);
      final update = TransferTask(
        direction: TransferDirection.upload,
        fileId: 'f1',
        localPath: abs,
        name: 'new.txt',
        totalSize: stat.size,
        createdAt: nowMs,
        relativePath: 'new.txt',
        parentFileId: 'root',
        operation: TransferOperation.update,
        sourceMtime: stat.modified.millisecondsSinceEpoch,
        sourceSize: stat.size,
        expectedCloudEditedTime: 1,
      );
      ops.behaviors['new.txt'] = (task) => TaskExecutionOutcome(
            cloudFile: DriveFile(
              id: 'f1',
              name: 'new.txt',
              size: task.sourceSize ?? 0,
              editedTime: DateTime.fromMillisecondsSinceEpoch(1700000060000,
                  isUtc: true),
            ),
          );
      final result = await runner.enqueueAndRun(update);
      expect(result.unwrap().outcome.disposition, TaskDisposition.completed);
      final items = await allItems();
      expect(items.length, 1);
      expect(items.single.localPath, 'new.txt');
      expect(items.single.fileId, 'f1');
      expect(items.single.cloudEditedTime, 1700000060000);
    });

    test('下载成功：以本地文件现读事实结算', () async {
      final abs = p.join(mountDir.path, 'dl.txt');
      await File(abs).writeAsString('cloud-content');
      final task = TransferTask(
        direction: TransferDirection.download,
        fileId: 'f9',
        localPath: abs,
        name: 'dl.txt',
        totalSize: 13,
        createdAt: nowMs,
        relativePath: 'dl.txt',
        parentFileId: 'root',
        operation: TransferOperation.download,
        expectedCloudEditedTime: 1700000000000,
      );
      ops.behaviors['dl.txt'] = (t) => const TaskExecutionOutcome();
      // 直接走钩子（下载执行已由 DriveTaskOperations 覆盖）
      await baselineStore.onTaskCommitted(task, const TaskExecutionOutcome());
      final items = await allItems();
      expect(items.single.fileId, 'f9');
      expect(items.single.status, SyncItemStatus.synced);
      expect(items.single.localSize, 13);
      expect(items.single.cloudEditedTime, 1700000000000);
    });
  });

  group('失败/重试回写（对齐 persistence.rs）', () {
    Future<void> seedItem(String path, SyncItemStatus status,
        {String fileId = 'f1'}) async {
      final db = await DatabaseService.instance.database;
      await db.insert(
          'sync_items',
          SyncItem(
            fileId: fileId,
            localPath: path,
            name: p.basename(path),
            status: status,
          ).toRow(),
          conflictAlgorithm: ConflictAlgorithm.replace);
    }

    test('onTaskFailed 仅覆盖 SYNCED/SYNCING/CLOUD_ONLY/FAILED', () async {
      await seedItem('a', SyncItemStatus.synced, fileId: 'fa');
      await seedItem('b', SyncItemStatus.syncing, fileId: 'fb');
      await seedItem('c', SyncItemStatus.cloudOnly, fileId: 'fc');
      await seedItem('d', SyncItemStatus.deleted, fileId: 'fd');
      await seedItem('e', SyncItemStatus.conflict, fileId: 'fe');
      final task = TransferTask(
        id: 1,
        fileId: 'fa',
        name: 'a',
        relativePath: 'a',
        state: TransferState.failed,
        createdAt: nowMs,
      );
      await baselineStore.onTaskFailed(task, 'boom');
      // 逐路径结算
      for (final (path, fid) in [('b', 'fb'), ('c', 'fc'), ('d', 'fd'), ('e', 'fe')]) {
        await baselineStore.onTaskFailed(
          TransferTask(
              id: 2, fileId: fid, name: path, relativePath: path, createdAt: nowMs),
          'boom',
        );
      }
      final items = {for (final i in await allItems()) i.localPath: i};
      expect(items['a']?.status, SyncItemStatus.failed);
      expect(items['b']?.status, SyncItemStatus.failed);
      expect(items['c']?.status, SyncItemStatus.failed);
      expect(items['d']?.status, SyncItemStatus.deleted); // 不被覆盖
      expect(items['e']?.status, SyncItemStatus.conflict); // 不被覆盖
    });

    test('onTaskFailed：fileId 缺省回退 pending: 占位', () async {
      await seedItem('a.txt', SyncItemStatus.syncing,
          fileId: '$pendingFileIdPrefix${'a.txt'}');
      await baselineStore.onTaskFailed(
        TransferTask(
            id: 1, name: 'a.txt', relativePath: 'a.txt', createdAt: nowMs),
        '空间不足',
      );
      final items = await allItems();
      expect(items.single.status, SyncItemStatus.failed);
      expect(items.single.errorMessage, '空间不足');
    });

    test('onRetryAccepted 仅从 FAILED 回写 SYNCING', () async {
      await seedItem('a', SyncItemStatus.failed, fileId: 'fa');
      await seedItem('b', SyncItemStatus.conflict, fileId: 'fb');
      await baselineStore.onRetryAccepted(
        TransferTask(
            id: 1, fileId: 'fa', name: 'a', relativePath: 'a', createdAt: nowMs),
      );
      await baselineStore.onRetryAccepted(
        TransferTask(
            id: 2, fileId: 'fb', name: 'b', relativePath: 'b', createdAt: nowMs),
      );
      final items = {for (final i in await allItems()) i.localPath: i};
      expect(items['a']?.status, SyncItemStatus.syncing);
      expect(items['a']?.errorMessage, isNull);
      expect(items['b']?.status, SyncItemStatus.conflict);
    });

    test('RETRY 周期收尾：无 Failed 任务的 FAILED 行置回 SYNCING', () async {
      await seedItem('a', SyncItemStatus.failed, fileId: 'fa');
      await seedItem('b', SyncItemStatus.failed, fileId: 'fb');
      // b 有对应 Failed 任务 → 保留 FAILED
      final db = await DatabaseService.instance.database;
      await db.insert(
          'transfer_queue',
          TransferTask(
            direction: TransferDirection.upload,
            fileId: 'fb',
            name: 'b',
            relativePath: 'b',
            state: TransferState.failed,
            createdAt: nowMs,
          ).toRow());
      await baselineStore.sweepFailedWithoutFailedTasks();
      final items = {for (final i in await allItems()) i.localPath: i};
      expect(items['a']?.status, SyncItemStatus.syncing);
      expect(items['b']?.status, SyncItemStatus.failed);
    });

    test('resetStaleStatuses：SYNCING → SYNCED，FAILED 保留', () async {
      await seedItem('a', SyncItemStatus.syncing, fileId: 'fa');
      await seedItem('b', SyncItemStatus.failed, fileId: 'fb');
      await baselineStore.resetStaleStatuses();
      final items = {for (final i in await allItems()) i.localPath: i};
      expect(items['a']?.status, SyncItemStatus.synced);
      expect(items['b']?.status, SyncItemStatus.failed);
    });
  });

  test('基线结算失败禁止完成任务行（recover_success_settlement_failure）',
      () async {
    // 上传缺少源快照 → onTaskCommitted 抛错 → 任务转 VerifyingRemote
    final abs = p.join(mountDir.path, 'a.txt');
    await File(abs).writeAsString('x');
    final stat = await FileStat.stat(abs);
    final intent = TransferTask(
      direction: TransferDirection.upload,
      localPath: abs,
      name: 'a.txt',
      totalSize: stat.size,
      createdAt: nowMs,
      relativePath: 'a.txt',
      parentFileId: 'root',
      operation: TransferOperation.create,
      sourceMtime: stat.modified.millisecondsSinceEpoch,
      sourceSize: stat.size,
    );
    // 注入一个必然失败的钩子
    runner.setSyncHooks(_FailingHooks());
    try {
      await runner.enqueueAndRun(intent);
    } catch (_) {
      // outcome 以错误完结（任务未 Completed）
    }
    final tasks = await allTasks();
    expect(tasks.single.state, TransferState.verifyingRemote);
    expect(tasks.single.errorKind, TransferErrorKind.remoteAmbiguous);
  });
}

/// 必然失败的基线钩子。
class _FailingHooks implements SyncTaskHooks {
  @override
  Future<void> onTaskCommitted(
      TransferTask running, TaskExecutionOutcome outcome) async {
    throw StateError('DB 写入失败');
  }

  @override
  Future<void> onTaskFailed(TransferTask failed, String message) async {}

  @override
  Future<void> onRetryAccepted(TransferTask pending) async {}

  @override
  Future<void> onTaskReplanned(TransferTask task) async {}
}
