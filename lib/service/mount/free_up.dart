/// 释放空间 —— 把已同步文件替换为按需下载占位符。
///
/// 严格对齐 Rust 原版 `src/commands/free_up.rs`：
/// - 多段安全校验（可信云树 + 本地非占位 + DB 成功基线一致 + 远端核验 + 无活动传输）
/// - 原子事务：暂存改名 → 创建占位符 → DB baseline CAS 更新，全程可回滚
/// - 启动时恢复中断的 free-up（见 [MountManager.recoverInterruptedFreeUp]）
///
/// Rust 侧校验依赖同步引擎（可信云树 / 路径租约）与 FILES_API（远端核验）。
/// Flutter 侧引擎接线属后续任务，这里把该职责切片抽象为 [FreeUpRemoteGate]，
/// 引擎就位后实现并注入；未注入时释放类操作拒绝执行，状态检查降级为
/// `not_synced`（对齐 Rust 引擎未启动分支）。
library;

import 'dart:io';
import 'dart:math';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/types/enums.dart';

/// 可释放空间候选项（基于 DB 基线枚举，实际释放前再逐项安全核验）。
class FreeableItem {
  /// 云端文件 ID
  final String fileId;

  /// 相对挂载目录的路径
  final String relPath;

  /// 文件名
  final String name;

  /// 本地已下载字节数
  final int size;

  const FreeableItem({
    required this.fileId,
    required this.relPath,
    required this.name,
    required this.size,
  });
}

/// 批量释放空间结果统计。
class FreeUpBatchResult {
  /// 成功释放的文件数
  final int freedCount;

  /// 因不满足条件被跳过的文件数
  final int skippedCount;

  /// 成功释放的总字节数
  final int freedBytes;

  /// 被跳过项的错误原因（与跳过项一一对应，便于前端提示）
  final List<String> errors;

  const FreeUpBatchResult({
    required this.freedCount,
    required this.skippedCount,
    required this.freedBytes,
    required this.errors,
  });
}

/// 远端文件核验快照（对齐 Rust FILES_API.get 用于释放校验的最小字段集）。
class FreeUpRemoteSnapshot {
  /// 云端文件 ID
  final String id;

  /// 云端文件大小（字节）
  final int size;

  /// 云端 editedTime（毫秒 epoch；无版本信息为 null → 核验失败）
  final int? editedTimeMs;

  const FreeUpRemoteSnapshot({
    required this.id,
    required this.size,
    required this.editedTimeMs,
  });
}

/// 路径排他租约（对齐 Rust begin_exclusive_path_activity 的守卫对象）。
abstract class FreeUpPathLease {
  /// 释放租约。
  void close();
}

/// 云端核验门面 —— 同步引擎与远端 API 的职责切片。
///
/// 对齐 Rust `SyncEngine`（cloud_tree_is_trusted / cloud_tree /
/// begin_exclusive_path_activity）与 `FILES_API`（get / verify_deleted）
/// 在释放空间路径上的全部依赖；引擎接线后由引擎侧实现。
abstract class FreeUpRemoteGate {
  /// 云端索引是否已追平（cloud_tree_is_trusted）。
  bool get cloudTreeTrusted;

  /// 可信云树中 [relPath] 对应的 fileId（不存在返回 null）。
  String? cloudFileIdAt(String relPath);

  /// 获取远端文件元数据（对齐 FILES_API.get）。
  Future<FreeUpRemoteSnapshot> fetchRemote(String fileId);

  /// 远端是否已被回收/删除（对齐 FILES_API.verify_deleted）。
  Future<bool> verifyDeleted(String fileId);

  /// 开始路径排他活动（对齐 begin_exclusive_path_activity；
  /// 引擎忙碌时应抛 [AppError]）。
  FreeUpPathLease beginExclusivePathActivity(String relPath);
}

/// 释放空间服务。
class FreeUpService {
  /// 挂载管理器
  final MountManager mount;

  /// 数据库
  final DatabaseService db;

  /// 云端核验门面（引擎未接线时为 null → 释放类操作拒绝执行）
  final FreeUpRemoteGate? gate;

  FreeUpService({
    required this.mount,
    required this.db,
    this.gate,
  });

