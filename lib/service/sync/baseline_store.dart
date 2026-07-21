// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// sync_items 基线结算（对齐 Rust `task_runner/settlement.rs` 的 sync_items
/// 部分 + `persistence.rs` 的兼容性状态回写）。
///
/// TaskRunner 完成任务行迁移后经钩子回调本存储，在同一个 DB 事务内完成：
/// - `pending:` 占位行清理（上传/下载成功都执行）
/// - Update 时同一 fileId 旧路径行清理（改名/移动收敛）
/// - 成功基线 upsert（status=SYNCED）
/// - Failed 时按旧状态白名单标记失败
/// - retry 接受 / replan 时 SYNCING 回写
/// - Committed 后 xattr 回写（fileId / downloaded，尽力而为）
library;

import 'dart:io';

import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// sync_items 基线结算存储（实现 TaskRunner 的同步钩子接缝）。
class SyncBaselineStore implements SyncTaskHooks {
  /// inode 身份映射（docs/design/10；引擎装配时注入共享实例）
  InodeIdentityStore identity = SqfliteInodeIdentityStore(DatabaseService.instance);

  /// 数据库服务
  final DatabaseService _db;

  /// 挂载管理器提供（xattr 回写用；未配置时跳过 xattr）
  final MountManager? Function() _mountProvider;

  /// 当前毫秒时钟（测试注入）
  final int Function() _nowMs;

