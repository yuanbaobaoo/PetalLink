/// planner 动作过滤链（对齐 Rust `src/sync/engine/action_filters.rs`）。
///
/// 固定调用顺序：
/// `filterSkippedPaths` → `detectRenames`（reconciliation.dart）→
/// `filterActiveTransferActions` → `filterAntiOscillation` →
/// `fillParentFileIds` → `addRescueFolderRecreations` →
/// `filterBlockedPathChanges` → `validateDeleteFromCloud` →
/// `dedupeDirectoryDeletes` → `dedupeLocalDescendants` →
/// `preserveDirsWithPendingBackups`。
library;

import 'dart:io';

import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/skip.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// 路径重叠（祖先/等于，双向）。
bool pathsOverlap(String a, String b) =>
    a == b || a.startsWith('$b/') || b.startsWith('$a/');

/// 路径/身份是否被路径恢复隔离（fileId 命中，或路径与新旧路径重叠）。
bool isBlockedPathIdentity(
  String path,
  String? fileId,
  List<BlockedPathChange> blocked,
) {
  for (final change in blocked) {
    if (fileId != null && fileId == change.fileId) return true;
    if (pathsOverlap(path, change.oldPath) ||
        pathsOverlap(path, change.newPath)) {
      return true;
    }
  }
  return false;
}

/// 移除命中跳过模式的相对路径动作。
void filterSkippedPaths(List<SyncAction> actions, List<String> skipPatterns) {
  actions.removeWhere((action) {
    final rel = action.relativePath;
    if (rel == null) return false;
    return MountSkip.shouldSkipRelativePath(rel, skipPatterns);
  });
}

/// 活动传输定义（对齐 Rust filter/path_recovery 口径）：
/// 阻塞态（Pending/Running/WaitingForNetwork/BackingOff/VerifyingRemote）
/// 或 RestartRequired 且 remote_result_file_id 非空白。
const String _activeTransferWhere =
    '(state IN (?, ?, ?, ?, ?) OR (state = ? AND '
    'remote_result_file_id IS NOT NULL AND trim(remote_result_file_id) <> ?))';

final List<Object?> _activeTransferArgs = [
  TransferState.pending.code,
  TransferState.running.code,
  TransferState.waitingForNetwork.code,
  TransferState.backingOff.code,
  TransferState.verifyingRemote.code,
  TransferState.restartRequired.code,
  '',
];

/// 查询全部活动传输任务。
Future<List<TransferTask>> queryActiveTransfers(Database db) async {
  final rows = await db.rawQuery(
    'SELECT * FROM transfer_queue WHERE $_activeTransferWhere '
    'ORDER BY created_at ASC',
    _activeTransferArgs,
  );
  return rows.map(TransferTask.fromRow).toList();
}

/// 移除与活动传输冲突的动作（同 fileId 或路径重叠，含源路径重叠）。
void filterActiveTransferActions(
  List<SyncAction> actions,
  List<TransferTask> activeTasks,
  Map<String, DbSnapshotEntry> dbSnapshot,
) {
  if (activeTasks.isEmpty) return;
  // fileId → DB 登记路径（源路径）
  final sourcePathById = <String, String>{};
  for (final entry in dbSnapshot.entries) {
    sourcePathById[entry.value.fileId] = entry.key;
  }
  actions.removeWhere((action) {
    final rel = action.relativePath;
    if (rel == null) return false;
    final fileId = action.fileId;
    final sourcePath = fileId != null ? sourcePathById[fileId] : null;
    for (final task in activeTasks) {
      if (fileId != null &&
          fileId.isNotEmpty &&
          task.fileId != null &&
          task.fileId == fileId) {
        AppLogger.d('动作被同 fileId 活动传输 ${task.id} 过滤: $rel');
        return true;
      }
      final taskRel = task.relativePath;
      if (taskRel != null) {
        if (pathsOverlap(taskRel, rel)) {
          AppLogger.d('动作被同路径活动传输 ${task.id} 过滤: $rel');
          return true;
        }
        if (sourcePath != null && pathsOverlap(taskRel, sourcePath)) {
          AppLogger.d('动作被源路径活动传输 ${task.id} 过滤: $rel');
          return true;
        }
      }
    }
    return false;
  });
}

