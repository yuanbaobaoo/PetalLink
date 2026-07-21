// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// 目录递归同步 —— 后台 BFS 子树双端对齐（对齐 Rust
/// `src/commands/folder_sync.rs` 的 `sync_folder_recursive_impl`）。
///
/// 云端 BFS 收集子树 → 建本地目录 → 本地真实文件扫描 →
/// 对齐（云端独有下载、本地独有上传；共有文件不做内容比较）→
/// 逐项发射 folder_sync_progress {done,total}（失败/延期也计数）。
library;

import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/mount/skip.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// 目录递归同步进度（对齐 Rust `folder_sync_progress` 事件负载）。
typedef FolderSyncProgress = ({int done, int total});

/// 目录递归同步执行器（由 SyncService 装配并持有）。
class FolderSyncRunner {
  /// 文件 API
  final FilesService _filesApi;

  /// 持久化传输执行器
  final TaskRunner _taskRunner;

  /// 挂载管理器提供（未配置时抛错）
  final MountManager Function() _mountProvider;

  /// 进度发射回调
  final void Function(FolderSyncProgress progress) _emitProgress;

  FolderSyncRunner({
    required FilesService filesApi,
    required TaskRunner taskRunner,
    required MountManager Function() mountProvider,
    required void Function(FolderSyncProgress progress) emitProgress,
  })  : _filesApi = filesApi,
        _taskRunner = taskRunner,
        _mountProvider = mountProvider,
        _emitProgress = emitProgress;

  /// BFS 子树双端对齐实现（对齐 Rust `sync_folder_recursive_impl`）。
  Future<int> run(
    SyncEngine engine, {
    required String folderId,
    required String relPath,
  }) async {
    final mount = _mountProvider();
    MountPath.validateRelativePath(relPath, allowEmpty: true);
    final destDir = relPath.isEmpty
        ? mount.mountDir
        : MountPath.safeJoinUnder(mount.mountDir, relPath);

    // 1. 云端 BFS
    final cloudFolders = <String>[];
    final cloudFiles = <(String, DriveFile)>[];
    final folderRelToId = <String, String>{'': folderId};
    final queue = <(String, String)>[(folderId, '')];
    while (queue.isNotEmpty) {
      final (id, path) = queue.removeAt(0);
      engine.ensureCycleActive();
      final result = await _filesApi.listAll(parentId: id);
      if (result.isErr) throw (result as Err).error;
      for (final child in (result as Ok<List<DriveFile>>).value) {
        if (MountSkip.shouldSkip(child.name, engine.skipPatterns)) continue;
        MountPath.validatePathSegment(child.name);
        final subrel = path.isEmpty ? child.name : '$path/${child.name}';
        if (child.isFolder) {
          cloudFolders.add(subrel);
          folderRelToId[subrel] = child.id;
          queue.add((child.id, subrel));
        } else {
          cloudFiles.add((subrel, child));
        }
      }
    }

    // 2. 建本地目录
    await Directory(destDir).create(recursive: true);
    for (final folder in cloudFolders) {
      try {
        await Directory(p.join(destDir, folder)).create(recursive: true);
      } catch (_) {
        // 尽力
      }
    }

    // 3. 本地扫描（仅真实文件；0 字节占位符跳过，0 字节真实文件保留；
    //    skipPatterns 对齐 Rust scan_dir_for_real_files(eng.skip_patterns())）
    final localFiles =
        await _scanRealFiles(mount, destDir, engine.skipPatterns);

    // 4. 对齐（共有文件不做内容比较）
    final localNames = localFiles.keys.toSet();
    final cloudNames = cloudFiles.map((e) => e.$1).toSet();
    final toDownload =
        cloudFiles.where((e) => !localNames.contains(e.$1)).toList();
    final toUpload =
        localNames.where((name) => !cloudNames.contains(name)).toList();
    final total = toDownload.length + toUpload.length;
    var done = 0;

    void emitProgress() {
      _emitProgress((done: done, total: total));
    }

    // 5. 补建云端父目录链（为上传）
    final pendingDirs = <String>{};
    for (final name in toUpload) {
      var parent = _parentSubrel(name);
      while (parent.isNotEmpty && !folderRelToId.containsKey(parent)) {
        pendingDirs.add(parent);
        parent = _parentSubrel(parent);
      }
    }
    final sortedDirs = pendingDirs.toList()
      ..sort((a, b) =>
          '/'.allMatches(a).length.compareTo('/'.allMatches(b).length));
    for (final dir in sortedDirs) {
      final dirName = dir.split('/').last;
      final parentRel = _parentSubrel(dir);
      final parentId = folderRelToId[parentRel];
      if (parentId == null) {
        AppLogger.w('目录 $dir 的父目录云端 ID 缺失，跳过建目录');
        continue;
      }
      final created = await _filesApi.createFolder(dirName, parentId: parentId);
      if (created.isOk) {
        final folder = (created as Ok<DriveFile>).value;
        folderRelToId[dir] = folder.id;
      } else {
        // 400/409 容错：重列父目录找同名复用
        final error = (created as Err).error;
        final status = error.driveStatus;
        if (status == 400 || status == 409) {
          final relisted = await _filesApi.listAll(parentId: parentId);
          final existing = relisted
              .unwrapOr(const [])
              .where((f) => f.isFolder && f.name == dirName)
              .firstOrNull;
          if (existing != null) {
            folderRelToId[dir] = existing.id;
          } else {
            AppLogger.w('目录 $dir 云端创建冲突后未找到同名目录，其内文件将延期');
            continue;
          }
        } else {
          AppLogger.w('目录 $dir 云端创建失败，其内文件将延期: $error');
          continue;
        }
      }
      try {
        await Directory(p.join(destDir, dir)).create(recursive: true);
      } catch (_) {
        // 尽力
      }
    }

    // 6. 下载循环（无论成败 done+1 并发射进度）
    for (final (subrel, file) in toDownload) {
      engine.ensureCycleActive();
      final fullRel = relPath.isEmpty ? subrel : '$relPath/$subrel';
      final absPath = p.join(destDir, subrel);
      var isUpdate = false;
      final type = await FileSystemEntity.type(absPath, followLinks: false);
      if (type == FileSystemEntityType.file) {
        final stat = await FileStat.stat(absPath);
        isUpdate = stat.size > 0;
      }
      final task = TransferTask(
        direction: isUpdate
            ? TransferDirection.downloadUpdate
            : TransferDirection.download,
        fileId: file.id,
        localPath: absPath,
        name: file.name,
        totalSize: file.size,
        createdAt: DateTime.now().millisecondsSinceEpoch,
        relativePath: fullRel,
        parentFileId: file.parentId,
        operation: isUpdate
            ? TransferOperation.downloadUpdate
            : TransferOperation.download,
        expectedCloudEditedTime: file.editedTime?.millisecondsSinceEpoch,
      );
      try {
        final result = await _taskRunner.enqueueAndRun(task);
        final outcome = result.unwrap().outcome;
        if (outcome.disposition != TaskDisposition.completed) {
          AppLogger.w('下载 $fullRel 进入恢复队列：${outcome.disposition.name}');
        }
      } catch (e) {
        AppLogger.w('下载 $fullRel 失败: $e');
      }
      done++;
      emitProgress();
    }

    // 7. 上传循环
    for (final name in toUpload) {
      engine.ensureCycleActive();
      final fullRel = relPath.isEmpty ? name : '$relPath/$name';
      final absPath = localFiles[name]!;
      final parentId = folderRelToId[_parentSubrel(name)];
      if (parentId == null) {
        AppLogger.w('上传 $fullRel 缺少云端父目录，延期');
        done++;
        emitProgress();
        continue;
      }
      final stat = await FileStat.stat(absPath);
      final task = TransferTask(
        direction: TransferDirection.upload,
        localPath: absPath,
        name: p.basename(absPath),
        totalSize: stat.size,
        createdAt: DateTime.now().millisecondsSinceEpoch,
        relativePath: fullRel,
        parentFileId: parentId,
        operation: TransferOperation.create,
        sourceMtime: stat.modified.millisecondsSinceEpoch,
        sourceSize: stat.size,
      );
      try {
        final result = await _taskRunner.enqueueAndRun(task);
        final outcome = result.unwrap().outcome;
        if (outcome.disposition == TaskDisposition.completed &&
            outcome.cloudFile != null) {
          // 即时更新内存云树
          engine.cloudIndex.insert(fullRel, outcome.cloudFile!);
        } else if (outcome.disposition != TaskDisposition.completed) {
          AppLogger.w('上传 $fullRel 进入恢复队列：${outcome.disposition.name}');
        }
      } catch (e) {
        AppLogger.w('上传 $fullRel 失败: $e');
      }
      done++;
      emitProgress();
    }
    return done;
  }

