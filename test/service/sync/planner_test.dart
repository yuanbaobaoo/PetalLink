import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// SyncPlanner 三方 diff 全动作矩阵测试（对齐 Rust planner.rs `_decide` 决策表）。

LocalFileEntry localEntry(
  String rel, {
  int size = 100,
  int mtime = 1700000000000,
  bool isFolder = false,
  bool isPlaceholder = false,
}) {
  return LocalFileEntry(
    absolutePath: '/mnt/$rel',
    relativePath: rel,
    size: size,
    mtime: mtime,
    isFolder: isFolder,
    isPlaceholder: isPlaceholder,
  );
}

DriveFile cloudFile(
  String id,
  String name, {
  bool isFolder = false,
  int size = 100,
  int? editedMs,
  String? parentId,
}) {
  return DriveFile(
    id: id,
    name: name,
    category: isFolder ? FileCategory.Folder : FileCategory.Document,
    size: size,
    parentFolder: parentId != null ? [parentId] : null,
    editedTime: editedMs != null
        ? DateTime.fromMillisecondsSinceEpoch(editedMs, isUtc: true)
        : null,
  );
}

DbSnapshotEntry dbEntry(
  String fileId, {
  int? localMtime = 1700000000000,
  int? localSize = 100,
  int? cloudEditedTime = 1700000000000,
  SyncItemStatus status = SyncItemStatus.Synced,
  bool isFolder = false,
}) {
  return DbSnapshotEntry(
    fileId: fileId,
    localMtime: localMtime,
    localSize: localSize,
    cloudEditedTime: cloudEditedTime,
    status: status,
    isFolder: isFolder,
  );
}

SyncSnapshot snap({
  Map<String, LocalFileEntry>? local,
  Map<String, DriveFile>? cloud,
  Map<String, DbSnapshotEntry>? db,
  bool trusted = true,
  bool startup = false,
}) {
  return SyncSnapshot(
    local: local ?? {},
    cloud: cloud ?? {},
    db: db ?? {},
    cloudTreeTrusted: trusted,
    isStartupResume: startup,
  );
}