/// 反振荡：recently_deleted 中的路径，除 DeleteFromCloud（删除确认）外一律移除。
void filterAntiOscillation(
  List<SyncAction> actions,
  Map<String, int> recentlyDeletedPaths,
) {
  actions.removeWhere((action) {
    final rel = action.relativePath;
    if (rel == null) return false;
    if (!recentlyDeletedPaths.containsKey(rel)) return false;
    return action.actionType != SyncActionType.deleteFromCloud;
  });
}

/// 回填缺失的父目录 fileId（从 pathToId 反查父路径）。
void fillParentFileIds(
  List<SyncAction> actions,
  Map<String, String> pathToId,
) {
  for (final action in actions) {
    final rel = action.relativePath;
    if (action.parentFileId != null || rel == null || !rel.contains('/')) {
      continue;
    }
    final parent = rel.substring(0, rel.lastIndexOf('/'));
    action.parentFileId = pathToId[parent];
  }
}

/// 救援性目录重建：动作的所有祖先目录中，本地有、云端没有、DB 有、
/// 且不在 recentlyDeleted 的，追加 CreateFolder（云端已删除但内有内容需救援）。
void addRescueFolderRecreations(
  List<SyncAction> actions, {
  required Map<String, dynamic> local,
  required Map<String, DriveFile> cloud,
  required Map<String, DbSnapshotEntry> db,
  required Map<String, int> recentlyDeletedPaths,
  required String mountDir,
}) {
  // 已有动作的路径全集（对齐 Rust：所有动作类型，不只 CreateFolder）——
  // 祖先目录已被任何动作覆盖时不再追加救援重建
  final existing = actions
      .map((a) => a.relativePath)
      .whereType<String>()
      .toSet();
  final rescues = <String>{};
  const rescueTypes = {
    SyncActionType.upload,
    SyncActionType.moveInCloud,
    SyncActionType.backupBeforeCloudDelete,
    SyncActionType.createConflictCopy,
  };
  for (final action in actions) {
    final rel = action.relativePath;
    if (rel == null) continue;
    final isRescueSource = rescueTypes.contains(action.actionType) ||
        (action.actionType == SyncActionType.createFolder &&
            action.cloudFile == null);
    if (!isRescueSource) continue;
    // 全部祖先前缀
    var index = rel.indexOf('/');
    while (index > 0) {
      final ancestor = rel.substring(0, index);
      if (local.containsKey(ancestor) &&
          !cloud.containsKey(ancestor) &&
          db.containsKey(ancestor) &&
          !recentlyDeletedPaths.containsKey(ancestor) &&
          !existing.contains(ancestor)) {
        rescues.add(ancestor);
      }
      index = rel.indexOf('/', index + 1);
    }
  }
  // 按深度升序（父先建）
  final sorted = rescues.toList()
    ..sort((a, b) => pathDepth(a).compareTo(pathDepth(b)));
  for (final ancestor in sorted) {
    actions.add(SyncAction(
      actionType: SyncActionType.createFolder,
      relativePath: ancestor,
      localPath: local[ancestor]?.absolutePath as String?,
      reason: '云端已删除但内有内容需救援 → 重建目录到云端',
    ));
  }
}

/// 移除被路径恢复隔离的动作（含被隔离结构根子树内的动作）。
void filterBlockedPathChanges(
  List<SyncAction> actions,
  List<BlockedPathChange> blocked,
) {
  if (blocked.isEmpty) return;
  // 先收集被隔离的 CreateFolder 根
  final blockedFolderRoots = <String>[];
  for (final action in actions) {
    final rel = action.relativePath;
    if (rel == null) continue;
    if (action.actionType == SyncActionType.createFolder &&
        isBlockedPathIdentity(rel, action.fileId, blocked)) {
      blockedFolderRoots.add(rel);
    }
  }
  actions.removeWhere((action) {
    final rel = action.relativePath;
    if (rel == null) return false;
    if (isBlockedPathIdentity(rel, action.fileId, blocked)) return true;
    return blockedFolderRoots.any((root) => isInSubtree(rel, root));
  });
}