  /// 递归收集目录下的真实文件（subrel → 绝对路径；
  /// 跳过应排除项与 0 字节占位符，0 字节真实文件保留）。
  ///
  /// [skipPatterns] 必须传引擎配置的跳过模式（对齐 Rust
  /// `scan_dir_for_real_files(..., eng.skip_patterns(), ...)`），
  /// 否则 `.DS_Store` / `~$*` 等会被误上传。
  Future<Map<String, String>> _scanRealFiles(
    MountManager mount,
    String root,
    List<String> skipPatterns,
  ) async {
    final out = <String, String>{};
    Future<void> walk(String current, String prefix) async {
      await for (final entity in Directory(current).list(followLinks: false)) {
        final name = p.basename(entity.path);
        if (MountSkip.shouldSkip(name, skipPatterns)) continue;
        final type =
            await FileSystemEntity.type(entity.path, followLinks: false);
        if (type == FileSystemEntityType.directory) {
          final sub = prefix.isEmpty ? name : '$prefix/$name';
          await walk(entity.path, sub);
        } else if (type == FileSystemEntityType.file) {
          final stat = await FileStat.stat(entity.path);
          if (stat.size == 0 && await mount.isPlaceholderFile(entity.path)) {
            continue;
          }
          final sub = prefix.isEmpty ? name : '$prefix/$name';
          out[sub] = entity.path;
        }
      }
    }

    try {
      await walk(root, '');
    } catch (e) {
      AppLogger.w('目录本地扫描失败，按空处理: $e');
    }
    return out;
  }

  /// 取上级子路径（无 `/` → 空串）。
  String _parentSubrel(String subrel) {
    final index = subrel.lastIndexOf('/');
    return index < 0 ? '' : subrel.substring(0, index);
  }
}
