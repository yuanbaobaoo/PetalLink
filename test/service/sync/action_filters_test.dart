import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/engine/action_filters.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/sync_actions.dart';

/// 动作过滤器测试（对齐 Rust `src/sync/engine/action_filters.rs`）。
void main() {
  group('addRescueFolderRecreations', () {
    LocalFileEntry entry(String abs) => LocalFileEntry(
          absolutePath: abs,
          relativePath: abs,
          size: 0,
          mtime: 0,
          isFolder: true,
          isPlaceholder: false,
        );

    test('existing 集合含全部动作路径（对齐 Rust：不只 CreateFolder）', () {
      final actions = [
        // 已有动作覆盖 'dir'（非 CreateFolder 类型）
        SyncAction(
            actionType: SyncActionType.deleteFromCloud, relativePath: 'dir'),
        // 救援源：dir 内文件上传
        SyncAction(
            actionType: SyncActionType.upload,
            relativePath: 'dir/sub/file.txt'),
      ];
      addRescueFolderRecreations(
        actions,
        local: {
          'dir': entry('/mnt/dir'),
          'dir/sub': entry('/mnt/dir/sub'),
        },
        cloud: const {},
        db: const {
          'dir': DbSnapshotEntry(fileId: 'd'),
          'dir/sub': DbSnapshotEntry(fileId: 's'),
        },
        recentlyDeletedPaths: const {},
        mountDir: '/mnt',
      );

      // 'dir' 已被既有动作覆盖 → 不得重复追加 CreateFolder('dir')
      // （修复前 existing 仅含 CreateFolder 路径，会误追加）
      final dirCreations = actions.where((a) =>
          a.actionType == SyncActionType.createFolder &&
          a.relativePath == 'dir');
      expect(dirCreations, isEmpty);
      // 'dir/sub' 无动作覆盖 → 应救援重建
      expect(
        actions.any((a) =>
            a.actionType == SyncActionType.createFolder &&
            a.relativePath == 'dir/sub'),
        isTrue,
      );
    });
  });
}