  /// 检查文件是否可安全释放本地空间。
  /// 返回 "safe" | "not_in_cloud" | "not_synced"。
  ///
  /// 对齐 Rust `sync_check_safe_free_up` + `SyncEngine::can_safely_free_up`；
  /// 引擎（gate）未启动时按不安全处理，返回 "not_synced"。
  Future<String> checkSafeFreeUp(String relPath, String fileId) async {
    final g = gate;
    if (g == null) return 'not_synced';

    // 1. 云端索引必须已追平
    if (!g.cloudTreeTrusted) return 'not_synced';
    // 2. 可信云树中必须存在同一 fileId
    if (g.cloudFileIdAt(relPath) != fileId) return 'not_in_cloud';
    // 3. 无该路径的活动传输
    if (await _hasActiveTransfer(relPath: relPath)) return 'not_synced';
    // 4. DB 基线必须存在且为已同步
    final SyncItem? record;
    try {
      record = await MountManager.findByFileId(await db.database, fileId);
    } catch (_) {
      return 'not_synced';
    }
    if (record == null ||
        record.localPath != relPath ||
        record.status != SyncItemStatus.Synced) {
      return 'not_synced';
    }
    // 5. 本地文件必须存在且与数据库基线一致；缺失文件或占位符均不可释放。
    final String absPath = p.join(mount.mountDir, record.localPath);
    final FileStat meta;
    try {
      meta = await FileStat.stat(absPath);
    } catch (_) {
      return 'not_synced';
    }
    // dart:io FileStat.stat 对缺失文件不抛异常而是返回 notFound 类型
    if (meta.type == FileSystemEntityType.notFound) return 'not_synced';
    final mtime = meta.modified.millisecondsSinceEpoch;
    if (record.localMtime != mtime || record.localSize != meta.size) {
      return 'not_synced';
    }
    return 'safe';
  }

  /// 枚举目录（含子树）下可释放空间的文件候选项。
  ///
  /// 仅基于 DB 成功同步基线筛选 status=SYNCED 且非目录的记录，供前端弹窗预览；
  /// 实际释放前由 [freeUpSpace] 逐项重新核验，避免预览与释放之间状态漂移造成误释放。
  /// 路径匹配用精确前缀加路径分隔符边界，避免 `docs` 误匹配 `docs-backup`。
  ///
  /// - [folderRelPath] 目录相对挂载根的路径，传空串表示从根枚举
  Future<List<FreeableItem>> listFreeableInFolder(String folderRelPath) async {
    final rawDb = await db.database;
    final List<Map<String, Object?>> rows;
    if (folderRelPath.isEmpty) {
      rows = await rawDb.query('sync_items',
          where: 'status = ? AND is_folder = 0',
          whereArgs: [SyncItemStatus.Synced.code]);
    } else {
      final prefix = '$folderRelPath/';
      rows = await rawDb.query(
        'sync_items',
        where: 'status = ? AND is_folder = 0 '
            'AND (local_path = ? OR substr(local_path, 1, ?) = ?)',
        whereArgs: [
          SyncItemStatus.Synced.code,
          folderRelPath,
          prefix.length,
          prefix,
        ],
      );
    }
    return rows
        .map((row) => SyncItem.fromRow(row))
        .map((record) => FreeableItem(
              fileId: record.fileId,
              relPath: record.localPath,
              name: record.name,
              size: record.localSize ?? 0,
            ))
        .toList();
  }

  /// 将已同步文件替换为按需下载占位符，返回实际释放的字节数。
  ///
  /// 对齐 Rust `sync_free_up_space`（入口校验）+ `free_up_one`（原子事务）。
  /// [localPath] 传入时校验其与 [relPath] 一致，避免路径错配释放错误文件。
  Future<int> freeUpSpace({
    required String fileId,
    required String relPath,
    required int size,
    String? localPath,
  }) async {
    final g = gate;
    if (g == null) {
      throw AppError.config('同步引擎未启动，拒绝释放空间');
    }
    if (localPath != null) {
      // 校验前端传入的绝对路径与 rel_path 一致，避免路径错配释放错误文件。
      final frontendRel =
          MountPath.relativePathFromMount(mount.mountDir, localPath);
      if (frontendRel != relPath) {
        throw AppError.config(
            '释放空间路径不一致：rel_path=$relPath, local_path=$localPath');
      }
    }
    return _freeUpOne(g, fileId, relPath, size);
  }

