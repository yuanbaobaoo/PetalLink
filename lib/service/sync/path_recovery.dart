// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// 路径变更恢复 —— 云端移动/改名的本地与 DB 基线收敛。
///
/// 严格对齐 Rust 原版 `src/sync/path_recovery.rs`：
/// 每个同步周期内、云端 checkpoint 可信之后、本地扫描与规划之前运行。
/// 按 DB 基线的 fileId 在可信云树中找唯一新路径；差异路径经完整校验链
/// （类型一致/版本更新证明/活动传输隔离/本地身份核验/子树一致性）后，
/// 原子执行本地 rename（不覆盖）+ DB 子树重键（保留内容基线字段）。
library;

import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/types/enums.dart';

/// 被活动传输隔离的路径变更（本轮同步对其隔离规划）。
class BlockedPathChange {
  /// 旧路径（DB 基线路径）
  final String oldPath;

  /// 新路径（云端当前路径）
  final String newPath;

  /// 云端 fileId
  final String fileId;

  const BlockedPathChange({
    required this.oldPath,
    required this.newPath,
    required this.fileId,
  });
}

/// 路径恢复结果汇总（对齐 Rust `PathRecoverySummary`）。
class PathRecoverySummary {
  /// 成功重键的子树根数
  int rekeyedRoots = 0;

  /// 本地身份无法确认（未标记）而跳过的候选数
  int skippedUnmarked = 0;

  /// 被活动传输或校验失败隔离的变更
  final List<BlockedPathChange> blockedChanges = [];
}

/// 路径租约获取回调：对新旧路径取排他租约，返回释放闭包。
/// 获取失败应抛 [AppError]（该候选按 blocked 处理）。
typedef PathLeaseAcquirer = Future<void Function()> Function(
  String oldPath,
  String newPath,
);

/// 路径是否位于子树内（含根本身）。
bool isInSubtree(String path, String root) =>
    path == root || path.startsWith('$root/');

/// 子树路径重键：root → newRoot。
String rekeyPath(String path, String root, String newRoot) {
  if (path == root) return newRoot;
  if (path.startsWith('$root/')) return '$newRoot${path.substring(root.length)}';
  throw AppError.generic('路径不在预期子树内：$path');
}

/// 路径深度（'/' 数量）。
int pathDepth(String path) => '/'.allMatches(path).length;

/// 路径变更恢复器。
class PathRecovery {
  /// 数据库服务
  final DatabaseService _db;

  /// 挂载管理器（xattr 身份核验）
  final MountManager _mount;

  /// 当前毫秒时钟
  final int Function() _nowMs;

