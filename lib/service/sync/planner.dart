/// 同步规划器 —— 3-way diff（本地 vs 云端 vs DB 基线）。
///
/// 严格对齐 Rust 原版 `src/sync/planner.rs`：
/// 输入 `SyncSnapshot { local, cloud, db, cloudTreeTrusted, isStartupResume }`，
/// 对 local ∪ cloud ∪ db 的每一路径执行 `_decide` 决策表，
/// 输出动作列表（不含无 cloudFile 的 Skip；云端不可信时抑制删除动作）。
library;

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/types/enums.dart';

/// DB 记录快照（只取 plan 需要的字段，对齐 Rust `DbSnapshotEntry`）。
class DbSnapshotEntry {
  /// 云端 fileId（pending: 前缀表示待上传占位项）
  final String fileId;

  /// 本地 mtime 基线（毫秒 epoch）
  final int? localMtime;

  /// 本地大小基线
  final int? localSize;

  /// 云端 editedTime 基线（毫秒 epoch）
  final int? cloudEditedTime;

  /// 同步状态
  final SyncItemStatus status;

  /// 是否目录
  final bool isFolder;

  const DbSnapshotEntry({
    required this.fileId,
    this.localMtime,
    this.localSize,
    this.cloudEditedTime,
    this.status = SyncItemStatus.synced,
    this.isFolder = false,
  });

  /// 从完整 SyncItem 提取快照
  factory DbSnapshotEntry.fromItem(SyncItem item) {
    return DbSnapshotEntry(
      fileId: item.fileId,
      localMtime: item.localMtime,
      localSize: item.localSize,
      cloudEditedTime: item.cloudEditedTime,
      status: item.status,
      isFolder: item.isFolder,
    );
  }
}

/// 同步快照（3 方数据视图，对齐 Rust `SyncSnapshot`）。
class SyncSnapshot {
  /// 本地文件条目（relPath → entry）
  final Map<String, LocalFileEntry> local;

  /// 云端文件树（relPath → DriveFile）
  final Map<String, DriveFile> cloud;

  /// DB 同步记录（relPath → DB 快照）
  final Map<String, DbSnapshotEntry> db;

  /// cloud 是否来自完整分页并与 cursor 同批原子提交的可信 checkpoint。
  /// false 时「云端不存在」只是未知事实，不能驱动任一方向的删除。
  final bool cloudTreeTrusted;

  /// 是否为启动恢复期（影响删除语义）
  final bool isStartupResume;

  const SyncSnapshot({
    required this.local,
    required this.cloud,
    required this.db,
    this.cloudTreeTrusted = false,
    this.isStartupResume = false,
  });
}

/// 本地是否变更（mtime 或 size 与 DB 不同，对齐 Rust `is_local_changed`）。
bool isLocalChanged(LocalFileEntry local, DbSnapshotEntry db) {
  final dbMtime = db.localMtime;
  if (dbMtime == null) return true; // 首次记录
  if (local.mtime != dbMtime) return true;
  // 同时检查 localSize（避免 mtime 精度不足漏判）
  final dbSize = db.localSize;
  if (dbSize != null && local.size != dbSize) return true;
  return false;
}

/// 云端是否变更（仅比较 editedTime，云端时间为权威基准，
/// 对齐 Rust `is_cloud_changed`）。
bool isCloudChanged(DriveFile cloud, DbSnapshotEntry db) {
  final edited = cloud.editedTime;
  if (edited == null) return false;
  final cloudMs = edited.millisecondsSinceEpoch;
  final dbMs = db.cloudEditedTime;
  if (dbMs == null) return true;
  return cloudMs != dbMs;
}

/// 同步规划器（对齐 Rust `SyncPlanner`）。
class SyncPlanner {
  /// 执行 diff，返回动作列表（跳过无 cloudFile 的 Skip 类型）。
  List<SyncAction> plan(SyncSnapshot snapshot) {
    // 收集全部路径（local ∪ cloud ∪ db）
    final allPaths = <String>{
      ...snapshot.local.keys,
      ...snapshot.cloud.keys,
      ...snapshot.db.keys,
    };

    final actions = <SyncAction>[];
    for (final relPath in allPaths) {
      final action = _decide(relPath, snapshot);
      if (action == null) continue;
      if (!snapshot.cloudTreeTrusted &&
          (action.actionType == SyncActionType.deleteFromLocal ||
              action.actionType == SyncActionType.deleteFromCloud)) {
        AppLogger.w('云端 checkpoint 不可信，抑制删除动作: '
            '${action.actionType.wireName} $relPath');
        continue;
      }
      // 过滤 Skip；例外：携带 cloudFile 的 Skip 是 pending 占位项的收敛动作
      // （上次失败实为成功），必须放行到 engine 结算真实 fileId。
      if (action.actionType == SyncActionType.skip &&
          action.cloudFile == null) {
        continue;
      }
      actions.add(action);
    }
    return actions;
  }

