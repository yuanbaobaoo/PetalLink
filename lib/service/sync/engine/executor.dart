/// 同步动作执行器（对齐 Rust `src/sync/executor.rs` + `executor/`）。
///
/// - Upload/Download → 构造 TransferTask 走 TaskRunner.enqueueAndRun
///   （同路径仲裁在 TaskRunner 准入层）
/// - CreatePlaceholder/CreateFolder/MoveInCloud/DeleteFromCloud/
///   DeleteFromLocal/CreateConflictCopy/BackupBeforeCloudDelete/Skip
///   直接执行
/// - 并发槽位 = 配置并发数；活动门拒绝时返回 deferred（取消不是同步失败）
/// - DeleteFromLocal 三重防线：基线快照复核（符号链接红线）→
///   远端删除证明 → 复核后确认删除
library;

import 'dart:async';
import 'dart:io';

import 'package:path/path.dart' as p;
import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/conflict.dart';
import 'package:petal_link/service/sync/engine/coordination.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// 同步动作执行器。
class SyncExecutor {
  /// inode 身份映射（引擎 setExecutor 时注入共享实例；docs/design/10）
  InodeIdentityStore identity =
      SqfliteInodeIdentityStore(DatabaseService.instance);

  /// 经批量通道取本地文件 inode 的身份（无 provider/无记录 → null）。
  Future<String?> _ownerOf(MountManager m, String localPath) async {
    final provider = m.inodeBatchProvider;
    if (provider == null) return null;
    final inodes = await provider([localPath]);
    final inode = inodes[localPath];
    if (inode == null) return null;
    return (await identity.lookup(inode))?.fileId;
  }

  /// 文件 API
  final FilesService filesApi;

  /// 上传执行器（冲突本地赢回传）
  final UploadService uploadApi;

  /// 下载执行器（冲突副本下载）
  final DownloadService downloadApi;

  /// 挂载管理器
  final MountManager? mount;

  /// 冲突解决器
  final ConflictResolver conflictResolver;

  /// 数据库
  final DatabaseService db;

  /// 持久化传输执行器
  final TaskRunner? taskRunner;

  /// 并发槽位提供（AppConfig.concurrency）
  final Future<int> Function()? concurrencyProvider;

  /// 活动门登记（引擎注入；引擎已停时抛错 → deferred）
  final ActivityGuard Function(String? path)? beginActivity;

  /// 当前毫秒时钟
  final int Function() _nowMs;

