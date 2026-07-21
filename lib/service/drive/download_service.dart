import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';
import 'package:petal_link/service/drive/drive_http.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/xattr_service.dart';
import 'package:petal_link/types/enums.dart';

// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// 下载进度回调（参数：已下载字节，总字节）。对齐 Rust `ProgressFn`。
typedef DownloadProgressFn = void Function(int received, int total);

/// 构造 `.tmp` 临时文件路径：将后缀加到完整目标路径之后。
///
/// `.tmp` 后缀是 load-bearing 的：watcher 和 scanner 会忽略它，下载中断时
/// 可以保留已落盘内容而不会被误判为本地新增文件。
String tmpPath(String dest) => '$dest.tmp';

/// 构造断点元数据路径。它同样以 `.tmp` 结尾，确保不会被扫描器上传。
String resumeMetadataPath(String dest) => '$dest.download-meta.tmp';

/// 构造断点元数据写入阶段使用的暂存路径。
String _resumeMetadataStagingPath(String dest) => '$dest.download-meta-write.tmp';

/// 永久失败、取消或显式重启时由任务层调用；网络失败不要调用。
///
/// 尽力删除 `.tmp` 与断点元数据（对齐 Rust discard_resume_artifacts）。
void discardResumeArtifacts(String dest) {
  _deleteBestEffort(tmpPath(dest));
  _removeResumeMetadata(dest);
}

/// 尽力删除已提交及写入中的断点元数据。
void _removeResumeMetadata(String dest) {
  _deleteBestEffort(resumeMetadataPath(dest));
  _deleteBestEffort(_resumeMetadataStagingPath(dest));
}

void _deleteBestEffort(String path) {
  try {
    final file = File(path);
    if (file.existsSync()) file.deleteSync();
  } catch (_) {
    // 尽力删除
  }
}

/// 调度器已知的云端版本约束（对齐 Rust `DownloadExpectation`）。
///
/// 提供约束时，API 会在写入前拒绝已经过期的任务，避免旧任务下载了一个
/// 新版本后仍按旧版本结算同步基线。未提供的字段不参与校验。
class DownloadExpectation {
  /// 期望的云端 editedTime（毫秒 epoch）
  final int? editedTimeMs;

  /// 期望的云端大小（字节）
  final int? size;

  /// 期望的内容哈希（sha256 或 contentHash，大小写不敏感）
  final String? contentHash;

  /// 已下载文件；仅当本地版本未变化时才允许替换
  final LocalDestinationSnapshot? destinationSnapshot;

  /// 首次下载仅可写入空路径或当前云端文件未改动的占位符
  final String? placeholderFileId;

  const DownloadExpectation({
    this.editedTimeMs,
    this.size,
    this.contentHash,
    this.destinationSnapshot,
    this.placeholderFileId,
  });
}

/// 安装下载结果前必须保持不变的本地文件快照（对齐 Rust `LocalDestinationSnapshot`）。
class LocalDestinationSnapshot {
  /// 修改时间（毫秒 epoch）
  final int mtimeMs;

  /// 大小（字节）
  final int size;

  const LocalDestinationSnapshot({required this.mtimeMs, required this.size});
}

/// 下载 API 服务 —— 版本校验后使用 Range 断点下载到 `.tmp`，完成后原子替换。
///
/// 严格对齐 Rust 原版 `src/drive/download_api.rs`：
/// - 下载前 GET 元数据取得版本身份（editedTime/sha256/etag/…）写入 sidecar
/// - 仅在断点身份与当前云端版本一致时续传（Range）；版本漂移丢弃重下
/// - 206 必须匹配 Content-Range；200 表示服务端忽略 Range，截断从 0 写；
///   416/Range 不匹配只允许安全回退一次到 offset=0
/// - 最终长度/sha256/远端版本复核、flush 落盘后才原子 rename
///
/// 占位落点核验对齐 Rust `verify_local_destination`：占位属主（state ==
/// placeholder 且 owner fileId 匹配）核验需注入 [XattrService]（生产经
/// GlobalBinding 注入 ChannelXattrService）；未注入时退化为
/// 「不存在或 0 字节常规文件」检查。
class DownloadService {
  final MateHttpClient _client;

  /// Drive API base
  final String _driveBase;

  /// xattr 读写（占位属主核验；为空时退化为仅「0 字节常规文件」检查）
  final XattrService? _xattr;

