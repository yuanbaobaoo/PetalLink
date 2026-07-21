/// 云端树索引 —— BFS 全量构建 + Changes 增量回放 + 可信 checkpoint 持久化。
///
/// 严格对齐 Rust 原版 `src/sync/cloud_tree.rs` + `src/core/cache_paths.rs`
/// + `src/sync/sync_state_store.rs`：
/// - BFS 并发 8、单目录失败重试 2 次；BFS 只构建候选树，无任何本地副作用
/// - checkpoint {tree, path_to_id, cursor} 原子写：临时文件 + fsync →
///   旧版本备份 → 同目录 rename → 尽力 fsync 父目录
/// - 缓存文件 `cloudtree_<escaped>.json` / `syncstate_<escaped>.json`
///   （escaped 转义规则对齐 cache_paths.rs）
/// - `complete=true`、非空 cursor、tree/pathToId 内部一致才可加载为 trusted；
///   严格完整扫描得到的空 tree 是合法的云盘状态，不能按条目数量判坏
///
/// 平台差异说明：dart:io 无 hard link 与目录 fsync 的可靠 API，
/// 备份用文件复制、父目录 fsync 为尽力而为（rename 同目录原子性仍是提交边界）。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/mount/skip.dart';

/// BFS 最大并发数（对齐 Rust INDEXING_CONCURRENCY）
const int indexingConcurrency = 8;

/// BFS 单目录最大重试次数
const int indexingMaxRetries = 2;

/// 缓存路径工具（对齐 Rust `src/core/cache_paths.rs`）。
class CachePaths {
  CachePaths._();

  /// 挂载路径转义：保留 `[A-Za-z0-9._-]`，其余替换为 `_`。
  static String escapeMountPath(String mountDir) {
    final buf = StringBuffer();
    for (final rune in mountDir.runes) {
      final c = String.fromCharCode(rune);
      final safe = (rune >= 0x30 && rune <= 0x39) || // 0-9
          (rune >= 0x41 && rune <= 0x5A) || // A-Z
          (rune >= 0x61 && rune <= 0x7A) || // a-z
          c == '.' ||
          c == '_' ||
          c == '-';
      buf.write(safe ? c : '_');
    }
    return buf.toString();
  }

  /// 云端树 checkpoint 文件：`<support>/cloudtree_<escaped>.json`
  static Future<File> cloudTreeCacheFile(String mountDir) async {
    final dir = await AppPaths.supportDir();
    return File(
        p.join(dir.path, 'cloudtree_${escapeMountPath(mountDir)}.json'));
  }

  /// 本地快照缓存文件：`<support>/syncstate_<escaped>.json`
  static Future<File> syncStateCacheFile(String mountDir) async {
    final dir = await AppPaths.supportDir();
    return File(
        p.join(dir.path, 'syncstate_${escapeMountPath(mountDir)}.json'));
  }

  /// 旧版独立 cursor 文件（仅清理用）：`<support>/changes_cursor_<escaped>.txt`
  static Future<File> legacyChangesCursorFile(String mountDir) async {
    final dir = await AppPaths.supportDir();
    return File(
        p.join(dir.path, 'changes_cursor_${escapeMountPath(mountDir)}.txt'));
  }

  /// 清理指定挂载目录对应的全部缓存文件（错误忽略）。
  static Future<void> clearForMount(String mountDir) async {
    for (final f in [
      await cloudTreeCacheFile(mountDir),
      await syncStateCacheFile(mountDir),
      await legacyChangesCursorFile(mountDir),
    ]) {
      try {
        if (await f.exists()) await f.delete();
      } catch (_) {
        // 尽力清理
      }
    }
  }

  /// 清理 support 目录下全部历史缓存文件（登出/清缓存用）。
  static Future<void> clearAll() async {
    final dir = await AppPaths.supportDir();
    if (!await dir.exists()) return;
    await for (final entity in dir.list(followLinks: false)) {
      if (entity is! File) continue;
      final name = p.basename(entity.path);
      if (name.startsWith('cloudtree_') ||
          name.startsWith('syncstate_') ||
          name.startsWith('changes_cursor_')) {
        try {
          await entity.delete();
        } catch (_) {
          // 尽力清理
        }
      }
    }
  }
}