  PathRecovery({
    required DatabaseService db,
    required MountManager mount,
    int Function()? nowMs,
  })  : _db = db,
        _mount = mount,
        _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch);

  /// 恢复已核验的远端路径变更（对齐 Rust
  /// `recover_verified_remote_path_changes`）。
  ///
  /// [cloudTree] 必须是可信 checkpoint 的树（relPath → DriveFile）。
  /// 返回恢复汇总；DB 等全局错误直接抛出（调用方应 restore 周期请求并中止）。
  Future<PathRecoverySummary> recoverVerifiedRemotePathChanges(
    Map<String, DriveFile> cloudTree,
    PathLeaseAcquirer acquireLeases,
  ) async {
    final summary = PathRecoverySummary();

    // fileId → 云树唯一路径；重复 fileId → null（两个候选都不用）
    final cloudById = <String, ({String path, DriveFile file})?>{};
    for (final entry in cloudTree.entries) {
      final id = entry.value.id;
      if (cloudById.containsKey(id)) {
        cloudById[id] = null;
      } else {
        cloudById[id] = (path: entry.key, file: entry.value);
      }
    }

    var baselines = await _loadAll();
    // 候选：云树中 fileId 有唯一新路径且与 DB 路径不同
    final candidates = <SyncItem>[];
    for (final record in baselines) {
      final found = cloudById[record.fileId];
      if (found != null && found.path != record.localPath) {
        candidates.add(record);
      }
    }
    // 浅层目录优先、同级目录优先于文件
    candidates.sort((a, b) {
      final byDepth = pathDepth(a.localPath).compareTo(pathDepth(b.localPath));
      if (byDepth != 0) return byDepth;
      final aFolder = a.isFolder ? 0 : 1;
      final bFolder = b.isFolder ? 0 : 1;
      return aFolder.compareTo(bFolder);
    });

    final rekeyedSubtrees = <String>[];
    final blockedIdentities = <BlockedPathChange>[];

    bool insideRekeyed(String path) =>
        rekeyedSubtrees.any((root) => isInSubtree(path, root));
    bool insideBlocked(String path) => blockedIdentities.any(
        (c) => isInSubtree(path, c.oldPath) || isInSubtree(path, c.newPath));

    for (final candidate in candidates) {
      final oldPath = candidate.localPath;
      final found = cloudById[candidate.fileId];
      if (found == null) continue;
      final newPath = found.path;

      if (insideRekeyed(oldPath) || insideRekeyed(newPath)) continue;
      if (insideBlocked(oldPath) || insideBlocked(newPath)) {
        _pushBlocked(summary, oldPath, newPath, candidate.fileId);
        continue;
      }

      // 重读基线，确认记录仍存在
      baselines = await _loadAll();
      final current = baselines.where((r) =>
          r.fileId == candidate.fileId && r.localPath == oldPath);
      if (current.isEmpty) continue;

      final void Function() release;
      try {
        release = await acquireLeases(oldPath, newPath);
      } catch (e) {
        AppLogger.w('路径恢复租约获取失败，本轮隔离: $oldPath → $newPath: $e');
        _pushBlocked(summary, oldPath, newPath, candidate.fileId);
        continue;
      }

      try {
        final outcome = await _recoverOne(
          record: current.first,
          newPath: newPath,
          cloudFile: found.file,
          cloudById: cloudById,
          cloudTree: cloudTree,
          baselines: baselines,
        );
        switch (outcome) {
          case _RecoverOutcome.rekeyed:
            if (current.first.isFolder) rekeyedSubtrees.add(newPath);
            summary.rekeyedRoots++;
          case _RecoverOutcome.unmarked:
            summary.skippedUnmarked++;
          case _RecoverOutcome.blocked:
            _pushBlocked(summary, oldPath, newPath, candidate.fileId);
        }
      } catch (e) {
        AppLogger.w('路径恢复失败，本轮隔离: $oldPath → $newPath: $e');
        _pushBlocked(summary, oldPath, newPath, candidate.fileId);
      } finally {
        release();
      }
    }
    summary.blockedChanges.addAll(blockedIdentities);
    return summary;
  }

  void _pushBlocked(
    PathRecoverySummary summary,
    String oldPath,
    String newPath,
    String fileId,
  ) {
    final exists = summary.blockedChanges.any((c) =>
        c.oldPath == oldPath && c.newPath == newPath && c.fileId == fileId);
    if (!exists) {
      summary.blockedChanges.add(
          BlockedPathChange(oldPath: oldPath, newPath: newPath, fileId: fileId));
    }
  }

  /// 单候选校验链 + 执行（对齐 Rust `recover_one`）。
  Future<_RecoverOutcome> _recoverOne({
    required SyncItem record,
    required String newPath,
    required DriveFile cloudFile,
    required Map<String, ({String path, DriveFile file})?> cloudById,
    required Map<String, DriveFile> cloudTree,
    required List<SyncItem> baselines,
  }) async {
    final oldPath = record.localPath;
    MountPath.validateRelativePath(oldPath);
    MountPath.validateRelativePath(newPath);

    // 1. 同一 fileId 在基线中恰好一条
    final sameId =
        baselines.where((r) => r.fileId == record.fileId).toList();
    if (sameId.length != 1) {
      throw AppError.generic('路径恢复拒绝：fileId 对应多条基线，放弃猜测');
    }
    // 2. 云端与基线的 folder/file 类型一致
    if (cloudFile.isFolder != record.isFolder) {
      throw AppError.generic('路径恢复拒绝：云端与本地基线类型不一致');
    }
    // 3. 文件夹：禁止移入自身子树
    if (record.isFolder && isInSubtree(newPath, oldPath)) {
      throw AppError.generic('路径恢复拒绝：目录不能移入自身子树');
    }
    // 4. 文件：要求云端 editedTime 严格更新（等待 checkpoint 追平）
    if (!record.isFolder) {
      final cloudMs = cloudFile.editedTime?.millisecondsSinceEpoch;
      final baseMs = record.cloudEditedTime;
      if (cloudMs == null || baseMs == null || cloudMs <= baseMs) {
        throw AppError.generic('路径恢复拒绝：云端缺少更新版本证明，等待 checkpoint 追平');
      }
    }
    // 5. 活动传输检查 → 隔离
    if (await _hasActiveTransfer(record.fileId, oldPath, newPath)) {
      return _RecoverOutcome.blocked;
    }
    // 6. 本地文件系统四分支
    final oldAbs = p.join(_mount.mountDir, oldPath);
    final newAbs = p.join(_mount.mountDir, newPath);
    final oldType = await FileSystemEntity.type(oldAbs, followLinks: false);
    final newType = await FileSystemEntity.type(newAbs, followLinks: false);
    final oldExists = oldType != FileSystemEntityType.notFound;
    final newExists = newType != FileSystemEntityType.notFound;

    final bool localAlreadyMoved;
    if (oldExists && newExists) {
      throw AppError.generic('路径恢复拒绝：源和目标同时存在，拒绝覆盖');
    } else if (oldExists) {
      final owner = await _mount.xattr.get(oldAbs, xattrFileId);
      if (owner != record.fileId) {
        // 本地身份无法确认 → 交回既有同步逻辑
        return _RecoverOutcome.unmarked;
      }
      _verifyLocalType(oldType, record);
      localAlreadyMoved = false;
    } else if (newExists) {
      final owner = await _mount.xattr.get(newAbs, xattrFileId);
      if (owner != record.fileId) {
        throw AppError.generic('路径恢复拒绝：目标被其他本地内容占用');
      }
      _verifyLocalType(newType, record);
      localAlreadyMoved = true;
    } else {
      return _RecoverOutcome.unmarked;
    }

    // 7. 子树一致性：旧子树内每条基线，其 fileId 云树唯一路径必须等于重键期望
    for (final sub in baselines) {
      if (!isInSubtree(sub.localPath, oldPath)) continue;
      final found = cloudById[sub.fileId];
      if (found == null) continue;
      final expected = rekeyPath(sub.localPath, oldPath, newPath);
      if (found.path != expected) {
        throw AppError.generic('路径恢复拒绝：远端子树路径不一致：${sub.localPath}');
      }
    }
    // 8. 旧子树之外不得有记录已占据新子树
    for (final sub in baselines) {
      if (isInSubtree(sub.localPath, oldPath)) continue;
      if (isInSubtree(sub.localPath, newPath)) {
        throw AppError.generic('路径恢复拒绝：目标子树已被其他基线占据');
      }
    }
    // 9. 可信云树确认目标身份
    if (cloudTree[newPath]?.id != record.fileId) {
      throw AppError.generic('路径恢复拒绝：云树无法确认目标 fileId');
    }

    // 10. 本地 rename（不覆盖）
    if (!localAlreadyMoved) {
      await _ensureSafeTargetParent(newAbs);
      await _renameNoReplace(oldAbs, newAbs);
    }

    // 11. DB 子树重键（单事务；结构移动不证明内容版本，保留内容基线字段）
    await _rekeyDbSubtree(record, oldPath, newPath, cloudFile);
    AppLogger.i('路径恢复完成：$oldPath → $newPath');
    return _RecoverOutcome.rekeyed;
  }

  /// 校验本地类型与基线一致（符号链接一律拒绝）。
  void _verifyLocalType(FileSystemEntityType type, SyncItem record) {
    if (type == FileSystemEntityType.link) {
      throw AppError.generic('路径恢复拒绝：拒绝处理符号链接');
    }
    final isDir = type == FileSystemEntityType.directory;
    final isFile = type == FileSystemEntityType.file;
    if (record.isFolder && !isDir) {
      throw AppError.generic('路径恢复拒绝：本地类型与目录基线不一致');
    }
    if (!record.isFolder && !isFile) {
      throw AppError.generic('路径恢复拒绝：本地类型与文件基线不一致');
    }
  }

  /// 逐级创建目标父目录；已存在的每一级必须是非符号链接目录。
  Future<void> _ensureSafeTargetParent(String newAbs) async {
    final parent = p.dirname(newAbs);
    if (parent == _mount.mountDir || parent == '.') return;
    final rel = p.relative(parent, from: _mount.mountDir);
    final segments = rel.split(p.separator);
    var current = _mount.mountDir;
    for (final segment in segments) {
      if (segment.isEmpty || segment == '.' || segment == '..') {
        throw AppError.generic('路径恢复拒绝：目标父路径片段不安全');
      }
      current = p.join(current, segment);
      final type = await FileSystemEntity.type(current, followLinks: false);
      if (type == FileSystemEntityType.notFound) {
        await Directory(current).create();
      } else if (type != FileSystemEntityType.directory) {
        throw AppError.generic('路径恢复拒绝：目标父路径不是安全目录');
      }
    }
  }

  /// 原子 rename，目标已存在时拒绝（对齐 Rust rename_no_replace 的
  /// 非 macOS 回退：先查存在再 rename）。
  Future<void> _renameNoReplace(String oldAbs, String newAbs) async {
    if (await FileSystemEntity.type(newAbs, followLinks: false) !=
        FileSystemEntityType.notFound) {
      throw AppError.generic('路径恢复拒绝：目标已存在，拒绝覆盖');
    }
    final type = await FileSystemEntity.type(oldAbs, followLinks: false);
    if (type == FileSystemEntityType.directory) {
      await Directory(oldAbs).rename(newAbs);
    } else if (type == FileSystemEntityType.file) {
      await File(oldAbs).rename(newAbs);
    } else {
      throw AppError.generic('路径恢复拒绝：源不是普通文件或目录');
    }
  }

  /// DB 子树重键（对齐 Rust `rekey_db_subtree`）：
  /// 逐行 DELETE 旧路径 + 重键路径重插；根记录额外更新云
  /// name/parent/size/cloudEditedTime，其余内容字段全部保留；
  /// 同时把落在新旧子树内、无远端结果的 RestartRequired 任务置为 Canceled。
  Future<void> _rekeyDbSubtree(
    SyncItem root,
    String oldRoot,
    String newRoot,
    DriveFile cloudRoot,
  ) async {
    final db = await _db.database;
    await db.transaction((txn) async {
      final rows = await txn.query('sync_items');
      final subtree = rows
          .map(SyncItem.fromRow)
          .where((r) => isInSubtree(r.localPath, oldRoot))
          .toList();
      for (final record in subtree) {
        await txn.delete('sync_items',
            where: 'file_id = ? AND local_path = ?',
            whereArgs: [record.fileId, record.localPath]);
        final rekeyed = rekeyPath(record.localPath, oldRoot, newRoot);
        final isRoot = record.localPath == oldRoot;
        final updated = isRoot
            ? record.copyWith(
                localPath: rekeyed,
                parentFolderId: cloudRoot.parentId,
                name: cloudRoot.name,
                size: cloudRoot.size,
                cloudEditedTime:
                    cloudRoot.editedTime?.millisecondsSinceEpoch,
              )
            : record.copyWith(localPath: rekeyed);
        await txn.insert('sync_items', updated.toRow());
      }
      // 失效的旧重规划任务（无远端结果）→ Canceled
      await txn.rawUpdate(
        'UPDATE transfer_queue SET state = ?, error_kind = NULL, '
        'error_message = ?, finished_at = ?, next_retry_at = NULL, '
        'state_revision = state_revision + 1 '
        'WHERE state = ? '
        'AND (remote_result_file_id IS NULL OR trim(remote_result_file_id) = ?) '
        'AND (file_id = ? OR relative_path = ? OR relative_path = ? '
        'OR substr(relative_path, 1, ?) = ? OR substr(relative_path, 1, ?) = ?)',
        [
          TransferState.Canceled.code,
          '路径恢复已使旧重规划任务失效',
          _nowMs(),
          TransferState.RestartRequired.code,
          '',
          root.fileId,
          oldRoot,
          newRoot,
          oldRoot.length + 1,
          '$oldRoot/',
          newRoot.length + 1,
          '$newRoot/',
        ],
      );
    });
  }

  /// 活动传输检查（对齐 Rust `has_active_transfer`）：
  /// 阻塞态 或 （RestartRequired 且远端结果非空白），且 fileId 匹配或
  /// relativePath 落在新/旧子树内。
  Future<bool> _hasActiveTransfer(
    String fileId,
    String oldRoot,
    String newRoot,
  ) async {
    final db = await _db.database;
    final rows = await db.rawQuery(
      'SELECT COUNT(*) AS c FROM transfer_queue WHERE '
      '(state IN (?, ?, ?, ?, ?) OR '
      '(state = ? AND remote_result_file_id IS NOT NULL '
      'AND trim(remote_result_file_id) <> ?)) '
      'AND (file_id = ? OR relative_path = ? OR relative_path = ? '
      'OR substr(relative_path, 1, ?) = ? OR substr(relative_path, 1, ?) = ?)',
      [
        TransferState.Pending.code,
        TransferState.Running.code,
        TransferState.WaitingForNetwork.code,
        TransferState.BackingOff.code,
        TransferState.VerifyingRemote.code,
        TransferState.RestartRequired.code,
        '',
        fileId,
        oldRoot,
        newRoot,
        oldRoot.length + 1,
        '$oldRoot/',
        newRoot.length + 1,
        '$newRoot/',
      ],
    );
    final count = rows.first['c'];
    return count is int && count > 0;
  }

  Future<List<SyncItem>> _loadAll() async {
    final db = await _db.database;
    final rows = await db.query('sync_items');
    return rows.map(SyncItem.fromRow).toList();
  }
}

/// 单候选恢复结果分类。
enum _RecoverOutcome {
  /// 已重键
  rekeyed,

  /// 本地身份无法确认，交回既有同步逻辑
  unmarked,

  /// 活动传输隔离
  blocked,
}