  /// 单路径决策（对齐 Rust `_decide`）。
  SyncAction? _decide(String relPath, SyncSnapshot snap) {
    final local = snap.local[relPath];
    final cloud = snap.cloud[relPath];
    final db = snap.db[relPath];

    final localExists = local != null;
    final localHasContent = local != null && !local.isPlaceholder;
    final cloudExists = cloud != null;
    final dbExists = db != null;

    // === 文件夹 ===
    if (cloud != null && cloud.isFolder) {
      if (!localExists) {
        // 会话内本地删除目录 → 同步删除云端（用户主动行为）
        if (dbExists && !snap.isStartupResume) {
          return SyncAction(
            actionType: SyncActionType.deleteFromCloud,
            relativePath: relPath,
            fileId: cloud.id,
            reason: '本地目录已删除 → 同步删除云端',
          );
        }
        // 启动恢复期 + DELETED tombstone → 跳过（不重建）
        if (dbExists &&
            snap.isStartupResume &&
            db.status == SyncItemStatus.deleted) {
          return null;
        }
        // 否则本地缺失 → 创建文件夹
        return SyncAction(
          actionType: SyncActionType.createFolder,
          relativePath: relPath,
          fileId: cloud.id,
          parentFileId: cloud.parentId,
          cloudFile: cloud,
          reason: '云端文件夹 → 本地创建',
        );
      }
      // 双方都已有文件夹 → skip
      return null;
    }

    // === 全缺席 ===
    if (!localExists && !cloudExists && !dbExists) return null;

    // === 三方都存在（文件）===
    if (localHasContent && cloudExists && dbExists) {
      // pending: 占位项 + 云端已有 → 上次「失败」其实成功，收敛为已同步。
      // 用 Skip 携带真实 cloudFile：engine 结算时 upsert 真实 fileId +
      // status=SYNCED + 清理 pending 孤儿行，避免重复上传与 Download 覆盖。
      if (db.fileId.startsWith(pendingFileIdPrefix)) {
        return SyncAction(
          actionType: SyncActionType.skip,
          relativePath: relPath,
          fileId: cloud.id,
          cloudFile: cloud,
          reason: 'pending 占位项发现云端已有 → 收敛为已同步（上次失败实为成功）',
        );
      }
      final localChanged = isLocalChanged(local, db);
      final cloudChanged = isCloudChanged(cloud, db);
      if (localChanged && cloudChanged) {
        return SyncAction(
          actionType: SyncActionType.createConflictCopy,
          relativePath: relPath,
          fileId: cloud.id,
          localPath: local.absolutePath,
          cloudFile: cloud,
          reason: '三方都存在，本地/云端均已修改 → 冲突',
        );
      } else if (localChanged) {
        return SyncAction(
          actionType: SyncActionType.upload,
          relativePath: relPath,
          fileId: cloud.id,
          localPath: local.absolutePath,
          reason: '本地已修改 → 上传',
        );
      } else if (cloudChanged) {
        return SyncAction(
          actionType: SyncActionType.download,
          relativePath: relPath,
          fileId: cloud.id,
          localPath: local.absolutePath,
          cloudFile: cloud,
          reason: '云端已修改 → 下载',
        );
      }
      return null; // 未变化 → skip
    }

    // === 本地有内容 + 云端有 + 无 DB（首次记录兜底，由 reconcile 补 DB）===
    if (localExists && cloudExists && !dbExists) {
      return SyncAction(
        actionType: SyncActionType.skip,
        relativePath: relPath,
        fileId: cloud.id,
        reason: '双方都有但无 DB 记录 → skip，由 reconcile 补 DB',
      );
    }

    // === 本地有 + 云端无 ===
    if (localExists && !cloudExists) {
      if (dbExists) {
        // pending: 占位项（新增上传失败 / retry 后仍未成功）→ 重新计划上传。
        // 绝不能走 BackupBeforeCloudDelete / DeleteFromLocal（数据丢失）。
        // FAILED 状态的占位项不再自动重试，留给用户手动重试。
        if (db.fileId.startsWith(pendingFileIdPrefix)) {
          if (db.status == SyncItemStatus.failed) {
            return null;
          }
          return SyncAction(
            actionType: SyncActionType.upload,
            relativePath: relPath,
            localPath: local.absolutePath,
            reason: 'pending 占位项（上传待重试）→ 重新上传',
          );
        }
        // 启动恢复期删除守卫：DB 有真实 fileId 且本地未改的文件，
        // 绝不直接删除，等下一次 BFS 成功后重新判定。
        if (snap.isStartupResume && !isLocalChanged(local, db)) {
          return SyncAction(
            actionType: SyncActionType.skip,
            relativePath: relPath,
            fileId: db.fileId,
            reason: '启动恢复期 cloud_tree 不可信，跳过删除待复核',
          );
        }
        // 文件夹：同样生成 DeleteFromLocal，由 engine 层判断是否需要保留
        if (local.isFolder) {
          return SyncAction(
            actionType: SyncActionType.deleteFromLocal,
            relativePath: relPath,
            fileId: db.fileId,
            localPath: local.absolutePath,
            reason: '云端已删除文件夹 → 同步删除本地',
          );
        }
        // 文件：本地有未上传的真实修改 → 改名备份副本（冲突保护）
        if (localHasContent && isLocalChanged(local, db)) {
          return SyncAction(
            actionType: SyncActionType.backupBeforeCloudDelete,
            relativePath: relPath,
            fileId: db.fileId,
            localPath: local.absolutePath,
            reason: '云端已删除但本地有未上传修改 → 备份副本',
          );
        }
        // 未改 / 占位 → 删除本地（匹配云端删除）
        return SyncAction(
          actionType: SyncActionType.deleteFromLocal,
          relativePath: relPath,
          fileId: db.fileId,
          localPath: local.absolutePath,
          reason: '云端已删除 → 删除本地',
        );
      }
      if (!localHasContent) {
        // 本地占位符且无 DB → 孤儿占位符清理
        return SyncAction(
          actionType: SyncActionType.deleteFromLocal,
          relativePath: relPath,
          localPath: local.absolutePath,
          reason: '孤儿占位符 → 清理',
        );
      }
      // 本地新文件夹 → 新建云端文件夹
      if (local.isFolder) {
        return SyncAction(
          actionType: SyncActionType.createFolder,
          relativePath: relPath,
          localPath: local.absolutePath,
          reason: '本地新增文件夹 → 创建云端文件夹',
        );
      }
      // 本地新文件 → 上传
      return SyncAction(
        actionType: SyncActionType.upload,
        relativePath: relPath,
        localPath: local.absolutePath,
        reason: '本地新文件 → 上传',
      );
    }

    // === 本地无 + 云端有 ===
    if (!localExists && cloudExists) {
      if (dbExists && !snap.isStartupResume) {
        // 会话内删除 → 双向删除云端
        return SyncAction(
          actionType: SyncActionType.deleteFromCloud,
          relativePath: relPath,
          fileId: cloud.id,
          reason: '会话内删除 → 双向删除云端',
        );
      }
      // 启动恢复期 / 无 DB：检查是否是用户主动删除的 tombstone
      if (dbExists &&
          snap.isStartupResume &&
          db.status == SyncItemStatus.deleted) {
        return SyncAction(
          actionType: SyncActionType.skip,
          relativePath: relPath,
          reason: '用户已删除（tombstone）→ 跳过',
        );
      }
      // 启动恢复期 或 无 DB → 创建占位符
      final reason = snap.isStartupResume && dbExists
          ? '启动后恢复删除 → 重建占位'
          : '云端新文件 → 创建占位';
      return SyncAction(
        actionType: SyncActionType.createPlaceholder,
        relativePath: relPath,
        fileId: cloud.id,
        cloudFile: cloud,
        reason: reason,
      );
    }

    // === 本地无 + 云端无 + DB 有（双方都删了，或云端树缓存滞后）===
    // 不发 API，由 engine 在周期末尾统一清 DB 残余。
    return null;
  }
}
