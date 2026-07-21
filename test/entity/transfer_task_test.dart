import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/types/enums.dart';

/// 构造覆盖全部 26 列的任务
TransferTask _fullTask() {
  return const TransferTask(
    id: 42,
    direction: TransferDirection.downloadUpdate,
    fileId: 'file-1',
    localPath: '/Users/x/sync/dir/a.txt',
    name: 'a.txt',
    totalSize: 2048,
    transferred: 1024,
    state: TransferState.backingOff,
    errorMessage: '网络抖动',
    createdAt: 1750000000000,
    finishedAt: 1750000060000,
    serverId: 'server-1',
    uploadId: 'upload-1',
    resumeOffset: 512,
    sessionUrl: 'https://upload.example.com/session/abc',
    relativePath: 'dir/a.txt',
    parentFileId: 'parent-1',
    operation: TransferOperation.downloadUpdate,
    sourceMtime: 1749999900000,
    sourceSize: 2048,
    expectedCloudEditedTime: 1749999950000,
    attemptCount: 3,
    nextRetryAt: 1750000100000,
    errorKind: TransferErrorKind.network,
    remoteResultFileId: 'remote-1',
    stateRevision: 7,
  );
}

void main() {
  group('TransferTask', () {
    group('fromRow / toRow 往返', () {
      test('全字段往返保持一致（26 列）', () {
        final task = _fullTask();

        final row = task.toRow();
        // 26 列全部输出（id > 0 时含 id）
        expect(row.length, 26);
        expect(row['direction'], 3);
        expect(row['state'], 3);
        expect(row['operation'], 3);
        expect(row['error_kind'], 0);

        final restored = TransferTask.fromRow(row);

        expect(restored.id, task.id);
        expect(restored.direction, task.direction);
        expect(restored.fileId, task.fileId);
        expect(restored.localPath, task.localPath);
        expect(restored.name, task.name);
        expect(restored.totalSize, task.totalSize);
        expect(restored.transferred, task.transferred);
        expect(restored.state, task.state);
        expect(restored.errorMessage, task.errorMessage);
        expect(restored.createdAt, task.createdAt);
        expect(restored.finishedAt, task.finishedAt);
        expect(restored.serverId, task.serverId);
        expect(restored.uploadId, task.uploadId);
        expect(restored.resumeOffset, task.resumeOffset);
        expect(restored.sessionUrl, task.sessionUrl);
        expect(restored.relativePath, task.relativePath);
        expect(restored.parentFileId, task.parentFileId);
        expect(restored.operation, task.operation);
        expect(restored.sourceMtime, task.sourceMtime);
        expect(restored.sourceSize, task.sourceSize);
        expect(restored.expectedCloudEditedTime,
            task.expectedCloudEditedTime);
        expect(restored.attemptCount, task.attemptCount);
        expect(restored.nextRetryAt, task.nextRetryAt);
        expect(restored.errorKind, task.errorKind);
        expect(restored.remoteResultFileId, task.remoteResultFileId);
        expect(restored.stateRevision, task.stateRevision);
      });

      test('可空字段全 null 往返', () {
        const task = TransferTask(name: 'b.txt', createdAt: 1);

        final row = task.toRow();
        // id=0 不输出 id 列（交给 AUTOINCREMENT）
        expect(row.containsKey('id'), isFalse);

        final restored = TransferTask.fromRow(row);
        expect(restored.fileId, isNull);
        expect(restored.localPath, isNull);
        expect(restored.errorMessage, isNull);
        expect(restored.finishedAt, isNull);
        expect(restored.serverId, isNull);
        expect(restored.sessionUrl, isNull);
        expect(restored.operation, isNull);
        expect(restored.errorKind, isNull);
        expect(restored.state, TransferState.pending);
        expect(restored.direction, TransferDirection.upload);
      });

      test('fromRow 容忍 String 数字', () {
        final task = TransferTask.fromRow({
          'id': '42',
          'direction': '1',
          'name': 'a.txt',
          'total_size': '2048',
          'transferred': '1024',
          'state': '7',
          'created_at': '1750000000000',
          'resume_offset': '512',
          'attempt_count': '3',
          'error_kind': '8',
          'state_revision': '7',
        });

        expect(task.id, 42);
        expect(task.direction, TransferDirection.download);
        expect(task.totalSize, 2048);
        expect(task.state, TransferState.failed);
        expect(task.resumeOffset, 512);
        expect(task.attemptCount, 3);
        expect(task.errorKind, TransferErrorKind.sessionExpired);
        expect(task.stateRevision, 7);
      });

      test('fromRow 未知枚举码按安全默认处理', () {
        final task = TransferTask.fromRow({
          'name': 'a.txt',
          'direction': 99,
          'state': 99,
          'operation': 99,
          'error_kind': 99,
        });

        expect(task.direction, TransferDirection.upload);
        expect(task.state, TransferState.pending);
        expect(task.operation, isNull);
        expect(task.errorKind, isNull);
      });
    });

    group('progress', () {
      test('正常计算并 clamp 到 0-1', () {
        const task = TransferTask(
          name: 'a',
          createdAt: 0,
          totalSize: 2000,
          transferred: 500,
        );

        expect(task.progress, 0.25);
      });

      test('totalSize 为 0 时返回 0.0（防除零）', () {
        const task = TransferTask(name: 'a', createdAt: 0);

        expect(task.progress, 0.0);
      });

      test('transferred 超过 totalSize 时 clamp 到 1.0', () {
        const task = TransferTask(
          name: 'a',
          createdAt: 0,
          totalSize: 100,
          transferred: 300,
        );

        expect(task.progress, 1.0);
      });
    });

    group('状态机辅助', () {
      test('isTerminal / isActive 透传 state', () {
        const running = TransferTask(
          name: 'a',
          createdAt: 0,
          state: TransferState.running,
        );
        const done = TransferTask(
          name: 'a',
          createdAt: 0,
          state: TransferState.completed,
        );

        expect(running.isActive, isTrue);
        expect(running.isTerminal, isFalse);
        expect(done.isTerminal, isTrue);
      });

      test('canTransitionTo 校验合法转移', () {
        const task = TransferTask(
          name: 'a',
          createdAt: 0,
          state: TransferState.running,
        );

        expect(task.canTransitionTo(TransferState.completed), isTrue);
        expect(task.canTransitionTo(TransferState.pending), isFalse);
      });
    });

    group('copyWith', () {
      test('替换字段并可显式清空可空字段', () {
        final task = _fullTask();

        final copy = task.copyWith(
          transferred: 2048,
          state: TransferState.completed,
        );
        expect(copy.transferred, 2048);
        expect(copy.state, TransferState.completed);
        expect(copy.sessionUrl, task.sessionUrl);

        final cleared = task.copyWith(sessionUrl: null, errorKind: null);
        expect(cleared.sessionUrl, isNull);
        expect(cleared.errorKind, isNull);
        expect(cleared.errorMessage, task.errorMessage);
      });
    });
  });
}