  /// inode 属主查询（docs/design/10；占位属主核验优先数据源，
  /// 未注入或查不到时回退 xattr fileId——过渡期为新旧占位兼容）
  final Future<String?> Function(String path)? _inodeOwnerProvider;

  DownloadService(this._client,
      {String driveBase = driveApiBase,
      XattrService? xattr,
      Future<String?> Function(String path)? inodeOwnerProvider})
      : _driveBase = driveBase,
        _xattr = xattr,
        _inodeOwnerProvider = inodeOwnerProvider;

  /// 下载文件到 [destPath]；版本由 API 每次从云端读取并校验
  /// （对齐 Rust `download`）。
  Future<AppResult<void>> download(
    String fileId,
    String destPath, {
    DownloadProgressFn? onProgress,
  }) {
    return downloadWithExpectation(fileId, destPath,
        onProgress: onProgress);
  }

  /// 带调度器版本约束的断点下载（对齐 Rust `download_with_expectation`）。
  Future<AppResult<void>> downloadWithExpectation(
    String fileId,
    String destPath, {
    DownloadExpectation? expectation,
    DownloadProgressFn? onProgress,
  }) {
    return _guard(
      () => _downloadWithExpectation(
          fileId, destPath, expectation, onProgress),
      'download',
    );
  }