  /// 批量释放多个文件的本地空间，逐项独立执行。
  ///
  /// 单项失败（如并发改动、远端版本漂移）只记录原因并跳过，不中断整体释放；
  /// 每项独立持有路径租约，互不阻塞。返回成功/跳过计数与释放总字节。
  Future<FreeUpBatchResult> freeUpBatch(List<FreeableItem> items) async {
    var freedCount = 0;
    var skippedCount = 0;
    var freedBytes = 0;
    final errors = <String>[];
    for (final item in items) {
      try {
        final bytes = await freeUpSpace(
          fileId: item.fileId,
          relPath: item.relPath,
          size: item.size,
        );
        freedCount++;
        freedBytes += bytes;
      } on AppError catch (e) {
        skippedCount++;
        errors.add('${item.name}：${e.message}');
      }
    }
    return FreeUpBatchResult(
      freedCount: freedCount,
      skippedCount: skippedCount,
      freedBytes: freedBytes,
      errors: errors,
    );
  }

  /// 恢复中断的 free-up（启动时调用）。
  /// 委托 [MountManager.recoverInterruptedFreeUp]。
  Future<int> recoverInterruptedFreeUp() async {
    final rawDb = await db.database;
    return mount.recoverInterruptedFreeUp(rawDb);
  }

