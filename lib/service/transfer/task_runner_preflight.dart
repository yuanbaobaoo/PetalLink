/// 传输任务静态前置校验（对齐 Rust `task_runner/preflight.rs` 的 `validate_static`）。
///
/// 在任务进入 Running 前校验可安全重放的静态条件：
/// 路径屏障、本地源快照、断点会话完整性、下载目标安全性。
/// 校验失败持久化为 [TransferState.failed]（Validation）或
/// [TransferState.restartRequired]（LocalChanged，需回 planner 重新规划）。
library;

import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/types/enums.dart';

/// 待确认基线 fileId 前缀（对齐 Rust `PENDING_FILE_ID_PREFIX`）
const String pendingFileIdPrefix = 'pending:';

/// 统一的前置校验失败状态、分类与消息（对齐 Rust `PreflightFailure`）。
class PreflightFailure implements Exception {
  /// 拒绝后应迁移到的状态
  final TransferState target;

  /// 错误分类
  final TransferErrorKind kind;

  /// 用户可读原因
  final String message;

  const PreflightFailure({
    required this.target,
    required this.kind,
    required this.message,
  });

  /// 构造静态校验失败（永久失败）
  const PreflightFailure.validation(String message)
      : this(
          target: TransferState.failed,
          kind: TransferErrorKind.validation,
          message: message,
        );

  /// 构造本地内容变化失败（需重新规划）
  const PreflightFailure.localChanged(String message)
      : this(
          target: TransferState.restartRequired,
          kind: TransferErrorKind.localChanged,
          message: message,
        );

  /// 构造远端结果不确定失败（等待核验）
  const PreflightFailure.remoteAmbiguous(String message)
      : this(
          target: TransferState.verifyingRemote,
          kind: TransferErrorKind.remoteAmbiguous,
          message: message,
        );

  /// 生成前置校验失败补丁（对齐 Rust `PreflightFailure::patch`）。
  TransferPatch patch({required int nowMs}) {
    final finished = target == TransferState.failed;
    return TransferPatch(
      errorKind: SetPatch(kind),
      errorMessage: SetPatch(message),
      nextRetryAt: const ClearPatch(),
      finishedAt: finished ? SetPatch(nowMs) : const ClearPatch(),
    );
  }

  @override
  String toString() => message;
}

