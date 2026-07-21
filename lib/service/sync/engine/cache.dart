/// 引擎云树缓存切片（对齐 Rust `src/sync/engine/cache.rs`）。
///
/// 管理 live 云树状态（tree/pathToId/rootFolderId/cursor/trusted/
/// incrementalSinceFull）与 checkpoint 的加载、安装、全量/增量刷新。
/// 所有破坏性决策（reconcile、墓碑清理、规划产生成功基线、free-up）
/// 都要求 trusted==true；任何刷新失败立即撤销信任（fail-closed）。
library;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/sync/cloud_tree.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/action_filters.dart';
import 'package:petal_link/service/sync/engine/coordination.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/types/enums.dart';

// RecoveredCloudFile 统一定义在 task_runner_contracts（引擎与 runner 共享），
// 此处 import + re-export 保持既有引用路径不变。
import 'package:petal_link/service/transfer/task_runner_contracts.dart'
    show RecoveredCloudFile;
export 'package:petal_link/service/transfer/task_runner_contracts.dart'
    show RecoveredCloudFile;

/// live 云树索引（引擎内存态）。
class CloudTreeIndex {
  CloudTreeIndex();

  /// 云端树（relPath → DriveFile）
  final Map<String, DriveFile> tree = {};

  /// 路径索引（relPath → fileId，含 `""` → rootFolderId）
  final Map<String, String> pathToId = {};

  /// 根目录 fileId
  String? rootFolderId;

  /// 当前增量游标
  String? cursor;

  /// 是否可信（仅当来自完整、崩溃一致的 checkpoint）
  bool trusted = false;

  /// 连续增量计数（达阈值强制全量）
  int incrementalSinceFull = 0;

  /// 安装 checkpoint 到 live 状态（先撤信任再换数据，最后恢复信任）。
  void install(CloudTreeCache checkpoint, {bool trusted = true}) {
    this.trusted = false;
    tree
      ..clear()
      ..addAll(checkpoint.tree);
    pathToId
      ..clear()
      ..addAll(checkpoint.pathToId);
    rootFolderId = checkpoint.rootFolderId;
    cursor = checkpoint.cursor;
    this.trusted = trusted;
  }

  /// 撤销信任（旧树仅供展示，禁止驱动破坏性操作）。
  void revokeTrust() {
    trusted = false;
  }

  /// 插入/更新单条云树条目（同步 pathToId）。
  void insert(String relPath, DriveFile file) {
    tree[relPath] = file;
    pathToId[relPath] = file.id;
  }

  /// 移除单条云树条目（不级联）。
  void remove(String relPath) {
    tree.remove(relPath);
    pathToId.remove(relPath);
  }

  /// 移除整棵子树。
  void removeSubtree(String root) {
    final doomed =
        tree.keys.where((k) => k == root || k.startsWith('$root/')).toList();
    for (final k in doomed) {
      tree.remove(k);
      pathToId.remove(k);
    }
  }

  /// 同 fileId 的其他路径（陈旧路径清理用）。
  List<String> otherPathsOf(String fileId, String keepPath) {
    return tree.entries
        .where((e) => e.value.id == fileId && e.key != keepPath)
        .map((e) => e.key)
        .toList();
  }
}