void main() {
  final planner = SyncPlanner();

  group('三方都存在', () {
    test('双方未变化 → 无动作', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions, isEmpty);
    });

    test('本地变更 → Upload', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt', mtime: 1700000060000)},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.upload);
      expect(actions.single.fileId, 'f1');
    });

    test('本地 size 变更（mtime 相同）→ Upload（v3 精度兜底）', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt', size: 200)},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.upload);
    });

    test('云端变更 → Download', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000060000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.download);
      expect(actions.single.cloudFile?.id, 'f1');
    });

    test('双端均变更 → CreateConflictCopy', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt', mtime: 1700000060000)},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000060000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.createConflictCopy);
    });

    test('pending: 占位 + 云端已有 → Skip 携带 cloudFile（收敛为已同步）', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('$pendingFileIdPrefix${'a.txt'}')},
      ));
      expect(actions.single.actionType, SyncActionType.skip);
      expect(actions.single.cloudFile?.id, 'f1');
    });

    test('无 DB 记录 → Skip（无 cloudFile 被过滤，交给 reconcile）', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
      ));
      expect(actions, isEmpty);
    });
  });

  group('本地有 + 云端无', () {
    test('本地新文件 → Upload（无 fileId）', () {
      final actions = planner.plan(snap(
        local: {'new.txt': localEntry('new.txt')},
      ));
      expect(actions.single.actionType, SyncActionType.upload);
      expect(actions.single.fileId, isNull);
    });

    test('本地新目录 → CreateFolder', () {
      final actions = planner.plan(snap(
        local: {'dir': localEntry('dir', isFolder: true)},
      ));
      expect(actions.single.actionType, SyncActionType.createFolder);
      expect(actions.single.fileId, isNull);
    });

    test('孤儿占位符 → DeleteFromLocal（无 fileId）', () {
      final actions = planner.plan(snap(
        local: {'ghost.txt': localEntry('ghost.txt', size: 0, isPlaceholder: true)},
      ));
      expect(actions.single.actionType, SyncActionType.deleteFromLocal);
      expect(actions.single.fileId, isNull);
    });

    test('云端已删 + 本地未改 → DeleteFromLocal', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.deleteFromLocal);
      expect(actions.single.fileId, 'f1');
    });

    test('云端已删 + 本地已改 → BackupBeforeCloudDelete', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt', mtime: 1700000060000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.backupBeforeCloudDelete);
    });

    test('云端已删目录 → DeleteFromLocal（engine 决定是否保留）', () {
      final actions = planner.plan(snap(
        local: {'dir': localEntry('dir', isFolder: true)},
        db: {'dir': dbEntry('f1', isFolder: true)},
      ));
      expect(actions.single.actionType, SyncActionType.deleteFromLocal);
    });

    test('pending: 占位 + 云端无 → 重新 Upload', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        db: {'a.txt': dbEntry('$pendingFileIdPrefix${'a.txt'}')},
      ));
      expect(actions.single.actionType, SyncActionType.upload);
      expect(actions.single.fileId, isNull);
    });

    test('pending: 占位 + FAILED → 无动作（等手动重试）', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        db: {
          'a.txt': dbEntry('$pendingFileIdPrefix${'a.txt'}',
              status: SyncItemStatus.Failed),
        },
      ));
      expect(actions, isEmpty);
    });

    test('启动恢复期 + 本地未改 → Skip（删除守卫，被过滤）', () {
      final actions = planner.plan(snap(
        local: {'a.txt': localEntry('a.txt')},
        db: {'a.txt': dbEntry('f1')},
        startup: true,
      ));
      expect(actions, isEmpty);
    });
  });

  group('本地无 + 云端有', () {
    test('云端新文件 → CreatePlaceholder', () {
      final actions = planner.plan(snap(
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
      ));
      expect(actions.single.actionType, SyncActionType.createPlaceholder);
      expect(actions.single.fileId, 'f1');
    });

    test('会话内删除 → DeleteFromCloud', () {
      final actions = planner.plan(snap(
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions.single.actionType, SyncActionType.deleteFromCloud);
    });

    test('启动恢复期 + tombstone → 跳过（不重建）', () {
      final actions = planner.plan(snap(
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1', status: SyncItemStatus.Deleted)},
        startup: true,
      ));
      expect(actions, isEmpty);
    });

    test('启动恢复期 + 有 DB 非 tombstone → CreatePlaceholder（重建占位）', () {
      final actions = planner.plan(snap(
        cloud: {'a.txt': cloudFile('f1', 'a.txt', editedMs: 1700000000000)},
        db: {'a.txt': dbEntry('f1')},
        startup: true,
      ));
      expect(actions.single.actionType, SyncActionType.createPlaceholder);
    });

    test('云端文件夹 + 本地缺失 + 无 DB → CreateFolder', () {
      final actions = planner.plan(snap(
        cloud: {'dir': cloudFile('f1', 'dir', isFolder: true)},
      ));
      expect(actions.single.actionType, SyncActionType.createFolder);
      expect(actions.single.cloudFile?.id, 'f1');
    });

    test('云端文件夹 + 会话内本地删除 → DeleteFromCloud', () {
      final actions = planner.plan(snap(
        cloud: {'dir': cloudFile('f1', 'dir', isFolder: true)},
        db: {'dir': dbEntry('f1', isFolder: true)},
      ));
      expect(actions.single.actionType, SyncActionType.deleteFromCloud);
    });

    test('双方都有文件夹 → 无动作', () {
      final actions = planner.plan(snap(
        local: {'dir': localEntry('dir', isFolder: true)},
        cloud: {'dir': cloudFile('f1', 'dir', isFolder: true)},
        db: {'dir': dbEntry('f1', isFolder: true)},
      ));
      expect(actions, isEmpty);
    });
  });

  group('可信边界', () {
    test('云端不可信 → 抑制双向删除动作', () {
      final actions = planner.plan(snap(
        local: {'gone.txt': localEntry('gone.txt')},
        cloud: {'c.txt': cloudFile('f2', 'c.txt', editedMs: 1700000000000)},
        db: {
          'gone.txt': dbEntry('f1'),
          'c.txt': dbEntry('f2'),
        },
        trusted: false,
      ));
      // DeleteFromLocal（gone.txt）与 DeleteFromCloud（c.txt）都被抑制
      expect(
          actions.where((a) =>
              a.actionType == SyncActionType.deleteFromLocal ||
              a.actionType == SyncActionType.deleteFromCloud),
          isEmpty);
    });

    test('云端不可信 → 上传/占位等非删除动作保留', () {
      final actions = planner.plan(snap(
        local: {'new.txt': localEntry('new.txt')},
        cloud: {'c.txt': cloudFile('f2', 'c.txt', editedMs: 1700000000000)},
        trusted: false,
      ));
      expect(actions.map((a) => a.actionType),
          containsAll([SyncActionType.upload, SyncActionType.createPlaceholder]));
    });
  });

  group('双方都删了', () {
    test('本地无 + 云端无 + DB 有 → 无动作（engine 周期末清残余）', () {
      final actions = planner.plan(snap(
        db: {'a.txt': dbEntry('f1')},
      ));
      expect(actions, isEmpty);
    });
  });
}
