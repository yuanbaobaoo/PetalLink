import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

void main() {
  late Directory tempDir;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('transfer_service_test');
    DatabaseService.debugDatabasePath = '${tempDir.path}/petal_link.db';
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  Future<TransferTask> enqueueTask(TransferService service) async {
    final result = await service.enqueue(TransferTask(
      direction: TransferDirection.upload,
      fileId: 'fid1',
      localPath: '/tmp/a.bin',
      name: 'a.bin',
      totalSize: 1000,
      operation: TransferOperation.create,
      createdAt: DateTime.now().millisecondsSinceEpoch,
    ));
    return result.unwrap();
  }

  group('TransferService.updateResumeSession（对齐 Rust update_resume）', () {
    test('写入会话身份与断点偏移到任务行', () async {
      final service = TransferService(DatabaseService.instance);
      final task = await enqueueTask(service);

      final result = await service.updateResumeSession(
        task.id,
        serverId: 'srv1',
        uploadId: 'u1',
        resumeOffset: 512,
        sessionUrl: 'https://session.url/x',
      );

      expect(result.isOk, isTrue);
      final tasks = (await service.getAllTasks()).unwrap();
      final stored = tasks.singleWhere((t) => t.id == task.id);
      expect(stored.serverId, 'srv1');
      expect(stored.uploadId, 'u1');
      expect(stored.sessionUrl, 'https://session.url/x');
      expect(stored.resumeOffset, 512);
      expect(stored.transferred, 512);
    });

    test('会话 URL 轮换时即使偏移不变也必须落库（无 transferred CAS）', () async {
      final service = TransferService(DatabaseService.instance);
      final task = await enqueueTask(service);

      await service.updateResumeSession(
        task.id,
        serverId: 'srv1',
        uploadId: 'u1',
        resumeOffset: 512,
        sessionUrl: 'https://session.url/old',
      );
      // 偏移不变、URL 轮换 → 仍更新
      final result = await service.updateResumeSession(
        task.id,
        serverId: 'srv1',
        uploadId: 'u1',
        resumeOffset: 512,
        sessionUrl: 'https://session.url/new',
      );

      expect(result.isOk, isTrue);
      final stored = (await service.getAllTasks())
          .unwrap()
          .singleWhere((t) => t.id == task.id);
      expect(stored.sessionUrl, 'https://session.url/new');
    });

    test('非零断点缺少 session_url → 配置错误', () async {
      final service = TransferService(DatabaseService.instance);
      final task = await enqueueTask(service);

      final result = await service.updateResumeSession(
        task.id,
        serverId: 'srv1',
        uploadId: 'u1',
        resumeOffset: 512,
        sessionUrl: '',
      );

      expect(result.isErr, isTrue);
    });
  });
}