/// 引擎云树缓存 mixin（对齐 Rust engine/cache.rs 的 impl 切片）。
mixin EngineCache on SyncEngineBase {
  /// 云树是否可信
  @override
  bool cloudTreeIsTrusted() => cloudIndex.trusted;

  /// 启动期加载或全量重建云树（对齐 Rust `load_or_refresh_cloud_tree`）。
  ///
  /// 返回 true = 安装了持久 checkpoint（置不可信，仍需增量 catch-up）；
  /// false = 已完成可信全量构建。
  @override
  Future<bool> loadOrRefreshCloudTree() async {
    final persisted = await loadPersistedCloudTree(mountDir);
    if (persisted != null) {
      // catch-up 前不得驱动破坏性决策
      cloudIndex.install(persisted, trusted: false);
      AppLogger.i('已安装持久 checkpoint（待增量追平）：${persisted.tree.length} 项');
      return true;
    }
    await updateRuntimeAndBroadcast((r) {
      r.isIndexing = true;
      r.syncPhase = SyncPhase.indexingStartup;
    });
    try {
      await buildAndCommitFullCheckpoint();
    } catch (e) {
      cloudIndex.revokeTrust();
      restoreIdleRuntimeAfterError();
      rethrow;
    }
    await updateRuntimeAndBroadcast((r) {
      r.isIndexing = false;
      r.indexingScannedFolders = 0;
      r.indexingDiscoveredItems = 0;
      r.syncPhase = null;
    });
    return false;
  }

  /// 全量构建并提交可信 checkpoint（对齐 Rust
  /// `build_and_commit_full_checkpoint`）。
  @override
  Future<void> buildAndCommitFullCheckpoint() async {
    try {
      final startCursor = (await changesApi.getStartCursor()).unwrap();
      final candidate = await refreshCloudTree(
        filesApi,
        onProgress: (scanned, items) {
          // 索引进度尽力发布（不阻断扫描）
          updateRuntimeAndBroadcast((r) {
            r.isIndexing = true;
            r.indexingScannedFolders = scanned;
            r.indexingDiscoveredItems = items;
          }).catchError((_) => state);
        },
      );
      final catchUp = (await changesApi.listAllChanges(startCursor)).unwrap();
      applyChangesToCandidate(
        catchUp.changes,
        candidate.tree,
        candidate.pathToId,
        candidate.rootFolderId,
      );
      final checkpoint = CloudTreeCache.newTrusted(
        candidate.rootFolderId,
        candidate.tree,
        candidate.pathToId,
        catchUp.checkpoint,
      );
      await persistCloudCheckpoint(mountDir, checkpoint);
      cloudIndex.install(checkpoint);
      cloudIndex.incrementalSinceFull = 0;
    } catch (e) {
      // 失败保留旧树展示但撤销破坏性操作信任
      cloudIndex.revokeTrust();
      rethrow;
    }
  }

  /// 周期内云端全量刷新（手动，对齐 Rust `refresh_cloud_full_for_cycle`）。
  @override
  Future<void> refreshCloudFullForCycle() async {
    await updateRuntimeAndBroadcast((r) {
      r.isIndexing = true;
      r.syncPhase = SyncPhase.indexingManual;
    });
    try {
      await buildAndCommitFullCheckpoint();
    } catch (e) {
      restoreIdleRuntimeAfterError();
      rethrow;
    }
    await updateRuntimeAndBroadcast((r) {
      r.isIndexing = false;
      r.indexingScannedFolders = 0;
      r.indexingDiscoveredItems = 0;
    });
  }

  /// 周期内云端增量刷新（对齐 Rust `refresh_cloud_incremental_for_cycle`）。
  @override
  Future<void> refreshCloudIncrementalForCycle() async {
    await updateRuntimeAndBroadcast((r) {
      r.syncPhase = SyncPhase.queryingChanges;
    });
    await tryIncrementalOrFullRefresh();
    await updateRuntimeAndBroadcast((r) {
      r.syncPhase = SyncPhase.syncingAutoIncremental;
    });
  }

  /// 增量优先、连续 300 次强制全量（对齐 Rust
  /// `try_incremental_or_full_refresh`；任何增量失败回退全量，fail-closed）。
  @override
  Future<void> tryIncrementalOrFullRefresh() async {
    if (cloudIndex.incrementalSinceFull >=
        SyncEngineBase.incrementalForcedFullThreshold) {
      AppLogger.i('连续 ${cloudIndex.incrementalSinceFull} 次增量，强制全量 BFS');
      await updateRuntimeAndBroadcast((r) {
        r.isIndexing = true;
        r.syncPhase = SyncPhase.indexingAutoFull;
      });
      try {
        await buildAndCommitFullCheckpoint();
      } finally {
        await updateRuntimeAndBroadcast((r) {
          r.isIndexing = false;
          r.indexingScannedFolders = 0;
          r.indexingDiscoveredItems = 0;
        });
      }
      return;
    }
    final cursor = cloudIndex.cursor;
    if (cursor == null || cursor.trim().isEmpty) {
      await buildAndCommitFullCheckpoint();
      return;
    }
    try {
      final catchUp = (await changesApi.listAllChanges(cursor)).unwrap();
      // 在克隆候选上回放，全部成功才提交
      final candidateTree = Map<String, DriveFile>.of(cloudIndex.tree);
      final candidateIndex = Map<String, String>.of(cloudIndex.pathToId);
      applyChangesToCandidate(
        catchUp.changes,
        candidateTree,
        candidateIndex,
        cloudIndex.rootFolderId,
      );
      final checkpoint = CloudTreeCache.newTrusted(
        cloudIndex.rootFolderId,
        candidateTree,
        candidateIndex,
        catchUp.checkpoint,
      );
      await persistCloudCheckpoint(mountDir, checkpoint);
      cloudIndex.install(checkpoint);
      cloudIndex.incrementalSinceFull++;
    } catch (e) {
      AppLogger.w('增量刷新失败，回退全量 BFS: $e');
      cloudIndex.revokeTrust();
      await updateRuntimeAndBroadcast((r) {
        r.isIndexing = true;
        r.syncPhase = SyncPhase.indexingAutoFull;
      });
      try {
        await buildAndCommitFullCheckpoint();
      } finally {
        await updateRuntimeAndBroadcast((r) {
          r.isIndexing = false;
          r.indexingScannedFolders = 0;
          r.indexingDiscoveredItems = 0;
        });
      }
    }
  }

  /// 提交任务恢复确认的远端文件（对齐 Rust `commit_recovered_cloud_files`）。
  @override
  Future<void> commitRecoveredCloudFiles(
    List<RecoveredCloudFile> recovered,
  ) async {
    if (recovered.isEmpty) return;
    if (!cloudIndex.trusted) {
      throw AppError.generic('云端 checkpoint 不可信，拒绝发布恢复结果');
    }
    final candidateTree = Map<String, DriveFile>.of(cloudIndex.tree);
    final candidateIndex = Map<String, String>.of(cloudIndex.pathToId);
    for (final item in recovered) {
      // 删同 fileId 的其他陈旧路径
      final stale = candidateTree.entries
          .where((e) =>
              e.value.id == item.file.id && e.key != item.relativePath)
          .map((e) => e.key)
          .toList();
      for (final path in stale) {
        candidateTree.remove(path);
        candidateIndex.remove(path);
      }
      candidateTree[item.relativePath] = item.file;
      candidateIndex[item.relativePath] = item.file.id;
    }
    final cursor = cloudIndex.cursor;
    if (cursor == null || cursor.trim().isEmpty) {
      throw AppError.generic('云端 checkpoint 缺少有效 cursor');
    }
    final checkpoint = CloudTreeCache.newTrusted(
      cloudIndex.rootFolderId,
      candidateTree,
      candidateIndex,
      cursor,
    );
    try {
      await persistCloudCheckpoint(mountDir, checkpoint);
    } catch (e) {
      cloudIndex.revokeTrust();
      rethrow;
    }
    cloudIndex.install(checkpoint);
  }

  /// 提交恢复结果；失败时请求全量重建可信视图
  /// （对齐 Rust `commit_recovery_checkpoint`）。
  @override
  Future<void> commitRecoveryCheckpoint(
    List<RecoveredCloudFile> recovered,
  ) async {
    try {
      await commitRecoveredCloudFiles(recovered);
    } catch (e) {
      AppLogger.w('恢复结果提交失败，请求全量重建: $e');
      cycle.request(CycleRequest.of(
          [CycleRequest.localRescan, CycleRequest.cloudFull]));
      rethrow;
    }
  }

  /// 清理 DELETED 墓碑（仅可信时；对齐 Rust
  /// `purge_deleted_tombstones_if_trusted`）。
  @override
  Future<void> purgeDeletedTombstonesIfTrusted(
    List<BlockedPathChange> blocked,
  ) async {
    if (!cloudIndex.trusted) {
      throw AppError.generic('云端 checkpoint 不可信，拒绝清理删除墓碑');
    }
    final db = await this.db.database;
    final rows = await db.query('sync_items',
        columns: ['local_path', 'file_id'],
        where: 'status = ?',
        whereArgs: [SyncItemStatus.deleted.code]);
    for (final row in rows) {
      final path = row['local_path'] as String? ?? '';
      final fileId = row['file_id'] as String? ?? '';
      if (path.isEmpty) continue;
      if (isBlockedPathIdentity(path, fileId, blocked)) continue;
      if (cloudIndex.tree.containsKey(path)) continue;
      await db.delete('sync_items',
          where: 'local_path = ? AND status = ?',
          whereArgs: [path, SyncItemStatus.deleted.code]);
    }
  }
}
