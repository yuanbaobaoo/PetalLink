import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/about_service.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';
import 'package:petal_link/service/drive/drive_http.dart';
import 'package:petal_link/types/enums.dart';

// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// 上传进度回调：接收 0.0..=1.0 上传比例（对齐 Rust `ProgressFn`）。
typedef UploadProgressFn = void Function(double ratio);

/// 断点续传进度回调（serverId, uploadId, 已上传字节偏移, sessionUrl）。
///
/// 供调用方把会话字段持久化到 transfer_queue 行（崩溃续传唯一 token 是
/// sessionUrl）。对齐 Rust `ResumeProgressFn`。
typedef ResumeProgressFn = void Function(
  String serverId,
  String uploadId,
  int offset,
  String sessionUrl,
);

/// 上传 API 服务 —— 小文件 multipart/related + 大文件分片断点续传 + 更新覆盖。
///
/// 严格对齐 Rust 原版 `src/drive/upload_api/`（routing / multipart / protocol /
/// chunk / resumable）：
///
/// - ≤ 20MB：`multipart/related` 单请求（**不是** multipart/form-data）
/// - > 20MB：可恢复会话——POST init 拿 `Location` 会话 URL → PUT 分块
///   （`Content-Range`）→ 服务端 `rangeList` 是唯一可信断点 → 全部字节确认后
///   最终状态轮询最多 5×3s
/// - 写歧义不猜偏移：连接/超时/5xx/响应体丢失先对同一会话发状态查询，
///   只按服务端确认 offset 前进，禁止用 `offset+chunkLen` 推算
class UploadService {
  final MateHttpClient _client;

  /// Upload API base
  final String _uploadBase;

  /// 小文件 multipart 与大文件断点续传的路由阈值（20MiB，对齐 Rust）
  static const int smallLargeThreshold = 20 * 1024 * 1024;

  /// 华为官方 SDK 允许的最小分片大小
  static const int minChunkSize = 256 * 1024;

  /// 服务端未建议分片大小时使用的默认值（2MiB，对齐 Rust）
  static const int defaultChunkSize = 2 * 1024 * 1024;

  /// REST 接口允许的单片大小上限（64MiB，对齐 Rust）
  static const int maxChunkSize = 64 * 1024 * 1024;

  /// 明确未提交的连接失败允许的单分片本地尝试次数
  final int chunkRetries;

  /// 分片全部发完后的最终状态查询轮询次数（华为服务端异步合并，立即查询常得 308）
  final int finalStatusMaxPolls;

  /// 最终状态查询间隔（默认 3s；第 n 次本地重试退避基数）
  final Duration finalPollInterval;

  /// 分片本地重试的退避基数（第 attempt 次睡 attempt × 该值）
  final Duration chunkRetryDelayUnit;

  /// 最终状态查询失败续查的间隔
  final Duration finalPollErrorDelay;

  UploadService(
    this._client, {
    String uploadBase = uploadApiBase,
    this.chunkRetries = 3,
    this.finalStatusMaxPolls = 5,
    this.finalPollInterval = const Duration(seconds: 3),
    this.chunkRetryDelayUnit = const Duration(seconds: 1),
    this.finalPollErrorDelay = const Duration(seconds: 1),
  }) : _uploadBase = uploadBase;

  // ═══════════════════════════════════════════════════════════════════
  // 路由与执行器级入口
  // ═══════════════════════════════════════════════════════════════════