/// 可原子提交的云端树、路径索引与增量游标检查点（对齐 Rust `CloudTreeCache`）。
class CloudTreeCache {
  /// 根目录 fileId（BFS 动态发现）
  final String? rootFolderId;

  /// 相对路径 → 云端文件
  final Map<String, DriveFile> tree;

  /// 相对路径 → fileId（含 `""` → rootFolderId 反查项）
  final Map<String, String> pathToId;

  /// 与 tree/pathToId 同批应用完成的 Changes 末页 newStartCursor
  final String? cursor;

  /// 候选是否已完成全量/增量应用（旧缓存无此字段 → false 不可信）
  final bool complete;

  const CloudTreeCache({
    this.rootFolderId,
    required this.tree,
    required this.pathToId,
    this.cursor,
    this.complete = false,
  });

  /// 构造一个可提交的完整候选 checkpoint（对齐 Rust `new_trusted`）。
  factory CloudTreeCache.newTrusted(
    String? rootFolderId,
    Map<String, DriveFile> tree,
    Map<String, String> pathToId,
    String cursor,
  ) {
    // 根目录本身不在 tree 中，但增量 merge 需要 rootId → "" 反查
    final index = Map<String, String>.of(pathToId);
    if (rootFolderId != null) {
      index.putIfAbsent('', () => rootFolderId);
    }
    final checkpoint = CloudTreeCache(
      rootFolderId: rootFolderId,
      tree: tree,
      pathToId: index,
      cursor: cursor,
      complete: true,
    );
    checkpoint.validateTrusted();
    return checkpoint;
  }

  /// 校验 checkpoint 是否足以作为删除决策的可信远端事实。
  void validateTrusted() {
    if (!complete) {
      throw AppError.generic('云端 checkpoint 未完整提交');
    }
    final c = cursor;
    if (c == null || c.trim().isEmpty) {
      throw AppError.generic('云端 checkpoint 缺少有效 cursor');
    }
    final seenIds = <String>{};
    for (final entry in tree.entries) {
      if (entry.key.isEmpty || entry.value.id.trim().isEmpty) {
        throw AppError.generic('云端 checkpoint 包含空路径或空 fileId');
      }
      if (!seenIds.add(entry.value.id)) {
        throw AppError.generic('云端 checkpoint 中 fileId 重复：${entry.value.id}');
      }
      if (pathToId[entry.key] != entry.value.id) {
        throw AppError.generic('云端 checkpoint 的路径索引不一致：${entry.key}');
      }
    }
    for (final entry in pathToId.entries) {
      if (entry.key.isEmpty) {
        if (rootFolderId != entry.value) {
          throw AppError.generic('云端 checkpoint 的根目录索引不一致');
        }
        continue;
      }
      if (tree[entry.key]?.id != entry.value) {
        throw AppError.generic('云端 checkpoint 包含孤立路径索引：${entry.key}');
      }
    }
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde 输出）。
  Map<String, dynamic> toJson() {
    return {
      'root_folder_id': rootFolderId,
      'tree': tree.map((k, v) => MapEntry(k, v.toJson())),
      'path_to_id': pathToId,
      'cursor': cursor,
      'complete': complete,
    };
  }

  /// 从 JSON 构造；字段缺失时按旧缓存处理（不可信）。
  factory CloudTreeCache.fromJson(Map<String, dynamic> json) {
    final rawTree = json['tree'];
    final tree = <String, DriveFile>{};
    if (rawTree is Map) {
      for (final entry in rawTree.entries) {
        final value = entry.value;
        if (entry.key is String && value is Map<String, dynamic>) {
          tree[entry.key as String] = DriveFile.fromJson(value);
        }
      }
    }
    final rawIndex = json['path_to_id'];
    final pathToId = <String, String>{};
    if (rawIndex is Map) {
      for (final entry in rawIndex.entries) {
        if (entry.key is String && entry.value is String) {
          pathToId[entry.key as String] = entry.value as String;
        }
      }
    }
    return CloudTreeCache(
      rootFolderId: json['root_folder_id'] as String?,
      tree: tree,
      pathToId: pathToId,
      cursor: json['cursor'] as String?,
      complete: json['complete'] == true,
    );
  }
}

/// BFS folder 节点。
class _BfsNode {
  final String? folderId;
  final String path;
  final int retries;