/// 校验任务可安全执行所需的静态条件（对齐 Rust `validate_static`）。
///
/// [mountRoot] 为本地挂载根绝对路径；[isPlaceholder] 判定 0 字节占位符，
/// 缺省时保守视为非占位（任何既有下载目标都会判为本地内容，需重新规划）。
/// 通过时返回任务的操作类型；失败抛 [PreflightFailure]。
Future<TransferOperation> validateStaticTask(
  TransferTask task, {
  required String? mountRoot,
  Future<bool> Function(String path)? isPlaceholder,
}) async {
  final operation = task.operation;
  if (operation == null) {
    throw const PreflightFailure.validation('任务缺少 operation');
  }
  final rel = task.relativePath;
  if (rel == null || rel.isEmpty) {
    throw const PreflightFailure.validation('任务缺少相对路径');
  }
  try {
    MountPath.validateRelativePath(rel);
  } catch (e) {
    throw PreflightFailure.validation('$e');
  }
  if (mountRoot == null || mountRoot.isEmpty) {
    throw const PreflightFailure.validation('挂载根目录不存在或不可访问');
  }
  final mountStat = await FileStat.stat(mountRoot);
  if (mountStat.type != FileSystemEntityType.directory) {
    throw const PreflightFailure.validation('挂载根路径不是目录');
  }
  final localPathRaw = task.localPath;
  if (localPathRaw == null || localPathRaw.isEmpty) {
    throw const PreflightFailure.validation('任务缺少本地路径');
  }
  if (!p.isAbsolute(localPathRaw) || p.join(mountRoot, rel) != localPathRaw) {
    throw const PreflightFailure.validation('任务绝对路径与挂载相对路径不一致');
  }
  if (task.totalSize < 0 ||
      task.resumeOffset < 0 ||
      task.resumeOffset > task.totalSize) {
    throw const PreflightFailure.validation('任务大小或断点偏移非法');
  }

  bool hasNonempty(String? value) => value != null && value.trim().isNotEmpty;

  switch (operation) {
    case TransferOperation.create:
    case TransferOperation.update:
      if (task.direction != TransferDirection.upload) {
        throw const PreflightFailure.validation('上传 operation 与 direction 不一致');
      }
      if (operation == TransferOperation.create && hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('Create 任务不能携带 fileId');
      }
      final fileId = task.fileId;
      if (operation == TransferOperation.update &&
          !(fileId != null &&
              fileId.trim().isNotEmpty &&
              !fileId.startsWith(pendingFileIdPrefix))) {
        throw const PreflightFailure.validation('Update 任务缺少真实 fileId');
      }
      if (task.resumeOffset > 0 && !hasNonempty(task.sessionUrl)) {
        throw const PreflightFailure.validation(
            '非零上传断点缺少 session_url，拒绝作为全新请求重放');
      }
      if (p.dirname(rel) != '.' && !hasNonempty(task.parentFileId)) {
        throw const PreflightFailure.validation('子目录上传缺少 parentId');
      }
      final stat = await FileStat.stat(localPathRaw);
      if (stat.type != FileSystemEntityType.file) {
        throw const PreflightFailure.validation('本地上传源不存在或不是普通文件');
      }
      final actualMtime = stat.modified.millisecondsSinceEpoch;
      final actualSize = stat.size;
      if (task.sourceMtime != actualMtime ||
          task.sourceSize != actualSize ||
          task.totalSize != actualSize) {
        throw const PreflightFailure.localChanged('本地上传源已变化，需要重新规划');
      }
    case TransferOperation.download:
      if (task.direction != TransferDirection.download) {
        throw const PreflightFailure.validation('Download operation 与 direction 不一致');
      }
      if (!hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('下载任务缺少 fileId');
      }
      if (task.expectedCloudEditedTime == null) {
        throw const PreflightFailure.validation('下载任务缺少云端版本');
      }
      await _ensureDownloadParent(localPathRaw, mountRoot);
      final stat = await FileStat.stat(localPathRaw);
      if (stat.type == FileSystemEntityType.directory) {
        throw const PreflightFailure.validation('下载目标不能是目录');
      }
      if (stat.type != FileSystemEntityType.notFound) {
        final placeholder =
            await (isPlaceholder?.call(localPathRaw) ?? Future.value(false));
        if (stat.type != FileSystemEntityType.file ||
            stat.size != 0 ||
            !placeholder) {
          throw const PreflightFailure.localChanged('下载目标已出现本地内容，需要重新规划');
        }
      }
    case TransferOperation.downloadUpdate:
      if (task.direction != TransferDirection.downloadUpdate) {
        throw const PreflightFailure.validation(
            'DownloadUpdate operation 与 direction 不一致');
      }
      if (!hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('更新下载任务缺少 fileId');
      }
      if (task.expectedCloudEditedTime == null) {
        throw const PreflightFailure.validation('更新下载缺少云端版本');
      }
      await _ensureDownloadParent(localPathRaw, mountRoot);
      // 不跟随符号链接（对齐 Rust symlink_metadata）
      final targetType =
          await FileSystemEntity.type(localPathRaw, followLinks: false);
      if (targetType == FileSystemEntityType.notFound) {
        throw const PreflightFailure.localChanged('更新下载目标已不存在，需要重新规划');
      }
      final stat = await FileStat.stat(localPathRaw);
      final mtime = stat.modified.millisecondsSinceEpoch;
      if (targetType == FileSystemEntityType.link ||
          targetType != FileSystemEntityType.file ||
          task.sourceMtime == null ||
          task.sourceSize == null ||
          task.sourceMtime != mtime ||
          task.sourceSize != stat.size) {
        throw const PreflightFailure.localChanged(
            '更新下载目标已变化或缺少版本快照，需要重新规划');
      }
    case TransferOperation.delete:
    case TransferOperation.move:
    case TransferOperation.rename:
    case TransferOperation.createFolder:
      // Flutter 扩展操作（Rust 原版不产生此类任务行）：
      // 远端写操作无本地路径屏障，仅校验必需字段；
      // files API 均带写后验证，可安全重放。
      _validateRemoteOperation(task, operation, hasNonempty);
  }
  return operation;
}