  /// 释放单个已同步文件的本地空间：安全核验通过后把原文件替换为占位符。
  ///
  /// 所有前置条件（可信云树、本地非占位、基线一致、远端核验、无活动传输）与
  /// 原子 staging/回滚逻辑均集中在此。返回成功释放的字节数。
  Future<int> _freeUpOne(
    FreeUpRemoteGate g,
    String fileId,
    String relPath,
    int size,
  ) async {
    final lp = MountPath.safeJoinUnder(mount.mountDir, relPath);
    final lease = g.beginExclusivePathActivity(relPath);
    try {
      if (size < 0 || !g.cloudTreeTrusted) {
        throw AppError.generic('云端索引尚未追平，拒绝释放本地唯一副本');
      }

      // ---- 本地快照：必须是已下载的普通文件 ----
      final type = await FileSystemEntity.type(lp, followLinks: false);
      if (type != FileSystemEntityType.file ||
          await mount.isPlaceholderFile(lp)) {
        throw AppError.generic('待释放目标不是已下载的普通文件');
      }
      final FileStat metadataSnapshot;
      try {
        metadataSnapshot = await FileStat.stat(lp);
      } catch (e) {
        throw AppError.generic('读取待释放文件失败：$e');
      }
      final sourceMtime = metadataSnapshot.modified.millisecondsSinceEpoch;
      final sourceSize = metadataSnapshot.size;
      if (sourceSize != size) {
        throw AppError.generic('待释放文件大小已变化，请刷新后重试');
      }

      // ---- 活动传输 + 成功同步基线 ----
      if (await _hasActiveTransfer(relPath: relPath, fileId: fileId)) {
        throw AppError.generic('该文件存在活动传输任务，暂不能释放空间');
      }
      final baseline =
          await MountManager.findByFileId(await db.database, fileId);
      if (baseline == null || baseline.localPath != relPath) {
        throw AppError.generic('找不到与路径匹配的成功同步基线');
      }
      if (baseline.status != SyncItemStatus.Synced ||
          baseline.localMtime != sourceMtime ||
          baseline.localSize != sourceSize ||
          baseline.size != size) {
        throw AppError.generic('本地内容与最后成功同步基线不一致，拒绝释放');
      }
      if (g.cloudFileIdAt(relPath) != fileId) {
        throw AppError.generic('可信云树中不存在同一 fileId');
      }

      // ---- 远端核验（位于两次本地与数据库检查之间）----
      final remote = await g.fetchRemote(fileId);
      if (remote.id != fileId ||
          remote.size != size ||
          baseline.cloudEditedTime == null ||
          remote.editedTimeMs != baseline.cloudEditedTime ||
          await g.verifyDeleted(fileId)) {
        throw AppError.generic('远端副本不存在、已回收、大小或版本与成功基线不一致');
      }

      // ---- 远端核验期间本地/DB 未变化复核 ----
      final currentType =
          await FileSystemEntity.type(lp, followLinks: false);
      final FileStat currentMetadata;
      try {
        currentMetadata = await FileStat.stat(lp);
      } catch (e) {
        throw AppError.generic('释放前复核本地文件失败：$e');
      }
      if (currentType != FileSystemEntityType.file ||
          currentMetadata.size != sourceSize ||
          currentMetadata.modified.millisecondsSinceEpoch != sourceMtime) {
        throw AppError.generic('远端核验期间本地文件已变化，拒绝删除');
      }
      if (await _hasActiveTransfer(relPath: relPath, fileId: fileId)) {
        throw AppError.generic('释放租约已失效，请刷新后重试');
      }
      final current =
          await MountManager.findByFileId(await db.database, fileId);
      if (current == null ||
          current.localPath != baseline.localPath ||
          current.status != baseline.status ||
          current.localMtime != baseline.localMtime ||
          current.localSize != baseline.localSize ||
          current.cloudEditedTime != baseline.cloudEditedTime) {
        throw AppError.generic('释放租约已失效，请刷新后重试');
      }

      // ---- 原子暂存（watcher 忽略的同目录 .hwcloud_freeup- 文件）----
      final stagingPath = await _allocateFreeUpStagingPath(lp);
      try {
        await mount.xattr.set(lp, xattrFreeUpRelativePath, relPath);
      } catch (e) {
        throw AppError.generic('写入释放空间恢复标记失败：$e');
      }
      // 持久化恢复标记（xattr 落盘后再暂存）
      RandomAccessFile? markerRaf;
      try {
        markerRaf = await File(lp).open();
        await markerRaf.flush();
      } catch (e) {
        throw AppError.generic('持久化释放空间恢复标记失败：$e');
      } finally {
        await markerRaf?.close();
      }
      try {
        await File(lp).rename(stagingPath);
      } catch (e) {
        throw AppError.generic('暂存待释放文件失败：$e');
      }

      // ---- 创建占位符 ----
      try {
        await mount.createPlaceholderStrict(relPath, fileId, size);
      } catch (e) {
        final rollback = await _attemptRestore(lp, stagingPath, fileId);
        throw AppError.generic('创建占位符失败：$e；文件恢复结果：$rollback');
      }

      // ---- DB baseline CAS 更新 ----
      final rawDb = await db.database;
      final int changed;
      try {
        changed = await rawDb.rawUpdate(
          'UPDATE sync_items '
          'SET status = ?, local_size = 0, error_message = NULL '
          'WHERE file_id = ? AND local_path = ? AND status = ? '
          'AND local_mtime = ? AND local_size = ?',
          [
            SyncItemStatus.CloudOnly.code,
            fileId,
            relPath,
            SyncItemStatus.Synced.code,
            sourceMtime,
            sourceSize,
          ],
        );
      } catch (e) {
        final rollback = await _attemptRestore(lp, stagingPath, fileId);
        throw AppError.generic('提交释放空间基线失败：$e；文件恢复结果：$rollback');
      }
      if (changed != 1) {
        final rollback = await _attemptRestore(lp, stagingPath, fileId);
        throw AppError.generic('释放空间后基线发生并发变化；文件恢复结果：$rollback');
      }

      // ---- 清理暂存 ----
      try {
        await File(stagingPath).delete();
      } catch (removeError) {
        String restoreMsg;
        var restored = false;
        try {
          await _restoreStagedFreeUp(lp, stagingPath, fileId);
          restored = true;
          restoreMsg = '成功';
        } catch (e) {
          restoreMsg = '$e';
        }
        // 对齐 Rust：文件未恢复时跳过基线回滚并按成功上报
        String baselineMsg = '成功';
        if (restored) {
          try {
            await _rollbackFreeUpBaseline(
                fileId, relPath, sourceMtime, sourceSize);
          } catch (e) {
            baselineMsg = '$e';
          }
        }
        throw AppError.generic('清理释放空间暂存文件失败：$removeError；'
            '文件恢复：$restoreMsg；基线恢复：$baselineMsg');
      }

      // 实际释放的字节数（供批量释放统计）
      return sourceSize;
    } finally {
      lease.close();
    }
  }

  /// 尝试恢复暂存文件，返回用户可读的结果描述。
  Future<String> _attemptRestore(
      String lp, String stagingPath, String fileId) async {
    try {
      await _restoreStagedFreeUp(lp, stagingPath, fileId);
      return '已恢复';
    } catch (e) {
      return '$e';
    }
  }

