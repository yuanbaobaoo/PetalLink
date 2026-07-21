/// 基于 inode 的本地移动检测与 planner 动作合并
/// （对标 CMP `InodeMoveDetector` + `LocalMoveActionReconciler`，
/// 设计 docs/design/10 §4.3）。
///
/// 核心性质：cp 复制产生新 inode，因此副本天然被当新文件处理——
/// 原 xattr 方案的复制消歧分支在结构上不需要存在。
library;

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/engine/cache.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/service/sync/sync_actions.dart';

/// 扫描快照中基于稳定 inode 配对出的本地移动。
class DetectedMove {
  /// 文件系统 inode
  final int inode;

  /// 云端身份（来自 inode 映射）
  final String fileId;

  /// 旧相对路径（映射中记录的上次路径）
  final String oldRelativePath;

  /// 新相对路径（本轮扫描路径）
  final String newRelativePath;

  /// 新路径的绝对路径（执行器本地身份复核用）
  final String? newLocalPath;

  const DetectedMove({
    required this.inode,
    required this.fileId,
    required this.oldRelativePath,
    required this.newRelativePath,
    this.newLocalPath,
  });
}

/// 基于稳定 inode 的移动检测（对标 CMP `InodeMoveDetector.detectMoves`）。
///
/// 同 inode 且路径改变才输出 move；成功识别后立即更新路径映射
/// （不自动 purge，由完整扫描提交者统一执行）。
Future<List<DetectedMove>> detectMoves(
  Iterable<LocalFileEntry> entries,
  InodeIdentityStore identity,
) async {
  final sorted = entries.where((e) => e.inode != null).toList()
    ..sort((a, b) => a.relativePath.compareTo(b.relativePath));
  final moves = <DetectedMove>[];
  for (final e in sorted) {
    final old = await identity.lookup(e.inode!);
    if (old == null) continue;
    if (old.relativePath == e.relativePath) continue;
    AppLogger.i('检测到本地文件路径变化（inode 未变）：'
        '${old.relativePath} → ${e.relativePath}');
    moves.add(DetectedMove(
      inode: e.inode!,
      fileId: old.fileId,
      oldRelativePath: old.relativePath,
      newRelativePath: e.relativePath,
      newLocalPath: e.absolutePath,
    ));
    await identity.upsert(e.inode!, e.relativePath, old.fileId);
  }
  return moves;
}

/// 目录整体移动时只保留最上层移动（对标 CMP `collapseNested`）：
/// 后代路径由云端子树移动一并完成。
List<DetectedMove> collapseNested(List<DetectedMove> moves) {
  final accepted = <DetectedMove>[];
  final byDepth = moves.toList()
    ..sort((a, b) => '/'.allMatches(a.oldRelativePath).length.compareTo(
        '/'.allMatches(b.oldRelativePath).length));
  for (final move in byDepth) {
    final nested = accepted.any((parent) =>
        move.oldRelativePath.startsWith('${parent.oldRelativePath}/') &&
        move.newRelativePath ==
            parent.newRelativePath +
                move.oldRelativePath
                    .substring(parent.oldRelativePath.length));
    if (!nested) accepted.add(move);
  }
  return accepted;
}

/// 把 inode 检测到的本地移动合并为最小云端移动动作集合
/// （对标 CMP `LocalMoveActionReconciler.reconcile`）：
/// 移除被移动根覆盖的 planner 动作（upload/deleteFromCloud 等），
/// 替换为 MoveInCloud。云树缺旧路径条目时保守跳过（按删+增处理）。
void applyDetectedMoves(
  List<SyncAction> actions,
  List<DetectedMove> detected,
  CloudTreeIndex cloud,
) {
  if (detected.isEmpty) return;
  final collapsed = collapseNested(detected);
  // 仅合并能证明云端身份的移动（云树有旧路径条目）；
  // 无法证明的保守按删+增处理（本轮不覆盖其 planner 动作）
  final moves = <DetectedMove>[];
  for (final move in collapsed) {
    if (cloud.tree[move.oldRelativePath] == null) {
      AppLogger.w('移动源不在可信云树，按删+增处理: ${move.oldRelativePath}');
      continue;
    }
    moves.add(move);
  }
  if (moves.isEmpty) return;
  final removedRoots = <String>{
    for (final m in moves) ...[m.oldRelativePath, m.newRelativePath],
  };
  actions.removeWhere((action) {
    final rel = action.relativePath;
    if (rel == null) return false;
    return removedRoots
        .any((root) => rel == root || rel.startsWith('$root/'));
  });
  for (final move in moves) {
    final remote = cloud.tree[move.oldRelativePath]!;
    final parentPath = move.newRelativePath.contains('/')
        ? move.newRelativePath
            .substring(0, move.newRelativePath.lastIndexOf('/'))
        : '';
    actions.add(SyncAction(
      actionType: SyncActionType.moveInCloud,
      relativePath: move.newRelativePath,
      localPath: move.newLocalPath,
      fileId: move.fileId,
      cloudFile: remote,
      parentFileId:
          parentPath.isEmpty ? cloud.rootFolderId : cloud.pathToId[parentPath],
      reason: 'inode 未变且路径变化 → 云端移动 '
          '${move.oldRelativePath} → ${move.newRelativePath}',
    ));
  }
}