  /// 执行器级入口：按任务行执行下载（Download/DownloadUpdate）。
  ///
  /// 对齐 Rust `TransferOperations::execute` 的下载分支：
  /// - 期望约束来自任务行（expectedCloudEditedTime / totalSize /
  ///   DownloadUpdate 的本地目标快照 / Download 的占位符身份）
  /// - [onProgress] 接收已下载字节数
  /// - [isOnline] 网络门控钩子：返回 false 时以网络错误拒绝执行
  Future<AppResult<void>> downloadForTask(
    TransferTask task, {
    void Function(int received)? onProgress,
    bool Function()? isOnline,
  }) {
    return _guard(() async {
      final operation = task.operation;
      if (operation == null) {
        throw AppError.generic('任务缺少 operation');
      }
      if (operation != TransferOperation.download &&
          operation != TransferOperation.downloadUpdate) {
        throw AppError.generic('该 operation 不支持传输执行');
      }
      final localPath = task.localPath;
      if (localPath == null || localPath.isEmpty) {
        throw AppError.generic('任务缺少本地路径');
      }
      final fileId = task.fileId;
      if (fileId == null || fileId.isEmpty) {
        throw AppError.generic('下载任务缺少 fileId');
      }
      if (isOnline != null && !isOnline()) {
        throw AppError.driveNetwork('网络离线，下载已暂停');
      }

      final LocalDestinationSnapshot? snapshot;
      if (operation == TransferOperation.downloadUpdate) {
        final mtime = task.sourceMtime;
        if (mtime == null) {
          throw AppError.generic('更新下载缺少本地目标修改时间快照');
        }
        final size = task.sourceSize;
        if (size == null) {
          throw AppError.generic('更新下载缺少本地目标大小快照');
        }
        if (size < 0) {
          throw AppError.generic('更新下载本地目标大小非法');
        }
        snapshot = LocalDestinationSnapshot(mtimeMs: mtime, size: size);
      } else {
        snapshot = null;
      }

      final expectation = DownloadExpectation(
        editedTimeMs: task.expectedCloudEditedTime,
        size: task.totalSize >= 0 ? task.totalSize : null,
        destinationSnapshot: snapshot,
        placeholderFileId:
            operation == TransferOperation.download ? fileId : null,
      );

      await _downloadWithExpectation(
        fileId,
        localPath,
        expectation,
        onProgress == null
            ? null
            : (received, total) => onProgress(received),
      );
    }, 'downloadForTask');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 下载主流程（对齐 Rust download_with_expectation）
  // ═══════════════════════════════════════════════════════════════════

  Future<void> _downloadWithExpectation(
    String fileId,
    String destPath,
    DownloadExpectation? expectation,
    DownloadProgressFn? onProgress,
  ) async {
    if (fileId.isEmpty) {
      throw AppError.generic('下载 file_id 不能为空');
    }
    final parent = File(destPath).parent;
    if (parent.path.isNotEmpty && parent.path != '.') {
      try {
        await parent.create(recursive: true);
      } catch (e) {
        throw AppError.generic('创建下载目录失败：$e');
      }
    }

    final _ResumeMetadata remote;
    try {
      remote = await _fetchRemoteMetadata(fileId);
    } on AppError catch (e) {
      _cleanupIfPermanent(destPath, e);
      rethrow;
    }
    if (expectation != null && !_matchesExpectation(remote, expectation)) {
      discardResumeArtifacts(destPath);
      throw AppError.generic('云端文件版本已变化，当前下载任务已过期，请重新规划同步');
    }

    final tmp = tmpPath(destPath);
    var offset = await _validatedResumeOffset(destPath, remote);
    await _writeResumeMetadata(destPath, remote);

    // 上次响应已经写完，但在最终核验或 rename 前断网/崩溃：不重复下载。
    if (File(tmp).existsSync() && offset == remote.size) {
      return _verifyAndInstall(fileId, destPath, remote, expectation);
    }

    // 空文件没有内容请求也可以安全落盘。
    if (remote.size == 0) {
      final raf = await _createTmp(tmp);
      await raf.close();
      return _verifyAndInstall(fileId, destPath, remote, expectation);
    }

    // Range 不匹配或 416 时只允许在本次调用中安全回退一次到 offset=0。
    var restartedFromZero = offset == 0;
    while (true) {
      final ({Response<ResponseBody> response, bool authReplayed}) sent;
      try {
        sent = await _sendContentRequest(fileId, offset, remote.etag);
      } on AppError catch (e) {
        _cleanupIfPermanent(destPath, e);
        rethrow;
      }
      final response = sent.response;
      final status = response.statusCode ?? 0;

      if (status == 416 && offset > 0 && !restartedFromZero) {
        discardResumeArtifacts(destPath);
        await _writeResumeMetadata(destPath, remote);
        offset = 0;
        restartedFromZero = true;
        continue;
      }
      if (status < 200 || status >= 300) {
        final error = httpErrorFromResponse(
            response, RequestSemantics.read, sent.authReplayed);
        _cleanupIfPermanent(destPath, error);
        throw error;
      }

      final int writeOffset;
      try {
        writeOffset = _validatedResponseOffset(response, offset, remote.size);
      } on _RangeProtocolError catch (e) {
        if (offset > 0 && !restartedFromZero) {
          AppLogger.w('Range 响应不可信，从 0 重启: ${e.message}');
          discardResumeArtifacts(destPath);
          await _writeResumeMetadata(destPath, remote);
          offset = 0;
          restartedFromZero = true;
          continue;
        }
        discardResumeArtifacts(destPath);
        throw AppError.generic(e.message);
      }

      final raf = writeOffset == 0
          ? await _createTmp(tmp)
          : await _openTmpAppend(tmp);
      var received = writeOffset;
      onProgress?.call(received, remote.size);
      final body = response.data;
      try {
        if (body != null) {
          await for (final chunk in body.stream) {
            await raf.writeFrom(chunk);
            received += chunk.length;
            onProgress?.call(received, remote.size);
          }
        }
      } catch (e) {
        await _flushAndClose(raf);
        if (e is DioException) {
          throw AppError.fromDioException(e,
              semantics: RequestSemantics.read,
              authAlreadyReplayed: sent.authReplayed);
        }
        throw AppError.driveTransportWithSubmission(
          DriveTransportKind.responseBody,
          requestMayHaveReachedServer: false,
          authAlreadyReplayed: sent.authReplayed,
          cause: e.toString(),
        );
      }
      await _flushAndClose(raf);

      final actualSize = await _tmpLength(tmp);
      if (actualSize != remote.size) {
        if (actualSize > remote.size) {
          discardResumeArtifacts(destPath);
          throw AppError.generic(
              '下载长度异常：期望 ${remote.size} 字节，实际 $actualSize 字节');
        }
        // 某些代理会干净地提前结束响应；保留部分文件，下一次继续 Range。
        throw AppError.driveNetwork(
            '下载响应提前结束：期望 ${remote.size} 字节，已接收 $actualSize 字节');
      }

      return _verifyAndInstall(fileId, destPath, remote, expectation);
    }
  }

  /// 发送内容 GET；遇到 401 时刷新 token 并原样重放一次。
  Future<({Response<ResponseBody> response, bool authReplayed})>
      _sendContentRequest(String fileId, int offset, String? etag) {
    return _client.requestRawAuthed<ResponseBody>(
      'GET',
      '$_driveBase/files/${urlEncoding(fileId)}?form=content',
      headers: {
        if (offset > 0) 'Range': 'bytes=$offset-',
        'If-Match': ?etag,
      },
      responseType: ResponseType.stream,
      semantics: RequestSemantics.read,
      followRedirects: true,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 云端版本元数据（对齐 Rust fetch_remote_metadata）
  // ═══════════════════════════════════════════════════════════════════

  /// 获取并严格校验下载所需的云端版本元数据。
  Future<_ResumeMetadata> _fetchRemoteMetadata(String fileId) async {
    final sent = await _client.requestRawAuthed<String>(
      'GET',
      '$_driveBase/files/${urlEncoding(fileId)}?fields=*',
      responseType: ResponseType.plain,
      semantics: RequestSemantics.read,
    );
    final response = sent.response;
    final status = response.statusCode ?? 0;
    if (status < 200 || status >= 300) {
      throw httpErrorFromResponse(
          response, RequestSemantics.read, sent.authReplayed);
    }
    final headerEtag = response.headers.value('etag');

    final Object? decoded;
    try {
      decoded = jsonDecode(response.data ?? '');
    } catch (e) {
      throw AppError.driveTransportWithSubmission(
        DriveTransportKind.decode,
        requestMayHaveReachedServer: false,
        authAlreadyReplayed: sent.authReplayed,
        cause: e.toString(),
      );
    }
    if (decoded is! Map<String, dynamic>) {
      throw AppError.generic('下载元数据缺少有效 id');
    }
    final body = decoded;

    final returnedId = body['id'];
    if (returnedId is! String || returnedId.isEmpty) {
      throw AppError.generic('下载元数据缺少有效 id');
    }
    if (returnedId != fileId) {
      throw AppError.generic('下载元数据 id 与请求不一致');
    }
    final size = _parseU64(body['size']);
    if (size == null) {
      throw AppError.generic('下载元数据缺少有效 size');
    }
    final revision =
        _scalarString(body['contentVersion']) ?? _scalarString(body['version']);
    final editedRaw = body['editedTime'];
    final editedTimeMs = editedRaw is String
        ? DateTime.tryParse(editedRaw)?.toUtc().millisecondsSinceEpoch
        : null;
    final sha256 =
        _nonEmptyString(body['sha256']) ?? _nonEmptyString(body['fileSha256']);
    final contentHash = _nonEmptyString(body['contentHash']) ??
        _nonEmptyString(body['hash']) ??
        _nonEmptyString(body['md5']) ??
        _nonEmptyString(body['md5Checksum']);
    final etag =
        _nonEmptyString(headerEtag) ?? _nonEmptyString(body['etag']);

    return _ResumeMetadata(
      fileId: fileId,
      size: size,
      revision: revision,
      editedTimeMs: editedTimeMs,
      etag: etag,
      sha256: sha256,
      contentHash: contentHash,
    );
  }

  /// 仅在断点身份与当前云端版本一致时返回可续传偏移。
  Future<int> _validatedResumeOffset(
      String destPath, _ResumeMetadata current) async {
    final tmp = tmpPath(destPath);
    if (!File(tmp).existsSync()) {
      _removeResumeMetadata(destPath);
      return 0;
    }

    final stored = await _readResumeMetadata(destPath);
    if (stored != current || !current.hasStableIdentity) {
      discardResumeArtifacts(destPath);
      return 0;
    }
    final length = await _tmpLength(tmp, '读取断点文件长度失败');
    if (length > current.size) {
      discardResumeArtifacts(destPath);
      return 0;
    }
    return length;
  }

  /// 复核长度、哈希及两端版本后原子安装临时文件。
  Future<void> _verifyAndInstall(
    String fileId,
    String destPath,
    _ResumeMetadata downloaded,
    DownloadExpectation? expectation,
  ) async {
    final tmp = tmpPath(destPath);
    final actualSize = await _tmpLength(tmp, '读取临时文件失败');
    if (actualSize != downloaded.size) {
      if (actualSize > downloaded.size) {
        discardResumeArtifacts(destPath);
      }
      throw AppError.driveNetwork('断点文件尚未下载完整');
    }

    final expectedSha256 = downloaded.sha256;
    if (expectedSha256 != null) {
      final actualSha256 = await _sha256File(tmp);
      if (actualSha256.toLowerCase() != expectedSha256.toLowerCase()) {
        discardResumeArtifacts(destPath);
        throw AppError.generic('下载文件 sha256 校验失败');
      }
    }

    // 内容读取结束后再取一次元数据，防止无 ETag 时把两个云端版本混为一次成功。
    final _ResumeMetadata current;
    try {
      current = await _fetchRemoteMetadata(fileId);
    } on AppError catch (e) {
      _cleanupIfPermanent(destPath, e);
      rethrow;
    }
    if (current != downloaded) {
      discardResumeArtifacts(destPath);
      throw AppError.generic('下载期间云端文件发生变化，已丢弃旧断点并等待重新下载');
    }

    await _verifyLocalDestination(destPath, expectation);

    // POSIX rename 在同一文件系统内原子替换旧目标；失败时保留 .tmp 供重试。
    try {
      await File(tmp).rename(destPath);
    } catch (e) {
      throw AppError.generic('安装下载文件失败：$e');
    }
    _removeResumeMetadata(destPath);
    AppLogger.i('下载完成: $fileId → $destPath');
  }

  /// 安装前确认本地目标仍为空缺、原快照或本文件的未改占位符。
  Future<void> _verifyLocalDestination(
      String destPath, DownloadExpectation? expectation) async {
    final exp = expectation;
    if (exp == null) return;

    /// lstat 语义（不跟随符号链接）；不存在时返回 notFound 而非抛异常
    FileStat lstat() {
      try {
        return Link(destPath).statSync();
      } catch (e) {
        throw AppError.generic('安装下载结果前读取目标路径失败：$e');
      }
    }

    final snapshot = exp.destinationSnapshot;
    if (snapshot != null) {
      final stat = lstat();
      if (stat.type == FileSystemEntityType.notFound) {
        throw AppError.generic('安装下载结果前读取原文件失败：目标不存在');
      }
      final mtimeMs = stat.modified.millisecondsSinceEpoch;
      if (stat.type == FileSystemEntityType.link ||
          stat.type != FileSystemEntityType.file ||
          stat.size != snapshot.size ||
          mtimeMs != snapshot.mtimeMs) {
        throw AppError.generic('下载期间本地目标已被修改，已保留用户内容和下载临时文件');
      }
      return;
    }

    final placeholderId = exp.placeholderFileId;
    if (placeholderId != null) {
      final stat = lstat();
      if (stat.type == FileSystemEntityType.notFound) return;
      // 对齐 Rust verify_local_destination：占位属主核验
      // （state == placeholder 且 owner fileId 匹配才允许覆盖）；
      // 属主数据源：inode 映射优先（docs/design/10），xattr 过渡回退
      final xattr = _xattr;
      final isPlaceholder = xattr != null &&
          (await xattr.get(destPath, xattrState)) == statePlaceholder;
      String? owner;
      final inodeOwner = _inodeOwnerProvider;
      if (inodeOwner != null) {
        owner = await inodeOwner(destPath);
      }
      owner ??= xattr != null ? await xattr.get(destPath, xattrFileId) : null;
      if (stat.type == FileSystemEntityType.link ||
          stat.type != FileSystemEntityType.file ||
          stat.size != 0 ||
          (xattr != null && (!isPlaceholder || owner != placeholderId))) {
        throw AppError.generic('下载期间目标路径出现用户内容，已拒绝覆盖并保留下载临时文件');
      }
    }
  }

  /// 判断远端版本是否满足调度器提供的全部约束。
  bool _matchesExpectation(_ResumeMetadata remote, DownloadExpectation exp) {
    final editedTime = exp.editedTimeMs;
    if (editedTime != null && remote.editedTimeMs != editedTime) return false;
    final size = exp.size;
    if (size != null && remote.size != size) return false;
    final hash = exp.contentHash;
    if (hash != null) {
      final actual = remote.sha256 ?? remote.contentHash;
      if (actual == null || actual.toLowerCase() != hash.toLowerCase()) {
        return false;
      }
    }
    return true;
  }

  /// 返回写入起点。`200` 表示服务端忽略 Range，调用方必须截断后从 0 写。
  int _validatedResponseOffset(
    Response<dynamic> response,
    int requestedOffset,
    int expectedTotal,
  ) {
    final status = response.statusCode ?? 0;
    if (status == 200) return 0;
    if (status == 206) {
      final value = response.headers.value('Content-Range');
      if (value == null) {
        throw const _RangeProtocolError('Range 响应缺少 Content-Range');
      }
      final parsed = _parseContentRange(value);
      if (parsed == null) {
        throw const _RangeProtocolError('Range 响应的 Content-Range 无效');
      }
      final (start, end, total) = parsed;
      if (start != requestedOffset ||
          total != expectedTotal ||
          end < start ||
          end >= total) {
        throw _RangeProtocolError(
            'Range 响应不匹配：请求 $requestedOffset，响应 $start-$end/$total');
      }
      return start;
    }
    throw _RangeProtocolError('下载返回了不支持的成功状态码：$status');
  }

  /// 解析 `bytes start-end/total` 响应范围。
  (int, int, int)? _parseContentRange(String value) {
    final trimmed = value.trim();
    if (!trimmed.startsWith('bytes ')) return null;
    final rest = trimmed.substring(6);
    final slash = rest.indexOf('/');
    if (slash < 0) return null;
    final range = rest.substring(0, slash);
    final dash = range.indexOf('-');
    if (dash < 0) return null;
    final start = int.tryParse(range.substring(0, dash));
    final end = int.tryParse(range.substring(dash + 1));
    final total = int.tryParse(rest.substring(slash + 1));
    if (start == null || end == null || total == null) return null;
    return (start, end, total);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 断点元数据 sidecar（对齐 Rust ResumeMetadata 持久化）
  // ═══════════════════════════════════════════════════════════════════

  /// 尝试读取断点元数据；缺失、读失败或损坏均视为不可续传。
  Future<_ResumeMetadata?> _readResumeMetadata(String dest) async {
    try {
      final text = await File(resumeMetadataPath(dest)).readAsString();
      final json = jsonDecode(text);
      if (json is! Map<String, dynamic>) return null;
      return _ResumeMetadata.fromJson(json);
    } catch (_) {
      return null;
    }
  }

  /// 先同步暂存文件再原子提交断点元数据。
  Future<void> _writeResumeMetadata(
      String dest, _ResumeMetadata metadata) async {
    final bytes = jsonEncode(metadata.toJson());
    final staging = _resumeMetadataStagingPath(dest);
    final target = resumeMetadataPath(dest);
    try {
      final raf = await File(staging).open(mode: FileMode.write);
      try {
        await raf.writeString(bytes);
        await raf.flush();
      } finally {
        await raf.close();
      }
    } catch (e) {
      throw AppError.generic('写入下载断点元数据失败：$e');
    }
    try {
      await File(staging).rename(target);
    } catch (e) {
      throw AppError.generic('提交下载断点元数据失败：$e');
    }
  }

  /// 仅对判定为永久失败的错误清除断点，暂态失败保留现场。
  void _cleanupIfPermanent(String destPath, AppError error) {
    final keep = switch (error) {
      DriveApiError(:final statusCode) => statusCode == null ||
          const {401, 408, 409, 425, 429}.contains(statusCode) ||
          (statusCode >= 500 && statusCode <= 599),
      TokenError() || AuthError() || GenericError() => true,
      ConfigError() || QuotaExceededError() => false,
    };
    if (!keep) {
      discardResumeArtifacts(destPath);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 文件系统辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 创建（截断）临时文件。
  Future<RandomAccessFile> _createTmp(String tmp) async {
    try {
      final raf = await File(tmp).open(mode: FileMode.write);
      await raf.flush();
      return raf;
    } catch (e) {
      throw AppError.generic('创建临时文件失败：$e');
    }
  }

  /// 以追加模式打开临时文件。
  Future<RandomAccessFile> _openTmpAppend(String tmp) async {
    try {
      return await File(tmp).open(mode: FileMode.append);
    } catch (e) {
      throw AppError.generic('打开临时文件失败：$e');
    }
  }

  /// flush（fsync）后关闭。
  Future<void> _flushAndClose(RandomAccessFile raf) async {
    try {
      await raf.flush();
    } catch (e) {
      try {
        await raf.close();
      } catch (_) {}
      throw AppError.generic('同步临时文件失败：$e');
    }
    await raf.close();
  }

  /// 读取临时文件长度。
  Future<int> _tmpLength(String tmp, [String ctx = '读取临时文件长度失败']) async {
    try {
      return await File(tmp).length();
    } catch (e) {
      throw AppError.generic('$ctx：$e');
    }
  }

  /// 流式计算文件 SHA-256；打开或读取失败直接返回错误。
  Future<String> _sha256File(String path) async {
    final sink = _DigestSink();
    final chunked = sha256.startChunkedConversion(sink);
    try {
      await for (final chunk in File(path).openRead()) {
        chunked.add(chunk);
      }
    } catch (e) {
      chunked.close();
      throw AppError.generic('读取临时文件校验失败：$e');
    }
    chunked.close();
    final digest = sink.value;
    if (digest == null) {
      throw AppError.generic('读取临时文件校验失败：摘要缺失');
    }
    return digest.toString();
  }

  // ═══════════════════════════════════════════════════════════════════
  // JSON 字段辅助（对齐 Rust parse_u64 / scalar_string / nonempty_string）
  // ═══════════════════════════════════════════════════════════════════

  /// 从无符号整数或十进制字符串读取字节数。
  int? _parseU64(Object? value) {
    if (value is int) return value >= 0 ? value : null;
    if (value is String) {
      final parsed = int.tryParse(value);
      return parsed != null && parsed >= 0 ? parsed : null;
    }
    return null;
  }

  /// 将非空字符串或数值标量转换为版本字符串。
  String? _scalarString(Object? value) {
    if (value is String && value.isNotEmpty) return value;
    if (value is num) return value.toString();
    return null;
  }

  /// 克隆可选的非空 JSON 字符串。
  String? _nonEmptyString(Object? value) {
    if (value is String && value.isNotEmpty) return value;
    return null;
  }

  /// 把抛 [AppError] 的实现包装为 [AppResult]（Service 惯例：错误以 Err 返回）。
  Future<AppResult<T>> _guard<T>(Future<T> Function() body, String op) async {
    try {
      return Ok(await body());
    } on AppError catch (e) {
      return Err(e);
    } catch (e, st) {
      AppLogger.e('$op 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }
}

/// 与临时内容绑定的持久化断点身份（对齐 Rust `ResumeMetadata`）。
class _ResumeMetadata {
  final String fileId;
  final int size;
  final String? revision;
  final int? editedTimeMs;
  final String? etag;
  final String? sha256;
  final String? contentHash;

  const _ResumeMetadata({
    required this.fileId,
    required this.size,
    this.revision,
    this.editedTimeMs,
    this.etag,
    this.sha256,
    this.contentHash,
  });

  /// 判断元数据是否含可阻止跨版本续传的稳定身份。
  bool get hasStableIdentity =>
      revision != null ||
      editedTimeMs != null ||
      etag != null ||
      sha256 != null ||
      contentHash != null;

  /// 序列化为 sidecar JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'file_id': fileId,
      'size': size,
      'revision': revision,
      'edited_time_ms': editedTimeMs,
      'etag': etag,
      'sha256': sha256,
      'content_hash': contentHash,
    };
  }

  /// 从 sidecar JSON 构造；关键字段类型不符视为损坏（抛 FormatException）。
  factory _ResumeMetadata.fromJson(Map<String, dynamic> json) {
    final fileId = json['file_id'];
    final size = json['size'];
    if (fileId is! String || size is! int) {
      throw const FormatException('损坏的断点元数据');
    }
    String? optStr(String k) {
      final v = json[k];
      return v is String ? v : null;
    }

    final editedTime = json['edited_time_ms'];
    return _ResumeMetadata(
      fileId: fileId,
      size: size,
      revision: optStr('revision'),
      editedTimeMs: editedTime is int ? editedTime : null,
      etag: optStr('etag'),
      sha256: optStr('sha256'),
      contentHash: optStr('content_hash'),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is _ResumeMetadata &&
        other.fileId == fileId &&
        other.size == size &&
        other.revision == revision &&
        other.editedTimeMs == editedTimeMs &&
        other.etag == etag &&
        other.sha256 == sha256 &&
        other.contentHash == contentHash;
  }

  @override
  int get hashCode => Object.hash(
      fileId, size, revision, editedTimeMs, etag, sha256, contentHash);
}

/// Range 协议错误（携带用户可读消息，由主流程决定是否回退到 0）
class _RangeProtocolError implements Exception {
  final String message;
  const _RangeProtocolError(this.message);

  @override
  String toString() => message;
}

/// 收集流式 SHA-256 结果的 Sink
class _DigestSink implements Sink<Digest> {
  Digest? value;

  @override
  void add(Digest data) => value = data;

  @override
  void close() {}
}