  SyncExecutor({
    required this.filesApi,
    required this.uploadApi,
    required this.downloadApi,
    required this.db,
    this.mount,
    ConflictResolver? conflictResolver,
    this.taskRunner,
    this.concurrencyProvider,
    this.beginActivity,
    int Function()? nowMs,
  })  : conflictResolver = conflictResolver ?? ConflictResolver(),
        _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch);

  // ═══════════════════════════════════════════════════════════════════
  // 批量执行
  // ═══════════════════════════════════════════════════════════════════

  /// 并发执行全部动作，结果按原序返回（对齐 Rust `execute_all`）。
  Future<List<ActionResult>> executeAll(List<SyncAction> actions) async {
    if (actions.isEmpty) return const [];
    await pruneTransferHistory(100);
    final concurrency =
        ((await concurrencyProvider?.call()) ?? 6).clamp(1, 20);
    final results = List<ActionResult?>.filled(actions.length, null);
    var running = 0;
    var index = 0;
    final completer = Completer<void>();

    void pump() {
      while (index < actions.length && running < concurrency) {
        final i = index++;
        running++;
        unawaited(() async {
          try {
            results[i] = await _executeGuarded(actions[i]);
          } catch (e, st) {
            AppLogger.e('动作 ${actions[i]} 执行异常', e, st);
            results[i] = ActionResult.fail('动作执行异常：$e');
          } finally {
            running--;
            pump();
            if (index >= actions.length &&
                running == 0 &&
                !completer.isCompleted) {
              completer.complete();
            }
          }
        }());
      }
      if (actions.isEmpty && !completer.isCompleted) completer.complete();
    }

    pump();
    await completer.future;
    return [
      for (final r in results) r ?? const ActionResult.fail('动作未执行'),
    ];
  }

  /// 单个动作带活动门执行（门拒绝/引擎停止 → deferred）。
  Future<ActionResult> _executeGuarded(SyncAction action) async {
    ActivityGuard? guard;
    try {
      guard = beginActivity?.call(action.relativePath);
    } catch (e) {
      return ActionResult.defer('$e');
    }
    try {
      return await executeOne(action);
    } finally {
      guard?.close();
    }
  }

  /// 执行单个动作（无活动门包装；引擎顺序路径用）。
  Future<ActionResult> executeOne(SyncAction action) async {
    try {
      switch (action.actionType) {
        case SyncActionType.upload:
        case SyncActionType.download:
          return await _executeTransferAction(action);
        case SyncActionType.createPlaceholder:
          return await _doCreatePlaceholder(action);
        case SyncActionType.createFolder:
          return await _doCreateFolder(action);
        case SyncActionType.moveInCloud:
          return await _doMoveInCloud(action);
        case SyncActionType.deleteFromCloud:
          return await _doDeleteFromCloud(action);
        case SyncActionType.deleteFromLocal:
          return await _doDeleteFromLocal(action);
        case SyncActionType.createConflictCopy:
          return await _doConflict(action);
        case SyncActionType.backupBeforeCloudDelete:
          return await _doBackupBeforeCloudDelete(action);
        case SyncActionType.skip:
          return ActionResult.ok(errorMessage: action.reason);
      }
    } catch (e) {
      return ActionResult.fail('$e');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 传输类动作（TaskRunner 准入）
  // ═══════════════════════════════════════════════════════════════════

  /// 构造动作对应的 Pending 任务（对齐 Rust `pending_task_for_action`）。
  Future<TransferTask> pendingTaskForAction(SyncAction action) async {
    final rel = action.relativePath;
    final localPath = action.localPath;
    if (rel == null || localPath == null) {
      throw AppError.generic('传输动作缺少相对路径或本地路径');
    }
    final cloud = action.cloudFile;
    final isUpload = action.actionType == SyncActionType.upload;

    final TransferOperation operation;
    final TransferDirection direction;
    if (isUpload) {
      operation = action.fileId != null
          ? TransferOperation.update
          : TransferOperation.create;
      direction = TransferDirection.upload;
    } else {
      // 本地已是普通文件且非空占位 → 更新下载
      final type = await FileSystemEntity.type(localPath, followLinks: false);
      var isUpdate = type == FileSystemEntityType.file;
      if (isUpdate) {
        final stat = await FileStat.stat(localPath);
        final m = mount;
        final isPlaceholder =
            stat.size == 0 && m != null && await m.isPlaceholderFile(localPath);
        isUpdate = !isPlaceholder;
      }
      operation = isUpdate
          ? TransferOperation.downloadUpdate
          : TransferOperation.download;
      direction = isUpdate
          ? TransferDirection.downloadUpdate
          : TransferDirection.download;
    }

    // 源快照（Create/Update/DownloadUpdate 取本地 metadata）
    int? sourceMtime;
    int? sourceSize;
    if (operation != TransferOperation.download) {
      final stat = await FileStat.stat(localPath);
      if (stat.type != FileSystemEntityType.file) {
        throw AppError.generic('传输源不存在或不是普通文件：$localPath');
      }
      sourceMtime = stat.modified.millisecondsSinceEpoch;
      sourceSize = stat.size;
    }

    return TransferTask(
      direction: direction,
      fileId: action.fileId,
      localPath: localPath,
      name: rel.split('/').last,
      totalSize: isUpload ? (sourceSize ?? 0) : (cloud?.size ?? 0),
      createdAt: _nowMs(),
      relativePath: rel,
      parentFileId: action.parentFileId ?? cloud?.parentId,
      operation: operation,
      sourceMtime: sourceMtime,
      sourceSize: sourceSize,
      expectedCloudEditedTime:
          cloud?.editedTime?.millisecondsSinceEpoch,
    );
  }

  /// 传输动作执行（对齐 Rust `execute_transfer_action`）：
  /// Completed → 成功（携带云端元数据）；其他调度去向 → deferred。
  Future<ActionResult> _executeTransferAction(SyncAction action) async {
    final runner = taskRunner;
    if (runner == null) {
      return const ActionResult.fail('TaskRunner 未初始化');
    }
    final TransferTask task;
    try {
      task = await pendingTaskForAction(action);
    } catch (e) {
      return ActionResult.fail('$e');
    }
    final AppResult<EnqueuedTaskOutcome> result;
    try {
      result = await runner.enqueueAndRun(task);
    } catch (e) {
      return ActionResult.fail('$e');
    }
    if (result.isErr) {
      return ActionResult.fail('${(result as Err).error}');
    }
    final outcome = (result as Ok<EnqueuedTaskOutcome>).value.outcome;
    if (outcome.disposition == TaskDisposition.completed) {
      return ActionResult.ok(cloudFile: outcome.cloudFile);
    }
    return ActionResult.defer('传输已调度为 ${outcome.disposition.name}');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 直接执行类动作
  // ═══════════════════════════════════════════════════════════════════

  /// CreatePlaceholder：创建本地占位符 + 直接 upsert CLOUD_ONLY 基线
  /// （防孤儿占位符）。
  Future<ActionResult> _doCreatePlaceholder(SyncAction action) async {
    final m = mount;
    final cloud = action.cloudFile;
    final rel = action.relativePath;
    if (m == null || cloud == null || rel == null) {
      return const ActionResult.fail('创建占位符缺少挂载/云端元数据/相对路径');
    }
    await m.createPlaceholderIfNeeded(rel, cloud.id, cloud.size);
    final rawDb = await db.database;
    await rawDb.insert(
      'sync_items',
      SyncItem(
        fileId: cloud.id,
        localPath: rel,
        parentFolderId: cloud.parentId,
        name: cloud.name,
        size: cloud.size,
        cloudEditedTime: cloud.editedTime?.millisecondsSinceEpoch,
        status: SyncItemStatus.cloudOnly,
      ).toRow(),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
    return ActionResult.ok(cloudFile: cloud);
  }

  /// CreateFolder：云端已有 → 本地建目录；本地新增 → 云端建目录。
  Future<ActionResult> _doCreateFolder(SyncAction action) async {
    final rel = action.relativePath;
    if (rel == null) {
      return const ActionResult.fail('创建目录缺少相对路径');
    }
    final cloud = action.cloudFile;
    if (cloud != null) {
      final m = mount;
      if (m == null) return const ActionResult.ok();
      await m.ensureFolder(rel);
      return ActionResult.ok(cloudFile: cloud);
    }
    final name = rel.split('/').last;
    final result =
        await filesApi.createFolder(name, parentId: action.parentFileId);
    if (result.isErr) {
      return ActionResult.fail('${(result as Err).error}');
    }
    return ActionResult.ok(
        cloudFile: (result as Ok<DriveFile>).value);
  }

  /// MoveInCloud：本地身份复核 → 目标冲突检查 → 云端 update →
  /// 失败时 GET 复核幂等收敛（对齐 Rust `do_move_in_cloud`）。
  Future<ActionResult> _doMoveInCloud(SyncAction action) async {
    final m = mount;
    final rel = action.relativePath;
    final localPath = action.localPath;
    final fileId = action.fileId;
    final parentFileId = action.parentFileId;
    if (m == null ||
        rel == null ||
        localPath == null ||
        fileId == null ||
        parentFileId == null) {
      return const ActionResult.defer('云端移动缺少必要事实，等待重新规划');
    }
    // 本地身份复核（inode 映射，docs/design/10 §4.5：取代 xattr owner 核验）
    final String? owner;
    try {
      owner = await _ownerOf(m, localPath);
    } catch (_) {
      return const ActionResult.defer('本地路径或 fileId 已变化');
    }
    if (owner != fileId) {
      return const ActionResult.defer('本地路径或 fileId 已变化');
    }
    // 目标目录同名冲突检查（未发送远端写入前的 fail-closed）
    final listResult = await filesApi.listAll(parentId: parentFileId);
    if (listResult.isErr) {
      return ActionResult.defer('目标目录核验失败：${(listResult as Err).error}');
    }
    final targetName = rel.split('/').last;
    for (final sibling in (listResult as Ok<List<DriveFile>>).value) {
      if (sibling.name == targetName && sibling.id != fileId) {
        return const ActionResult.defer('目标目录已存在同名远端文件，拒绝覆盖并等待重新规划');
      }
    }
    final updateResult = await filesApi.update(fileId,
        newName: targetName, newParentFolder: parentFileId);
    if (updateResult.isOk) {
      return ActionResult.ok(
          cloudFile: (updateResult as Ok<DriveFile>).value);
    }
    // 失败 → GET 复核（响应丢失幂等收敛）
    final getResult = await filesApi.get(fileId);
    if (getResult.isErr) {
      return ActionResult.defer(
          '远端路径变更结果不确定：${(getResult as Err).error}');
    }
    final remote = (getResult as Ok<DriveFile>).value;
    final parents = remote.parentFolder ?? const <String>[];
    if (remote.id == fileId &&
        remote.name == targetName &&
        parents.length == 1 &&
        parents.first == parentFileId) {
      return ActionResult.ok(cloudFile: remote);
    }
    return const ActionResult.defer('远端路径变更尚未生效，等待重新规划');
  }

  /// DeleteFromCloud：删除失败经 verifyDeleted 收敛。
  Future<ActionResult> _doDeleteFromCloud(SyncAction action) async {
    final fileId = action.fileId;
    if (fileId == null) {
      return const ActionResult.fail('云端删除缺少 fileId');
    }
    final result = await filesApi.delete(fileId);
    if (result.isOk) return const ActionResult.ok();
    final verify = await filesApi.verifyDeleted(fileId);
    if (verify.isErr) {
      return ActionResult.defer('云端删除结果不确定：${(verify as Err).error}');
    }
    if ((verify as Ok<bool>).value) {
      return const ActionResult.ok();
    }
    return const ActionResult.fail('远端核验显示文件仍未回收');
  }

  /// BackupBeforeCloudDelete：改名备份本地已修改内容（副本下轮作为全新
  /// 本地文件 Upload）。
  Future<ActionResult> _doBackupBeforeCloudDelete(SyncAction action) async {
    final localPath = action.localPath;
    if (localPath == null) return const ActionResult.ok();
    if (await FileSystemEntity.type(localPath, followLinks: false) ==
        FileSystemEntityType.notFound) {
      return const ActionResult.ok();
    }
    final stat = await FileStat.stat(localPath);
    final copyPath = await dedupeCopyPath(
      localPath,
      '本地副本',
      stat.modified,
    );
    try {
      await File(localPath).rename(copyPath);
    } catch (e) {
      return ActionResult.fail('备份本地修改失败：$e');
    }
    final m = mount;
    if (m != null) {
      try {
        await m.clearPlaceholderXattr(copyPath);
      } catch (_) {
        // 尽力清理
      }
    }
    AppLogger.i('云端删除前已备份本地修改：$localPath → $copyPath');
    return const ActionResult.ok();
  }

  /// CreateConflictCopy：60s 规则判定胜者，输方复制为副本
  /// （对齐 Rust `do_conflict`）。
  Future<ActionResult> _doConflict(SyncAction action) async {
    final m = mount;
    final localPath = action.localPath;
    final cloud = action.cloudFile;
    if (m == null || localPath == null || cloud == null) {
      return const ActionResult.fail('冲突处理缺少本地路径或云端元数据');
    }
    final stat = await FileStat.stat(localPath);
    final resolution = await conflictResolver.resolve(
      localPath,
      cloud,
      stat.modified.millisecondsSinceEpoch,
    );
    AppLogger.i(resolution.logMessage);
    final expectation = DownloadExpectation(
      editedTimeMs: cloud.editedTime?.millisecondsSinceEpoch,
      size: cloud.size,
      contentHash: cloud.contentHash,
      placeholderFileId: cloud.id,
    );

    if (resolution.winner == ConflictSide.cloud) {
      // 云端胜：本地改名副本 → 下载云端到原路径
      try {
        await File(localPath).rename(resolution.copyPath);
      } catch (e) {
        return ActionResult.fail('冲突备份失败，绝不覆盖本地修改：$e');
      }
      final download = await downloadApi.downloadWithExpectation(
        cloud.id,
        localPath,
        expectation: expectation,
      );
      if (download.isErr) {
        // 下载失败 → 副本改名回原路径
        try {
          await File(resolution.copyPath).rename(localPath);
        } catch (e) {
          AppLogger.e('冲突副本回滚失败', e);
        }
        return ActionResult.fail('冲突下载失败：${(download as Err).error}');
      }
      try {
        await m.markDownloaded(localPath);
        await m.clearPlaceholderXattr(resolution.copyPath);
      } catch (_) {
        // 尽力
      }
      return ActionResult.ok(cloudFile: cloud);
    }

    // 本地胜：下载云端旧版到副本 → 上传本地覆盖云端
    final download = await downloadApi.downloadWithExpectation(
      cloud.id,
      resolution.copyPath,
      expectation: expectation,
    );
    if (download.isErr) {
      return ActionResult.fail('云端旧版备份失败：${(download as Err).error}');
    }
    try {
      await m.clearPlaceholderXattr(resolution.copyPath);
    } catch (_) {
      // 尽力
    }
    final upload = await uploadApi.uploadUpdate(
      cloud.id,
      localPath,
      parentId: cloud.parentId,
    );
    if (upload.isErr) {
      return ActionResult.fail('冲突上传失败：${(upload as Err).error}');
    }
    return ActionResult.ok(cloudFile: (upload as Ok<DriveFile>).value);
  }

  // ═══════════════════════════════════════════════════════════════════
  // DeleteFromLocal（三重防线）
  // ═══════════════════════════════════════════════════════════════════

  /// DeleteFromLocal：基线快照复核 → 远端删除证明 → 复核后确认删除
  /// （对齐 Rust `local_delete.rs`）。
  Future<ActionResult> _doDeleteFromLocal(SyncAction action) async {
    final localPath = action.localPath;
    if (localPath == null) {
      // 纯 DB 清理场景
      return const ActionResult.ok();
    }
    final m = mount;
    if (m == null) {
      return const ActionResult.fail('挂载管理器未配置');
    }
    final rel = action.relativePath;
    final Map<String, SyncItem> baselines;
    try {
      final rawDb = await db.database;
      final rows = await rawDb.query('sync_items');
      baselines = {};
      for (final row in rows) {
        final item = SyncItem.fromRow(row);
        if (baselines.containsKey(item.localPath)) {
          return const ActionResult.defer('基线存在重复 local_path，保留本地内容');
        }
        baselines[item.localPath] = item;
      }
    } catch (e) {
      return ActionResult.defer('读取同步基线失败：$e');
    }

    // 第一次快照校验
    try {
      await _verifyLocalDeleteSnapshot(
        localPath,
        rel,
        baselines,
        allowOrphanPlaceholder: action.fileId == null,
      );
    } catch (e) {
      return ActionResult.defer('$e');
    }

    // 远端删除证明（尽量贴近不可逆删除）
    final fileId = action.fileId;
    if (fileId != null) {
      if (fileId.startsWith(pendingFileIdPrefix)) {
        return const ActionResult.defer('待上传记录没有可核验的远端删除事实');
      }
      final verify = await filesApi.verifyDeleted(fileId);
      if (verify.isErr) {
        return ActionResult.defer('无法确认云端已删除：${(verify as Err).error}');
      }
      if (!(verify as Ok<bool>).value) {
        return const ActionResult.defer('云端文件仍存在，取消本地删除并等待重新规划');
      }
    }

    // 远端核验期间被改 → 取消
    try {
      await _verifyLocalDeleteSnapshot(
        localPath,
        rel,
        baselines,
        allowOrphanPlaceholder: action.fileId == null,
      );
    } catch (e) {
      return ActionResult.defer('远端核验期间本地已变化：$e');
    }

    if (await FileSystemEntity.type(localPath, followLinks: false) ==
        FileSystemEntityType.notFound) {
      return const ActionResult.ok();
    }
    try {
      await m.deleteLocalConfirmed(localPath);
    } catch (e) {
      return ActionResult.defer('删除本地失败：$e');
    }
    return const ActionResult.ok();
  }

  /// 递归校验待删除子树仍与基线一致（对齐 Rust
  /// `verify_local_delete_snapshot`；符号链接红线：永不删除符号链接）。
  Future<void> _verifyLocalDeleteSnapshot(
    String path,
    String? rel,
    Map<String, SyncItem> baselines, {
    required bool allowOrphanPlaceholder,
  }) async {
    final m = mount!;
    final type = await FileSystemEntity.type(path, followLinks: false);
    if (type == FileSystemEntityType.link) {
      throw AppError.generic('拒绝删除符号链接：$path');
    }
    if (type == FileSystemEntityType.directory) {
      final baseline = rel != null ? baselines[rel] : null;
      final stat = await FileStat.stat(path);
      if (baseline == null ||
          !baseline.isFolder ||
          baseline.localMtime != stat.modified.millisecondsSinceEpoch ||
          baseline.localSize != stat.size) {
        throw AppError.generic('目录在删除执行前发生变化：$path');
      }
      await for (final entity in Directory(path).list(followLinks: false)) {
        final childRel =
            rel == null ? null : '$rel/${p.basename(entity.path)}';
        await _verifyLocalDeleteSnapshot(
          entity.path,
          childRel,
          baselines,
          allowOrphanPlaceholder: false,
        );
      }
      return;
    }
    if (type != FileSystemEntityType.file) {
      throw AppError.generic('拒绝删除非普通文件：$path');
    }
    // 占位符：孤儿允许；否则要求基线记录
    if (await m.isPlaceholderFile(path)) {
      final stat = await FileStat.stat(path);
      if (stat.size == 0) {
        if (allowOrphanPlaceholder) return;
        final baseline = rel != null ? baselines[rel] : null;
        if (baseline != null && !baseline.isFolder) return;
        throw AppError.generic('占位符缺少同步基线：$path');
      }
    }
    // 真实文件：必须与基线一致
    final baseline = rel != null ? baselines[rel] : null;
    final stat = await FileStat.stat(path);
    if (baseline == null ||
        baseline.isFolder ||
        baseline.localMtime != stat.modified.millisecondsSinceEpoch ||
        baseline.localSize != stat.size) {
      throw AppError.generic('文件在删除执行前发生变化：$path');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 传输历史修剪
  // ═══════════════════════════════════════════════════════════════════

  /// 修剪已结束任务历史（保留最近 [keep] 条，对齐 Rust
  /// `prune_transfer_history`）。
  Future<void> pruneTransferHistory(int keep) async {
    try {
      final rawDb = await db.database;
      await rawDb.rawDelete(
        'DELETE FROM transfer_queue WHERE state IN (?, ?, ?) AND id NOT IN ('
        'SELECT id FROM transfer_queue WHERE state IN (?, ?, ?) '
        'ORDER BY id DESC LIMIT ?)',
        [
          TransferState.completed.code,
          TransferState.failed.code,
          TransferState.canceled.code,
          TransferState.completed.code,
          TransferState.failed.code,
          TransferState.canceled.code,
          keep,
        ],
      );
    } catch (e) {
      AppLogger.d('修剪传输历史失败（忽略）: $e');
    }
  }
}