  SyncBaselineStore({
    required DatabaseService db,
    MountManager? Function()? mountProvider,
    int Function()? nowMs,
  })  : _db = db,
        _mountProvider = mountProvider ?? (() => null),
        _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch);

  // ═══════════════════════════════════════════════════════════════════
  // SyncTaskHooks：Committed 基线结算 + xattr 回写
  // ═══════════════════════════════════════════════════════════════════

  /// 任务完成后的基线结算（对齐 Rust `settle_success` 的 sync_items 部分）。
  ///
  /// 版本事实来源：
  /// - 上传（Create/Update）：取入队快照 sourceMtime/sourceSize（实际送达的
  ///   字节），不读路径当前内容（上传期间的编辑留给下轮 planner 以 Update 处理）
  /// - 下载（Download/DownloadUpdate）：现读本地文件 metadata，必须是普通文件
  @override
  Future<void> onTaskCommitted(
    TransferTask running,
    TaskExecutionOutcome outcome,
  ) async {
    final operation = running.operation;
    final rel = running.relativePath;
    final localPath = running.localPath;
    if (operation == null || rel == null || localPath == null) {
      AppLogger.w('任务 ${running.id} 缺少 operation/relativePath/localPath，'
          '跳过基线结算');
      return;
    }
    final isUpload = operation == TransferOperation.create ||
        operation == TransferOperation.update;
    final isDownload = operation == TransferOperation.download ||
        operation == TransferOperation.downloadUpdate;
    if (!isUpload && !isDownload) return;

    // ---- 版本事实 ----
    final int localMtime;
    final int localSize;
    if (isUpload) {
      final mtime = running.sourceMtime;
      final size = running.sourceSize;
      if (mtime == null || size == null) {
        throw AppError.generic('上传任务缺少源快照，拒绝结算基线');
      }
      localMtime = mtime;
      localSize = size;
    } else {
      final stat = await FileStat.stat(localPath);
      if (stat.type != FileSystemEntityType.file) {
        throw AppError.generic('下载结算时本地文件不存在或不是普通文件');
      }
      localMtime = stat.modified.millisecondsSinceEpoch;
      localSize = stat.size;
    }

    // ---- 云端事实 ----
    final String fileId;
    final String name;
    final int size;
    final int? cloudEditedTime;
    final String? parentFolderId;
    if (isUpload) {
      final cloud = outcome.cloudFile;
      if (cloud == null) {
        throw AppError.generic('上传成功但缺少远端文件结果，拒绝结算');
      }
      fileId = cloud.id;
      name = cloud.name;
      size = cloud.size;
      cloudEditedTime = cloud.editedTime?.millisecondsSinceEpoch;
      parentFolderId = cloud.parentId ?? running.parentFileId;
    } else {
      final fid = running.fileId;
      if (fid == null) {
        throw AppError.generic('下载任务缺少 fileId，拒绝结算');
      }
      fileId = fid;
      name = running.name;
      size = running.totalSize;
      cloudEditedTime = running.expectedCloudEditedTime;
      parentFolderId = running.parentFileId;
    }
    final finishedAt = _nowMs();

    final db = await _db.database;
    await db.transaction((txn) async {
      // 1. Update 且新云端版本缺失时保留旧云端版本
      int? preservedEditedTime;
      if (operation == TransferOperation.update && cloudEditedTime == null) {
        final rows = await txn.query('sync_items',
            columns: ['cloud_edited_time'],
            where: 'file_id = ?',
            whereArgs: [fileId],
            limit: 1);
        if (rows.isNotEmpty) {
          final v = rows.first['cloud_edited_time'];
          preservedEditedTime = v is int ? v : int.tryParse('$v');
        }
      }
      // 2. pending: 占位行清理（PK 是 (file_id, local_path)，upsert 不会
      //    覆盖 pending 占位行，必须显式删）
      await txn.delete('sync_items',
          where: 'local_path = ? AND file_id = ?',
          whereArgs: [rel, '$pendingFileIdPrefix$rel']);
      // 3. 仅 Update：清同一 fileId 的改名/移动旧路径行
      if (operation == TransferOperation.update) {
        await txn.delete('sync_items',
            where: 'file_id = ? AND local_path <> ?',
            whereArgs: [fileId, rel]);
      }
      // 4. upsert 成功基线
      final item = SyncItem(
        fileId: fileId,
        localPath: rel,
        parentFolderId: parentFolderId,
        name: name,
        isFolder: false,
        size: size,
        localSize: localSize,
        localMtime: localMtime,
        cloudEditedTime: cloudEditedTime ?? preservedEditedTime,
        lastSyncTime: finishedAt,
        status: SyncItemStatus.synced,
      );
      await txn.insert('sync_items', item.toRow(),
          conflictAlgorithm: ConflictAlgorithm.replace);
    });

    // ---- 身份记账（Committed 后；docs/design/10 §4.7/§5：
    // 确定性记账取代 xattr 补写——失败不阻塞结算，但不静默丢失身份语义）
    final mount = _mountProvider();
    if (mount != null) {
      if (isUpload) {
        // 仅当源快照仍匹配才记账（上传期间被编辑则不记）
        try {
          final stat = await FileStat.stat(localPath);
          if (stat.type == FileSystemEntityType.file &&
              stat.modified.millisecondsSinceEpoch == localMtime &&
              stat.size == localSize) {
            await _upsertInodeIdentity(mount, localPath, fileId);
          }
        } catch (e) {
          AppLogger.w('上传后 inode 身份记账失败（忽略）: $e');
        }
      } else {
        // 下载：先删占位再下载 → inode 更换，必须重新记账（新 inode）
        try {
          await mount.markDownloaded(localPath);
        } catch (e) {
          AppLogger.w('下载后回写 downloaded xattr 失败（忽略）: $e');
        }
        try {
          await _upsertInodeIdentity(mount, localPath, fileId);
        } catch (e) {
          AppLogger.w('下载后 inode 身份记账失败（忽略）: $e');
        }
      }
    }
  }

  /// 经批量通道取文件 inode 并写入身份映射（程序自己操作文件的确定性记账）。
  Future<void> _upsertInodeIdentity(
      MountManager mount, String localPath, String fileId) async {
    final provider = mount.inodeBatchProvider;
    if (provider == null) return;
    final rel = MountPath.relativePathFromMount(mount.mountDir, localPath);
    final inodes = await provider([localPath]);
    final inode = inodes[localPath];
    if (inode != null) {
      await identity.upsert(inode, rel, fileId);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // SyncTaskHooks：Failed / retry / replan 回写
  // ═══════════════════════════════════════════════════════════════════

  /// 任务永久失败时标记 sync_items（对齐 Rust `mark_compatibility_sync_failed`）。
  ///
  /// 仅覆盖旧状态 ∈ {SYNCED, SYNCING, CLOUD_ONLY, FAILED} 的行
  /// （DELETED/CONFLICT 不会被覆盖）；fileId 缺省回退 `pending:` 占位。
  @override
  Future<void> onTaskFailed(TransferTask failed, String message) async {
    final rel = failed.relativePath;
    if (rel == null) return;
    final fileId = failed.fileId ?? '$pendingFileIdPrefix$rel';
    final db = await _db.database;
    await db.rawUpdate(
      'UPDATE sync_items SET status = ?, error_message = ? '
      'WHERE file_id = ? AND local_path = ? AND status IN (?, ?, ?, ?)',
      [
        SyncItemStatus.failed.code,
        message,
        fileId,
        rel,
        SyncItemStatus.synced.code,
        SyncItemStatus.syncing.code,
        SyncItemStatus.cloudOnly.code,
        SyncItemStatus.failed.code,
      ],
    );
  }

  /// retry 接受时的 SYNCING 回写（对齐 Rust `accept_retry_after_preflight`：
  /// 仅当旧状态为 FAILED 时回写）。
  @override
  Future<void> onRetryAccepted(TransferTask pending) async {
    final rel = pending.relativePath;
    if (rel == null) return;
    final fileId = pending.fileId ?? '$pendingFileIdPrefix$rel';
    final db = await _db.database;
    await db.rawUpdate(
      'UPDATE sync_items SET status = ?, error_message = NULL '
      'WHERE file_id = ? AND local_path = ? AND status = ?',
      [SyncItemStatus.syncing.code, fileId, rel, SyncItemStatus.failed.code],
    );
  }

  /// replan 接受时的 SYNCING 回写（无旧状态条件，对齐 Rust
  /// `replan_task` 的 `update_compatibility_sync_status(..., None)`）。
  @override
  Future<void> onTaskReplanned(TransferTask task) async {
    final rel = task.relativePath;
    if (rel == null) return;
    final fileId = task.fileId ?? '$pendingFileIdPrefix$rel';
    final db = await _db.database;
    await db.rawUpdate(
      'UPDATE sync_items SET status = ?, error_message = NULL '
      'WHERE file_id = ? AND local_path = ?',
      [SyncItemStatus.syncing.code, fileId, rel],
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 周期级基线维护
  // ═══════════════════════════════════════════════════════════════════

  /// 启动期清理滞留 SYNCING（对齐 Rust `reset_stale_statuses`；
  /// FAILED 保留）。
  Future<void> resetStaleStatuses() async {
    final db = await _db.database;
    await db.rawUpdate(
      'UPDATE sync_items SET status = ? WHERE status = ?',
      [SyncItemStatus.synced.code, SyncItemStatus.syncing.code],
    );
  }

  /// 全局重试收尾：无对应 Failed 任务的 FAILED 行置回 SYNCING
  /// （对齐 Rust RETRY 位的批量 SQL）。
  Future<void> sweepFailedWithoutFailedTasks() async {
    final db = await _db.database;
    await db.rawUpdate(
      'UPDATE sync_items SET status = ?, error_message = NULL '
      'WHERE status = ? AND NOT EXISTS('
      'SELECT 1 FROM transfer_queue task '
      'WHERE task.relative_path = sync_items.local_path AND task.state = ?)',
      [
        SyncItemStatus.syncing.code,
        SyncItemStatus.failed.code,
        TransferState.failed.code,
      ],
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 通用查询（reconciliation / results / facade 复用）
  // ═══════════════════════════════════════════════════════════════════

  /// 加载全部 sync_items（对齐 Rust `repository::load_all`）。
  Future<List<SyncItem>> loadAll() async {
    final db = await _db.database;
    final rows = await db.query('sync_items');
    return rows.map(SyncItem.fromRow).toList();
  }

  /// upsert 单条基线（INSERT OR REPLACE，PK=(file_id, local_path)）。
  Future<void> upsert(SyncItem item, [DatabaseExecutor? executor]) async {
    final db = executor ?? await _db.database;
    await db.insert('sync_items', item.toRow(),
        conflictAlgorithm: ConflictAlgorithm.replace);
  }

  /// 是否存在匹配路径的活动传输（活动 = 非 Completed/Failed/Canceled）。
  Future<bool> hasActiveTransferAt(String relPath) async {
    final db = await _db.database;
    final rows = await db.rawQuery(
      'SELECT COUNT(*) AS c FROM transfer_queue '
      'WHERE relative_path = ? AND state NOT IN (?, ?, ?)',
      [
        relPath,
        TransferState.completed.code,
        TransferState.failed.code,
        TransferState.canceled.code,
      ],
    );
    final count = rows.first['c'];
    return count is int && count > 0;
  }
}
