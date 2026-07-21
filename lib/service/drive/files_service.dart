import 'dart:convert';

import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/drive/ascii_json.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';
import 'package:petal_link/service/drive/drive_http.dart';

/// URL 编码（query 参数与 path segment 共用）。
///
/// 严格对齐 Rust `files_api::urlencoding`（percent_encoding 自定义集）：
/// 仅保留 RFC 3986 unreserved（A-Za-z0-9-_.~），其余字节一律按 UTF-8
/// percent 编码（大写 hex，空格为 %20）。
///
/// 注意不能用 `Uri.encodeComponent`（它保留 `!'()*` 不编码，与 Rust 不一致），
/// 也不能用 `Uri.encodeQueryComponent`（空格编码为 `+`）。
String urlEncoding(String s) {
  final buf = StringBuffer();
  for (final b in utf8.encode(s)) {
    final isUnreserved = (b >= 0x41 && b <= 0x5A) || // A-Z
        (b >= 0x61 && b <= 0x7A) || // a-z
        (b >= 0x30 && b <= 0x39) || // 0-9
        b == 0x2D || // -
        b == 0x5F || // _
        b == 0x2E || // .
        b == 0x7E; // ~
    if (isUnreserved) {
      buf.writeCharCode(b);
    } else {
      buf.write('%');
      buf.write(b.toRadixString(16).padLeft(2, '0').toUpperCase());
    }
  }
  return buf.toString();
}

/// 华为文件夹 mimeType（对齐 Rust `FOLDER_MIME_TYPE`）
const String folderMimeType = 'application/vnd.huawei-apps.folder';

/// 文件 API 服务 —— 华为 Drive Files API 封装。
///
/// 严格对齐 Rust 原版 `src/drive/files_api/`（read / request / response / write）：
///
/// - **parentFolder 查询语法**：不用 parentFolder 参数，而用
///   `queryParam='<id>' in parentFolder`（单引号包裹，根目录用 `'root'`）
/// - **asciiJsonEncode**：写操作 application/json 请求体必须 ASCII-only，
///   否则中文名报 400 `21004002 fileName can not be blank`
/// - **严格协议解析**：list/get/search 响应逐字段校验，schema 歧义直接失败，
///   绝不把坏页当可信空页
/// - **写后核验**：写操作成功合同是 `200 + File`；核验同一 fileId 及目标
///   name/唯一 parent；非幂等新建先在父目录查重，响应不确定时按
///   `parentFolder + fileName` 唯一核验收敛
/// - **软删除**：`PATCH {recycled: true}`（`DELETE` 是永久删除）；
///   响应不确定时 `GET` 404 或 `recycled=true` 才算已删除
class FilesService {
  final MateHttpClient _client;

  /// listAll 的客户端分页上限（对齐 Rust PaginationPolicy，生产 1000）。
  ///
  /// 华为只定义单页大小上限，没有定义目录总页数。客户端仍需要有限上限来
  /// 避免服务端 cursor 循环或异常数据导致永久索引；达到上限且仍有下一页时
  /// 必须失败，不能返回部分树。
  final int _maxPages;

  /// 生产单页大小（华为官方上限）
  static const int productionPageSize = 100;

  /// 生产分页上限
  static const int productionMaxPages = 1000;