  /// 路由上传：≤ 20MB → 小文件 multipart，否则分片续传。
  ///
  /// 对齐 Rust `UploadApi::upload`。
  Future<AppResult<DriveFile>> upload(
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  }) {
    return _guard(() async {
      final size = await _fileLength(filePath);
      if (size <= smallLargeThreshold) {
        return _uploadSmall(filePath, parentId: parentId, onProgress: onProgress);
      }
      return _uploadResume(
        filePath,
        parentId: parentId,
        onProgress: onProgress,
        onResumeProgress: onResumeProgress,
      );
    }, 'upload');
  }

  /// 小文件 multipart/related 上传（对齐 Rust `upload_small`）。
  Future<AppResult<DriveFile>> uploadSmall(
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
  }) {
    return _guard(
      () => _uploadSmall(filePath, parentId: parentId, onProgress: onProgress),
      'uploadSmall',
    );
  }

  /// 大文件 resume 分片上传（对齐 Rust `upload_resume`）。
  ///
  /// [resume] 为持久化的断点会话；其 `startOffset` 只是本地提示，真正起点
  /// 必须由同一 session URL 的状态查询确认。
  Future<AppResult<DriveFile>> uploadResume(
    String filePath, {
    String? parentId,
    ResumeSession? resume,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  }) {
    return _guard(
      () => _uploadResume(
        filePath,
        parentId: parentId,
        resume: resume,
        onProgress: onProgress,
        onResumeProgress: onResumeProgress,
      ),
      'uploadResume',
    );
  }

  /// 更新云端已有文件（PATCH multipart/related，用于冲突解决）。
  ///
  /// > 20MiB 的既有文件替换明确拒绝（当前不支持安全的 resumable update）；
  /// PATCH 失败必须保留旧文件，禁止回退为新建。对齐 Rust `upload_update`。
  Future<AppResult<DriveFile>> uploadUpdate(
    String fileId,
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
  }) {
    return _guard(
      () => _uploadUpdate(fileId, filePath,
          parentId: parentId, onProgress: onProgress),
      'uploadUpdate',
    );
  }

  /// 执行器级入口：按任务行执行上传（Create/Update）。
  ///
  /// 对齐 Rust `TransferOperations::execute` 的上传分支：
  /// - 校验 operation 与本地路径，核验入队源快照（mtime/size 未变才执行）
  /// - 任务行已有 sessionUrl 时按持久化会话走续传恢复（偏移以服务端
  ///   rangeList 为准），否则按大小路由
  /// - [onProgress] 接收已传输字节数（ratio × totalSize 截断）
  /// - [onResumeProgress] 供调用方持久化会话字段到任务行
  /// - [isOnline] 网络门控钩子：返回 false 时以网络错误拒绝执行
  ///
  /// 远端前置核验（同名冲突、云端 editedTime 比对）由任务编排层负责，
  /// 不在本层职责内。
  Future<AppResult<DriveFile>> uploadForTask(
    TransferTask task, {
    void Function(int transferred)? onProgress,
    ResumeProgressFn? onResumeProgress,
    bool Function()? isOnline,
  }) {
    return _guard(() async {
      final operation = task.operation;
      if (operation == null) {
        throw AppError.generic('任务缺少 operation');
      }
      final localPath = task.localPath;
      if (localPath == null || localPath.isEmpty) {
        throw AppError.generic('任务缺少本地路径');
      }
      if (operation != TransferOperation.Create &&
          operation != TransferOperation.Update) {
        throw AppError.generic('该 operation 不支持传输执行');
      }
      if (isOnline != null && !isOnline()) {
        throw AppError.driveNetwork('网络离线，上传已暂停');
      }

      // 远端写入前再次核验上传源快照。
      await _verifySourceSnapshot(task, localPath);

      final UploadProgressFn? ratioFn = onProgress == null
          ? null
          : (ratio) => onProgress(
              (ratio.clamp(0.0, 1.0) * task.totalSize).toInt());

      if (operation == TransferOperation.Update) {
        final fileId = task.fileId;
        if (fileId == null || fileId.isEmpty) {
          throw AppError.generic('更新上传任务缺少 fileId');
        }
        return _uploadUpdate(fileId, localPath,
            parentId: task.parentFileId, onProgress: ratioFn);
      }

      // 持久化会话即使偏移为零也必须走续传核验。
      final sessionUrl = task.sessionUrl;
      if (sessionUrl != null && sessionUrl.trim().isNotEmpty) {
        final session = ResumeSession(
          serverId: task.serverId ?? '',
          uploadId: task.uploadId ?? '',
          sessionUrl: sessionUrl,
          chunkSize: 0,
          startOffset: task.resumeOffset,
        );
        return _uploadResume(
          localPath,
          parentId: task.parentFileId,
          resume: session,
          onProgress: ratioFn,
          onResumeProgress: onResumeProgress,
        );
      }
      return _uploadForTaskRoute(
        localPath,
        parentId: task.parentFileId,
        onProgress: ratioFn,
        onResumeProgress: onResumeProgress,
      );
    }, 'uploadForTask');
  }

  /// uploadForTask 的大小路由（跳过外层 _guard，复用本层实现）。
  Future<DriveFile> _uploadForTaskRoute(
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  }) async {
    final size = await _fileLength(filePath);
    if (size <= smallLargeThreshold) {
      return _uploadSmall(filePath, parentId: parentId, onProgress: onProgress);
    }
    return _uploadResume(
      filePath,
      parentId: parentId,
      onProgress: onProgress,
      onResumeProgress: onResumeProgress,
    );
  }

  /// 仅散列仍匹配持久任务快照的上传源才允许执行（对齐 Rust verify_source_snapshot）。
  Future<void> _verifySourceSnapshot(
      TransferTask task, String localPath) async {
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

  // ═══════════════════════════════════════════════════════════════════
  // 小文件 multipart/related（对齐 Rust upload_api/multipart.rs）
  // ═══════════════════════════════════════════════════════════════════

  Future<DriveFile> _uploadSmall(
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
  }) async {
    await _ensureCapacityFor(filePath);
    final fileName = _fileName(filePath);
    final boundary = _newBoundary();
    final metadata = _buildMetadataJson(fileName, parentId);
    final fileBytes = await _readFileBytes(filePath);
    final body =
        _buildMultipartRelated(boundary, utf8.encode(metadata), fileBytes);

    final sent = await _sendMultipart(
        'POST', '$_uploadBase/files?uploadType=multipart', boundary, body);
    final response = sent.response;
    final status = response.statusCode ?? 0;
    if (status < 200 || status >= 300) {
      throw httpErrorFromResponse(
          response, RequestSemantics.write, sent.authReplayed);
    }
    onProgress?.call(1.0);
    final bodyJson = _decodeBodyMap(response.data, '小文件上传',
        isWrite: true, authReplayed: sent.authReplayed);
    final file = _completeUploadFile(bodyJson, fileBytes.length, fileName);
    if (file == null) {
      throw _remoteAmbiguity(
          '小文件上传返回 2xx，但文件身份/名称/长度不完整或不匹配', sent.authReplayed);
    }
    AppLogger.i('小文件上传成功: $fileName (${file.id})');
    return file;
  }

  Future<DriveFile> _uploadUpdate(
    String fileId,
    String filePath, {
    String? parentId,
    UploadProgressFn? onProgress,
  }) async {
    final size = await _rejectUnsafeLargeUpdate(fileId, filePath);
    await _ensureCapacityFor(filePath);
    final fileName = _fileName(filePath);
    final boundary = _newBoundary();
    final metadata = _buildMetadataJson(fileName, parentId);
    final fileBytes = await _readFileBytes(filePath);
    final body =
        _buildMultipartRelated(boundary, utf8.encode(metadata), fileBytes);

    final sent = await _sendMultipart('PATCH',
        '$_uploadBase/files/$fileId?uploadType=multipart', boundary, body);
    final response = sent.response;
    final status = response.statusCode ?? 0;
    if (status < 200 || status >= 300) {
      AppLogger.w('PATCH 更新失败，保留云端旧文件: $fileId ($status)');
      throw httpErrorFromResponse(
          response, RequestSemantics.write, sent.authReplayed);
    }
    final bodyJson = _decodeBodyMap(response.data, 'PATCH 更新',
        isWrite: true, authReplayed: sent.authReplayed);
    onProgress?.call(1.0);
    final file = _completeUploadFile(bodyJson, size, fileName);
    if (file == null) {
      throw _remoteAmbiguity(
          'PATCH 更新返回 2xx，但文件身份/名称/长度不完整或不匹配', sent.authReplayed);
    }
    return file;
  }

  /// 发送 multipart/related 请求（401 由 requestRawAuthed 单次重放）。
  Future<({Response<String> response, bool authReplayed})> _sendMultipart(
    String method,
    String url,
    String boundary,
    Uint8List body,
  ) {
    return _client.requestRawAuthed<String>(
      method,
      url,
      data: body,
      headers: {
        'Content-Type': 'multipart/related; boundary=$boundary',
        'Content-Length': body.length.toString(),
      },
      responseType: ResponseType.plain,
      semantics: RequestSemantics.write,
      sendTimeout: const Duration(seconds: 120),
    );
  }

  /// 拒绝会迫使已有 fileId 退化为新建的大文件覆盖请求。
  Future<int> _rejectUnsafeLargeUpdate(String fileId, String filePath) async {
    final size = await _fileLength(filePath);
    if (size > smallLargeThreshold) {
      throw AppError.generic(
          '现有云端文件更新大小超过 20 MiB，当前不支持安全的 resumable update；'
          '已保留原文件，禁止回退为新建（fileId=$fileId）');
    }
    return size;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 大文件断点续传（对齐 Rust upload_api/resumable.rs）
  // ═══════════════════════════════════════════════════════════════════

  Future<DriveFile> _uploadResume(
    String filePath, {
    String? parentId,
    ResumeSession? resume,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  }) async {
    final totalSize = await _fileLength(filePath);
    await _ensureCapacityFor(filePath);
    final fileName = _fileName(filePath);

    // 1. 初始化或恢复 resume 会话。已有会话必须先向服务端查询确认偏移。
    final ResumeSession session;
    final bool verifyServerOffset;
    final existing = resume;
    if (existing != null) {
      session = existing;
      verifyServerOffset = true;
    } else {
      final initialized = await _initResumeSessionWithAuthReplay(
          fileName, parentId, totalSize);
      // 通知调用方持久化会话信息（含 sessionUrl，断点续传必需）
      _notifyResumeProgress(
          initialized, 0, totalSize, null, onResumeProgress);
      session = initialized;
      verifyServerOffset = false;
    }
    return _uploadResumeSession(
      filePath,
      session,
      verifyServerOffset,
      onProgress,
      onResumeProgress,
    );
  }

  /// 提交一次非幂等会话初始化；401 刷新后重放一次，其余失败不自动新建第二个会话。
  Future<ResumeSession> _initResumeSessionWithAuthReplay(
    String fileName,
    String? parentId,
    int totalSize,
  ) async {
    try {
      return await _initResumeSession(fileName, parentId, totalSize);
    } on AppError catch (e) {
      if (e.driveStatus != 401) {
        // 初始化 POST 的响应可能已丢失，绝不能回退为另一个 create。
        AppLogger.w('resume 会话初始化失败，保留结构化错误并停止新建重放: $e');
        rethrow;
      }
      final newToken = await _client.forceRefreshToken();
      if (newToken == null || newToken.isEmpty) {
        throw AppError.tokenRefreshFailed();
      }
      return _initResumeSession(fileName, parentId, totalSize);
    }
  }

  /// 提交一次会话初始化（对齐 Rust `init_resume_session`）。
  Future<ResumeSession> _initResumeSession(
    String fileName,
    String? parentId,
    int totalSize,
  ) async {
    final metadata = _buildMetadataJson(fileName, parentId);
    final Response<String> response;
    try {
      response = await _client.requestRaw<String>(
        'POST',
        '$_uploadBase/files?uploadType=resume',
        data: metadata,
        headers: {
          'X-Upload-Content-Length': totalSize.toString(),
          'Content-Type': 'application/json',
        },
        responseType: ResponseType.plain,
      );
    } on DioException catch (e) {
      throw AppError.fromDioException(e, semantics: RequestSemantics.write);
    }
    final status = response.statusCode ?? 0;
    if (status < 200 || status >= 300) {
      throw httpErrorFromResponse(response, RequestSemantics.write, false);
    }

    // ★ 关键：从 Location 响应头获取会话 URL（Google Drive 风格断点续传）。
    // 华为 API 变更后 body 仅含 {"sliceSize":...}，不含 serverId/uploadId，
    // 后续分片 PUT 必须直接用 Location 头返回的 URL。
    final sessionUrl = response.headers.value('Location') ?? '';

    final init = _decodeBodyMap(response.data, '上传会话初始化', isWrite: true);
    AppLogger.i('resume 会话初始化响应: status=$status '
        'hasLocation=${sessionUrl.isNotEmpty} body=$init');

    // 解析 body 中的标识字段（旧 API 兼容）
    final serverId = _firstString(init, const ['serverId', 'id', 'fileId']);
    final uploadId = init['uploadId'] is String
        ? init['uploadId'] as String
        : '';
    final chunkSize = _asNonNegativeInt(init['sliceSize']) ?? 0;

    if (serverId == null && sessionUrl.isEmpty) {
      throw _remoteAmbiguity(
          '上传会话响应缺少 Location/serverId（可用字段: ${init.keys.toList()}）',
          false);
    }

    return ResumeSession(
      serverId: serverId ?? '',
      uploadId: uploadId,
      sessionUrl: sessionUrl,
      chunkSize: chunkSize,
      startOffset: 0,
    );
  }

  /// 使用一个已初始化的会话上传（对齐 Rust `upload_resume_session`）。
  Future<DriveFile> _uploadResumeSession(
    String filePath,
    ResumeSession session,
    bool verifyServerOffset,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  ) async {
    final totalSize = await _fileLength(filePath);
    final raf = await _openFile(filePath);

    try {
      var offset = 0;
      if (verifyServerOffset) {
        final observed = await _querySessionStatus(session, totalSize);
        final finalFile = observed.finalFile;
        if (finalFile != null) return finalFile;
        offset = observed.uploaded;
      }

      if (offset > totalSize) {
        throw _remoteAmbiguity(
            '服务端断点偏移 $offset 超过本地文件长度 $totalSize', false);
      }
      _notifyResumeProgress(
          session, offset, totalSize, onProgress, onResumeProgress);

      var finalStatusPolls = 0;
      while (true) {
        if (offset < totalSize) {
          finalStatusPolls = 0;
          final chunkSize = _validatedChunkSize(session.chunkSize);
          final chunkLen = min(chunkSize, totalSize - offset);
          final chunk = await _readChunk(raf, offset, chunkLen);

          _ChunkResult? chunkResult;
          AppError? lastError;
          for (var attempt = 1; attempt <= chunkRetries; attempt++) {
            try {
              chunkResult =
                  await _putChunk(session, chunk, offset, chunkLen, totalSize);
              break;
            } on AppError catch (e) {
              // 308/状态查询可能已轮换 Location；即使随后解析失败，也先持久化
              // 当前会话 URL，避免恢复时回到已失效的旧地址。
              _notifyResumeProgress(
                  session, offset, totalSize, onProgress, onResumeProgress);
              lastError = e;
              final retryLocally =
                  attempt < chunkRetries && _shouldRetryChunkLocally(e);
              if (retryLocally) {
                await Future<void>.delayed(chunkRetryDelayUnit * attempt);
              } else {
                break;
              }
            }
          }
          final result = chunkResult ??
              (throw lastError ?? AppError.generic('分片上传失败'));
          final finalFile = result.finalFile;
          if (finalFile != null) return finalFile;
          if (result.uploaded > totalSize) {
            throw _remoteAmbiguity(
                '服务端确认偏移 ${result.uploaded} 超过本地文件长度 $totalSize', false);
          }
          if (result.uploaded == offset) {
            throw _remoteAmbiguity(
                '服务端状态查询未确认当前分片，停止本地偏移推进', false);
          }
          offset = result.uploaded;
          _notifyResumeProgress(
              session, offset, totalSize, onProgress, onResumeProgress);
          continue;
        }

        // 数据范围已全部确认，但只有最终 200 + 完整 File 才能结算完成。
        finalStatusPolls += 1;
        try {
          final result = await _querySessionStatus(session, totalSize);
          final finalFile = result.finalFile;
          if (finalFile != null) return finalFile;
          if (result.uploaded < totalSize) {
            offset = result.uploaded;
            _notifyResumeProgress(
                session, offset, totalSize, onProgress, onResumeProgress);
            continue;
          }
          if (finalStatusPolls < finalStatusMaxPolls) {
            final waitMs = (result.processTimeMs ??
                    finalPollInterval.inMilliseconds)
                .clamp(250, finalPollInterval.inMilliseconds);
            await Future<void>.delayed(Duration(milliseconds: waitMs));
            continue;
          }
        } on AppError catch (e) {
          if (_isRemoteAmbiguity(e)) {
            _notifyResumeProgress(
                session, offset, totalSize, onProgress, onResumeProgress);
            rethrow;
          }
          if (_authAlreadyReplayed(e)) rethrow;
          if (finalStatusPolls < finalStatusMaxPolls) {
            _notifyResumeProgress(
                session, offset, totalSize, onProgress, onResumeProgress);
            AppLogger.w('最终上传状态查询失败，继续查询同一会话: $e');
            await Future<void>.delayed(finalPollErrorDelay);
            continue;
          }
          _notifyResumeProgress(
              session, offset, totalSize, onProgress, onResumeProgress);
          AppLogger.w('最终上传状态仍不确定，交由任务层远端核验: $e');
          throw _remoteAmbiguity(
              '最终上传状态查询失败：$e', _authAlreadyReplayed(e));
        }

        throw _remoteAmbiguity('所有字节已由服务端确认，但未返回最终文件元数据', false);
      }
    } finally {
      await raf.close();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 单分片提交与状态查询（对齐 Rust upload_api/chunk.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// PUT 单个分片。对请求阶段不确定、5xx 或成功响应无法解析的情况，不按本地
  /// 长度猜偏移，先查询同一会话的服务端状态。
  Future<_ChunkResult> _putChunk(
    ResumeSession session,
    Uint8List chunk,
    int offset,
    int chunkLen,
    int totalSize,
  ) async {
    final chunkEndExclusive = offset + chunkLen;
    if (chunkLen == 0 ||
        chunk.length != chunkLen ||
        chunkEndExclusive > totalSize) {
      throw AppError.generic('非法上传分片边界');
    }
    final url = _sessionRequestUrl(session);
    final end = offset + chunkLen - 1;
    final contentRange = 'bytes $offset-$end/$totalSize';

    final ({Response<String> response, bool authReplayed}) sent;
    try {
      sent = await _client.requestRawAuthed<String>(
        'PUT',
        url,
        data: chunk,
        headers: {
          'Content-Range': contentRange,
          'Content-Length': chunk.length.toString(),
          'Content-Type': 'application/octet-stream',
        },
        responseType: ResponseType.plain,
        semantics: RequestSemantics.write,
        sendTimeout: const Duration(seconds: 120),
      );
    } on AppError catch (e) {
      if (e is DriveApiError && e.transportKind != null) {
        return _reconcileUncertainChunk(session, totalSize, offset, e);
      }
      rethrow;
    }

    final response = sent.response;
    final authReplayed = sent.authReplayed;
    _updateSessionLocation(session, response);
    final status = response.statusCode ?? 0;

    if (status == 308) {
      final body = _decodeBodyMap(response.data, '308 分片响应',
          isWrite: true, authReplayed: authReplayed, ambiguity: true);
      _updateSessionChunkSize(session, body);
      final uploaded = _parseConfirmedOffset(body, totalSize);
      if (uploaded <= offset) {
        throw _remoteAmbiguity(
            '308 未确认当前分片：本地起点 $offset，服务端确认偏移 $uploaded',
            authReplayed);
      }
      return _incompleteResult(body, uploaded);
    }

    if (status < 200 || status >= 300) {
      final shouldQuery = status >= 500 || status == 408;
      final original = _uploadResponseError(
          response, RequestSemantics.write, authReplayed,
          sessionSensitive: true);
      if (shouldQuery) {
        return _reconcileUncertainChunk(session, totalSize, offset, original);
      }
      throw original;
    }

    final Map<String, dynamic> body;
    try {
      body = _decodeBodyMap(response.data, '分片成功响应',
          isWrite: true, authReplayed: authReplayed, ambiguity: true);
    } on AppError catch (e) {
      return _reconcileUncertainChunk(session, totalSize, offset, e);
    }
    final completed = _completeUploadFile(body, totalSize);
    if (completed != null) {
      return _ChunkResult(
          uploaded: totalSize, isFinal: true, finalFile: completed);
    }

    // 兼容旧接口的中间 2xx `size`，但只信任服务端显式数值，绝不本地相加。
    final uploaded = _asNonNegativeInt(body['size']);
    if (uploaded != null && uploaded <= totalSize && uploaded > offset) {
      return _ChunkResult(
        uploaded: uploaded,
        processTimeMs: _asNonNegativeInt(body['processTime']),
      );
    }

    final original = _remoteAmbiguity(
        '分片返回 2xx，但既无完整 File 也无有效服务端确认偏移', authReplayed);
    return _reconcileUncertainChunk(session, totalSize, offset, original);
  }

  /// 通过同一会话查询收敛不确定写入，未推进时保留原错误。
  Future<_ChunkResult> _reconcileUncertainChunk(
    ResumeSession session,
    int totalSize,
    int previousOffset,
    AppError original,
  ) async {
    try {
      final result = await _querySessionStatus(session, totalSize);
      if (result.isFinal || result.uploaded != previousOffset) return result;
      throw original;
    } on AppError catch (queryError) {
      if (_isRemoteAmbiguity(queryError)) rethrow;
      AppLogger.w('分片结果不确定且会话查询失败，保留原始写入歧义: $queryError');
      throw original;
    }
  }

  /// 对同一 session 发零长度状态查询；查询本身是只读语义，401 也只重放一次。
  Future<_ChunkResult> _querySessionStatus(
    ResumeSession session,
    int totalSize,
  ) async {
    final url = _sessionRequestUrl(session);
    final sent = await _client.requestRawAuthed<String>(
      'PUT',
      url,
      data: Uint8List(0),
      headers: {
        'Content-Range': 'bytes */$totalSize',
        'Content-Length': '0',
      },
      responseType: ResponseType.plain,
      semantics: RequestSemantics.read,
      sendTimeout: const Duration(seconds: 120),
    );
    final response = sent.response;
    final authReplayed = sent.authReplayed;
    _updateSessionLocation(session, response);
    final status = response.statusCode ?? 0;

    if (status == 308) {
      final body = _decodeBodyMap(response.data, '上传状态 308 响应',
          authReplayed: authReplayed, ambiguity: true);
      _updateSessionChunkSize(session, body);
      final uploaded = _parseConfirmedOffset(body, totalSize);
      return _incompleteResult(body, uploaded);
    }

    if (status < 200 || status >= 300) {
      throw _uploadResponseError(response, RequestSemantics.read, authReplayed,
          sessionSensitive: true);
    }

    final body = _decodeBodyMap(response.data, '上传状态成功响应',
        authReplayed: authReplayed, ambiguity: true);
    final completed = _completeUploadFile(body, totalSize);
    if (completed != null) {
      return _ChunkResult(
          uploaded: totalSize, isFinal: true, finalFile: completed);
    }
    final uploaded = _asNonNegativeInt(body['size']);
    if (uploaded != null && uploaded <= totalSize) {
      return _ChunkResult(
        uploaded: uploaded,
        processTimeMs: _asNonNegativeInt(body['processTime']),
      );
    }
    throw _remoteAmbiguity(
        '上传状态返回 2xx，但缺少完整 File 或有效服务端确认偏移', authReplayed);
  }

  /// 根据服务端确认偏移构造未完成结果。
  _ChunkResult _incompleteResult(Map<String, dynamic> body, int uploaded) {
    return _ChunkResult(
      uploaded: uploaded,
      processTimeMs: _asNonNegativeInt(body['processTime']),
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 会话协议（对齐 Rust upload_api/protocol.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 返回服务端会话地址；缺少两种合法身份时按远端不确定失败。
  String _sessionRequestUrl(ResumeSession session) {
    if (session.sessionUrl.isNotEmpty) return session.sessionUrl;
    if (session.serverId.isNotEmpty && session.uploadId.isNotEmpty) {
      return '$_uploadBase/files/${session.serverId}?uploadId=${session.uploadId}';
    }
    throw _remoteAmbiguity('断点上传会话缺少 Location，也缺少完整 serverId/uploadId', false);
  }

  /// 从响应中的非空 Location 更新会话地址。
  void _updateSessionLocation(
      ResumeSession session, Response<dynamic> response) {
    final location = response.headers.value('Location');
    if (location == null) return;
    if (location.trim().isEmpty) {
      throw _remoteAmbiguity('上传响应返回空 Location', false);
    }
    session.sessionUrl = location;
  }

  /// 接受并校验服务端建议的分片大小。
  void _updateSessionChunkSize(ResumeSession session, Map<String, dynamic> body) {
    final sliceSize = _asNonNegativeInt(body['sliceSize']);
    if (sliceSize != null) {
      _validatedChunkSize(sliceSize);
      session.chunkSize = sliceSize;
    }
  }

  /// 只接受从 0 开始、连续、无重叠且不越界的已接收范围。空数组明确表示 0；
  /// 缺字段、hole、重叠或格式异常都不能回退成本地 `offset + chunk_len`。
  int _parseConfirmedOffset(Map<String, dynamic> body, int totalSize) {
    final ranges = body['rangeList'];
    if (ranges is! List) {
      throw _remoteAmbiguity('308 响应缺少 rangeList', false);
    }
    if (ranges.isEmpty) return 0;

    var expectedStart = 0;
    for (final range in ranges) {
      if (range is! String) {
        throw _remoteAmbiguity('rangeList 含非字符串元素', false);
      }
      final dash = range.indexOf('-');
      if (dash < 0 || range.substring(dash + 1).contains('-')) {
        throw _remoteAmbiguity('非法上传范围：$range', false);
      }
      final start = _parseU64Strict(range.substring(0, dash));
      if (start == null) {
        throw _remoteAmbiguity('非法上传范围起点：$range', false);
      }
      final end = _parseU64Strict(range.substring(dash + 1));
      if (end == null) {
        throw _remoteAmbiguity('非法上传范围终点：$range', false);
      }
      if (start != expectedStart || end < start || end >= totalSize) {
        throw _remoteAmbiguity(
            '上传范围不连续或越界：$range，期望起点 $expectedStart，总长度 $totalSize',
            false);
      }
      expectedStart = end + 1;
    }
    return expectedStart;
  }

  /// 归一化并校验服务端分片大小，越界时拒绝分配缓冲区。
  int _validatedChunkSize(int chunkSize) {
    final size = chunkSize == 0 ? defaultChunkSize : chunkSize;
    if (size < minChunkSize || size > maxChunkSize) {
      throw _remoteAmbiguity(
          '服务端分片大小 $size 不在允许范围 $minChunkSize..=$maxChunkSize', false);
    }
    return size;
  }

  /// 发布比例及可持久化会话偏移，不自行推进偏移。
  void _notifyResumeProgress(
    ResumeSession session,
    int offset,
    int totalSize,
    UploadProgressFn? onProgress,
    ResumeProgressFn? onResumeProgress,
  ) {
    if (onProgress != null) {
      final ratio = totalSize == 0 ? 1.0 : offset / totalSize;
      onProgress(ratio.clamp(0.0, 1.0));
    }
    onResumeProgress?.call(
        session.serverId, session.uploadId, offset, session.sessionUrl);
  }

  /// 构造「写请求可能已到达服务端」的恢复型错误（对齐 Rust remote_ambiguity）。
  AppError _remoteAmbiguity(String cause, bool authAlreadyReplayed) {
    return AppError.driveTransportWithSubmission(
      DriveTransportKind.decode,
      requestMayHaveReachedServer: true,
      authAlreadyReplayed: authAlreadyReplayed,
      cause: cause,
    );
  }

  /// 判断错误是否要求沿同一会话远端核验而非重新新建。
  bool _isRemoteAmbiguity(AppError error) {
    if (error is DriveApiError) {
      if (error.requestMayHaveReachedServer && error.transportKind != null) {
        return true;
      }
    }
    return error.errorCode == 'upload_session_expired';
  }

  /// 只在请求明确未到服务端的连接失败上做短暂本地重试。任何可能已提交的
  /// 写入都必须交回持久化恢复策略。
  bool _shouldRetryChunkLocally(AppError error) {
    return error is DriveApiError &&
        error.statusCode == null &&
        error.transportKind == DriveTransportKind.connect &&
        !error.requestMayHaveReachedServer;
  }

  /// 判断错误是否已消耗唯一一次认证刷新重放。
  bool _authAlreadyReplayed(AppError error) {
    return error is DriveApiError && error.authAlreadyReplayed;
  }

  /// 读取上传错误响应，并将失效会话与普通 HTTP 失败区分。
  AppError _uploadResponseError(
    Response<dynamic> response,
    RequestSemantics semantics,
    bool authReplayed, {
    required bool sessionSensitive,
  }) {
    final status = response.statusCode ?? 0;
    final body = AppError.responseBodyString(response);
    final upper = body.toUpperCase();
    if (sessionSensitive &&
        (status == 404 ||
            status == 410 ||
            upper.contains('CONTENT_NOT_FOUND') ||
            upper.contains('UPLOAD_ID_NOT_FOUND'))) {
      return AppError.driveUploadSessionExpired(status,
          authAlreadyReplayed: authReplayed);
    }
    return httpErrorFromResponse(response, semantics, authReplayed);
  }

  /// 仅在文件身份、长度及可选名称完整匹配时接受最终结果。
  DriveFile? _completeUploadFile(
    Object? body,
    int expectedSize, [
    String? expectedName,
  ]) {
    if (body is! Map<String, dynamic>) return null;
    final file = DriveFile.tryFromJson(body);
    if (file == null) return null;
    final sizeMatches = file.size == expectedSize;
    final nameMatches = file.name.trim().isNotEmpty &&
        (expectedName == null || file.name == expectedName);
    if (file.id.trim().isNotEmpty && sizeMatches && nameMatches) {
      return file;
    }
    return null;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 通用辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 构造 metadata JSON（multipart 路径用普通 JSON，容忍 UTF-8，不需 asciiJsonEncode）。
  String _buildMetadataJson(String fileName, String? parentId) {
    return jsonEncode({
      'fileName': fileName,
      if (parentId != null && parentId.isNotEmpty) 'parentFolder': [parentId],
    });
  }

  /// 构造 multipart/related body（对齐 Rust build_multipart_related）。
  Uint8List _buildMultipartRelated(
    String boundary,
    List<int> metadata,
    List<int> fileBytes,
  ) {
    final builder = BytesBuilder();
    builder.add(utf8.encode('--$boundary\r\n'));
    builder.add(
        utf8.encode('Content-Type: application/json; charset=UTF-8\r\n\r\n'));
    builder.add(metadata);
    builder.add(utf8.encode('\r\n'));
    builder.add(utf8.encode('--$boundary\r\n'));
    builder.add(utf8.encode('Content-Type: application/octet-stream\r\n\r\n'));
    builder.add(fileBytes);
    builder.add(utf8.encode('\r\n--$boundary--\r\n'));
    return builder.toBytes();
  }

  /// 生成 multipart boundary（对齐 Rust `hwcloud_{timestamp_micros}`）。
  String _newBoundary() =>
      'hwcloud_${DateTime.now().microsecondsSinceEpoch}';

  /// 读取本地长度并查询云盘剩余配额；任一步失败都拒绝上传。
  Future<void> _ensureCapacityFor(String filePath) async {
    final size = await _fileLength(filePath);
    final result = await AboutService(_client).ensureCapacity(size);
    if (result.isErr) throw (result as Err<void>).error;
  }

  /// 读取文件长度（元数据失败 → Generic）。
  Future<int> _fileLength(String filePath) async {
    try {
      return await File(filePath).length();
    } catch (e) {
      throw AppError.generic('读取文件元数据失败：$e');
    }
  }

  /// 读取整个文件（multipart 路径，≤ 20MiB）。
  Future<Uint8List> _readFileBytes(String filePath) async {
    try {
      return await File(filePath).readAsBytes();
    } catch (e) {
      throw AppError.generic('读取文件失败：$e');
    }
  }

  /// 打开文件供分片读取。
  Future<RandomAccessFile> _openFile(String filePath) async {
    try {
      return await File(filePath).open();
    } catch (e) {
      throw AppError.generic('打开文件失败：$e');
    }
  }

  /// 读取指定偏移的分片（短读视为失败，对齐 Rust read_exact）。
  Future<Uint8List> _readChunk(
      RandomAccessFile raf, int offset, int chunkLen) async {
    try {
      await raf.setPosition(offset);
    } catch (e) {
      throw AppError.generic('文件定位失败：$e');
    }
    final bytes = await raf.read(chunkLen);
    if (bytes.length != chunkLen) {
      throw AppError.generic('读取分片失败：期望 $chunkLen 字节，实际 ${bytes.length}');
    }
    return bytes;
  }

  /// 提取文件名（末段路径；无则回退 "file"，对齐 Rust file_name 处理）。
  String _fileName(String filePath) {
    final normalized = filePath.replaceAll('\\', '/');
    final name = normalized.split('/').last;
    return name.isEmpty ? 'file' : name;
  }

  /// 解码 JSON 响应体为对象；失败按写歧义或解码错误处理。
  Map<String, dynamic> _decodeBodyMap(
    String? raw,
    String ctx, {
    bool isWrite = false,
    bool authReplayed = false,
    bool ambiguity = false,
  }) {
    final Object? value;
    try {
      value = jsonDecode(raw ?? '');
    } catch (e) {
      if (ambiguity) {
        throw _remoteAmbiguity('$ctx无法解析：$e', authReplayed);
      }
      throw AppError.driveTransportWithSubmission(
        DriveTransportKind.decode,
        requestMayHaveReachedServer: isWrite,
        authAlreadyReplayed: authReplayed,
        cause: '解析$ctx失败：$e',
      );
    }
    if (value is! Map<String, dynamic>) {
      if (ambiguity) {
        throw _remoteAmbiguity('$ctx不是 JSON 对象', authReplayed);
      }
      throw AppError.driveTransportWithSubmission(
        DriveTransportKind.decode,
        requestMayHaveReachedServer: isWrite,
        authAlreadyReplayed: authReplayed,
        cause: '解析$ctx失败：响应顶层不是对象',
      );
    }
    return value;
  }

  /// 按字段顺序取首个字符串值。
  String? _firstString(Map<String, dynamic> json, List<String> keys) {
    for (final k in keys) {
      final v = json[k];
      if (v is String) return v;
    }
    return null;
  }

  /// 读取非负整数（对齐 Rust `Value::as_u64`：仅非负整数）。
  int? _asNonNegativeInt(Object? v) {
    if (v is int && v >= 0) return v;
    return null;
  }

  /// 严格解析无符号十进制整数（拒绝符号/空白，对齐 Rust u64 parse）。
  int? _parseU64Strict(String s) {
    if (s.isEmpty || !RegExp(r'^\d+$').hasMatch(s)) return null;
    return int.tryParse(s);
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

/// 可持久化并向服务端重新核验的断点上传会话（对齐 Rust `ResumeSession`）。
class ResumeSession {
  /// 华为 resume 上传会话标识（v2 兼容；新 API 可为空）
  String serverId;

  /// 华为 uploadId（v2 兼容；新 API 可为空）
  String uploadId;

  /// init 响应 `Location` 头给出的会话 URL（断点续传唯一 token）。
  ///
  /// 非空时分片 PUT 直接用它，不再用 serverId/uploadId 拼接。
  String sessionUrl;

  /// API 建议的分片大小（init 响应 body `sliceSize`），0 表示用默认值
  int chunkSize;

  /// 本地持久化的续传偏移提示。恢复时不会直接信任该值，而是先查询同一会话
  /// 的 `rangeList`；新建会话时为 0。
  final int startOffset;

  ResumeSession({
    this.serverId = '',
    this.uploadId = '',
    this.sessionUrl = '',
    this.chunkSize = 0,
    this.startOffset = 0,
  });
}

/// 单次分片请求或会话状态查询的服务端确认结果。
class _ChunkResult {
  /// 仅来自服务端 `rangeList`/`size` 的确认偏移；禁止用本地分片长度推算
  final int uploaded;

  /// 是否为最终响应（含完整文件元数据）
  final bool isFinal;

  /// 最终响应的完整文件元数据
  final DriveFile? finalFile;

  /// 服务端建议在再次查询前等待的毫秒数
  final int? processTimeMs;

  const _ChunkResult({
    required this.uploaded,
    this.isFinal = false,
    this.finalFile,
    this.processTimeMs,
  });
}
