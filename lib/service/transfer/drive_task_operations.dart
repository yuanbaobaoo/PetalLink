// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'dart:io';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/file_hasher.dart';
import 'package:petal_link/service/mount/stability.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// TaskRunner 的 Drive 操作执行适配层（对齐 Rust
/// `src/sync/executor/transfer_operations.rs` 的 `ExecutorTransferOperations`）。
///
/// 按 TransferOperation 分发执行：
/// - Create/Update → [UploadService.uploadForTask]（含源快照核验、同名冲突/
///   云端版本复核、完整元数据补取）
/// - Download/DownloadUpdate → [DownloadService.downloadForTask]
/// - Delete → [FilesService.deleteVerified]；Move/Rename/CreateFolder → 对应
///   files API（Flutter 扩展操作，Rust 原版不产生此类任务行）
/// - 上传前置：源快照 + >20MiB Update 拒绝 + [StabilityChecker] 稳定性检查
///   （本地变更 → LocalChanged/RestartRequired）
///
/// mount xattr 回写（fileId/downloaded 标记）属挂载引擎接缝，由后续
/// sync 引擎任务接线，本层不处理。
class DriveTaskOperations implements TaskOperations {
  /// 上传执行器
  final UploadService _uploadService;

  /// 下载执行器
  final DownloadService _downloadService;

  /// 文件元数据/写操作 API
  final FilesService _filesService;

  /// 上传前稳定性检查（可空 = 跳过，对齐 Rust stability: None 分支）
  final StabilityChecker? _stability;

  /// 本地文件 SHA-256（远端核验内容比对用，可空 = 跳过哈希比对）
  final FileHasher? _fileHasher;

  /// 在线判定钩子（透传执行器的网络门控）
  final bool Function()? _isOnline;

  /// 上传失败通知（对齐 Rust `upload_failed` 事件）
  final void Function(UploadFailureNotice notice)? _onUploadFailed;

  /// 睡眠（测试注入）
  final Future<void> Function(Duration) _sleep;

  DriveTaskOperations({
    required UploadService uploadService,
    required DownloadService downloadService,
    required FilesService filesService,
    StabilityChecker? stability,
    FileHasher? fileHasher,
    bool Function()? isOnline,
    void Function(UploadFailureNotice notice)? onUploadFailed,
    Future<void> Function(Duration)? sleep,
  })  : _uploadService = uploadService,
        _downloadService = downloadService,
        _filesService = filesService,
        _stability = stability,
        _fileHasher = fileHasher,
        _isOnline = isOnline,
        _onUploadFailed = onUploadFailed,
        _sleep = sleep ?? Future.delayed;
  /// 上传稳定性检查的复核延迟序列（对齐 Rust preflight 的 [0, 2, 3, 5]）
  static const List<int> stabilityRecheckSecs = [0, 2, 3, 5];

  // ═══════════════════════════════════════════════════════════════════
  // 后端前置校验（对齐 Rust TransferOperations::preflight）
  // ═══════════════════════════════════════════════════════════════════

