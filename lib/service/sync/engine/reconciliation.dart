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
import 'package:petal_link/service/sync/identity/detect_moves.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/core/logger/logger.dart';
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
    // inode 移动检测（docs/design/10 §4.3）：先于映射更新读取旧映射
    lastScanMoves = await detectMoves(entries, identity);
    await _updateInodeIdentity(entries);
    return {for (final e in entries) e.relativePath: e};
  }

  /// inode 身份映射更新（docs/design/10 阶段 1：仅写不读）。
  ///
  /// 有 DB 基线（非 pending/非墓碑）的扫描条目 → upsert(inode, rel, fileId)；
  /// 随后按本轮见到的 inode 集合 purge 陈旧记录。采集失败不阻断扫描。
  Future<void> _updateInodeIdentity(List<LocalFileEntry> entries) async {
    try {
      final withInode = entries.where((e) => e.inode != null).toList();
      // provider 未注入（无 inode 来源）时跳过；provider 在但目录为空
      // 时照常 purge（陈旧记录应随扫描清空）
      if (withInode.isEmpty && mount?.inodeBatchProvider == null) return;
      final db = await this.db.database;
      final rows = await db.query('sync_items',
          columns: ['local_path', 'file_id', 'status']);
      final fileIdByPath = <String, String>{
        for (final r in rows)
          if ((r['file_id'] as String).isNotEmpty &&
              !(r['file_id'] as String).startsWith(pendingFileIdPrefix) &&
              (r['status'] as int) != SyncItemStatus.deleted.code)
            r['local_path'] as String: r['file_id'] as String,
      };
      for (final e in withInode) {
        final fid = fileIdByPath[e.relativePath];
        if (fid != null) {
          await identity.upsert(e.inode!, e.relativePath, fid);
        }
      }
      await identity.purgeMissing(withInode.map((e) => e.inode!).toSet());
    } catch (e) {
      // 阶段 1 数据采集失败不影响同步主流程
      AppLogger.w('inode 身份映射更新失败（忽略）: $e');
    }
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
        if (existing.status == SyncItemStatus.deleted) {
          final next = localEntry.isPlaceholder
              ? SyncItemStatus.cloudOnly
              : SyncItemStatus.synced;
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
            status: SyncItemStatus.synced,
          ));
        }
        continue;
      }

      // 文件：凭 inode 映射身份补基线（docs/design/10 §4.4，
      // 取代 xattr fileId 三层关卡——复制产生新 inode，结构上
      // 不可能"同一身份多处出现"，无需任何消歧）
      final status = localEntry.isPlaceholder
          ? SyncItemStatus.cloudOnly
          : SyncItemStatus.synced;
      final inode = localEntry.inode;
      if (inode == null) continue; // 无 inode 数据 → 交 planner
      final identityRec = await identity.lookup(inode);
      if (identityRec == null) continue; // 本地新文件交 planner
      final fid = identityRec.fileId;
      if (isBlockedPathIdentity(rel, fid, blocked)) continue;
      // 云树同路径不存在 → 禁止制造已同步基线
      if (cloudFile == null) continue;
      // 同路径云端身份不一致 → 交 planner
      if (cloudFile.id != fid) continue;

      // 同 fileId 旧路径记录存在 → 迁移（改名/移动收敛）
      final oldPath = pathById[fid];
      if (oldPath != null && oldPath != rel) {
        final db2 = await this.db.database;
        await db2.transaction((txn) async {
          final oldRows = await txn.query('sync_items',
              where: 'file_id = ? AND local_path <> ?',
              whereArgs: [fid, rel]);
          await txn.delete('sync_items',
              where: 'file_id = ? AND local_path <> ?',
              whereArgs: [fid, rel]);
          // 保留旧记录的内容字段（不把目标当前 mtime/size 误记为已同步）
          final old = oldRows.isNotEmpty
              ? SyncItem.fromRow(oldRows.first)
              : null;
          await txn.insert(
            'sync_items',
            SyncItem(
              fileId: fid,
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
        fileId: fid,
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
      TransferState.completed.code,
      TransferState.canceled.code,
    ];

    await db.transaction((txn) async {
      final rows = await txn.query('sync_items');
      final records = rows.map(SyncItem.fromRow).toList();

      // 第一遍：FAILED 复核
      for (final record in records) {
        if (record.status != SyncItemStatus.failed) continue;
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
              SyncItemStatus.synced.code,
              record.fileId,
              record.localPath,
              SyncItemStatus.failed.code,
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
              SyncItemStatus.synced.code,
              record.fileId,
              record.localPath,
              SyncItemStatus.failed.code,
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
      [SyncItemStatus.failed.code],
    );
    final c = failedRows.first['c'];
    summary.remainingFailed = c is int ? c : int.tryParse('$c') ?? 0;
    return summary;
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
        record.status != SyncItemStatus.synced) {
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
