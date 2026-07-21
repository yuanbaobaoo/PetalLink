import 'dart:async';
import 'dart:collection';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart' show tmpPath;
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// TaskRunner 全量语义测试（对齐 Rust task_runner 状态机/调度/退避/核验/恢复）。
///
/// 测试夹具：sqflite_ffi 临时库 + 临时挂载目录（真实文件满足静态校验）
/// + fake 执行器（逐任务行为表）+ 可变假时钟 + 可控在线开关。

/// 单个任务的执行行为签名
typedef _Behavior = Future<TaskExecutionOutcome> Function(
  TransferTask task,
  TaskProgressCallbacks progress,
);

/// fake 执行器（对齐 TaskOperations 合同）
class _FakeOperations implements TaskOperations {
  /// 已开始的执行任务 id（按启动顺序）
  final List<int> executedOrder = [];

  /// 各任务后端前置校验调用计数
  final Map<int, int> preflightCalls = {};

  /// 逐任务行为（缺省走 defaultBehavior）
  final Map<int, _Behavior> behaviors = {};

  /// 默认行为
  _Behavior? defaultBehavior;

  /// verifyRemote 结果队列（按调用顺序消费）
  final Queue<RemoteVerification Function(TransferTask)> verifyResults =
      Queue();

  /// verifyRemote 调用记录
  final List<int> verifyCalls = [];

  @override
  Future<void> preflight(TransferTask task) async {
    preflightCalls[task.id] = (preflightCalls[task.id] ?? 0) + 1;
  }

  @override
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) {
    executedOrder.add(task.id);
    final behavior = behaviors[task.id] ?? defaultBehavior;
    if (behavior == null) {
      throw TaskAppError(AppError.generic('fake 缺少执行行为'));
    }
    return behavior(task, progress);
  }

  @override
  Future<RemoteVerification> verifyRemote(TransferTask task) async {
    verifyCalls.add(task.id);
    if (verifyResults.isEmpty) {
      return const RemoteAmbiguous('fake 默认仍不确定');
    }
    return verifyResults.removeFirst()(task);
  }
}

