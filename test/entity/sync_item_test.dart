import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/types/enums.dart';

/// 构造覆盖全部 13 列的同步项
SyncItem _fullItem() {
  return const SyncItem(
    fileId: 'file-123',
    localPath: 'dir/sub/a.txt',
    parentFolderId: 'parent-1',
    name: 'a.txt',
    isFolder: false,
    size: 2048,
    localSize: 2047,
    sha256: 'deadbeef',
    localMtime: 1750000000000,
    cloudEditedTime: 1750000100000,
    lastSyncTime: 1750000200000,
    status: SyncItemStatus.Synced,
    errorMessage: null,
  );
}

void main() {
  group('SyncItem', () {
    group('fromRow / toRow 往返', () {
      test('全字段往返保持一致（13 列）', () {
        final item = _fullItem();

        final row = item.toRow();
        expect(row.length, 13);
        expect(row['file_id'], 'file-123');
        expect(row['local_path'], 'dir/sub/a.txt');
        expect(row['is_folder'], 0);
        expect(row['status'], 0);

        final restored = SyncItem.fromRow(row);

        expect(restored.fileId, item.fileId);
        expect(restored.localPath, item.localPath);
        expect(restored.parentFolderId, item.parentFolderId);
        expect(restored.name, item.name);
        expect(restored.isFolder, item.isFolder);
        expect(restored.size, item.size);
        expect(restored.localSize, item.localSize);
        expect(restored.sha256, item.sha256);
        expect(restored.localMtime, item.localMtime);
        expect(restored.cloudEditedTime, item.cloudEditedTime);
        expect(restored.lastSyncTime, item.lastSyncTime);
        expect(restored.status, item.status);
        expect(restored.errorMessage, isNull);
      });

      test('文件夹 + 非默认状态往返', () {
        const item = SyncItem(
          fileId: 'folder-1',
          localPath: 'dir',
          name: 'dir',
          isFolder: true,
          status: SyncItemStatus.Conflict,
          errorMessage: '本地与云端同时修改',
        );

        final restored = SyncItem.fromRow(item.toRow());

        expect(restored.isFolder, isTrue);
        expect(restored.status, SyncItemStatus.Conflict);
        expect(restored.errorMessage, '本地与云端同时修改');
      });

      test('fromRow 容忍 String 数字', () {
        final item = SyncItem.fromRow({
          'file_id': 'f1',
          'local_path': 'a.txt',
          'name': 'a.txt',
          'is_folder': '1',
          'size': '2048',
          'local_size': '2047',
          'local_mtime': '1750000000000',
          'status': '5',
        });

        expect(item.isFolder, isTrue);
        expect(item.size, 2048);
        expect(item.localSize, 2047);
        expect(item.localMtime, 1750000000000);
        expect(item.status, SyncItemStatus.Conflict);
      });

      test('fromRow 未知 status 码回退 Synced', () {
        final item = SyncItem.fromRow({
          'file_id': 'f1',
          'local_path': 'a.txt',
          'name': 'a.txt',
          'status': 6, // 空缺值
        });

        expect(item.status, SyncItemStatus.Synced);
      });

      test('Deleted=7 tombstone 状态正确解析', () {
        final item = SyncItem.fromRow({
          'file_id': 'f1',
          'local_path': 'a.txt',
          'name': 'a.txt',
          'status': 7,
        });

        expect(item.status, SyncItemStatus.Deleted);
      });
    });

    group('fromJson / toJson 往返', () {
      test('全字段往返保持一致（camelCase 键）', () {
        final item = _fullItem().copyWith(
          status: SyncItemStatus.Syncing,
          errorMessage: '同步中',
        );

        final json = item.toJson();
        expect(json['fileId'], 'file-123');
        expect(json['localPath'], 'dir/sub/a.txt');
        expect(json['cloudEditedTime'], 1750000100000);
        expect(json['status'], 3);

        final restored = SyncItem.fromJson(json);
        expect(restored.fileId, item.fileId);
        expect(restored.localPath, item.localPath);
        expect(restored.status, SyncItemStatus.Syncing);
        expect(restored.errorMessage, '同步中');
        expect(restored.localSize, item.localSize);
      });
    });

    group('pending: 占位 fileId', () {
      test('前缀常量对齐 Rust PENDING_FILE_ID_PREFIX', () {
        expect(pendingFileIdPrefix, 'pending:');
      });

      test('isPendingUpload 识别占位项', () {
        const pending = SyncItem(
          fileId: 'pending:dir/a.txt',
          localPath: 'dir/a.txt',
          name: 'a.txt',
          status: SyncItemStatus.Failed,
        );
        const normal = SyncItem(
          fileId: 'file-123',
          localPath: 'dir/a.txt',
          name: 'a.txt',
        );

        expect(pending.isPendingUpload, isTrue);
        expect(normal.isPendingUpload, isFalse);
      });
    });

    group('copyWith', () {
      test('替换字段并可显式清空可空字段', () {
        final item = _fullItem();

        final copy = item.copyWith(status: SyncItemStatus.Failed, size: 4096);
        expect(copy.status, SyncItemStatus.Failed);
        expect(copy.size, 4096);
        expect(copy.sha256, item.sha256);

        final cleared = item.copyWith(sha256: null, errorMessage: null);
        expect(cleared.sha256, isNull);
        expect(cleared.errorMessage, isNull);
      });
    });
  });
}
