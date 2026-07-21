/// 引擎对账切片（对齐 Rust `src/sync/engine/reconciliation.rs`）。
///
/// 用可信云树 + 本地身份补 DB 基线（崩溃窗口收敛）、复核 FAILED 记录、
/// 清理双端缺席残余、xattr fileId 识别同目录改名（Upload+DeleteFromCloud
/// → MoveInCloud）、free-up 安全判定。
library;

import 'dart:io';

import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/action_filters.dart';
import 'package:petal_link/service/sync/engine/coordination.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// FAILED 记录复核汇总（对齐 Rust `FailedRecordReconciliation`）。
class FailedRecordReconciliation {
  /// 复核收敛为已同步的记录数
  int healed = 0;

  /// 清理的双端缺席残余数
  int purged = 0;

  /// 复核后仍失败的记录数
  int remainingFailed = 0;

  /// 因活动传输守卫而无法结算的记录数
  int transferBlocked = 0;
}

/// 引擎对账 mixin。
mixin EngineReconciliation on SyncEngineBase {
  // ═══════════════════════════════════════════════════════════════════
  // 本地扫描与 DB 快照
  // ═══════════════════════════════════════════════════════════════════

  /// 扫描本地挂载目录（对齐 Rust `scan_local`）。
  @override
  Future<Map<String, LocalFileEntry>> scanLocal() async {
    final m = mount;
    if (m == null) {
      throw AppError.config('挂载管理器未配置');
    }
    final entries = await m.scanLocal(skipPatterns);
    return {for (final e in entries) e.relativePath: e};
  }

  /// 加载 DB 基线快照（relPath → 快照；重复 local_path 报错拒绝规划）。
  @override
  Future<Map<String, DbSnapshotEntry>> loadDbSnapshot() async {
    final db = await this.db.database;
    final rows = await db.query('sync_items');
    final out = <String, DbSnapshotEntry>{};
    for (final row in rows) {
      final item = SyncItem.fromRow(row);
      if (out.containsKey(item.localPath)) {
        throw AppError.generic('sync_items 存在重复 local_path：'
            '${item.localPath}，拒绝规划');
      }
      out[item.localPath] = DbSnapshotEntry.fromItem(item);
    }
    return out;
  }

  // ═══════════════════════════════════════════════════════════════════
  // DB 基线对账（可信云树 + 本地身份补基线）
  // ═══════════════════════════════════════════════════════════════════

  /// 用可信云树 + 本地身份补 DB 基线（对齐 Rust `reconcile_db_records`）。
  @override
  Future<void> reconcileDbRecords(
    Map<String, LocalFileEntry> local,
    Map<String, DbSnapshotEntry> dbSnapshot,
    List<BlockedPathChange> blocked,
  ) async {
    final m = mount;
    if (m == null) return;
    final db = await this.db.database;
    // fileId → DB 登记路径
    final pathById = <String, String>{};
    for (final entry in dbSnapshot.entries) {
      pathById[entry.value.fileId] = entry.key;
    }

    for (final entry in local.entries) {
      final rel = entry.key;
      final localEntry = entry.value;
      if (isBlockedPathIdentity(rel, null, blocked)) continue;

      final existing = dbSnapshot[rel];
      if (existing != null) {
        // DELETED tombstone 复活（本地仍存在 = 删除未生效或被重建）
        if (existing.status == SyncItemStatus.Deleted) {
          final next = localEntry.isPlaceholder
              ? SyncItemStatus.CloudOnly
              : SyncItemStatus.Synced;
          await db.update('sync_items', {'status': next.code},
              where: 'local_path = ?', whereArgs: [rel]);
        }
        continue;
      }

      final cloudFile = cloudIndex.tree[rel];

      // 目录：可信云树同路径且是目录 → 补完整基线
      // （目录无 fileId xattr，借此收敛「远端创建成功但 DB 未写入」的崩溃窗口）
      if (localEntry.isFolder) {
        if (cloudFile != null && cloudFile.isFolder) {
          await baselineStore.upsert(SyncItem(
            fileId: cloudFile.id,
            localPath: rel,
            parentFolderId: cloudFile.parentId,
            name: cloudFile.name,
            isFolder: true,
            size: cloudFile.size,
            localSize: 0,
            localMtime: localEntry.mtime,
            cloudEditedTime: cloudFile.editedTime?.millisecondsSinceEpoch,
            lastSyncTime: nowMs(),
            status: SyncItemStatus.Synced,
          ));
        }
        continue;
      }

      // 文件：必须凭 xattr fileId 身份补基线
      final status = localEntry.isPlaceholder
          ? SyncItemStatus.CloudOnly
          : SyncItemStatus.Synced;
      final String? xattrIdRaw;
      try {
        xattrIdRaw = await m.xattr.get(localEntry.absolutePath, xattrFileId);
      } catch (_) {
        continue;
      }
      if (xattrIdRaw == null || xattrIdRaw.isEmpty) {
        continue; // 本地新文件交 planner
      }
      final xattrId = xattrIdRaw;
      if (isBlockedPathIdentity(rel, xattrId, blocked)) continue;
      // 云树同路径不存在 → 禁止制造已同步基线
      if (cloudFile == null) continue;
      // 复制文件（xattr 与云树 id 不一致）交 planner
      if (cloudFile.id != xattrId) continue;

      // 同 fileId 旧路径记录存在 → 迁移（改名/移动收敛）
      final oldPath = pathById[xattrId];
      if (oldPath != null && oldPath != rel) {
        final oldAbs = '${m.mountDir}/$oldPath';
        final oldType =
            await FileSystemEntity.type(oldAbs, followLinks: false);
        if (oldType != FileSystemEntityType.notFound) {
          // 无法证明旧路径已消失（复制/歧义），拒绝迁移
          continue;
        }
        final db2 = await this.db.database;
        await db2.transaction((txn) async {
          final oldRows = await txn.query('sync_items',
              where: 'file_id = ? AND local_path <> ?',
              whereArgs: [xattrId, rel]);
          await txn.delete('sync_items',
              where: 'file_id = ? AND local_path <> ?',
              whereArgs: [xattrId, rel]);
          // 保留旧记录的内容字段（不把目标当前 mtime/size 误记为已同步）
          final old = oldRows.isNotEmpty
              ? SyncItem.fromRow(oldRows.first)
              : null;
          await txn.insert(
            'sync_items',
            SyncItem(
              fileId: xattrId,
              localPath: rel,
              parentFolderId: cloudFile.parentId,
              name: cloudFile.name,
              isFolder: old?.isFolder ?? false,
              size: cloudFile.size,
              localSize: old?.localSize,
              sha256: old?.sha256,
              localMtime: old?.localMtime,
              cloudEditedTime:
                  cloudFile.editedTime?.millisecondsSinceEpoch,
              lastSyncTime: old?.lastSyncTime,
              status: old?.status ?? status,
              errorMessage: old?.errorMessage,
            ).toRow(),
            conflictAlgorithm: ConflictAlgorithm.replace,
          );
        });
        cycle.request(CycleRequest.of([CycleRequest.localRescan]));
        continue;
      }

      // 补新基线
      await baselineStore.upsert(SyncItem(
        fileId: xattrId,
        localPath: rel,
        name: rel.split('/').last,
        isFolder: false,
        localSize: localEntry.isPlaceholder ? null : localEntry.size,
        localMtime: localEntry.mtime,
        lastSyncTime: nowMs(),
        status: status,
      ));
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // FAILED 复核 + 残余清理
  // ═══════════════════════════════════════════════════════════════════

  /// FAILED 记录复核与双端缺席残余清理（对齐 Rust
  /// `reconcile_failed_and_purge_stale_records`）。
  @override
  Future<FailedRecordReconciliation> reconcileFailedAndPurgeStaleRecords(
    Map<String, LocalFileEntry> local,
    Map<String, DriveFile> cloud,
    List<BlockedPathChange> blocked,
  ) async {
    final summary = FailedRecordReconciliation();
    final db = await this.db.database;
    const transferGuard = 'NOT EXISTS('
        'SELECT 1 FROM transfer_queue tq WHERE tq.relative_path = ? '
        'AND tq.state NOT IN (?, ?))';
    final nonTerminal = [
      TransferState.Completed.code,
      TransferState.Canceled.code,
    ];

    await db.transaction((txn) async {
      final rows = await txn.query('sync_items');
      final records = rows.map(SyncItem.fromRow).toList();

      // 第一遍：FAILED 复核
      for (final record in records) {
        if (record.status != SyncItemStatus.Failed) continue;
        if (isBlockedPathIdentity(record.localPath, record.fileId, blocked)) {
          continue;
        }
        final localEntry = local[record.localPath];
        final cloudFile = cloud[record.localPath];
        if (localEntry == null && cloudFile == null) continue;
        if (localEntry == null || cloudFile == null) continue;
        if (record.fileId.startsWith(pendingFileIdPrefix)) continue;
        if (record.fileId != cloudFile.id) continue;
        if (localEntry.isFolder != cloudFile.isFolder) continue;

        if (record.isFolder && localEntry.isFolder && cloudFile.isFolder) {
          // 双方都是目录 → 直接复核收敛
          final updated = await txn.rawUpdate(
            'UPDATE sync_items SET parent_folder_id = ?, name = ?, '
            'is_folder = 1, size = ?, local_size = ?, sha256 = NULL, '
            'local_mtime = ?, cloud_edited_time = ?, last_sync_time = ?, '
            'status = ?, error_message = NULL '
            'WHERE file_id = ? AND local_path = ? AND status = ? '
            'AND $transferGuard',
            [
              cloudFile.parentId,
              cloudFile.name,
              cloudFile.size,
              0,
              localEntry.mtime,
              cloudFile.editedTime?.millisecondsSinceEpoch,
              nowMs(),
              SyncItemStatus.Synced.code,
              record.fileId,
              record.localPath,
              SyncItemStatus.Failed.code,
              record.localPath,
              ...nonTerminal,
            ],
          );
          if (updated > 0) {
            summary.healed += updated;
          } else {
            summary.transferBlocked++;
          }
          continue;
        }

        // 文件：本地/云端/基线三方版本事实全部收敛才愈合
        final converged = !localEntry.isPlaceholder &&
            record.localSize == localEntry.size &&
            record.localMtime == localEntry.mtime &&
            record.size == cloudFile.size &&
            record.cloudEditedTime != null &&
            record.cloudEditedTime ==
                cloudFile.editedTime?.millisecondsSinceEpoch;
        if (converged) {
          final updated = await txn.rawUpdate(
            'UPDATE sync_items SET status = ?, error_message = NULL '
            'WHERE file_id = ? AND local_path = ? AND status = ? '
            'AND $transferGuard',
            [
              SyncItemStatus.Synced.code,
              record.fileId,
              record.localPath,
              SyncItemStatus.Failed.code,
              record.localPath,
              ...nonTerminal,
            ],
          );
          if (updated > 0) {
            summary.healed += updated;
          } else {
            summary.transferBlocked++;
          }
        }
      }

      // 第二遍：双端缺席残余清理
      for (final record in records) {
        if (isBlockedPathIdentity(record.localPath, record.fileId, blocked)) {
          continue;
        }
        if (local.containsKey(record.localPath) ||
            cloud.containsKey(record.localPath)) {
          continue;
        }
        final deleted = await txn.rawDelete(
          'DELETE FROM sync_items WHERE local_path = ? AND $transferGuard',
          [record.localPath, record.localPath, ...nonTerminal],
        );
        if (deleted > 0) {
          summary.purged += deleted;
        } else {
          summary.transferBlocked++;
        }
      }
    });

    final db2 = await this.db.database;
    final failedRows = await db2.rawQuery(
      'SELECT COUNT(*) AS c FROM sync_items WHERE status = ?',
      [SyncItemStatus.Failed.code],
    );
    final c = failedRows.first['c'];
    summary.remainingFailed = c is int ? c : int.tryParse('$c') ?? 0;
    return summary;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 改名检测（xattr fileId 识别同目录改名）
  // ═══════════════════════════════════════════════════════════════════

  /// xattr fileId 识别改名：Upload（无 fileId）/ DeleteFromLocal（孤儿占位）
  /// → MoveInCloud（对齐 Rust `detect_renames`）。
  @override
  Future<void> detectRenames(
    List<SyncAction> actions,
    Map<String, DbSnapshotEntry> dbSnapshot,
  ) async {
    final m = mount;
    if (m == null) return;
    // fileId → (DB 路径, 条目)
    final dbById = <String, MapEntry<String, DbSnapshotEntry>>{};
    for (final entry in dbSnapshot.entries) {
      dbById[entry.value.fileId] = entry;
    }
    // fileId → (云树路径, DriveFile)
    final cloudById = <String, MapEntry<String, DriveFile>>{};
    for (final entry in cloudIndex.tree.entries) {
      cloudById[entry.value.id] = entry;
    }

    final deferredReplacementSources = <(String, String)>{};
    final supersededCloudPaths = <(String, String)>{};
    final renamedSources = <(String, String)>{};

    for (final action in actions) {
      final isCandidate =
          (action.actionType == SyncActionType.upload && action.fileId == null) ||
              (action.actionType == SyncActionType.deleteFromLocal &&
                  action.fileId == null);
      if (!isCandidate) continue;
      final rel = action.relativePath;
      final localPath = action.localPath;
      if (rel == null || localPath == null) continue;

      final String? fidRaw;
      try {
        fidRaw = await m.xattr.get(localPath, xattrFileId);
      } catch (_) {
        continue;
      }
      if (fidRaw == null || fidRaw.isEmpty) continue;
      final fid = fidRaw;
      final dbEntry = dbById[fid];
      if (dbEntry == null) continue;
      final oldDbPath = dbEntry.key;
      if (oldDbPath == rel) continue;
      final cloudEntry = cloudById[fid];
      if (cloudEntry == null) continue;

      // 旧本地路径核验（复制检测 / 占用检测）
      final oldAbs = '${m.mountDir}/$oldDbPath';
      final oldType =
          await FileSystemEntity.type(oldAbs, followLinks: false);
      if (oldType != FileSystemEntityType.notFound) {
        final String? owner;
        try {
          owner = await m.xattr.get(oldAbs, xattrFileId);
        } catch (_) {
          continue; // 读失败 → 拒绝改名检测
        }
        if (owner == fid) {
          // 复制检测：新旧路径同属一个 fileId → 新文件是副本，摘除其 fileId
          try {
            await m.xattr.remove(localPath, xattrFileId);
          } catch (_) {
            // 尽力摘除
          }
          continue;
        }
        // 旧路径已被别的文件占用 → 按移动处理，旧路径动作延期
        deferredReplacementSources.add((oldDbPath, fid));
      }

      // 改写为 MoveInCloud
      final cloudFile = cloudEntry.value;
      action.fileId = fid;
      action.cloudFile = cloudFile;
      action.actionType = SyncActionType.moveInCloud;
      final oldParent = oldDbPath.contains('/')
          ? oldDbPath.substring(0, oldDbPath.lastIndexOf('/'))
          : '';
      final newParent =
          rel.contains('/') ? rel.substring(0, rel.lastIndexOf('/')) : '';
      if (oldParent == newParent) {
        action.parentFileId = cloudFile.parentId;
        action.reason = '同目录改名检测：$oldDbPath → $rel（fileId=$fid，先于内容同步）';
      } else {
        action.parentFileId = newParent.isEmpty
            ? cloudIndex.rootFolderId
            : cloudIndex.pathToId[newParent];
        action.reason = '跨目录移动检测：$oldDbPath → $rel'
            '（目标 parent=${action.parentFileId}）';
      }
      final cloudCurrentPath = cloudEntry.key;
      if (cloudCurrentPath != rel) {
        supersededCloudPaths.add((cloudCurrentPath, fid));
      }
      renamedSources.add((oldDbPath, fid));
    }

    if (deferredReplacementSources.isEmpty &&
        supersededCloudPaths.isEmpty &&
        renamedSources.isEmpty) {
      return;
    }
    actions.removeWhere((action) {
      final rel = action.relativePath;
      final fid = action.fileId;
      if (rel == null) return false;
      if (fid != null && deferredReplacementSources.contains((rel, fid))) {
        return true;
      }
      if (action.actionType == SyncActionType.createPlaceholder &&
          fid != null &&
          supersededCloudPaths.contains((rel, fid))) {
        return true;
      }
      if (action.actionType == SyncActionType.deleteFromCloud) {
        for (final (oldPath, ofid) in renamedSources) {
          if ((rel == oldPath && fid == ofid) ||
              rel.startsWith('$oldPath/')) {
            return true;
          }
        }
      }
      return false;
    });
  }

  // ═══════════════════════════════════════════════════════════════════
  // free-up 安全判定
  // ═══════════════════════════════════════════════════════════════════

  /// 文件是否可安全释放本地空间（对齐 Rust `can_safely_free_up`）。
  @override
  Future<FreeUpCheckResult> canSafelyFreeUp(
    String relPath,
    String fileId,
  ) async {
    if (!cloudTreeIsTrusted()) return FreeUpCheckResult.notSynced;
    if (cloudIndex.tree[relPath]?.id != fileId) {
      return FreeUpCheckResult.notInCloud;
    }
    if (await baselineStore.hasActiveTransferAt(relPath)) {
      return FreeUpCheckResult.notSynced;
    }
    final SyncItem? record;
    try {
      record =
          await MountManager.findByFileId(await db.database, fileId);
    } catch (_) {
      return FreeUpCheckResult.notSynced;
    }
    if (record == null ||
        record.localPath != relPath ||
        record.status != SyncItemStatus.Synced) {
      return FreeUpCheckResult.notSynced;
    }
    final m = mount;
    if (m == null) return FreeUpCheckResult.notSynced;
    final absPath = '${m.mountDir}/${record.localPath}';
    final stat = await FileStat.stat(absPath);
    if (stat.type == FileSystemEntityType.notFound) {
      return FreeUpCheckResult.notSynced;
    }
    final mtime = stat.modified.millisecondsSinceEpoch;
    if (record.localMtime != mtime || record.localSize != stat.size) {
      return FreeUpCheckResult.notSynced;
    }
    return FreeUpCheckResult.safe;
  }
}