  const _BfsNode({this.folderId, required this.path, this.retries = 0});

  _BfsNode retry() =>
      _BfsNode(folderId: folderId, path: path, retries: retries + 1);
}

/// BFS 扫描进度回调：已扫描目录数、已发现条目数。
typedef CloudScanProgressFn = void Function(int scannedFolders, int items);

/// 构建云端文件树（BFS，对齐 Rust `refresh_cloud_tree`）。
///
/// 返回 (tree, pathToId, rootFolderId) 完整候选；调用方必须先从扫描前
/// cursor 重放 Changes，再把最终 cursor 与候选同批持久化后才允许安装。
/// 单目录失败重试 2 次；重试耗尽直接失败（子树缺失不可接受）。
Future<({Map<String, DriveFile> tree, Map<String, String> pathToId, String? rootFolderId})>
    refreshCloudTree(
  FilesService filesApi, {
  CloudScanProgressFn? onProgress,
}) async {
  final tree = <String, DriveFile>{};
  final pathToId = <String, String>{};
  String? rootFolderId;
  final visited = <String>{};

  final queue = <_BfsNode>[const _BfsNode(path: '')];
  var processedFolders = 0;

  AppLogger.i('开始 BFS 云端树构建');

  while (queue.isNotEmpty) {
    final batchSize =
        queue.length < indexingConcurrency ? queue.length : indexingConcurrency;
    final batch = queue.sublist(0, batchSize);
    queue.removeRange(0, batchSize);

    final results = await Future.wait(batch.map((node) async {
      final parentId = node.path.isEmpty ? null : node.folderId;
      final result = await filesApi.listAll(parentId: parentId);
      return (node, result);
    }));

    for (final (node, result) in results) {
      if (result.isErr) {
        if (node.retries < indexingMaxRetries) {
          AppLogger.w('BFS 单文件夹失败，重试: ${node.path} (${node.retries})');
          queue.add(node.retry());
          continue;
        }
        final label = node.path.isEmpty ? '/' : node.path;
        throw AppError.generic(
            '云端树刷新不完整：目录 $label 重试耗尽：${(result as Err).error}');
      }
      final files = (result as Ok<List<DriveFile>>).value;
      // 根目录第一层：动态发现 root folder ID
      if (node.path.isEmpty && rootFolderId == null) {
        rootFolderId = detectRootFolderId(files);
        // 对齐 CMP BfsCloudTreeRefresher：平局/缺失 fail-closed，
        // 不得带着未知 root 继续构建（否则整树挂在错误根下）
        if (rootFolderId == null && files.isNotEmpty) {
          throw AppError.generic('根目录 parentFolder 最高频平局或缺失，拒绝推断 root ID');
        }
      }
      for (final f in files) {
        // 跳过 .hwcloud_ 前缀内部文件
        if (f.name.startsWith(MountSkip.internalFilePrefix)) continue;
        MountPath.validatePathSegment(f.name);
        final relPath = node.path.isEmpty ? f.name : '${node.path}/${f.name}';
        tree[relPath] = f;
        pathToId[relPath] = f.id;
        // 候选扫描必须无本地副作用
        if (f.isFolder && !visited.contains(f.id)) {
          visited.add(f.id);
          queue.add(_BfsNode(folderId: f.id, path: relPath));
        }
      }
    }

    processedFolders += batchSize;
    if (processedFolders % 5 == 0 || queue.isEmpty) {
      AppLogger.i('云端刷新进度：已扫描 $processedFolders 个目录，累计 ${tree.length} 项，'
          '队列剩余 ${queue.length}');
      onProgress?.call(processedFolders, tree.length);
    }
  }

  AppLogger.i('云端全量刷新完成：${tree.length} 项（$processedFolders 个目录）');
  return (tree: tree, pathToId: pathToId, rootFolderId: rootFolderId);
}

/// 动态发现根目录真实 folder ID（对齐 Rust `detect_root_folder_id`）。
///
/// 取根级条目 parentFolder 中唯一的最高频值；最高频并列则 fail closed（null）。
String? detectRootFolderId(List<DriveFile> files) {
  final counter = <String, int>{};
  for (final f in files) {
    for (final id in f.parentFolder ?? const <String>[]) {
      counter[id] = (counter[id] ?? 0) + 1;
    }
  }
  if (counter.isEmpty) return null;
  final candidates = counter.entries.toList()
    ..sort((a, b) {
      final byCount = b.value.compareTo(a.value);
      return byCount != 0 ? byCount : a.key.compareTo(b.key);
    });
  final maxCount = candidates.first.value;
  if (candidates.length > 1 && candidates[1].value == maxCount) return null;
  return candidates.first.key;
}

/// 将一轮 Changes 增量回放到候选树（对齐 Rust `apply_changes_to_candidate`，
/// 全部 fail-closed：任一变更无法严格解释即抛错，调用方回退全量）。
void applyChangesToCandidate(
  List<DriveChange> changes,
  Map<String, DriveFile> tree,
  Map<String, String> pathToId,
  String? rootFolderId,
) {
  for (final change in changes) {
    if (change.kind == ChangeKind.removed) {
      final path = pathToId.entries
          .where((e) => e.value == change.fileId && e.key.isNotEmpty)
          .map((e) => e.key)
          .firstOrNull;
      if (path == null) {
        // 未知 fileId：幂等跳过（可能已随父目录删除）
        continue;
      }
      if (path.isEmpty) {
        throw AppError.generic('Changes 试图删除云盘根目录');
      }
      _removeCandidateSubtree(tree, pathToId, path);
      continue;
    }

    // Modified
    final file = change.file;
    if (file == null) {
      throw AppError.generic('Changes 缺少文件元数据：${change.fileId}');
    }
    MountPath.validatePathSegment(file.name);
    final parents = (file.parentFolder ?? const <String>[])
        .where((id) => id.trim().isNotEmpty)
        .toList();
    if (parents.length != 1 || parents.first == file.id) {
      throw AppError.generic('Changes 父目录不合法：${change.fileId}');
    }
    final parentId = parents.first;
    final String? parentPath;
    if (parentId == rootFolderId) {
      parentPath = '';
    } else {
      parentPath = pathToId.entries
          .where((e) => e.value == parentId)
          .map((e) => e.key)
          .firstOrNull;
      if (parentPath == null) {
        throw AppError.generic('Changes 父目录无法映射到已知路径：${change.fileId}');
      }
    }
    final desiredPath = parentPath.isEmpty ? file.name : '$parentPath/${file.name}';

    final oldPath = pathToId.entries
        .where((e) => e.value == file.id && e.key.isNotEmpty)
        .map((e) => e.key)
        .firstOrNull;
    if (oldPath != null) {
      if (oldPath != desiredPath) {
        // 禁止把目录移入自身子树
        if (desiredPath == oldPath || desiredPath.startsWith('$oldPath/')) {
          throw AppError.generic('Changes 试图把目录移入自身子树：$oldPath');
        }
        _rekeyCandidateSubtree(tree, pathToId, oldPath, desiredPath, file.id);
      }
      tree[desiredPath] = file;
      pathToId[desiredPath] = file.id;
    } else {
      final occupant = pathToId[desiredPath];
      if (occupant != null && occupant != file.id) {
        throw AppError.generic('Change 目标路径冲突：$desiredPath');
      }
      tree[desiredPath] = file;
      pathToId[desiredPath] = file.id;
    }
  }
}

/// 删除候选树中整棵子树（双 map 同步）。
void _removeCandidateSubtree(
  Map<String, DriveFile> tree,
  Map<String, String> pathToId,
  String root,
) {
  final doomed = tree.keys
      .where((k) => k == root || k.startsWith('$root/'))
      .toList();
  for (final k in doomed) {
    tree.remove(k);
    pathToId.remove(k);
  }
}

/// 候选树子树路径重键（目标被非本次移动条目占用时报错）。
void _rekeyCandidateSubtree(
  Map<String, DriveFile> tree,
  Map<String, String> pathToId,
  String oldRoot,
  String newRoot,
  String fileId,
) {
  final moving = tree.keys
      .where((k) => k == oldRoot || k.startsWith('$oldRoot/'))
      .toList();
  for (final old in moving) {
    final next = old == oldRoot ? newRoot : '$newRoot${old.substring(oldRoot.length)}';
    final occupant = pathToId[next];
    if (occupant != null && !moving.contains(next)) {
      throw AppError.generic('Change 目标路径冲突：$next');
    }
    final file = tree.remove(old);
    pathToId.remove(old);
    if (file != null) {
      tree[next] = file;
      pathToId[next] = file.id;
    }
  }
  // 根条目本身可能不在 tree（防御）
  pathToId[newRoot] = fileId;
}

/// 加载可信云端 checkpoint（对齐 Rust `load_persisted_cloud_tree`）。
///
/// 不存在、读失败、解析失败或内部不一致 → null（触发全量）。
Future<CloudTreeCache?> loadPersistedCloudTree(String mountDir) async {
  final file = await CachePaths.cloudTreeCacheFile(mountDir);
  if (!await file.exists()) return null;
  final String raw;
  try {
    raw = await file.readAsString();
  } catch (e) {
    AppLogger.w('读取云端 checkpoint 失败，将全量刷新: $e');
    return null;
  }
  final CloudTreeCache cache;
  try {
    final decoded = jsonDecode(raw);
    if (decoded is! Map<String, dynamic>) return null;
    cache = CloudTreeCache.fromJson(decoded);
  } catch (e) {
    AppLogger.w('解析云端 checkpoint 失败，将全量刷新: $e');
    return null;
  }
  try {
    cache.validateTrusted();
  } catch (e) {
    AppLogger.w('云端 checkpoint 不可信，将全量刷新: $e');
    return null;
  }
  AppLogger.i('从缓存加载可信云端 checkpoint（${cache.tree.length} 项）');
  return cache;
}

/// 原子提交完整可信 checkpoint（对齐 Rust `persist_cloud_checkpoint`）。
///
/// 候选先在同目录临时文件完整写入并 fsync，之后才 rename 覆盖正式文件。
/// 调用方必须把此函数的正常返回当作安装 live tree/path/cursor 的唯一提交门槛。
Future<void> persistCloudCheckpoint(
  String mountDir,
  CloudTreeCache checkpoint,
) async {
  checkpoint.validateTrusted();
  final file = await CachePaths.cloudTreeCacheFile(mountDir);
  final parent = Directory(file.parent.path);
  await parent.create(recursive: true);

  final json = const JsonEncoder.withIndent('  ').convert(checkpoint.toJson());
  final tmpFile = File('${file.path}.tmp');
  final bakFile = File('${file.path}.bak');
  try {
    if (await bakFile.exists()) await bakFile.delete();
  } catch (_) {
    // 尽力清理旧备份
  }

  // 1. 候选完整写入 + fsync
  final raf = await tmpFile.open(mode: FileMode.write);
  try {
    await raf.writeString(json);
    await raf.flush();
  } finally {
    await raf.close();
  }

  // 2. 保留旧版本（dart:io 无 hardlink，复制等价用于回滚）
  final hadPrevious = await file.exists();
  if (hadPrevious) {
    await file.copy(bakFile.path);
    await _syncParentDirectoryBestEffort(parent.path);
  }

  // 3. 同目录 rename（原子提交边界）
  try {
    await tmpFile.rename(file.path);
  } catch (e) {
    try {
      if (await bakFile.exists()) await bakFile.delete();
    } catch (_) {
      // 尽力清理
    }
    throw AppError.generic('云端 checkpoint 提交失败：$e');
  }

  // 4. 尽力 fsync 父目录（平台差异：失败记录日志，不阻断提交）
  await _syncParentDirectoryBestEffort(parent.path);

  // 5. 清理备份
  if (hadPrevious) {
    try {
      await bakFile.delete();
    } catch (_) {
      // 尽力清理
    }
  }
  AppLogger.i('可信云端 checkpoint 已提交（${checkpoint.tree.length} 项）');
}

/// 尽力 fsync 父目录元数据（对齐 Rust sync_parent_directory；
/// dart:io 在部分平台无法打开目录句柄，失败仅记日志）。
Future<void> _syncParentDirectoryBestEffort(String parentPath) async {
  try {
    final raf = await File(parentPath).open();
    try {
      await raf.flush();
    } finally {
      await raf.close();
    }
  } catch (e) {
    AppLogger.d('父目录 fsync 不可用（忽略）: $e');
  }
}

/// 清理未提交候选（退出时尽力调用；正式 checkpoint 永不破坏）。
Future<void> markCloudCacheIncompleteIfExists(String mountDir) async {
  final file = await CachePaths.cloudTreeCacheFile(mountDir);
  for (final suffix in const ['.tmp', '.bak']) {
    try {
      final f = File('${file.path}$suffix');
      if (await f.exists()) await f.delete();
    } catch (_) {
      // 尽力清理
    }
  }
}

// ═══════════════════════════════════════════════════════════════════════════
// 本地快照缓存（对齐 Rust sync_state_store.rs：syncstate_<escaped>.json）
// ═══════════════════════════════════════════════════════════════════════════

/// 本地文件快照条目（对齐 Rust `LocalFileSnapshotEntry`；isFolder 不持久化）。
class LocalSnapshotEntry {
  /// 修改时间（毫秒 epoch）
  final int mtime;