  /// 在远程写入前校验上传源快照、安全阈值与稳定性。
  @override
  Future<void> preflight(TransferTask task) async {
    final operation = task.operation;
    if (operation != TransferOperation.create &&
        operation != TransferOperation.update) {
      return;
    }
    final localPath = task.localPath;
    if (localPath == null || localPath.isEmpty) {
      throw const BackendPreflightFailure.restartRequired('上传任务缺少本地路径');
    }
    try {
      await _verifySourceSnapshot(task, localPath);
    } on AppError catch (e) {
      throw BackendPreflightFailure.restartRequired(e.message);
    }
    if (operation == TransferOperation.update &&
        task.totalSize > UploadService.smallLargeThreshold) {
      throw const BackendPreflightFailure.restartRequired(
          '现有云端文件超过 20 MiB，Huawei 当前接口不支持安全替换；已保留远端原文件');
    }
    final stability = _stability;
    if (stability == null) return;
    for (final delay in stabilityRecheckSecs) {
      if (delay > 0) await _sleep(Duration(seconds: delay));
      final result = await stability.check(localPath);
      switch (result) {
        case StabilityResult.stable:
          return;
        case StabilityResult.editing:
          throw const BackendPreflightFailure.restartRequired('用户正在编辑，等待重新规划');
        case StabilityResult.unstable:
          continue;
      }
    }
    throw const BackendPreflightFailure.restartRequired('文件尚不稳定，等待重新规划');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 执行分发（对齐 Rust TransferOperations::execute）
  // ═══════════════════════════════════════════════════════════════════

  /// 执行持久传输任务，并把进度与断点信息回写 TaskRunner。
  @override
  Future<TaskExecutionOutcome> execute(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    try {
      return await _executeInner(task, progress);
    } on TaskExecutionError {
      rethrow;
    } on AppError catch (e) {
      throw TaskAppError(e);
    }
  }

  /// 按操作类型分发执行。
  Future<TaskExecutionOutcome> _executeInner(
    TransferTask task,
    TaskProgressCallbacks progress,
  ) async {
    final operation = task.operation;
    if (operation == null) {
      throw AppError.generic('任务缺少 operation');
    }
    switch (operation) {
      case TransferOperation.create:
      case TransferOperation.update:
        return _executeUpload(task, operation, progress);
      case TransferOperation.download:
      case TransferOperation.downloadUpdate:
        final result = await _downloadService.downloadForTask(
          task,
          onProgress: progress.onDownloadProgress,
          isOnline: _isOnline,
        );
        result.unwrap();
        return const TaskExecutionOutcome();
      case TransferOperation.delete:
        final fileId = _requireFileId(task, '删除任务缺少 fileId');
        (await _filesService.deleteVerified(fileId)).unwrap();
        return const TaskExecutionOutcome();
      case TransferOperation.move:
        final fileId = _requireFileId(task, '移动任务缺少 fileId');
        final parentId = task.parentFileId;
        if (parentId == null || parentId.isEmpty) {
          throw AppError.generic('移动任务缺少目标 parentId');
        }
        // update 内部先 GET 当前唯一 parent 构造成对参数，幂等安全
        final moved =
            (await _filesService.update(fileId, newParentFolder: parentId))
                .unwrap();
        return TaskExecutionOutcome(cloudFile: moved);
      case TransferOperation.rename:
        final fileId = _requireFileId(task, '重命名任务缺少 fileId');
        final renamed = (await _filesService.rename(fileId, task.name)).unwrap();
        return TaskExecutionOutcome(cloudFile: renamed);
      case TransferOperation.createFolder:
        final created =
            (await _filesService.createFolder(task.name, parentId: task.parentFileId))
                .unwrap();
        return TaskExecutionOutcome(cloudFile: created);
    }
  }

  /// 执行上传任务（Create/Update），对齐 Rust execute 的上传分支。
  Future<TaskExecutionOutcome> _executeUpload(
    TransferTask task,
    TransferOperation operation,
    TaskProgressCallbacks progress,
  ) async {
    final localPath = task.localPath;
    if (localPath == null || localPath.isEmpty) {
      throw AppError.generic('任务缺少本地路径');
    }
    // 远端写入前再次核验上传源快照
    try {
      await _verifySourceSnapshot(task, localPath);
    } on AppError catch (e) {
      throw TaskRestartRequired(e.message);
    }
    if (operation == TransferOperation.update) {
      final fileId = _requireFileId(task, '更新上传任务缺少 fileId');
      final current = (await _filesService.get(fileId)).unwrap();
      final currentEdited = current.editedTime?.millisecondsSinceEpoch;
      if (current.id != fileId ||
          currentEdited != task.expectedCloudEditedTime) {
        throw const TaskRestartRequired('远端文件已在规划后变化，拒绝用旧任务覆盖');
      }
    } else {
      final siblings =
          (await _filesService.listAll(parentId: task.parentFileId)).unwrap();
      final collision = siblings.any((file) => file.name == task.name);
      if (collision) {
        throw const TaskRestartRequired('目标目录已存在同名远端文件，拒绝重复创建');
      }
    }
    final result = await _uploadService.uploadForTask(
      task,
      onProgress: progress.onProgress,
      onResumeProgress: progress.onResume,
      isOnline: _isOnline,
    );
    if (result.isErr) {
      final error = (result as Err<DriveFile>).error;
      // 对齐 Rust：仅上传调用失败时发布 upload_failed 通知；
      // rel_path 负载在 relative_path 为空时回退文件名
      _onUploadFailed?.call(UploadFailureNotice(
        name: task.name,
        relativePath: task.relativePath ?? task.name,
        error: '$error',
      ));
      throw TaskAppError(error);
    }
    final uploaded = (result as Ok<DriveFile>).value;
    // 完整元数据补取：editedTime 缺失时 GET 一次，仍不完整 → 远端核验
    if (uploaded.editedTime == null) {
      final full = await _filesService.get(uploaded.id);
      if (full.isOk) {
        final file = (full as Ok<DriveFile>).value;
        if (file.id == uploaded.id && file.editedTime != null) {
          return TaskExecutionOutcome(cloudFile: file);
        }
        AppLogger.w('上传已返回 ID 但完整元数据仍未就绪，等待远端核验: ${uploaded.id}');
        return TaskExecutionOutcome(
          cloudFile: file,
          disposition: TaskDisposition.verifyingRemote,
        );
      }
      AppLogger.w('上传已返回 ID 但完整元数据补取失败，等待远端核验: ${uploaded.id}');
      return TaskExecutionOutcome(
        cloudFile: uploaded,
        disposition: TaskDisposition.verifyingRemote,
      );
    }
    return TaskExecutionOutcome(cloudFile: uploaded);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 远端核验（对齐 Rust TransferOperations::verify_remote）
  // ═══════════════════════════════════════════════════════════════════

  /// 根据远程 ID、父目录、大小、时间与哈希核实写入结果。
  @override
  Future<RemoteVerification> verifyRemote(TransferTask task) async {
    final operation = task.operation;
    switch (operation) {
      case TransferOperation.create:
        return _verifyCreate(task);
      case TransferOperation.update:
        return _verifyUpdate(task);
      default:
        return const RemoteAmbiguous('该任务不是可核验的上传写入');
    }
  }

  /// 核验 Create 写入：优先按持久化结果 ID，否则按父目录窗口扫描。
  Future<RemoteVerification> _verifyCreate(TransferTask task) async {
    final remoteId = task.remoteResultFileId;
    if (remoteId != null && remoteId.trim().isNotEmpty) {
      final result = await _filesService.get(remoteId);
      if (result.isErr) {
        final error = (result as Err<DriveFile>).error;
        if (error.driveStatus == 404) {
          return const RemoteAmbiguous('上传曾返回远端 ID，但该资源当前不可见；禁止重复创建');
        }
        throw error;
      }
      final file = (result as Ok<DriveFile>).value;
      if (file.id != remoteId ||
          file.name != task.name ||
          file.size != (task.sourceSize ?? task.totalSize)) {
        return const RemoteAmbiguous('远端结果 ID 存在，但名称或大小与创建任务不一致');
      }
      final localSha256 = await _sourceSha256IfCurrent(task, file);
      if (!_contentHashMatches(file, localSha256)) {
        return const RemoteAmbiguous('远端结果 ID 的 content_hash 与上传源不一致');
      }
      return RemoteCommitted(file);
    }

    final expectedSize = task.sourceSize ?? task.totalSize;
    final candidates = <DriveFile>[];
    final missingTimeCandidates = <DriveFile>[];
    // 核验窗口以持久任务为锚，并覆盖慢速或中断续传
    final lowerBound = task.createdAt - 120000;
    final upperBound = task.createdAt + 30 * 24 * 60 * 60 * 1000;
    final listing =
        (await _filesService.listAll(parentId: task.parentFileId)).unwrap();
    for (final file in listing) {
      final parent = task.parentFileId;
      final parentMatches = parent == null ||
          (file.parentFolder != null &&
              file.parentFolder!.length == 1 &&
              file.parentFolder!.first == parent);
      if (file.name != task.name ||
          file.size != expectedSize ||
          !parentMatches) {
        continue;
      }
      final createdAt = file.createdTime?.millisecondsSinceEpoch;
      if (createdAt == null) {
        missingTimeCandidates.add(file);
      } else if (createdAt >= lowerBound && createdAt <= upperBound) {
        candidates.add(file);
      }
    }
    final needsHash = candidates.any(_hasComparableHash) ||
        missingTimeCandidates.any(_hasComparableHash);
    final localSha256 =
        needsHash ? await _sourceSha256IfCurrent(task, null) : null;
    candidates.retainWhere((file) => _contentHashMatches(file, localSha256));
    missingTimeCandidates
        .retainWhere((file) => _contentHashMatches(file, localSha256));
    if (candidates.length == 1) {
      return RemoteCommitted(candidates.first);
    }
    if (candidates.isEmpty) {
      if (missingTimeCandidates.isNotEmpty) {
        return const RemoteAmbiguous('发现同名同大小资源但缺少创建时间，无法排除重复文件');
      }
      return const RemoteNotCommitted();
    }
    return const RemoteAmbiguous('父目录内存在多个符合创建任务的远端资源');
  }

  /// 核验 Update 写入：按 fileId 直查，比对版本与内容身份。
  Future<RemoteVerification> _verifyUpdate(TransferTask task) async {
    final fileId = _requireFileId(task, 'Update 核验缺少 fileId');
    final result = await _filesService.get(fileId);
    if (result.isErr) {
      final error = (result as Err<DriveFile>).error;
      if (error.driveStatus == 404) {
        return const RemoteAmbiguous('待更新的既有远端文件已不可见，禁止降级创建');
      }
      throw error;
    }
    final file = (result as Ok<DriveFile>).value;
    final remoteResultId = task.remoteResultFileId;
    if (remoteResultId != null && remoteResultId.trim().isNotEmpty) {
      // 已持久化的结果 ID 可在 editedTime 滞后时证明提交完成
      if (remoteResultId == fileId &&
          file.id == fileId &&
          file.name == task.name &&
          file.size == (task.sourceSize ?? task.totalSize)) {
        return RemoteCommitted(file);
      }
      return const RemoteAmbiguous('更新已返回远端结果 ID，但当前资源身份不一致');
    }
    final editedMs = file.editedTime?.millisecondsSinceEpoch;
    if (editedMs == task.expectedCloudEditedTime) {
      return const RemoteNotCommitted();
    }
    if (file.id == fileId &&
        file.name == task.name &&
        file.size == (task.sourceSize ?? task.totalSize) &&
        editedMs != null) {
      return RemoteCommitted(file);
    }
    return const RemoteAmbiguous('远端版本已变化，但内容身份与本次更新不一致');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 源快照与内容哈希辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 确认上传源的类型、修改时间与大小仍匹配持久任务（对齐 Rust
  /// `verify_source_snapshot`）。
  Future<void> _verifySourceSnapshot(TransferTask task, String localPath) async {
    final FileStat stat;
    try {
      stat = await FileStat.stat(localPath);
    } catch (e) {
      throw AppError.generic('读取上传源失败：$e');
    }
    final mtimeMs = stat.modified.millisecondsSinceEpoch;
    if (stat.type != FileSystemEntityType.file ||
        task.sourceMtime != mtimeMs ||
        task.sourceSize != stat.size ||
        task.totalSize != stat.size) {
      throw AppError.generic('本地上传源在执行前发生变化');
    }
  }

  /// 仅散列仍匹配持久任务快照的上传源（对齐 Rust `source_sha256_if_current`）。
  ///
  /// [file] 非空时先判断远端是否有可比哈希，无则直接跳过本地散列。
  Future<String?> _sourceSha256IfCurrent(TransferTask task, DriveFile? file) async {
    final hasher = _fileHasher;
    final localPath = task.localPath;
    if (hasher == null || localPath == null) return null;
    if (file != null && !_hasComparableHash(file)) return null;
    try {
      await _verifySourceSnapshot(task, localPath);
    } on AppError {
      return null;
    }
    final sha256 = await hasher.hashFile(localPath);
    try {
      await _verifySourceSnapshot(task, localPath);
    } on AppError {
      return null;
    }
    return sha256;
  }

  /// 返回可与本地 SHA-256 直接比较的云端哈希（对齐 Rust `comparable_sha256`）。
  bool _hasComparableHash(DriveFile file) {
    final hash = file.contentHash?.trim();
    if (hash == null || hash.length != 64) return false;
    for (final unit in hash.codeUnits) {
      final isHex = (unit >= 0x30 && unit <= 0x39) ||
          (unit >= 0x61 && unit <= 0x66) ||
          (unit >= 0x41 && unit <= 0x46);
      if (!isHex) return false;
    }
    return true;
  }

  /// 在双方都有可比哈希时校验内容一致性（对齐 Rust `content_hash_matches`）。
  bool _contentHashMatches(DriveFile file, String? localSha256) {
    final remote = file.contentHash?.trim();
    final comparable =
        remote != null && remote.length == 64 && _hasComparableHash(file);
    if (comparable && localSha256 != null) {
      return remote.toLowerCase() == localSha256.toLowerCase();
    }
    return true;
  }

  /// 读取必需的 fileId，缺失时抛通用错误。
  String _requireFileId(TransferTask task, String message) {
    final fileId = task.fileId;
    if (fileId == null || fileId.isEmpty) {
      throw AppError.generic(message);
    }
    return fileId;
  }
}
