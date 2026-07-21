/// 引擎结果结算切片（对齐 Rust `src/sync/engine/results.rs`）。
///
/// 总原则：先提交 DB 事务，再发布内存缓存。
/// - 成功删除类动作清 sync_items 路径（目录级联）
/// - 非 deferred 失败按白名单标记 FAILED（deferred/Skip/pending: 不写失败基线）
/// - Upload/Download 不在此结算（TaskRunner 已原子结算）
/// - 结构类动作 upsert 基线；MoveInCloud 保留旧内容基线字段
/// - 内存云树与 recentlyDeleted（5 分钟 TTL）随后发布
library;

import 'dart:io';

import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/action_filters.dart';
import 'package:petal_link/service/sync/engine/executor.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎结果结算 mixin。
mixin EngineResults on SyncEngineBase {
  /// 按顺序执行动作（对齐 Rust `execute_actions_ordered`）：
  /// 云端目录创建（无 cloudFile 的 CreateFolder）按深度升序逐个执行并
  /// 立即结算（父目录 fileId 供后续动作回填）；其余动作并发执行。
  @override
  Future<List<ActionResult>> executeActionsOrdered(
    SyncExecutor exec,
    List<SyncAction> actions,
  ) async {
    final results = List<ActionResult?>.filled(actions.length, null);

    // 1. 云端目录创建：按深度升序逐个执行（父先建）
    final folderIndexes = <int>[
      for (var i = 0; i < actions.length; i++)
        if (actions[i].actionType == SyncActionType.createFolder &&
            actions[i].cloudFile == null)
          i,
    ]..sort((a, b) => pathDepth(actions[a].relativePath ?? '')
        .compareTo(pathDepth(actions[b].relativePath ?? '')));
    var foldersCreated = false;
    for (final i in folderIndexes) {
      fillParentFileIds([actions[i]], cloudIndex.pathToId);
      final result = await exec.executeOne(actions[i]);
      if (result.success) {
        try {
          await applyResults([actions[i]], [result]);
          results[i] = result;
        } catch (e) {
          results[i] = ActionResult(
            success: false,
            deferred: true,
            errorMessage: '云端目录已创建，但本地基线结算失败，等待重新收敛：$e',
          );
        }
        foldersCreated = true;
      } else {
        results[i] = result;
      }
    }
    if (foldersCreated) {
      await updateRuntimeAndBroadcast((r) {
        r.contentChanged = true;
      });
    }

    // 2. 其余动作并发执行
    fillParentFileIds(actions, cloudIndex.pathToId);
    final remaining = <int>[
      for (var i = 0; i < actions.length; i++)
        if (results[i] == null) i,
    ];
    final executed =
        await exec.executeAll([for (final i in remaining) actions[i]]);
    for (var k = 0; k < remaining.length; k++) {
      results[remaining[k]] = k < executed.length
          ? executed[k]
          : const ActionResult.fail('动作未执行');
    }
    return [
      for (final r in results) r ?? const ActionResult.fail('动作未执行'),
    ];
  }

  /// 应用执行结果（对齐 Rust `apply_results`）。
  @override
  Future<void> applyResults(
    List<SyncAction> actions,
    List<ActionResult> results,
  ) async {
    final db = await this.db.database;

    // 预先计算云端删除子树（目录 → 全部后代路径）
    final deleteSubtrees = <String, List<String>>{};
    for (var i = 0; i < actions.length; i++) {
      final action = actions[i];
      final rel = action.relativePath;
      if (!results[i].success ||
          action.actionType != SyncActionType.deleteFromCloud ||
          rel == null) {
        continue;
      }
      if (cloudIndex.tree[rel]?.isFolder == true) {
        deleteSubtrees[rel] = cloudIndex.tree.keys
            .where((k) => k == rel || k.startsWith('$rel/'))
            .toList();
      } else {
        deleteSubtrees[rel] = [rel];
      }
    }

    final hasMove =
        actions.any((a) => a.actionType == SyncActionType.moveInCloud);
    final settledCloudDeletes = <String>{};
    final settledDeletes = <String>{};
    // (rel, fileId, 结算用云端元数据——优先 result.cloudFile，对齐 Rust results.rs)
    final settledUpserts = <(String, String, DriveFile?)>{};

    await db.transaction((txn) async {
      // MoveInCloud 需要全部基线用于内容字段继承
      final List<SyncItem> moveBaselines;
      if (hasMove) {
        final rows = await txn.query('sync_items');
        moveBaselines = rows.map(SyncItem.fromRow).toList();
      } else {
        moveBaselines = const [];
      }

      for (var i = 0; i < actions.length; i++) {
        final action = actions[i];
        final result = results[i];
        final rel = action.relativePath;
        if (rel == null) continue;

        // a. 成功删除类
        if (result.success &&
            (action.actionType == SyncActionType.deleteFromCloud ||
                action.actionType == SyncActionType.deleteFromLocal ||
                action.actionType ==
                    SyncActionType.backupBeforeCloudDelete)) {
          if (action.actionType == SyncActionType.deleteFromCloud &&
              (deleteSubtrees[rel]?.length ?? 0) > 1) {
            final prefix = '$rel/';
            await txn.rawDelete(
              'DELETE FROM sync_items WHERE local_path = ? '
              'OR substr(local_path, 1, ?) = ?',
              [rel, prefix.length, prefix],
            );
            settledCloudDeletes.add(rel);
          } else {
            final fid = action.fileId ?? '';
            await txn.rawDelete(
              'DELETE FROM sync_items WHERE local_path = ? '
              'AND (? = ? OR file_id = ?)',
              [rel, fid, '', fid],
            );
            if (action.actionType == SyncActionType.deleteFromCloud) {
              settledCloudDeletes.add(rel);
            }
          }
          settledDeletes.add(rel);
          continue;
        }

        // b. 失败：仅非 deferred 且有真实 fileId 时标记 FAILED
        if (!result.success) {
          final fid = action.fileId;
          if (!result.deferred &&
              fid != null &&
              !fid.startsWith(pendingFileIdPrefix)) {
            await txn.rawUpdate(
              'UPDATE sync_items SET status = ?, error_message = ? '
              'WHERE file_id = ? AND local_path = ?',
              [
                SyncItemStatus.failed.code,
                result.errorMessage ?? '同步失败',
                fid,
                rel,
              ],
            );
          }
          continue;
        }

        // c. Skip：仅放行「pending: 上传恢复」收敛结算
        if (action.actionType == SyncActionType.skip) {
          if (action.cloudFile == null) continue;
          final pendingRows = await txn.query('sync_items',
              where: 'file_id = ? AND local_path = ?',
              whereArgs: ['$pendingFileIdPrefix$rel', rel]);
          if (pendingRows.isEmpty) continue;
          // 放行到下方 upsert 结算
        }

        // d. Upload/Download：不在此结算（TaskRunner 钩子已原子结算）。
        //    唯一例外：改名检测 Upload 清旧路径行。
        if (action.actionType == SyncActionType.upload ||
            action.actionType == SyncActionType.download) {
          if (action.actionType == SyncActionType.upload &&
              (action.reason ?? '').startsWith('同目录改名检测：')) {
            final fid = action.fileId;
            if (fid == null) {
              throw AppError.generic('改名检测 Upload 缺少 fileId');
            }
            await txn.delete('sync_items',
                where: 'file_id = ? AND local_path <> ?',
                whereArgs: [fid, rel]);
          }
          continue;
        }

        // e. 其余成功动作（CreatePlaceholder/CreateFolder/CreateConflictCopy/
        //    MoveInCloud/放行的 Skip）
        final defaultStatus = switch (action.actionType) {
          SyncActionType.createPlaceholder => SyncItemStatus.cloudOnly,
          SyncActionType.createConflictCopy => SyncItemStatus.conflict,
          _ => SyncItemStatus.synced,
        };
        final cloudFile = result.cloudFile ?? action.cloudFile;
        final fileId = cloudFile?.id ?? action.fileId;
        if (fileId == null) {
          AppLogger.w('动作 ${action.actionType.wireName} $rel 缺少 fileId，'
              '禁止合成 pending: ID 制造假成功基线');
          continue;
        }

        // MoveInCloud：内容字段继承旧基线（结构移动不证明内容版本）
        SyncItem? moveBaseline;
        if (action.actionType == SyncActionType.moveInCloud) {
          moveBaseline = moveBaselines
              .where((b) => b.fileId == fileId && b.localPath != rel)
              .firstOrNull;
          moveBaseline ??= moveBaselines
              .where((b) => b.fileId == fileId)
              .firstOrNull;
          if (moveBaseline == null) {
            throw AppError.generic('云端移动已确认但缺少原内容基线：$rel');
          }
        }

        // 本地事实（mtime/size/is_folder）
        final bool isFolder;
        final int? localSize;
        final int? localMtime;
        if (moveBaseline != null) {
          isFolder = moveBaseline.isFolder;
          localSize = moveBaseline.localSize;
          localMtime = moveBaseline.localMtime;
        } else {
          final settlePath = switch (action.actionType) {
            SyncActionType.createPlaceholder ||
            SyncActionType.createFolder ||
            SyncActionType.skip =>
              '$mountDir/$rel',
            _ => action.localPath,
          };
          if (settlePath == null) {
            isFolder = action.actionType == SyncActionType.createFolder;
            localSize = null;
            localMtime = null;
          } else {
            final type = await FileSystemEntity.type(settlePath,
                followLinks: false);
            if (type == FileSystemEntityType.link) {
              throw AppError.generic('结算目标为符号链接，拒绝写入基线：$rel');
            }
            final expectFolder =
                action.actionType == SyncActionType.createFolder;
            if (expectFolder && type != FileSystemEntityType.directory) {
              throw AppError.generic('本地目标类型不一致（期望目录）：$rel');
            }
            if (!expectFolder && type != FileSystemEntityType.file) {
              throw AppError.generic('本地目标类型不一致（期望文件）：$rel');
            }
            final stat = await FileStat.stat(settlePath);
            // Skip 收敛：本地大小必须与云端结果一致
            if (action.actionType == SyncActionType.skip &&
                cloudFile != null &&
                cloudFile.size != stat.size) {
              throw AppError.generic('待确认上传的本地大小与云端结果不一致，拒绝收敛成功');
            }
            isFolder = expectFolder;
            localSize = expectFolder ? 0 : stat.size;
            localMtime = stat.modified.millisecondsSinceEpoch;
          }
        }

        // CreateFolder 且云端新建（无 cloudFile 入参 → 拿到新 folderId）：
        // 先清路径下旧行防 dual 记录
        if (action.actionType == SyncActionType.createFolder &&
            action.cloudFile == null) {
          await txn.delete('sync_items',
              where: 'local_path = ?', whereArgs: [rel]);
        }
        // MoveInCloud：清同 fileId 旧路径行
        if (action.actionType == SyncActionType.moveInCloud) {
          await txn.delete('sync_items',
              where: 'file_id = ? AND local_path <> ?',
              whereArgs: [fileId, rel]);
        }
        // pending: 占位行显式清理
        if (!fileId.startsWith(pendingFileIdPrefix)) {
          await txn.delete('sync_items',
              where: 'local_path = ? AND file_id = ?',
              whereArgs: [rel, '$pendingFileIdPrefix$rel']);
        }

        final cloudEditedMs =
            cloudFile?.editedTime?.millisecondsSinceEpoch;
        final item = SyncItem(
          fileId: fileId,
          localPath: rel,
          parentFolderId: cloudFile?.parentId ?? action.parentFileId,
          name: rel.split('/').last,
          isFolder: isFolder,
          size: cloudFile?.size ?? 0,
          localSize: localSize,
          sha256: moveBaseline?.sha256,
          localMtime: localMtime,
          cloudEditedTime: cloudEditedMs ?? moveBaseline?.cloudEditedTime,
          lastSyncTime: moveBaseline?.lastSyncTime ?? nowMs(),
          status: moveBaseline?.status ?? defaultStatus,
          errorMessage: moveBaseline?.errorMessage,
        );
        await txn.insert('sync_items', item.toRow(),
            conflictAlgorithm: ConflictAlgorithm.replace);
        settledUpserts.add((rel, fileId, cloudFile));
      }
    });

    // ---- 内存发布（仅 DB 成功后）----
    final now = nowMs();
    for (final rel in settledDeletes) {
      recentlyDeletedPaths[rel] = now;
    }
    for (final rel in settledCloudDeletes) {
      cloudIndex.removeSubtree(rel);
      recentlyDeletedPaths[rel] = now;
    }
    for (final (rel, fileId, settledFile) in settledUpserts) {
      // 先移除同 fileId 的其他陈旧路径
      for (final stale in cloudIndex.otherPathsOf(fileId, rel)) {
        cloudIndex.remove(stale);
      }
      // 对齐 Rust results.rs：优先执行结果携带的权威云端元数据
      // （result.cloudFile ?? action.cloudFile），修复前只读
      // cloudIndex.tree / action.cloudFile，丢失 result.cloudFile
      final file = settledFile ?? cloudIndex.tree[rel];
      if (file != null) {
        cloudIndex.insert(rel, file);
      } else {
        cloudIndex.pathToId[rel] = fileId;
      }
    }
    // recentlyDeleted TTL 修剪
    recentlyDeletedPaths
        .removeWhere((_, ts) => now - ts > SyncEngineBase.recentlyDeletedTtlMs);
  }
}