  /// 大小（字节）
  final int size;

  /// 内容 SHA256（可空）
  final String? sha256;

  /// 是否目录（仅内存，不持久化）
  final bool isFolder;

  const LocalSnapshotEntry({
    required this.mtime,
    required this.size,
    this.sha256,
    this.isFolder = false,
  });
}

/// 本地文件快照存取器（对齐 Rust `SyncStateStore`）。
///
/// 当前 planner 以 DB 为基线（与 Rust 一致），本快照缓存作为
/// 本地扫描的辅助事实保留；登出/换挂载目录时随 cloudtree 一并清理。
class SyncStateStore {
  /// 挂载目录（绝对路径）
  final String mountDir;

  const SyncStateStore(this.mountDir);

  /// 缓存文件是否存在（首次启动判断）
  Future<bool> exists() async {
    return (await CachePaths.syncStateCacheFile(mountDir)).exists();
  }

  /// 加载快照；文件不存在/读失败/解析失败 → 空 map（不报错）。
  Future<Map<String, LocalSnapshotEntry>> load() async {
    final file = await CachePaths.syncStateCacheFile(mountDir);
    if (!await file.exists()) return {};
    try {
      final decoded = jsonDecode(await file.readAsString());
      if (decoded is! Map<String, dynamic>) return {};
      final out = <String, LocalSnapshotEntry>{};
      for (final entry in decoded.entries) {
        final v = entry.value;
        if (v is! Map<String, dynamic>) continue;
        final mtime = v['mtime'];
        final size = v['size'];
        if (mtime is! int || size is! int) continue;
        out[entry.key] = LocalSnapshotEntry(
          mtime: mtime,
          size: size,
          sha256: v['sha256'] as String?,
        );
      }
      return out;
    } catch (e) {
      AppLogger.w('读取本地快照缓存失败，按空处理: $e');
      return {};
    }
  }

  /// 全量覆写保存（创建父目录）。
  Future<void> save(Map<String, LocalSnapshotEntry> entries) async {
    final file = await CachePaths.syncStateCacheFile(mountDir);
    await Directory(file.parent.path).create(recursive: true);
    final json = <String, dynamic>{
      for (final e in entries.entries)
        e.key: {
          'mtime': e.value.mtime,
          'size': e.value.size,
          if (e.value.sha256 != null) 'sha256': e.value.sha256,
        },
    };
    await file.writeAsString(
        const JsonEncoder.withIndent('  ').convert(json));
  }

  /// 清理本挂载目录的 syncstate + cloudtree 缓存（登出用）。
  Future<void> clear() => CachePaths.clearForMount(mountDir);
}