  /// 仅在原路径空缺或仍是本文件占位符时恢复暂存内容。
  /// 对齐 Rust `restore_staged_free_up`。
  Future<void> _restoreStagedFreeUp(
      String lp, String stagingPath, String fileId) async {
    if (await FileSystemEntity.type(lp, followLinks: false) !=
        FileSystemEntityType.notFound) {
      if (await FileSystemEntity.type(lp, followLinks: false) !=
          FileSystemEntityType.file) {
        throw AppError.generic('原路径已出现非普通文件，已保留旧内容于 $stagingPath');
      }
      final String? state;
      try {
        state = await mount.xattr.get(lp, xattrState);
      } catch (e) {
        throw AppError.generic('读取回滚占位状态失败：$e');
      }
      final String? owner;
      try {
        owner = await mount.xattr.get(lp, xattrFileId);
      } catch (e) {
        throw AppError.generic('读取回滚占位身份失败：$e');
      }
      final isOwnedPlaceholder = state == statePlaceholder && owner == fileId;
      if (!isOwnedPlaceholder) {
        throw AppError.generic('原路径已出现新的用户文件，已保留旧内容于 $stagingPath');
      }
      try {
        await File(lp).delete();
      } catch (e) {
        throw AppError.generic('移除回滚占位符失败：$e');
      }
    }
    try {
      await File(stagingPath).rename(lp);
    } catch (e) {
      throw AppError.generic('恢复释放空间原文件失败：$e');
    }
    try {
      await mount.xattr.remove(lp, xattrFreeUpRelativePath);
    } catch (_) {
      // 尽力移除恢复标记
    }
  }

  /// 仅在释放空间基线未被并发改写时恢复已同步状态。
  /// 对齐 Rust `rollback_free_up_baseline`。
  Future<void> _rollbackFreeUpBaseline(
    String fileId,
    String relPath,
    int sourceMtime,
    int sourceSize,
  ) async {
    final rawDb = await db.database;
    final int changed;
    try {
      changed = await rawDb.rawUpdate(
        'UPDATE sync_items '
        'SET status = ?, local_size = ?, error_message = NULL '
        'WHERE file_id = ? AND local_path = ? AND status = ? '
        'AND local_mtime = ? AND local_size = 0',
        [
          SyncItemStatus.Synced.code,
          sourceSize,
          fileId,
          relPath,
          SyncItemStatus.CloudOnly.code,
          sourceMtime,
        ],
      );
    } catch (e) {
      throw AppError.generic('回滚释放空间基线失败：$e');
    }
    if (changed != 1) {
      throw AppError.generic('释放空间基线已并发变化，无法自动回滚');
    }
  }

  /// 在原文件同目录分配不存在的释放空间暂存路径。
  /// 对齐 Rust `allocate_free_up_staging_path`。
  Future<String> _allocateFreeUpStagingPath(String localPath) async {
    final parent = p.dirname(localPath);
    final random = Random.secure();
    for (var i = 0; i < 16; i++) {
      final candidate =
          p.join(parent, '.hwcloud_freeup-$pid-${randomHex64(random)}');
      if (await FileSystemEntity.type(candidate, followLinks: false) ==
          FileSystemEntityType.notFound) {
        return candidate;
      }
    }
    throw AppError.generic('无法分配释放空间临时路径');
  }

  /// 是否存在匹配路径（或文件）的活动传输任务。
  /// 对齐 Rust 对 transfer_queue 非终态（Completed/Failed/Canceled 之外）的检查。
  Future<bool> _hasActiveTransfer({required String relPath, String? fileId}) async {
    final rawDb = await db.database;
    final terminal = [
      TransferState.Completed.code,
      TransferState.Failed.code,
      TransferState.Canceled.code,
    ];
    final List<Map<String, Object?>> rows;
    if (fileId == null) {
      rows = await rawDb.rawQuery(
        'SELECT COUNT(*) AS c FROM transfer_queue '
        'WHERE relative_path = ? AND state NOT IN (?, ?, ?)',
        [relPath, ...terminal],
      );
    } else {
      rows = await rawDb.rawQuery(
        'SELECT COUNT(*) AS c FROM transfer_queue '
        'WHERE (relative_path = ? OR file_id = ?) AND state NOT IN (?, ?, ?)',
        [relPath, fileId, ...terminal],
      );
    }
    final count = rows.first['c'];
    return count is int && count > 0;
  }
}