/// 校验远端写操作任务的必需字段（Flutter 扩展分支）。
void _validateRemoteOperation(
  TransferTask task,
  TransferOperation operation,
  bool Function(String?) hasNonempty,
) {
  switch (operation) {
    case TransferOperation.delete:
      if (!hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('删除任务缺少 fileId');
      }
    case TransferOperation.move:
      if (!hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('移动任务缺少 fileId');
      }
      if (!hasNonempty(task.parentFileId)) {
        throw const PreflightFailure.validation('移动任务缺少目标 parentId');
      }
    case TransferOperation.rename:
      if (!hasNonempty(task.fileId)) {
        throw const PreflightFailure.validation('重命名任务缺少 fileId');
      }
      if (task.name.trim().isEmpty) {
        throw const PreflightFailure.validation('重命名任务缺少新名称');
      }
    case TransferOperation.createFolder:
      if (task.name.trim().isEmpty) {
        throw const PreflightFailure.validation('新建文件夹任务缺少名称');
      }
    default:
      // 其余操作不会到达此分支
      break;
  }
}

/// 校验并按需创建下载目标父目录（对齐 Rust `ensure_download_parent`）。
///
/// 防越界：逐段检查符号链接；父目录解析后必须仍在挂载根之下；
/// 下载目标为符号链接时拒绝文件操作。
Future<void> _ensureDownloadParent(String localPath, String mountRoot) async {
  final parent = p.dirname(localPath);
  if (parent == localPath) {
    throw const PreflightFailure.validation('下载目标缺少父目录');
  }
  final relativeParent = p.relative(parent, from: mountRoot);
  if (relativeParent == '..' || relativeParent.startsWith('../')) {
    throw const PreflightFailure.validation('下载父目录不在配置的挂载根目录之下');
  }
  final canonicalRoot = await Directory(mountRoot).resolveSymbolicLinks();

  var current = mountRoot;
  if (relativeParent != '.') {
    for (final segment in relativeParent.split('/')) {
      if (segment.isEmpty || segment == '.' || segment == '..') {
        throw const PreflightFailure.validation('下载父目录包含非法路径分量');
      }
      current = p.join(current, segment);
      final type = await FileSystemEntity.type(current, followLinks: false);
      if (type == FileSystemEntityType.link) {
        throw const PreflightFailure.validation('下载父目录包含符号链接，拒绝越界文件操作');
      }
      if (type == FileSystemEntityType.notFound) {
        try {
          await Directory(current).create();
        } catch (e) {
          throw PreflightFailure.validation('创建下载父目录失败：$e');
        }
        final recreated =
            await FileSystemEntity.type(current, followLinks: false);
        if (recreated == FileSystemEntityType.link ||
            recreated != FileSystemEntityType.directory) {
          throw const PreflightFailure.validation('下载父目录创建后被替换，拒绝继续');
        }
      } else if (type != FileSystemEntityType.directory) {
        throw const PreflightFailure.validation('下载父路径不是目录');
      }
    }
  }

  final canonicalParent = await Directory(parent).resolveSymbolicLinks();
  if (!p.isWithin(canonicalRoot, canonicalParent) &&
      canonicalParent != canonicalRoot) {
    throw const PreflightFailure.validation('下载父目录解析到挂载根目录之外');
  }
  final target = await FileSystemEntity.type(localPath, followLinks: false);
  if (target == FileSystemEntityType.link) {
    throw const PreflightFailure.validation('下载目标是符号链接，拒绝文件操作');
  }
}