void main() {
  late Directory mountDir;
  late TransferService service;

  /// 可变假时钟（毫秒 epoch）
  late int nowMs;

  /// 在线开关
  late bool online;

  /// 请求级网络失败上报计数
  late int reportCount;

  /// created_at 递增序列
  late int createdSeq;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    mountDir = Directory.systemTemp.createTempSync('task_runner_test');
    DatabaseService.debugDatabasePath = '${mountDir.path}/petal_link.db';
    service = TransferService(DatabaseService.instance);
    nowMs = 1700000000000;
    online = true;
    reportCount = 0;
    createdSeq = 1;
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (mountDir.existsSync()) {
      mountDir.deleteSync(recursive: true);
    }
  });

  // ═══════════════════════════════════════════════════════════════════
  // 任务构造与通用辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 入队并解包
  Future<TransferTask> seed(TransferTask task) async =>
      (await service.enqueue(task)).unwrap();

  /// 构造满足静态校验的上传任务（真实源文件，Create 操作）
  Future<TransferTask> makeUploadTask(
    String rel, {
    int size = 100,
    TransferOperation operation = TransferOperation.Create,
    String? fileId,
  }) async {
    final file = File(p.join(mountDir.path, rel));
    await file.create(recursive: true);
    await file.writeAsBytes(List<int>.filled(size, 7), flush: true);
    final stat = await file.stat();
    return TransferTask(
      direction: TransferDirection.Upload,
      fileId: fileId,
      localPath: file.path,
      name: p.basename(rel),
      totalSize: stat.size,
      relativePath: rel,
      parentFileId: p.dirname(rel) == '.' ? null : 'parent-id',
      operation: operation,
      sourceMtime: stat.modified.millisecondsSinceEpoch,
      sourceSize: stat.size,
      createdAt: createdSeq++,
    );
  }

  /// 构造下载任务（目标缺省不存在）
  TransferTask makeDownloadTask(
    String rel, {
    int size = 100,
    String fileId = 'cloud-id',
  }) {
    return TransferTask(
      direction: TransferDirection.Download,
      fileId: fileId,
      localPath: p.join(mountDir.path, rel),
      name: p.basename(rel),
      totalSize: size,
      relativePath: rel,
      parentFileId: p.dirname(rel) == '.' ? null : 'parent-id',
      operation: TransferOperation.Download,
      expectedCloudEditedTime: 1690000000000,
      createdAt: createdSeq++,
    );
  }

  /// 上传成功的标准结果（元数据满足成功核验）
  TaskExecutionOutcome uploadOk(TransferTask task) {
    final time = DateTime.fromMillisecondsSinceEpoch(nowMs);
    return TaskExecutionOutcome(
      cloudFile: DriveFile(
        id: 'fid-${task.id}',
        name: task.name,
        size: task.totalSize,
        createdTime: time,
        editedTime: time,
      ),
    );
  }

  /// 下载成功行为：真实落盘后返回完成
  Future<TaskExecutionOutcome> downloadOk(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    await File(task.localPath!)
        .writeAsBytes(List<int>.filled(task.totalSize, 3));
    return const TaskExecutionOutcome();
  }

  /// 创建 runner（大 tick 周期避免定时器干扰，测试手动 debugTick 驱动）
  TaskRunner createRunner(
    _FakeOperations ops, {
    int concurrency = 6,
    int maxAttempts = 5,
    Stream<NetworkTransition>? netTransitions,
    void Function()? onNetworkFailure,
  }) {
    return TaskRunner(
      transferService: service,
      operations: ops,
      isOnline: () => online,
      netTransitions: netTransitions,
      onRequestNetworkFailure: onNetworkFailure ?? () => reportCount++,
      nowMs: () => nowMs,
      concurrencyProvider: () async => concurrency,
      mountRootProvider: () => mountDir.path,
      tickInterval: const Duration(days: 365),
      maxAttempts: maxAttempts,
    );
  }

  /// 读取任务最新行
  Future<TransferTask> row(int id) async =>
      (await service.getTaskById(id)).unwrap()!;

  /// 轮询直至条件成立（处理 event loop 上的异步链式调度）
  Future<void> eventually(bool Function() cond, {String? reason}) async {
    for (var i = 0; i < 400; i++) {
      if (cond()) return;
      await Future<void>.delayed(const Duration(milliseconds: 5));
    }
    fail('条件未在预期时间内成立: ${reason ?? "(未说明)"}');
  }

  /// 冲刷微任务/事件队列（进度写链顺序化为 Future 链，需多轮冲刷）
  Future<void> flush() async {
    for (var i = 0; i < 30; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  }

  /// HTTP 状态错误
  DriveApiError httpError(
    int status, {
    bool mayReached = false,
    RetryAfter? retryAfter,
  }) {
    return DriveApiError(
      driveCode: DriveApiErrorCode.fromStatus,
      message: 'HTTP $status',
      statusCode: status,
      retryAfter: retryAfter,
      requestMayHaveReachedServer: mayReached,
    );
  }

  /// 传输阶段网络错误
  DriveApiError transportError(
    DriveTransportKind kind, {
    bool mayReached = false,
  }) {
    return DriveApiError(
      driveCode: DriveApiErrorCode.network,
      message: kind.name,
      transportKind: kind,
      requestMayHaveReachedServer: mayReached,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 状态机主路径
  // ═══════════════════════════════════════════════════════════════════

  group('状态机主路径', () {
    test('上传 Create：Pending→Running→Completed，结算字段正确', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('a.bin'));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.transferred, stored.totalSize);
      expect(stored.finishedAt, isNotNull);
      expect(stored.remoteResultFileId, 'fid-${task.id}');
      expect(stored.errorKind, isNull);
      expect(stored.errorMessage, isNull);
      expect(ops.executedOrder, [task.id]);
      await runner.dispose();
    });

    test('下载 Download：成功核验本地落盘后 Completed', () async {
      final ops = _FakeOperations()..defaultBehavior = downloadOk;
      final runner = createRunner(ops);
      final task = await seed(makeDownloadTask('d.bin'));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.remoteResultFileId, 'cloud-id');
      expect(File(task.localPath!).lengthSync(), task.totalSize);
      await runner.dispose();
    });

    test('不可重试错误（HTTP 400）直转 Failed（Validation + finishedAt）', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('b.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(httpError(400));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Failed);
      expect(stored.errorKind, TransferErrorKind.Validation);
      expect(stored.finishedAt, isNotNull);
      expect(ops.executedOrder, [task.id]);
      await runner.dispose();
    });

    test('本地源变化：静态校验拒绝 → RestartRequired（LocalChanged）', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('c.bin'));
      // start 前篡改源文件大小（enqueue 时 runner 未启动不会调度）
      await File(task.localPath!).writeAsBytes(List<int>.filled(50, 9));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.RestartRequired);
      expect(stored.errorKind, TransferErrorKind.LocalChanged);
      expect(ops.executedOrder, isEmpty);
      await runner.dispose();
    });

    test('离线入队：Pending 准入后转 WaitingForNetwork', () async {
      online = false;
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('e.bin'));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.WaitingForNetwork);
      expect(stored.errorKind, TransferErrorKind.Network);
      expect(ops.executedOrder, isEmpty);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 并发调度与 FIFO
  // ═══════════════════════════════════════════════════════════════════

  group('并发调度', () {
    test('并发上限：槽位 2 时第三个任务等待空槽', () async {
      final gates = <int, Completer<void>>{};
      var runningNow = 0;
      var maxConcurrent = 0;
      final ops = _FakeOperations();
      final runner = createRunner(ops, concurrency: 2);

      final ids = <int>[];
      for (final rel in ['f1.bin', 'f2.bin', 'f3.bin']) {
        final task = await seed(await makeUploadTask(rel));
        ids.add(task.id);
        gates[task.id] = Completer<void>();
        ops.behaviors[task.id] = (t, progress) async {
          runningNow++;
          if (runningNow > maxConcurrent) maxConcurrent = runningNow;
          await gates[t.id]!.future;
          runningNow--;
          return uploadOk(t);
        };
      }

      await runner.start();
      // 前两个任务立即准入，第三个等槽
      await eventually(() => ops.executedOrder.length == 2, reason: '两个任务准入');
      expect(ops.executedOrder, [ids[0], ids[1]]);

      gates[ids[0]]!.complete();
      await eventually(() => ops.executedOrder.length == 3, reason: '空槽后第三任务准入');
      expect(ops.executedOrder, [ids[0], ids[1], ids[2]]);
      expect(maxConcurrent, 2);

      gates[ids[1]]!.complete();
      gates[ids[2]]!.complete();
      await runner.idle;
      for (final id in ids) {
        expect((await row(id)).state, TransferState.Completed);
      }
      await runner.dispose();
    });

    test('created_at FIFO：同轮准入按创建顺序拉起', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final ids = <int>[];
      for (final rel in ['g1.bin', 'g2.bin', 'g3.bin', 'g4.bin']) {
        ids.add((await seed(await makeUploadTask(rel))).id);
      }

      await runner.start();
      await runner.idle;

      expect(ops.executedOrder, ids);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 退避重试
  // ═══════════════════════════════════════════════════════════════════

  group('退避重试', () {
    test('HTTP 500 指数退避序列：1s → 2s → 4s，attempt_count 递增', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('h.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(httpError(500));

      await runner.start();
      await runner.idle;

      var stored = await row(task.id);
      expect(stored.state, TransferState.BackingOff);
      expect(stored.errorKind, TransferErrorKind.Server);
      expect(stored.attemptCount, 1);
      expect(stored.nextRetryAt, nowMs + 1000);

      nowMs += 1000;
      await runner.debugTick();
      await runner.idle;
      stored = await row(task.id);
      expect(stored.state, TransferState.BackingOff);
      expect(stored.attemptCount, 2);
      expect(stored.nextRetryAt, nowMs + 2000);

      nowMs += 2000;
      await runner.debugTick();
      await runner.idle;
      stored = await row(task.id);
      expect(stored.state, TransferState.BackingOff);
      expect(stored.attemptCount, 3);
      expect(stored.nextRetryAt, nowMs + 4000);
      expect(ops.executedOrder, [task.id, task.id, task.id]);
      await runner.dispose();
    });

    test('退避未到期不执行', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('h2.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(httpError(500));

      await runner.start();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.BackingOff);

      // 仅推进 500ms（未到期）→ tick 不再执行
      nowMs += 500;
      await runner.debugTick();
      await runner.idle;
      expect(ops.executedOrder, [task.id]);
      expect((await row(task.id)).state, TransferState.BackingOff);
      await runner.dispose();
    });

    test('HTTP 429 遵守 Retry-After（优先于指数退避）', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('i.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(
          httpError(429, retryAfter: const RetryAfterDelay(120)));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.BackingOff);
      expect(stored.errorKind, TransferErrorKind.RateLimit);
      expect(stored.nextRetryAt, nowMs + 120000);
      expect(stored.attemptCount, 1);
      await runner.dispose();
    });

    test('重试预算耗尽：429 超限直转 Failed（RateLimit）', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops, maxAttempts: 2);
      final task = await seed(await makeUploadTask('j.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(httpError(429));

      await runner.start();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.BackingOff);

      nowMs += 1000;
      await runner.debugTick();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.BackingOff);

      nowMs += 2000;
      await runner.debugTick();
      await runner.idle;
      final stored = await row(task.id);
      expect(stored.state, TransferState.Failed);
      expect(stored.errorKind, TransferErrorKind.RateLimit);
      expect(stored.finishedAt, isNotNull);
      expect(ops.executedOrder.length, 3);
      await runner.dispose();
    });

    test('退避任务缺 next_retry_at：拒绝重放 → Failed（Validation）', () async {
      final ops = _FakeOperations();
      final task = await seed(await makeUploadTask('nb.bin'));
      // 种入 BackingOff 且 nextRetryAt 为空
      await service.transition(task.id, TransferState.Pending, TransferState.Running);
      await service.transition(task.id, TransferState.Running, TransferState.BackingOff);

      final runner = createRunner(ops);
      await runner.start();
      await runner.debugTick();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Failed);
      expect(stored.errorKind, TransferErrorKind.Validation);
      expect(ops.executedOrder, isEmpty);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 网络门控与远端核验
  // ═══════════════════════════════════════════════════════════════════

  group('网络门控与远端核验', () {
    test('连接失败 → WaitingForNetwork（不耗预算）+ 在线边沿上报', () async {
      final ops = _FakeOperations();
      final netCtrl = StreamController<NetworkTransition>.broadcast();
      // 模拟真实 NetGuard：请求级失败上报达到阈值后翻转为离线
      final runner = createRunner(
        ops,
        netTransitions: netCtrl.stream,
        onNetworkFailure: () {
          reportCount++;
          online = false;
        },
      );
      final task = await seed(await makeUploadTask('k.bin'));
      ops.behaviors[task.id] =
          (_, _) => throw TaskAppError(transportError(DriveTransportKind.connect));

      await runner.start();
      await runner.idle;

      var stored = await row(task.id);
      expect(stored.state, TransferState.WaitingForNetwork);
      expect(stored.errorKind, TransferErrorKind.Network);
      expect(stored.attemptCount, 0);
      expect(reportCount, 1);

      // 网络恢复事件 → 重新调度并成功
      ops.behaviors[task.id] = (t, p) async => uploadOk(t);
      online = true;
      netCtrl.add(NetworkTransition.online);
      await eventually(() => ops.executedOrder.length == 2, reason: '恢复后重跑');
      await runner.idle;
      stored = await row(task.id);
      expect(stored.state, TransferState.Completed);

      await netCtrl.close();
      await runner.dispose();
    });

    test('写操作传输阶段失败且可能已送达 → VerifyingRemote 核验', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('l.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(
          transportError(DriveTransportKind.timeout, mayReached: true));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.VerifyingRemote);
      expect(stored.errorKind, TransferErrorKind.RemoteAmbiguous);
      expect(stored.nextRetryAt, nowMs + 3000);
      await runner.dispose();
    });

    test('离线期间 VerifyingRemote 不调度；到期 + 在线才核验', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('m.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(
          transportError(DriveTransportKind.timeout, mayReached: true));

      await runner.start();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.VerifyingRemote);

      // 离线：到期也不核验
      online = false;
      nowMs += 3000;
      await runner.debugTick();
      await runner.idle;
      expect(ops.verifyCalls, isEmpty);

      // 恢复在线：核验提交 → Completed
      online = true;
      ops.verifyResults.add((t) => RemoteCommitted(DriveFile(
            id: 'fid-remote',
            name: t.name,
            size: t.totalSize,
            createdTime: DateTime.fromMillisecondsSinceEpoch(nowMs),
            editedTime: DateTime.fromMillisecondsSinceEpoch(nowMs),
          )));
      await runner.debugTick();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.remoteResultFileId, 'fid-remote');
      await runner.dispose();
    });

    test('远端核验通道暂不可用 → 保留 VerifyingRemote，15s 后再核', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('n.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(
          transportError(DriveTransportKind.timeout, mayReached: true));

      await runner.start();
      await runner.idle;

      ops.verifyResults.add((_) => throw AppError.generic('核验通道异常'));
      nowMs += 3000;
      await runner.debugTick();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.VerifyingRemote);
      expect(stored.errorMessage, contains('远端核验暂不可用'));
      expect(stored.nextRetryAt, nowMs + 15000);
      await runner.dispose();
    });

    test('RemoteNotCommitted：RestartRequired→Pending 重跑；'
        'SessionExpired 失效会话被原子清理', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('o.bin', size: 1024));
      ops.behaviors[task.id] = (_, progress) async {
        // 先持久化一段断点会话（等写链落库），再抛会话过期
        progress.onResume('srv', 'up', 512, 'https://session.url/x');
        await flush();
        throw const TaskAppError(DriveApiError(
          driveCode: DriveApiErrorCode.fromStatus,
          message: 'session expired',
          errorCode: 'upload_session_expired',
        ));
      };

      await runner.start();
      await runner.idle;

      var stored = await row(task.id);
      expect(stored.state, TransferState.VerifyingRemote);
      expect(stored.errorKind, TransferErrorKind.SessionExpired);
      // 会话已持久化（onResume 不节流立即落库）
      expect(stored.sessionUrl, 'https://session.url/x');
      expect(stored.resumeOffset, 512);

      // 核验确认未提交 → RestartRequired（clearUploadSession）→ Pending 重跑
      TransferTask? replayed;
      ops.verifyResults.add((_) => const RemoteNotCommitted());
      ops.behaviors[task.id] = (t, p) async {
        replayed = t;
        return uploadOk(t);
      };
      nowMs += 3000;
      await runner.debugTick();
      await runner.idle;

      stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(ops.executedOrder, [task.id, task.id]);
      // 重跑读取的任务行已清掉失效会话
      expect(replayed, isNotNull);
      expect(replayed!.sessionUrl, isNull);
      expect(replayed!.serverId, isNull);
      expect(replayed!.uploadId, isNull);
      expect(replayed!.resumeOffset, 0);
      expect(replayed!.transferred, 0);
      await runner.dispose();
    });

    test('RemoteAmbiguous 保留歧义：60s 后再核', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('p.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(
          transportError(DriveTransportKind.timeout, mayReached: true));

      await runner.start();
      await runner.idle;

      ops.verifyResults.add((_) => const RemoteAmbiguous('仍无法确认'));
      nowMs += 3000;
      await runner.debugTick();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.VerifyingRemote);
      expect(stored.errorKind, TransferErrorKind.RemoteAmbiguous);
      expect(stored.nextRetryAt, nowMs + 60000);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 崩溃恢复
  // ═══════════════════════════════════════════════════════════════════

  group('崩溃恢复', () {
    test('中断 Running 上传 → VerifyingRemote；核验 Committed 后结算', () async {
      final ops = _FakeOperations();
      final task = await seed(await makeUploadTask('q.bin'));
      await service.transition(task.id, TransferState.Pending, TransferState.Running);

      final runner = createRunner(ops);
      ops.verifyResults.add((t) => RemoteCommitted(DriveFile(
            id: 'fid-crashed',
            name: t.name,
            size: t.totalSize,
            createdTime: DateTime.fromMillisecondsSinceEpoch(nowMs),
            editedTime: DateTime.fromMillisecondsSinceEpoch(nowMs),
          )));

      await runner.start();
      await runner.idle;

      // 恢复后 next_retry_at 为空视为到期，启动泵内立即核验并结算
      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.remoteResultFileId, 'fid-crashed');
      expect(ops.executedOrder, isEmpty);
      await runner.dispose();
    });

    test('中断 Running 下载：断点取 .tmp 磁盘真值并续跑', () async {
      final ops = _FakeOperations()..defaultBehavior = downloadOk;
      final task = await seed(makeDownloadTask('r.bin', size: 100));
      await service.transition(task.id, TransferState.Pending, TransferState.Running);
      // 磁盘残留 40 字节断点
      await File(tmpPath(task.localPath!)).writeAsBytes(List<int>.filled(40, 5));

      TransferTask? resumed;
      final runner = createRunner(ops);
      ops.behaviors[task.id] = (t, progress) async {
        resumed = t;
        return downloadOk(t, progress);
      };

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      // 重启后 Pending 行携带 durable 断点
      expect(resumed, isNotNull);
      expect(resumed!.resumeOffset, 40);
      expect(resumed!.transferred, 40);
      await runner.dispose();
    });

    test('同路径多任务收敛：仅最新意图存活，旧任务抑制为 RestartRequired', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      // 同路径两条 Pending（createdAt 递增）
      const rel = 'same.bin';
      final file = File(p.join(mountDir.path, rel));
      await file.writeAsBytes(List<int>.filled(100, 7), flush: true);
      final stat = await file.stat();
      TransferTask intent() => TransferTask(
            direction: TransferDirection.Upload,
            localPath: file.path,
            name: rel,
            totalSize: stat.size,
            relativePath: rel,
            operation: TransferOperation.Create,
            sourceMtime: stat.modified.millisecondsSinceEpoch,
            sourceSize: stat.size,
            createdAt: createdSeq++,
          );
      final older = await seed(intent());
      final newer = await seed(intent());

      final runner = createRunner(ops);
      await runner.start();
      await runner.idle;

      final olderRow = await row(older.id);
      final newerRow = await row(newer.id);
      expect(olderRow.state, TransferState.RestartRequired);
      expect(olderRow.errorKind, TransferErrorKind.LocalChanged);
      expect(newerRow.state, TransferState.Completed);
      expect(ops.executedOrder, [newer.id]);
      await runner.dispose();
    });

    test('中断任务缺少 operation → Failed（Validation）', () async {
      final ops = _FakeOperations();
      final task = await seed(TransferTask(
        direction: TransferDirection.Upload,
        localPath: p.join(mountDir.path, 's.bin'),
        name: 's.bin',
        totalSize: 100,
        relativePath: 's.bin',
        createdAt: createdSeq++,
      ));
      await service.transition(task.id, TransferState.Pending, TransferState.Running);

      final runner = createRunner(ops);
      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Failed);
      expect(stored.errorKind, TransferErrorKind.Validation);
      await runner.dispose();
    });

    test('含远端结果 ID 的 RestartRequired 启动时提升为 VerifyingRemote', () async {
      final ops = _FakeOperations();
      final task = await seed(await makeUploadTask('t.bin'));
      // Pending→Running→RestartRequired（携带 remote_result_file_id）
      await service.transition(task.id, TransferState.Pending, TransferState.Running);
      await service.transition(
          task.id, TransferState.Running, TransferState.RestartRequired);
      final db = await DatabaseService.instance.database;
      await db.update(
        'transfer_queue',
        {'remote_result_file_id': 'fid-ambiguous'},
        where: 'id = ?',
        whereArgs: [task.id],
      );

      final runner = createRunner(ops);
      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.VerifyingRemote);
      expect(stored.errorKind, TransferErrorKind.RemoteAmbiguous);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 进度 CAS 与会话续传
  // ═══════════════════════════════════════════════════════════════════

  group('进度与会话', () {
    test('onProgress/onResume 落库；生命周期收束后的迟到回调被门禁拒绝', () async {
      final gate = Completer<void>();
      late TaskProgressCallbacks callbacks;
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('u.bin'));
      ops.behaviors[task.id] = (t, progress) async {
        callbacks = progress;
        progress.onProgress(50);
        progress.onResume('srv', 'up', 70, 'https://session.url/u');
        await gate.future;
        return uploadOk(t);
      };

      await runner.start();
      // 等执行进入 gate 并冲刷进度写链
      await eventually(() => ops.executedOrder.isNotEmpty, reason: '任务开始执行');
      await flush();
      // 直接读库验证进度落库（Running 期间）
      var stored = await row(task.id);
      expect(stored.state, TransferState.Running);
      expect(stored.resumeOffset, 70);
      expect(stored.sessionUrl, 'https://session.url/u');
      expect(stored.serverId, 'srv');

      gate.complete();
      await runner.idle;
      stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.transferred, stored.totalSize);

      // 迟到回调（旧 revision + 非 Running）不得落库
      nowMs += 1000;
      callbacks.onProgress(10);
      await flush();
      stored = await row(task.id);
      expect(stored.transferred, stored.totalSize);
      await runner.dispose();
    });

    test('断点续传任务：resumeOffset>0 凭 sessionUrl 通过静态校验并执行', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final base = await makeUploadTask('v.bin', size: 1024);
      final task = await seed(base.copyWith(
        resumeOffset: 512,
        sessionUrl: 'https://session.url/resume',
      ));

      TransferTask? executed;
      ops.behaviors[task.id] = (t, p) async {
        executed = t;
        return uploadOk(t);
      };

      await runner.start();
      await runner.idle;

      expect(executed, isNotNull);
      expect(executed!.resumeOffset, 512);
      expect(executed!.sessionUrl, 'https://session.url/resume');
      expect((await row(task.id)).state, TransferState.Completed);
      await runner.dispose();
    });

    test('断点无 sessionUrl：静态校验拒绝 → Failed（Validation）', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final base = await makeUploadTask('w.bin');
      final task = await seed(base.copyWith(resumeOffset: 512));

      await runner.start();
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Failed);
      expect(stored.errorKind, TransferErrorKind.Validation);
      expect(ops.executedOrder, isEmpty);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 命令面
  // ═══════════════════════════════════════════════════════════════════

  group('命令面', () {
    test('enqueue 校验：id!=0 / 非 Pending 拒绝', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await makeUploadTask('x.bin');

      final badId = await runner.enqueue(task.copyWith(id: 99));
      expect(badId.isErr, isTrue);

      final badState =
          await runner.enqueue(task.copyWith(state: TransferState.Running));
      expect(badState.isErr, isTrue);
      await runner.dispose();
    });

    test('retry：Failed → preflight → Pending → 重跑成功；attempt_count+1；'
        '后端前置校验只多跑一次', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('y.bin'));
      ops.behaviors[task.id] = (_, _) => throw TaskAppError(httpError(400));

      await runner.start();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.Failed);
      expect(ops.preflightCalls[task.id], 1);

      ops.behaviors[task.id] = (t, p) async => uploadOk(t);
      final result = await runner.retry(task.id);
      expect(result.isOk, isTrue);
      await runner.idle;

      final stored = await row(task.id);
      expect(stored.state, TransferState.Completed);
      expect(stored.attemptCount, 1);
      // 初始 1 次 + retry 内 1 次；链路内不重复（对齐 run_backend_preflight=false）
      expect(ops.preflightCalls[task.id], 2);
      expect(ops.executedOrder, [task.id, task.id]);
      await runner.dispose();
    });

    test('retry 非 Failed 任务被拒绝', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final task = await seed(await makeUploadTask('z.bin'));

      await runner.start();
      await runner.idle;
      expect((await row(task.id)).state, TransferState.Completed);

      final result = await runner.retry(task.id);
      expect(result.isErr, isTrue);
      await runner.dispose();
    });

    test('hasActive / clearCompleted / clearFailed / clearFinished', () async {
      final ops = _FakeOperations();
      final runner = createRunner(ops);
      final okTask = await seed(await makeUploadTask('c1.bin'));
      final failTask = await seed(await makeUploadTask('c2.bin'));
      ops.behaviors[okTask.id] = (t, p) async => uploadOk(t);
      ops.behaviors[failTask.id] = (_, _) => throw TaskAppError(httpError(400));

      await runner.start();
      await runner.idle;
      expect((await runner.hasActive()).unwrap(), isFalse);

      // clearCompleted 仅删 Completed
      await runner.clearCompleted();
      var all = (await service.getAllTasks()).unwrap();
      expect(all.length, 1);
      expect(all.single.id, failTask.id);

      // clearFailed 仅删 Failed
      await runner.clearFailed();
      expect((await service.getAllTasks()).unwrap(), isEmpty);

      // clearFinished = Completed + Failed
      final ok2 = await seed(await makeUploadTask('c3.bin'));
      final fail2 = await seed(await makeUploadTask('c4.bin'));
      ops.behaviors[ok2.id] = (t, p) async => uploadOk(t);
      ops.behaviors[fail2.id] = (_, _) => throw TaskAppError(httpError(400));
      await runner.debugTick();
      await runner.idle;
      await runner.clearFinished();
      expect((await service.getAllTasks()).unwrap(), isEmpty);
      await runner.dispose();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // 快照发布
  // ═══════════════════════════════════════════════════════════════════

  group('快照发布', () {
    test('revision 单调递增；内容含全量任务与活跃计数', () async {
      final ops = _FakeOperations()
        ..defaultBehavior = (t, p) async => uploadOk(t);
      final runner = createRunner(ops);
      final snapshots = <TransferQueueSnapshot>[];
      final sub = runner.snapshots.listen(snapshots.add);

      final task = await seed(await makeUploadTask('snap.bin'));
      await runner.start();
      await runner.idle;

      expect(snapshots, isNotEmpty);
      for (var i = 1; i < snapshots.length; i++) {
        expect(snapshots[i].revision, greaterThan(snapshots[i - 1].revision));
      }
      final last = runner.lastSnapshot!;
      expect(last.tasks.any((t) => t.id == task.id), isTrue);
      expect(last.activeCount, 0);

      await sub.cancel();
      await runner.dispose();
    });
  });
}
