import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

import '../auth/fake_http.dart';
import '../drive/drive_test_util.dart';
import '../mount/proc_xattr.dart';

/// 目录递归同步测试（对齐 Rust folder_sync.rs：
/// 后台 BFS 子树双端对齐 + folder_sync_progress {done,total} 进度）。

class _FakeOps extends TaskOperations {
  /// 已执行上传（relPath）
  final List<String> uploaded = [];

  /// 已执行下载（relPath）
  final List<String> downloaded = [];

  @override
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    final rel = task.relativePath ?? '';
    if (task.operation == TransferOperation.download ||
        task.operation == TransferOperation.downloadUpdate) {
      downloaded.add(rel);
      await File(task.localPath!).writeAsString('cloud-content');
      return const TaskExecutionOutcome();
    }
    uploaded.add(rel);
    return TaskExecutionOutcome(
      cloudFile: DriveFile(
        id: 'cloud-${task.name}',
        name: task.name,
        size: task.sourceSize ?? 0,
        parentFolder:
            task.parentFileId != null ? [task.parentFileId!] : null,
        editedTime:
            DateTime.fromMillisecondsSinceEpoch(1700000000000, isUtc: true),
      ),
    );
  }
}

void main() {
  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  late Directory mountDir;
  late Directory supportRoot;
  late SyncService service;
  late _FakeOps ops;
  late TaskRunner runner;

  setUp(() async {
    mountDir = Directory.systemTemp.createTempSync('folder_sync_mount');
    supportRoot = Directory.systemTemp.createTempSync('folder_sync_support');
    AppPaths.debugSupportRoot = supportRoot.path;
    DatabaseService.debugDatabasePath = '${supportRoot.path}/petal_link.db';

    // 云端：root/docs/{cloud.txt}；本地：docs/local.txt
    final adapter = FakeHttpAdapter((request) {
      final path = request.uri.path;
      if (path.endsWith('/changes/getStartCursor')) {
        return jsonResponse(
            {'category': 'drive#startCursor', 'startCursor': 'c0'});
      }
      if (path.endsWith('/changes')) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': const [],
          'newStartCursor': 'c1',
        });
      }
      if (path.endsWith('/files')) {
        final qp = request.uri.queryParameters['queryParam'] ?? '';
        if (qp.contains("'root'")) {
          return jsonResponse(fileListPageJson([
            folderJson(id: 'd1', name: 'docs', parentFolder: ['root']),
          ]));
        }
        if (qp.contains("'d1'")) {
          return jsonResponse(fileListPageJson([
            fileJson(
                id: 'f-cloud',
                name: 'cloud.txt',
                parentFolder: ['d1'],
                editedTime: '2023-11-14T22:13:20.000Z'),
          ]));
        }
        return jsonResponse(fileListPageJson(const []));
      }
      throw StateError('未处理请求: ${request.uri}');
    });
    final client = buildTestClient(adapter);
    final filesService = FilesService(client);
    final changesService = ChangesService(client);
    final config = ConfigService(
        DatabaseService.instance, const FlutterSecureStorage());
    await config.set('mount_path', mountDir.path);
    await config.set('poll_interval', '0');

    ops = _FakeOps();
    final procXattr = ProcXattrService();
    runner = TaskRunner(
      transferService: TransferService(DatabaseService.instance),
      operations: ops,
      nowMs: () => 1700000000000,
      mountRootProvider: () => mountDir.path,
      isPlaceholder: (path) async =>
          await procXattr.get(path, xattrState) == statePlaceholder,
    );
    NetGuard.instance.debugConfigure(probe: () async => true);
    service = SyncService(
      db: DatabaseService.instance,
      config: config,
      filesApi: filesService,
      changesApi: changesService,
      uploadApi: UploadService(client),
      downloadApi: DownloadService(client),
      netGuard: NetGuard.instance,
      taskRunner: runner,
      isLoggedIn: () async => true,
      xattr: procXattr,
    );
  });

  tearDown(() async {
    await service.dispose();
    await runner.dispose();
    NetGuard.instance.debugReset();
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    AppPaths.debugSupportRoot = null;
    if (mountDir.existsSync()) mountDir.deleteSync(recursive: true);
    if (supportRoot.existsSync()) supportRoot.deleteSync(recursive: true);
  });

  test('后台 BFS 子树双端对齐 + 进度事件 {done,total}', () async {
    await service.ensureEngineStarted();
    expect(service.isEngineStarted, isTrue);
    // 启动周期收敛后再放本地文件（避免被启动周期先行上传）
    await Future<void>.delayed(const Duration(milliseconds: 300));
    await Directory(p.join(mountDir.path, 'docs')).create(recursive: true);
    await File(p.join(mountDir.path, 'docs', 'local.txt'))
        .writeAsString('local-content');

    final progressFuture = service.folderSyncProgress
        .take(2)
        .toList()
        .timeout(const Duration(seconds: 15));
    await service.folderRecursive(folderId: 'd1', relPath: 'docs');
    final events = await progressFuture;

    // 进度：total=2（1 下载 + 1 上传），失败也计数
    expect(events.length, 2);
    expect(events.every((e) => e.total == 2), isTrue);
    expect(events.last.done, 2);
    // 双端对齐：云端独有的被下载，本地独有的被上传
    expect(ops.downloaded, contains('docs/cloud.txt'));
    expect(ops.uploaded, contains('docs/local.txt'));
    // 上传成功后内存云树即时更新
    await Future<void>.delayed(const Duration(milliseconds: 200));
    expect(service.currentState, isNotNull);
  });

  test('本地扫描应用 skipPatterns：.DS_Store 等跳过文件不上传', () async {
    await service.ensureEngineStarted();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    await Directory(p.join(mountDir.path, 'docs')).create(recursive: true);
    // 跳过文件（默认 skipPatterns：.DS_Store / .tmp / ~$* / .Trash）
    await File(p.join(mountDir.path, 'docs', '.DS_Store'))
        .writeAsString('junk');
    await File(p.join(mountDir.path, 'docs', '~\$lock.docx'))
        .writeAsString('junk');
    await File(p.join(mountDir.path, 'docs', 'local.txt'))
        .writeAsString('local-content');

    await service.folderRecursive(folderId: 'd1', relPath: 'docs');
    await Future<void>.delayed(const Duration(milliseconds: 300));

    // 对齐 Rust scan_dir_for_real_files(eng.skip_patterns())：
    // .DS_Store / ~$* 不得进入上传清单
    expect(ops.uploaded, isNot(contains('docs/.DS_Store')));
    expect(ops.uploaded, isNot(contains('docs/~\$lock.docx')));
    expect(ops.uploaded, contains('docs/local.txt'));
  });

  test('索引中拒绝 + 并发目录同步拒绝', () async {
    await service.ensureEngineStarted();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    // 第一个请求占据 folder guard（云端 BFS 期间）
    await service.folderRecursive(folderId: 'd1', relPath: 'docs');
    // 第二个并发请求被拒绝
    await expectLater(
      service.folderRecursive(folderId: 'd1', relPath: 'docs'),
      throwsA(anything),
    );
  });
}