/// 云端删除复核（防误删）：本地实际存在的非占位文件改为 Skip。
Future<void> validateDeleteFromCloud(
  List<SyncAction> actions,
  MountManager mount,
) async {
  for (final action in actions) {
    if (action.actionType != SyncActionType.deleteFromCloud) continue;
    final rel = action.relativePath;
    if (rel == null) continue;
    final abs = '${mount.mountDir}/$rel';
    final type = await FileSystemEntity.type(abs, followLinks: false);
    if (type == FileSystemEntityType.notFound) continue; // 本地确实没有 → 保留删除
    if (type == FileSystemEntityType.link) {
      action.actionType = SyncActionType.skip;
      action.reason = '本地路径为符号链接，无法证明文件已删除，跳过 DeleteFromCloud';
      continue;
    }
    final stat = await FileStat.stat(abs);
    if (stat.size == 0 && await mount.isPlaceholderFile(abs)) {
      continue; // 占位符 → 保留删除
    }
    action.actionType = SyncActionType.skip;
    action.reason = '防误删：本地文件实际存在（${stat.size} 字节），跳过 DeleteFromCloud';
    AppLogger.w(action.reason!);
  }
}

/// 云端目录删除去重：仅保留祖先（目录删除会级联）。
///
/// 仅当 planner 明确产生「云端确实是目录」的 DeleteFromCloud（云树中
/// is_folder）时，移除严格位于其下的其他 DeleteFromCloud。
void dedupeDirectoryDeletes(
  List<SyncAction> actions,
  Map<String, DriveFile> cloudTree,
) {
  final dirDeletes = actions
      .where((a) =>
          a.actionType == SyncActionType.deleteFromCloud &&
          a.relativePath != null &&
          cloudTree[a.relativePath]?.isFolder == true)
      .map((a) => a.relativePath!)
      .toList();
  if (dirDeletes.isEmpty) return;
  actions.removeWhere((action) {
    if (action.actionType != SyncActionType.deleteFromCloud) return false;
    final rel = action.relativePath;
    if (rel == null) return false;
    // 严格位于某目录删除之下（不含根本身）
    return dirDeletes
        .any((root) => rel != root && rel.startsWith('$root/'));
  });
}

/// 本地删除去重：祖先在 DeleteFromLocal 列表中的子孙动作移除。
void dedupeLocalDescendants(List<SyncAction> actions) {
  final localDeletes = actions
      .where((a) => a.actionType == SyncActionType.deleteFromLocal)
      .map((a) => a.relativePath)
      .whereType<String>()
      .toSet();
  if (localDeletes.length < 2) return;
  actions.removeWhere((action) {
    if (action.actionType != SyncActionType.deleteFromLocal) return false;
    final rel = action.relativePath;
    if (rel == null) return false;
    return localDeletes.any((root) => rel != root && rel.startsWith('$root/'));
  });
}

/// 保留有待处理备份的目录：DeleteFromLocal 目标目录下存在
/// BackupBeforeCloudDelete 路径时，移除该 DeleteFromLocal（给备份副本留栖身目录）。
void preserveDirsWithPendingBackups(List<SyncAction> actions) {
  final backupPaths = actions
      .where((a) => a.actionType == SyncActionType.backupBeforeCloudDelete)
      .map((a) => a.relativePath)
      .whereType<String>()
      .toList();
  if (backupPaths.isEmpty) return;
  actions.removeWhere((action) {
    if (action.actionType != SyncActionType.deleteFromLocal) return false;
    final rel = action.relativePath;
    if (rel == null) return false;
    return backupPaths.any((bp) => bp.startsWith('$rel/'));
  });
}
