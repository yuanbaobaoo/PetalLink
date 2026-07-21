import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/engine/cache.dart';
import 'package:petal_link/service/sync/identity/detect_moves.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/entity/drive_file.dart';

/// inode 移动检测与动作合并测试（对标 CMP `InodeMoveDetector` +
/// `LocalMoveActionReconciler`，设计 docs/design/10 §4.3）。
void main() {
  LocalFileEntry entry(String rel, int inode) => LocalFileEntry(
        absolutePath: '/mnt/$rel',
        relativePath: rel,
        size: 10,
        mtime: 1000,
        isFolder: false,
        isPlaceholder: false,
        inode: inode,
      );

  group('detectMoves', () {
    test('同 inode 路径变化 → 识别为移动并更新映射', () async {
      final identity = MemoryInodeIdentityStore();
      await identity.upsert(100, 'a.txt', 'fid-a');

      final moves = await detectMoves([entry('b.txt', 100)], identity);

      expect(moves, hasLength(1));
      expect(moves.single.oldRelativePath, 'a.txt');
      expect(moves.single.newRelativePath, 'b.txt');
      expect(moves.single.fileId, 'fid-a');
      // 检测后立即更新映射（确定性记账）
      expect((await identity.lookup(100))!.relativePath, 'b.txt');
    });

    test('复制（新 inode）不识别为移动，副本当新文件', () async {
      final identity = MemoryInodeIdentityStore();
      await identity.upsert(100, 'a.txt', 'fid-a');

      // cp a.txt b.txt → b.txt 是新 inode
      final moves =
          await detectMoves([entry('a.txt', 100), entry('b.txt', 200)], identity);

      expect(moves, isEmpty);
    });

    test('路径未变 → 无移动', () async {
      final identity = MemoryInodeIdentityStore();
      await identity.upsert(100, 'a.txt', 'fid-a');
      final moves = await detectMoves([entry('a.txt', 100)], identity);
      expect(moves, isEmpty);
    });

    test('未知 inode → 无移动（新文件）', () async {
      final identity = MemoryInodeIdentityStore();
      final moves = await detectMoves([entry('new.txt', 300)], identity);
      expect(moves, isEmpty);
    });
  });

  group('collapseNested（目录整体移动折叠）', () {
    test('目录移动只保留最上层，子项移动被折叠', () {
      final moves = [
        DetectedMove(
            inode: 1, fileId: 'd', oldRelativePath: 'a', newRelativePath: 'b'),
        DetectedMove(
            inode: 2,
            fileId: 'f',
            oldRelativePath: 'a/x.txt',
            newRelativePath: 'b/x.txt'),
      ];
      final collapsed = collapseNested(moves);
      expect(collapsed, hasLength(1));
      expect(collapsed.single.oldRelativePath, 'a');
    });

    test('独立移动不折叠', () {
      final moves = [
        DetectedMove(
            inode: 1, fileId: 'f1', oldRelativePath: 'a.txt', newRelativePath: 'b.txt'),
        DetectedMove(
            inode: 2, fileId: 'f2', oldRelativePath: 'c.txt', newRelativePath: 'd.txt'),
      ];
      expect(collapseNested(moves), hasLength(2));
    });
  });

  group('applyDetectedMoves（planner 动作合并）', () {
    CloudTreeIndex cloudWith(String path, String id, {String? parentId}) {
      final cloud = CloudTreeIndex();
      cloud.rootFolderId = 'root';
      cloud.insert(
          path,
          DriveFile(
              id: id,
              name: path.split('/').last,
              size: 10,
              parentFolder: [parentId ?? 'root']));
      return cloud;
    }

    test('替换 upload+deleteFromCloud 为 MoveInCloud', () {
      final cloud = cloudWith('a.txt', 'fid-a');
      final actions = [
        SyncAction(actionType: SyncActionType.upload, relativePath: 'b.txt'),
        SyncAction(
            actionType: SyncActionType.deleteFromCloud,
            relativePath: 'a.txt',
            fileId: 'fid-a'),
      ];
      final moves = [
        DetectedMove(
            inode: 1, fileId: 'fid-a', oldRelativePath: 'a.txt', newRelativePath: 'b.txt'),
      ];

      applyDetectedMoves(actions, moves, cloud);

      expect(actions, hasLength(1));
      final move = actions.single;
      expect(move.actionType, SyncActionType.moveInCloud);
      expect(move.relativePath, 'b.txt');
      expect(move.fileId, 'fid-a');
      expect(move.cloudFile?.id, 'fid-a');
      expect(move.parentFileId, 'root');
    });

    test('云树缺旧路径条目 → 跳过该移动（保守）', () {
      final cloud = CloudTreeIndex()..rootFolderId = 'root';
      final actions = [
        SyncAction(actionType: SyncActionType.upload, relativePath: 'b.txt'),
      ];
      applyDetectedMoves(
        actions,
        [DetectedMove(inode: 1, fileId: 'fid-a', oldRelativePath: 'a.txt', newRelativePath: 'b.txt')],
        cloud,
      );
      // 无法证明云端身份 → 保留原动作（按删+增处理）
      expect(actions.single.actionType, SyncActionType.upload);
    });

    test('目录移动：子树动作被移除，仅保留顶层 MoveInCloud', () {
      final cloud = CloudTreeIndex()..rootFolderId = 'root';
      cloud.insert('a', DriveFile(id: 'dir-a', name: 'a', size: 0, parentFolder: ['root']));
      cloud.insert('a/x.txt', DriveFile(id: 'fid-x', name: 'x.txt', size: 1, parentFolder: ['dir-a']));
      final actions = [
        SyncAction(actionType: SyncActionType.deleteFromCloud, relativePath: 'a', fileId: 'dir-a'),
        SyncAction(actionType: SyncActionType.deleteFromCloud, relativePath: 'a/x.txt', fileId: 'fid-x'),
        SyncAction(actionType: SyncActionType.upload, relativePath: 'b/x.txt'),
      ];
      final moves = [
        DetectedMove(inode: 1, fileId: 'dir-a', oldRelativePath: 'a', newRelativePath: 'b'),
        DetectedMove(inode: 2, fileId: 'fid-x', oldRelativePath: 'a/x.txt', newRelativePath: 'b/x.txt'),
      ];

      applyDetectedMoves(actions, moves, cloud);

      expect(actions, hasLength(1));
      expect(actions.single.actionType, SyncActionType.moveInCloud);
      expect(actions.single.relativePath, 'b');
      expect(actions.single.fileId, 'dir-a');
    });
  });
}