  FilesService(this._client, {int maxPages = productionMaxPages})
      : _maxPages = maxPages {
    if (maxPages <= 0) {
      throw const ConfigError(message: 'Files 分页上限必须大于 0');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 读取（对齐 Rust files_api/read.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 列举目录内容（单页）。
  ///
  /// `GET /files?fields=*&pageSize={n}&queryParam='<id>' in parentFolder[&cursor=]`
  /// parentId 为 null/空 时使用 `root`。对齐 Rust `FilesApi::list`。
  Future<AppResult<FileListResult>> list({
    String? parentId,
    String? cursor,
    int pageSize = productionPageSize,
  }) {
    return _guard(
      () => _list(parentId: parentId, cursor: cursor, pageSize: pageSize),
      'list',
    );
  }

  /// 连续 GET 目录全部内容（自动翻页）。
  ///
  /// 固定 pageSize=100；空的非终止页仍按 nextCursor 继续；重复 cursor 或
  /// 超过分页上限仍有下一页均失败（对齐 Rust `FilesApi::list_all`）。
  Future<AppResult<List<DriveFile>>> listAll({String? parentId}) {
    return _guard(() => _listAll(parentId: parentId), 'listAll');
  }

  /// 获取单个文件元数据。`GET /files/{id}?fields=*`（对齐 Rust `FilesApi::get`）。
  Future<AppResult<DriveFile>> get(String id) {
    return _guard(() => _get(id), 'get');
  }

  /// 搜索文件（按名称关键词）。
  ///
  /// `GET /files?fields=*&pageSize={n}&queryParam=fileName contains '<kw>'`
  /// 可叠加 `and '<parentId>' in parentFolder`；整段 query 只编码一次。
  /// 官方未定义单引号/反斜线转义规则，含这两类字符的输入 fail closed
  /// （对齐 Rust `FilesApi::search`）。
  Future<AppResult<FileListResult>> search(
    String keyword, {
    String? parentId,
    int pageSize = productionPageSize,
  }) {
    return _guard(
      () => _search(keyword, parentId: parentId, pageSize: pageSize),
      'search',
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 写入（对齐 Rust files_api/write.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 创建文件夹。
  ///
  /// 非幂等 POST：先在目标父目录内查重（唯一同名目录直接返回）；写请求失败
  /// 后再次按 `parentFolder + fileName` 唯一核验。唯一匹配视为已提交，零匹配
  /// 把原错误交还调用方，多匹配或核验失败拒绝再次 POST
  /// （对齐 Rust `FilesApi::create_folder`）。
  Future<AppResult<DriveFile>> createFolder(String name, {String? parentId}) {
    return _guard(() => _createFolder(name, parentId), 'createFolder');
  }

  /// 更新文件（重命名/移动/改描述）。
  ///
  /// 移动时先 GET 当前唯一 parent 构造成对 `addParentFolder/removeParentFolder`
  /// 参数；已在目标位置时不重复发送移动 PATCH（fileId 级幂等）。
  /// 对齐 Rust `FilesApi::update`。
  Future<AppResult<DriveFile>> update(
    String id, {
    String? newName,
    String? newParentFolder,
    String? description,
  }) {
    return _guard(
      () => _update(id,
          newName: newName,
          newParentFolder: newParentFolder,
          description: description),
      'update',
    );
  }

  /// 重命名并核验返回的 File 身份与最终名称（对齐 Rust `rename_file`）。
  Future<AppResult<DriveFile>> rename(String id, String newName) {
    return update(id, newName: newName);
  }

  /// 使用官方成对 parent query 参数移动文件。
  ///
  /// 调用方已持有可信旧 parent 时可直接使用，避免额外 GET
  /// （对齐 Rust `move_file`）。
  Future<AppResult<DriveFile>> moveFile(
    String id,
    String oldParentFolder,
    String newParentFolder,
  ) {
    return _guard(
      () => _moveFile(id, oldParentFolder, newParentFolder),
      'move',
    );
  }

  /// 删除文件（软删除，移入回收站"最近删除"）。
  ///
  /// **重要**：华为 `DELETE /drive/v1/files/{id}` 是**永久删除**；软删除必须
  /// 用 `PATCH {recycled: true}`。对齐 Rust `FilesApi::delete`。
  Future<AppResult<void>> delete(String id) {
    return _guard(() async {
      await _deleteVerified(id);
    }, 'delete');
  }

  /// 软删除并返回已经核验的 File 响应。
  ///
  /// 成功合同：`200 + File.id == 请求 id + recycled=true`
  /// （对齐 Rust `delete_verified`）。
  Future<AppResult<DriveFile>> deleteVerified(String id) {
    return _guard(() => _deleteVerified(id), 'delete');
  }

  /// 通过稳定 fileId 核验不确定的删除结果。
  ///
  /// GET 404 → true；200 且 `recycled` 为明确布尔值 → 返回该值
  /// （对齐 Rust `verify_deleted`）。
  Future<AppResult<bool>> verifyDeleted(String id) {
    return _guard(() => _verifyDeleted(id), 'verify delete');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 读取实现
  // ═══════════════════════════════════════════════════════════════════

  Future<FileListResult> _list({
    String? parentId,
    String? cursor,
    required int pageSize,
  }) async {
    _validatePageSize(pageSize);
    final folderToken =
        (parentId != null && parentId.isNotEmpty) ? parentId : 'root';
    _validateQueryLiteral(folderToken, 'parentFolder');
    final queryParam = "'$folderToken' in parentFolder";
    final url = StringBuffer(
      '$driveApiBase/files?fields=*&pageSize=$pageSize'
      '&queryParam=${urlEncoding(queryParam)}',
    );
    if (cursor != null && cursor.isNotEmpty) {
      url.write('&cursor=${urlEncoding(cursor)}');
    }

    final body = _parseJsonObject((await _client.get<String>(url.toString())).unwrap(), 'list');
    return _parseFileListPage(body, 'list');
  }

  Future<List<DriveFile>> _listAll({String? parentId}) async {
    final all = <DriveFile>[];
    String? cursor;
    final seenCursors = <String>{};

    for (var pageIndex = 0; pageIndex < _maxPages; pageIndex++) {
      final result = await _list(
          parentId: parentId, cursor: cursor, pageSize: productionPageSize);
      all.addAll(result.files);

      final nextCursor = result.nextCursor;
      if (nextCursor == null) return all;
      if (!seenCursors.add(nextCursor)) {
        throw _protocolError('listAll', 'nextCursor 重复或形成循环');
      }
      if (pageIndex + 1 >= _maxPages) {
        throw _protocolError(
            'listAll', '达到分页上限时服务端仍返回 nextCursor，结果不完整');
      }
      cursor = nextCursor;
    }

    throw _protocolError('listAll', '分页策略无可用页数');
  }

  Future<DriveFile> _get(String id) async {
    final url = '$driveApiBase${_filePath(id)}?fields=*';
    final body = _parseJsonObject((await _client.get<String>(url)).unwrap(), 'get');
    return _parseDriveFileStrict(body, 'get');
  }

  Future<FileListResult> _search(
    String keyword, {
    String? parentId,
    required int pageSize,
  }) async {
    _validatePageSize(pageSize);
    _validateQueryLiteral(keyword, '搜索关键词');
    var query = "fileName contains '$keyword'";
    if (parentId != null && parentId.isNotEmpty) {
      _validateQueryLiteral(parentId, 'parentFolder');
      query = "$query and '$parentId' in parentFolder";
    }
    final url = '$driveApiBase/files?fields=*&pageSize=$pageSize'
        '&queryParam=${urlEncoding(query)}';

    final body = _parseJsonObject((await _client.get<String>(url)).unwrap(), 'search');
    return _parseFileListPage(body, 'search');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 写入实现
  // ═══════════════════════════════════════════════════════════════════

  Future<DriveFile> _createFolder(String name, String? parentId) async {
    if (name.trim().isEmpty) {
      throw AppError.generic('文件夹名称不能为空');
    }
    final expectedParent = _canonicalParentId(parentId);

    final existing = await _findUniqueFolderInParent(name, expectedParent);
    if (existing != null) {
      AppLogger.i('创建文件夹前核验命中唯一同名目录，跳过 POST: ${existing.id}');
      return existing;
    }

    try {
      return await _createFolderOnce(name, parentId, expectedParent);
    } on AppError catch (submitError) {
      final DriveFile? verified;
      try {
        verified = await _findUniqueFolderInParent(name, expectedParent);
      } on AppError catch (verificationError) {
        throw AppError.generic(
            '创建文件夹结果不确定：$submitError；父目录唯一核验失败：$verificationError');
      }
      if (verified != null) {
        AppLogger.i('创建文件夹响应不确定，父目录唯一核验确认已提交: ${verified.id}');
        return verified;
      }
      // 只有明确的零匹配才把原错误交还调用方，允许稍后显式重试。
      rethrow;
    }
  }

  /// 提交一次非幂等目录创建，并严格核验 200 File 响应。
  Future<DriveFile> _createFolderOnce(
    String name,
    String? parentId,
    String expectedParent,
  ) async {
    final body = _buildCreateFolderBody(name, parentId);
    final encoded = asciiJsonEncode(body);
    final sent = await _client.requestRawAuthed<String>(
      'POST',
      '$driveApiBase/files?fields=*',
      data: encoded,
      headers: const {'Content-Type': 'application/json'},
      responseType: ResponseType.plain,
      semantics: RequestSemantics.write,
    );
    _requireWriteOk(sent, 'createFolder');
    final bodyJson = _parseJsonObject(sent.response.data ?? '', 'createFolder',
        isWrite: true, authReplayed: sent.authReplayed);
    final file = _parseVerifiedWrittenDriveFile(
        bodyJson, 'createFolder', sent.authReplayed);
    _verifyCreatedFolder(file, name, expectedParent,
        isWrite: true, authReplayed: sent.authReplayed);
    AppLogger.i('创建文件夹成功: $name (${file.id})');
    return file;
  }

  /// 在指定父目录中查找唯一同名目录，多匹配时返回歧义错误。
  Future<DriveFile?> _findUniqueFolderInParent(
    String name,
    String expectedParent,
  ) async {
    final requestParent = expectedParent != 'root' ? expectedParent : null;
    final listed = await _listAll(parentId: requestParent);
    final matches = <DriveFile>[];
    for (final file in listed) {
      if (file.name != name || !file.isFolder) continue;
      _verifyCreatedFolder(file, name, expectedParent);
      matches.add(file);
    }
    return switch (matches.length) {
      0 => null,
      1 => matches.first,
      final count => throw AppError.generic(
          '父目录 $expectedParent 中存在 $count 个同名文件夹，创建结果有歧义'),
    };
  }

  Future<DriveFile> _deleteVerified(String id) async {
    _validateFileId(id);
    final encoded = asciiJsonEncode(const {'recycled': true});
    final sent = await _client.requestRawAuthed<String>(
      'PATCH',
      '$driveApiBase${_filePath(id)}',
      data: encoded,
      headers: const {'Content-Type': 'application/json'},
      responseType: ResponseType.plain,
      semantics: RequestSemantics.write,
    );
    _requireWriteOk(sent, 'delete');
    final bodyJson = _parseJsonObject(sent.response.data ?? '', 'delete',
        isWrite: true, authReplayed: sent.authReplayed);
    final file =
        _parseVerifiedWrittenDriveFile(bodyJson, 'delete', sent.authReplayed);
    _verifyFileId(file, id, 'delete',
        isWrite: true, authReplayed: sent.authReplayed);
    if (bodyJson['recycled'] != true) {
      throw _protocolError('delete', '响应未明确确认 recycled=true',
          isWrite: true, authReplayed: sent.authReplayed);
    }
    AppLogger.i('删除成功（软删除）: $id');
    return file;
  }

  Future<bool> _verifyDeleted(String id) async {
    _validateFileId(id);
    final url = '$driveApiBase${_filePath(id)}?fields=*';
    final result = await _client.get<String>(url);
    if (result case Err<String>(:final error)) {
      if (error.driveStatus == 404) return true;
      throw error;
    }
    final body = _parseJsonObject(result.unwrap(), 'verify delete');
    final file = _parseVerifiedWrittenDriveFile(body, 'verify delete', false);
    _verifyFileId(file, id, 'verify delete');
    return switch (body['recycled']) {
      final bool recycled => recycled,
      _ => throw _protocolError('verify delete', '响应缺少明确 recycled 布尔值'),
    };
  }

  Future<DriveFile> _update(
    String id, {
    String? newName,
    String? newParentFolder,
    String? description,
  }) async {
    _validateFileId(id);
    if (newParentFolder != null) {
      _validateFileIdValue(newParentFolder, '目标 parentFolder');
      // Files:update 移动必须同时提交旧、新 parent。先读当前 parent 也让重复调用
      // 具备 fileId 级幂等性：若响应曾丢失但移动已提交，则不再次发送移动 PATCH。
      final current = await _get(id);
      _verifyFileId(current, id, 'move preflight');
      final currentParent = _singleParent(current, 'move preflight');
      if (currentParent == newParentFolder) {
        if (newName == null && description == null) return current;
        return _updateVerified(id, newName: newName, description: description);
      }
      return _updateVerified(
        id,
        newName: newName,
        moveParents: (currentParent, newParentFolder),
        description: description,
      );
    }
    return _updateVerified(id, newName: newName, description: description);
  }

  Future<DriveFile> _moveFile(
    String id,
    String oldParentFolder,
    String newParentFolder,
  ) async {
    _validateFileId(id);
    _validateFileIdValue(oldParentFolder, '旧 parentFolder');
    _validateFileIdValue(newParentFolder, '目标 parentFolder');
    if (oldParentFolder == newParentFolder) {
      final current = await _get(id);
      _verifyFileId(current, id, 'move');
      _verifyParent(current, newParentFolder, 'move');
      return current;
    }
    return _updateVerified(
        id, moveParents: (oldParentFolder, newParentFolder));
  }

  /// 提交一次更新，并核验文件身份及请求指定的名称或父目录。
  Future<DriveFile> _updateVerified(
    String id, {
    String? newName,
    (String oldParent, String newParent)? moveParents,
    String? description,
  }) async {
    final body = <String, dynamic>{
      'fileName': ?newName,
      'description': ?description,
    };
    final encoded = asciiJsonEncode(body);
    final sent = await _client.requestRawAuthed<String>(
      'PATCH',
      '$driveApiBase${_updatePath(id, moveParents)}',
      data: encoded,
      headers: const {'Content-Type': 'application/json'},
      responseType: ResponseType.plain,
      semantics: RequestSemantics.write,
    );
    _requireWriteOk(sent, 'update');
    final bodyJson = _parseJsonObject(sent.response.data ?? '', 'update',
        isWrite: true, authReplayed: sent.authReplayed);
    final file =
        _parseVerifiedWrittenDriveFile(bodyJson, 'update', sent.authReplayed);
    _verifyFileId(file, id, 'update',
        isWrite: true, authReplayed: sent.authReplayed);
    if (newName != null && file.name != newName) {
      throw _protocolError('rename', '响应 fileName 与目标名称不一致',
          isWrite: true, authReplayed: sent.authReplayed);
    }
    if (moveParents != null) {
      _verifyParent(file, moveParents.$2, 'move',
          isWrite: true, authReplayed: sent.authReplayed);
    }
    return file;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 请求构造（对齐 Rust files_api/request.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 构造更新路径，移动时附加成对父目录参数。
  static String _updatePath(
      String id, (String oldParent, String newParent)? moveParents) {
    final buf = StringBuffer('${_filePath(id)}?fields=*');
    if (moveParents != null) {
      buf.write('&addParentFolder=${urlEncoding(moveParents.$2)}');
      buf.write('&removeParentFolder=${urlEncoding(moveParents.$1)}');
    }
    return buf.toString();
  }

  /// 将 fileId 按单一 path segment 编码为资源路径。
  static String _filePath(String id) => '/files/${urlEncoding(id)}';

  /// 构造 createFolder 请求体：mimeType 必填，root 目录省略 parentFolder。
  static Map<String, dynamic> _buildCreateFolderBody(
      String name, String? parentId) {
    return {
      'fileName': name,
      'mimeType': folderMimeType,
      if (parentId != null && parentId.isNotEmpty && parentId != 'root')
        'parentFolder': [parentId],
    };
  }

  /// 校验单页大小位于华为接口允许的 1..=100 范围。
  static void _validatePageSize(int pageSize) {
    if (pageSize < 1 || pageSize > productionPageSize) {
      throw AppError.generic('Files pageSize 必须在 1..=100 范围内');
    }
  }

  /// 拒绝官方 DSL 未定义转义规则的查询字面量。
  static void _validateQueryLiteral(String value, String field) {
    if (value.contains("'") || value.contains('\\')) {
      throw AppError.generic('$field 包含华为 queryParam 尚未定义转义规则的字符');
    }
  }

  /// 校验通用 fileId 非空。
  static void _validateFileId(String id) => _validateFileIdValue(id, 'fileId');

  /// 校验指定语义字段中的 fileId 非空。
  static void _validateFileIdValue(String id, String field) {
    if (id.trim().isEmpty) {
      throw AppError.generic('$field 不能为空');
    }
  }

  /// 将缺失或空根目录标识归一化为 `root`，并拒绝其他空白标识。
  static String _canonicalParentId(String? parentId) {
    if (parentId == null || parentId.isEmpty || parentId == 'root') {
      return 'root';
    }
    _validateFileIdValue(parentId, 'parentFolder');
    return parentId;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 响应严格解析与写后核验（对齐 Rust files_api/response.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 严格解析 Files:list/search 单页。
  ///
  /// `files` 缺失、类型错误或任一条目不完整时整页失败；`nextCursor` 只接受
  /// 缺失/null/string，空字符串按终页处理。
  FileListResult _parseFileListPage(Map<String, dynamic> object, String ctx) {
    // 与 Rust 一致：category 缺失可容忍，但出现时必须恰为 "drive#fileList"
    // （显式 JSON null 同样拒绝）
    if (object.containsKey('category')) {
      final category = object['category'];
      if (category is! String || category != 'drive#fileList') {
        throw _protocolError(ctx, 'category 不是 drive#fileList');
      }
    }

    final rawFiles = object['files'];
    if (rawFiles is! List) {
      throw _protocolError(ctx, 'files 缺失或不是数组');
    }
    final files = <DriveFile>[];
    for (var i = 0; i < rawFiles.length; i++) {
      files.add(_parseDriveFileStrict(rawFiles[i], ctx, index: i));
    }

    final rawCursor = object['nextCursor'];
    final String? nextCursor;
    if (rawCursor == null) {
      nextCursor = null;
    } else if (rawCursor is String) {
      nextCursor = rawCursor.isEmpty ? null : rawCursor;
    } else {
      throw _protocolError(ctx, 'nextCursor 必须是字符串、null 或缺失');
    }

    return FileListResult(files: files, nextCursor: nextCursor);
  }

  /// 严格校验单个 File 的身份、类型、时间及父目录字段。
  DriveFile _parseDriveFileStrict(Object? value, String ctx, {int? index}) {
    final prefix = index != null ? 'files[$index]' : 'file';
    if (value is! Map<String, dynamic>) {
      throw _protocolError(ctx, '$prefix 必须是对象');
    }

    _requireNonEmptyString(value['id'], ctx, prefix, 'id');
    final nameValue =
        value.containsKey('fileName') ? value['fileName'] : value['name'];
    _requireNonEmptyString(nameValue, ctx, prefix, 'fileName');
    _requireNonEmptyString(value['mimeType'], ctx, prefix, 'mimeType');
    if (value.containsKey('category')) {
      final category = value['category'];
      if (category is! String || category != 'drive#file') {
        throw _protocolError(ctx, '$prefix.category 不是 drive#file');
      }
    }

    _validateOptionalNonNegativeInt(value['size'], ctx, prefix, 'size');
    _validateOptionalString(value['description'], ctx, prefix, 'description');
    _validateOptionalString(
        value['thumbnailLink'], ctx, prefix, 'thumbnailLink');
    for (final field in const [
      'sha256',
      'md5',
      'md5Checksum',
      'fileSha256',
      'hash',
      'contentHash',
    ]) {
      _validateOptionalString(value[field], ctx, prefix, field);
    }
    for (final field in const ['createdTime', 'editedTime']) {
      _validateOptionalTimestamp(value[field], ctx, prefix, field);
    }
    final parentFolder = value['parentFolder'];
    if (parentFolder != null) {
      if (parentFolder is! List ||
          !parentFolder.every((e) => e is String && e.isNotEmpty)) {
        throw _protocolError(
            ctx, '$prefix.parentFolder 必须是字符串数组（元素不能为空）或 null');
      }
    }

    final file = DriveFile.tryFromJson(value);
    if (file == null) {
      throw _protocolError(ctx, '$prefix 无法构造 DriveFile');
    }
    return file;
  }

  /// 严格核验写响应：写接口使用 `fields=*`，成功结果必须是可识别、非空的
  /// Huawei File，不能只凭任意 JSON/任意 2xx 推进本地状态。
  DriveFile _parseVerifiedWrittenDriveFile(
    Map<String, dynamic> object,
    String ctx,
    bool authReplayed,
  ) {
    if (object.containsKey('category')) {
      final category = object['category'];
      if (category is! String || category != 'drive#file') {
        throw _protocolError(ctx, '响应 category 不是 drive#file',
            isWrite: true, authReplayed: authReplayed);
      }
    }
    final file = DriveFile.tryFromJson(object);
    if (file == null) {
      throw _protocolError(ctx, '响应缺少文件必填字段',
          isWrite: true, authReplayed: authReplayed);
    }
    if (file.id.trim().isEmpty ||
        file.name.trim().isEmpty ||
        (file.mimeType?.trim().isEmpty ?? true)) {
      throw _protocolError(ctx, 'File 缺少非空 id/fileName/mimeType',
          isWrite: true, authReplayed: authReplayed);
    }
    final parentFolder = object['parentFolder'];
    if (parentFolder != null) {
      if (parentFolder is! List ||
          !parentFolder.every((e) => e is String && e.isNotEmpty)) {
        throw _protocolError(ctx, 'File.parentFolder 不是非空字符串数组或 null',
            isWrite: true, authReplayed: authReplayed);
      }
    }
    return file;
  }

  /// 仅接受华为 Files 写接口的 2xx→严格 200 合同。
  void _requireWriteOk(
    ({Response<String> response, bool authReplayed}) sent,
    String ctx,
  ) {
    final status = sent.response.statusCode ?? 0;
    if (status < 200 || status >= 300) {
      throw httpErrorFromResponse(
          sent.response, RequestSemantics.write, sent.authReplayed);
    }
    if (status != 200) {
      throw _protocolError(
          ctx, 'Huawei Files 写操作成功状态必须是 200，实际为 $status',
          isWrite: true, authReplayed: sent.authReplayed);
    }
  }

  /// 核验响应文件身份与请求标识一致。
  void _verifyFileId(
    DriveFile file,
    String expectedId,
    String ctx, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    if (file.id != expectedId) {
      throw _protocolError(ctx, '响应 File.id 与请求 fileId 不一致',
          isWrite: isWrite, authReplayed: authReplayed);
    }
  }

  /// 返回唯一非空父目录；多父或缺失时拒绝继续移动。
  String _singleParent(
    DriveFile file,
    String ctx, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    final parents = file.parentFolder;
    if (parents != null && parents.length == 1 && parents.first.isNotEmpty) {
      return parents.first;
    }
    throw _protocolError(ctx, '当前只支持一个非空 parentFolder，响应无法安全用于移动',
        isWrite: isWrite, authReplayed: authReplayed);
  }

  /// 核验文件唯一父目录与预期值一致。
  void _verifyParent(
    DriveFile file,
    String expectedParent,
    String ctx, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    if (_singleParent(file, ctx,
            isWrite: isWrite, authReplayed: authReplayed) !=
        expectedParent) {
      throw _protocolError(ctx, '响应 parentFolder 与目标父目录不一致',
          isWrite: isWrite, authReplayed: authReplayed);
    }
  }

  /// 核验新建目录的身份、名称、类型与唯一父目录。
  void _verifyCreatedFolder(
    DriveFile file,
    String expectedName,
    String expectedParent, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    const ctx = 'createFolder';
    if (file.id.trim().isEmpty) {
      throw _protocolError(ctx, '响应 File.id 为空',
          isWrite: isWrite, authReplayed: authReplayed);
    }
    if (file.name != expectedName) {
      throw _protocolError(ctx, '响应 fileName 与请求名称不一致',
          isWrite: isWrite, authReplayed: authReplayed);
    }
    if (file.mimeType != folderMimeType) {
      throw _protocolError(ctx, '响应 mimeType 不是 Huawei 文件夹类型',
          isWrite: isWrite, authReplayed: authReplayed);
    }
    _verifyParent(file, expectedParent, ctx,
        isWrite: isWrite, authReplayed: authReplayed);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 字段级校验
  // ═══════════════════════════════════════════════════════════════════

  /// RFC3339 时间格式（chrono `parse_from_rfc3339` 兼容：T/t/空格分隔，Z/z/偏移）
  static final RegExp _rfc3339Pattern = RegExp(
      r'^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$');

  /// 要求字段存在且为非空字符串。
  void _requireNonEmptyString(
      Object? value, String ctx, String prefix, String field) {
    if (value is! String || value.isEmpty) {
      throw _protocolError(ctx, '$prefix.$field 缺失、类型错误或为空');
    }
  }

  /// 校验可选字段为非负整数或空值。
  void _validateOptionalNonNegativeInt(
      Object? value, String ctx, String prefix, String field) {
    if (value == null) return;
    if (value is int && value >= 0) return;
    throw _protocolError(ctx, '$prefix.$field 必须是非负整数或 null');
  }

  /// 校验可选字段为字符串或空值。
  void _validateOptionalString(
      Object? value, String ctx, String prefix, String field) {
    if (value == null || value is String) return;
    throw _protocolError(ctx, '$prefix.$field 必须是字符串或 null');
  }

  /// 校验可选字段为 RFC 3339 时间或空值。
  void _validateOptionalTimestamp(
      Object? value, String ctx, String prefix, String field) {
    if (value == null) return;
    if (value is String &&
        _rfc3339Pattern.hasMatch(value) &&
        DateTime.tryParse(value) != null) {
      return;
    }
    throw _protocolError(ctx, '$prefix.$field 必须是 RFC3339 字符串或 null');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 通用辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 解析 JSON 响应体为对象；失败按协议错误处理（对齐 Rust parse_json_response）。
  Map<String, dynamic> _parseJsonObject(
    String raw,
    String ctx, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    final Object? value;
    try {
      value = jsonDecode(raw);
    } catch (e) {
      throw _protocolError(ctx, '响应不是合法 JSON：$e',
          isWrite: isWrite, authReplayed: authReplayed);
    }
    if (value is! Map<String, dynamic>) {
      throw _protocolError(ctx, '响应顶层必须是对象',
          isWrite: isWrite, authReplayed: authReplayed);
    }
    return value;
  }

  /// 构造协议/解码错误（对齐 Rust response_decode_error：写语义标记可能已提交）。
  AppError _protocolError(
    String ctx,
    String cause, {
    bool isWrite = false,
    bool authReplayed = false,
  }) {
    return AppError.driveTransportWithSubmission(
      DriveTransportKind.decode,
      requestMayHaveReachedServer: isWrite,
      authAlreadyReplayed: authReplayed,
      cause: '解析$ctx响应失败：$cause',
    );
  }

  /// 把抛 [AppError] 的实现包装为 [AppResult]（Service 惯例：错误以 Err 返回）。
  Future<AppResult<T>> _guard<T>(
      Future<T> Function() body, String op) async {
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
