import 'dart:convert';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';
import 'package:petal_link/service/drive/files_service.dart';

/// 变更类型（对齐 Rust `ChangeKind`）。
enum ChangeKind {
  /// 官方 `deleted=true` 硬删除，或真机兼容的 trashDone/recycled 软删除
  removed,

  /// 文件新增或元数据修改
  modified,
}

/// 单条严格解析后的变更（对齐 Rust `Change`）。
///
/// 删除事件可能只有顶层 `fileId`，因此不能用伪造的空 [DriveFile] 表示
/// tombstone。
class DriveChange {
  /// 变更类型
  final ChangeKind kind;

  /// 变更对应的云端 fileId
  final String fileId;

  /// 非删除变更携带的完整文件元数据（tombstone 可为 null）
  final DriveFile? file;

  const DriveChange({required this.kind, required this.fileId, this.file});
}

/// Changes 单页（对齐 Rust `ChangesPage`）。两个 cursor 字段语义不同，禁止合并。
class ChangesPage {
  /// 本页变更
  final List<DriveChange> changes;

  /// 同一轮 catch-up 的下一页 cursor；非空时必须继续
  final String? nextCursor;

  /// 仅末页可提交为下一轮 checkpoint
  final String? newStartCursor;

  const ChangesPage({
    this.changes = const [],
    this.nextCursor,
    this.newStartCursor,
  });
}

/// 一轮完整增量变更的结果：全部变更 + 可提交的 checkpoint 游标。
typedef ChangesCatchUp = ({List<DriveChange> changes, String checkpoint});

/// 变更 API 服务 —— 华为 Drive 增量变更接口。
///
/// 严格对齐 Rust 原版 `src/drive/changes_api.rs`：
/// - `nextCursor` 只用于同一轮增量拉取的续页；末页的 `newStartCursor`
///   才是下一轮轮询可提交的 checkpoint
/// - 任一页面或变更项无法严格解释时直接失败，由调用方保留旧 checkpoint
///   并回退可信全量刷新
class ChangesService {
  final MateHttpClient _client;

  /// 单轮增量追平允许请求的最大页数（对齐 Rust DEFAULT_MAX_CHANGE_PAGES）
  final int _maxPages;

  /// 生产页数上限
  static const int productionMaxPages = 10000;

  ChangesService(this._client, {int maxPages = productionMaxPages})
      : _maxPages = maxPages {
    if (maxPages <= 0) {
      throw const ConfigError(message: 'Changes 分页上限必须大于 0');
    }
  }

  /// 获取初始游标（对齐 Rust `get_start_cursor`）。
  ///
  /// `GET /changes/getStartCursor?fields=*`（`fields=*` 官方强制）。
  /// 华为 `/changes` 强制要求 cursor，初始 cursor 必须先经本端点获取。
  Future<AppResult<String>> getStartCursor() {
    return _guard(() async {
      final body = await _getJson(
          '$driveApiBase/changes/getStartCursor?fields=*', 'getStartCursor');
      _validateCategory(body, 'category', 'drive#startCursor', 'getStartCursor');
      return _requiredNonEmptyString(body, 'startCursor', 'getStartCursor');
    }, 'getStartCursor');
  }

  /// 拉取一页增量变更（对齐 Rust `list_changes`）。cursor 为华为接口必填项。
  Future<AppResult<ChangesPage>> listChanges(String cursor) {
    return _guard(() async {
      final validCursor = _requiredCursor(cursor, 'Changes:list');
      final url = '$driveApiBase/changes?fields=*&pageSize=100'
          '&includeDeleted=true&cursor=${urlEncoding(validCursor)}';
      final body = await _getJson(url, 'changes');
      return _parseChangesPage(body);
    }, 'listChanges');
  }

  /// 拉取完整的一轮增量变更（对齐 Rust `list_all_changes`）。
  ///
  /// 空 `changes` 不代表终页；只要 `nextCursor` 非空就继续。只有无非空
  /// `nextCursor` 且存在非空 `newStartCursor` 时才成功返回 checkpoint。
  Future<AppResult<ChangesCatchUp>> listAllChanges(String startCursor) {
    return _guard(() async {
      var cursor = _requiredCursor(startCursor, 'Changes:list_all');
      final seen = <String>{cursor};
      final all = <DriveChange>[];

      for (var pageNumber = 1; pageNumber <= _maxPages; pageNumber++) {
        final page = await listChanges(cursor);
        final body = page.unwrap();
        final pageCount = body.changes.length;
        all.addAll(body.changes);

        final nextCursor = body.nextCursor;
        if (nextCursor != null) {
          if (pageNumber == _maxPages) {
            throw _protocolError(
                'Changes:list 达到页数上限 $_maxPages 时仍有 nextCursor，拒绝返回部分结果');
          }
          if (!seen.add(nextCursor)) {
            throw _protocolError('Changes:list cursor 未推进或形成循环：$nextCursor');
          }
          cursor = nextCursor;
          continue;
        }

        final finalCursor = body.newStartCursor;
        if (finalCursor == null) {
          throw _protocolError('Changes:list 终页缺少非空 newStartCursor，无法提交 checkpoint');
        }
        if ((finalCursor == cursor && pageCount > 0) ||
            (finalCursor != cursor && seen.contains(finalCursor))) {
          throw _protocolError(
              'Changes:list 已累计 ${all.length} 条变更，但 newStartCursor 未推进或形成循环：$finalCursor');
        }
        AppLogger.i('Changes:list 已完整追平（$pageNumber 页，${all.length} 条）');
        return (changes: all, checkpoint: finalCursor);
      }

      throw _protocolError('Changes:list 未能在分页上限内结束');
    }, 'listAllChanges');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 响应严格解析（对齐 Rust Change/ChangesPage::from_json）
  // ═══════════════════════════════════════════════════════════════════

  /// 严格解析单页变更及两个用途不同的 cursor。
  ChangesPage _parseChangesPage(Map<String, dynamic> object) {
    _validateCategory(object, 'category', 'drive#changeList', 'Changes:list');

    final rawChanges = object['changes'];
    if (rawChanges == null) {
      throw _protocolError('Changes:list 响应缺少 changes 数组');
    }
    if (rawChanges is! List) {
      throw _protocolError('Changes:list 的 changes 不是数组');
    }

    final changes = <DriveChange>[];
    for (var i = 0; i < rawChanges.length; i++) {
      try {
        changes.add(_parseChange(rawChanges[i]));
      } on AppError catch (e) {
        throw _protocolError('Changes:list 第 ${i + 1} 个 change 无效：$e');
      }
    }

    return ChangesPage(
      changes: changes,
      nextCursor: _optionalCursor(object, 'nextCursor'),
      newStartCursor: _optionalCursor(object, 'newStartCursor'),
    );
  }

  /// 严格解析单条 change。无法安全解释的字段或语义让整页失败。
  DriveChange _parseChange(Object? value) {
    if (value is! Map<String, dynamic>) {
      throw _protocolError('change 条目不是对象');
    }
    final object = value;

    _validateCategory(object, 'category', 'drive#change', 'change');
    _validateCategory(object, 'type', 'File', 'change');
    _validateOptionalRfc3339(object, 'time', 'change');

    final fileId = _requiredNonEmptyString(object, 'fileId', 'change');
    final deleted = _requiredBool(object, 'deleted', 'change');
    final changeType = _optionalNonEmptyString(object, 'changeType', 'change');

    final rawFile = object['file'];
    final _ParsedChangeFile? parsedFile;
    if (rawFile == null) {
      parsedFile = null;
    } else {
      parsedFile = _parseChangeFile(rawFile);
    }

    if (parsedFile != null && parsedFile.id != fileId) {
      throw _protocolError(
          'change.file.id 与 fileId 不一致：${parsedFile.id} != $fileId');
    }

    final recycled = parsedFile?.recycled ?? false;
    final softDeleted = changeType == 'trashDone' || recycled;
    final kind =
        (deleted || softDeleted) ? ChangeKind.removed : ChangeKind.modified;

    final file = parsedFile?.file;
    if (kind == ChangeKind.modified && file == null) {
      throw _protocolError('非删除 change 缺少可完整解析的 file：$fileId');
    }
    if (kind == ChangeKind.modified && file!.parentFolder?.length != 1) {
      throw _protocolError('非删除 change 必须且只能有一个 parentFolder：$fileId');
    }

    return DriveChange(kind: kind, fileId: fileId, file: file);
  }

  /// 解析 change.file；删除 tombstone 的 file 可以只含 id。
  _ParsedChangeFile _parseChangeFile(Object? value) {
    if (value is! Map<String, dynamic>) {
      throw _protocolError('change.file 不是对象');
    }
    final object = value;
    _validateCategory(object, 'category', 'drive#file', 'change.file');

    final id = _requiredNonEmptyString(object, 'id', 'change.file');

    final String? name;
    final rawFileName = object['fileName'];
    if (rawFileName is String) {
      if (rawFileName.trim().isEmpty) {
        throw _protocolError('change.file 的 fileName 不能为空');
      }
      name = rawFileName;
    } else if (rawFileName == null) {
      name = _optionalNonEmptyString(object, 'name', 'change.file');
    } else {
      throw _protocolError('change.file 的 fileName 必须是字符串或 null');
    }

    _validateOptionalStringField(object, 'mimeType', 'change.file');
    _validateOptionalStringField(object, 'description', 'change.file');
    _validateOptionalStringField(object, 'thumbnailLink', 'change.file');
    for (final field in const [
      'sha256',
      'md5',
      'md5Checksum',
      'fileSha256',
      'hash',
      'contentHash',
    ]) {
      _validateOptionalStringField(object, field, 'change.file');
    }
    _validateOptionalRfc3339(object, 'createdTime', 'change.file');
    _validateOptionalRfc3339(object, 'editedTime', 'change.file');

    final parentFolder = object['parentFolder'];
    if (parentFolder != null) {
      if (parentFolder is! List) {
        throw _protocolError('change.file 的 parentFolder 必须是数组或 null');
      }
      if (!parentFolder
          .every((e) => e is String && e.trim().isNotEmpty)) {
        throw _protocolError('change.file 的 parentFolder 必须只包含非空字符串');
      }
    }

    final size = object['size'];
    if (size != null && size is! int) {
      throw _protocolError('change.file 的 size 必须是 i64 整数或 null');
    }

    final rawRecycled = object['recycled'];
    final bool recycled;
    if (rawRecycled == null) {
      recycled = false;
    } else if (rawRecycled is bool) {
      recycled = rawRecycled;
    } else {
      throw _protocolError('change.file 的 recycled 必须是布尔值或 null');
    }

    final file = name != null ? DriveFile.tryFromJson(object) : null;
    if (name != null && file == null) {
      throw _protocolError('change.file 无法解析为 DriveFile');
    }

    return _ParsedChangeFile(id: id, recycled: recycled, file: file);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 字段级校验（对齐 Rust changes_api 各 validate/required 辅助）
  // ═══════════════════════════════════════════════════════════════════

  /// RFC3339 时间格式
  static final RegExp _rfc3339Pattern = RegExp(
      r'^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$');

  /// 要求调用游标非空，否则返回协议错误。
  static String _requiredCursor(String cursor, String operation) {
    if (cursor.trim().isEmpty) {
      throw _protocolError('$operation 缺少非空 cursor');
    }
    return cursor;
  }

  /// 解析可缺失游标，并将空字符串视为未提供。
  static String? _optionalCursor(Map<String, dynamic> object, String field) {
    return switch (object[field]) {
      null => null,
      final String cursor when cursor.trim().isEmpty => null,
      final String cursor => cursor,
      _ => throw _protocolError('Changes:list 的 $field 必须是字符串、null 或缺失'),
    };
  }

  /// 从协议对象读取必需的非空字符串字段。
  static String _requiredNonEmptyString(
      Map<String, dynamic> object, String field, String context) {
    return switch (object[field]) {
      final String value when value.trim().isNotEmpty => value,
      final String _ => throw _protocolError('$context 的 $field 不能为空'),
      null => throw _protocolError('$context 缺少 $field'),
      _ => throw _protocolError('$context 的 $field 必须是字符串'),
    };
  }

  /// 从协议对象读取可选但一旦出现就必须非空的字符串。
  static String? _optionalNonEmptyString(
      Map<String, dynamic> object, String field, String context) {
    return switch (object[field]) {
      null => null,
      final String value when value.trim().isNotEmpty => value,
      final String _ => throw _protocolError('$context 的 $field 不能为空字符串'),
      _ => throw _protocolError('$context 的 $field 必须是字符串或 null'),
    };
  }

  /// 从协议对象读取必需布尔字段。
  static bool _requiredBool(
      Map<String, dynamic> object, String field, String context) {
    return switch (object[field]) {
      final bool value => value,
      null => throw _protocolError('$context 缺少 $field'),
      _ => throw _protocolError('$context 的 $field 必须是布尔值'),
    };
  }

  /// 校验可选类别字段与官方预期值一致。
  static void _validateCategory(
      Map<String, dynamic> object, String field, String expected, String context) {
    switch (object[field]) {
      case null:
        return;
      case final String value when value == expected:
        return;
      case final String value:
        throw _protocolError('$context 的 $field 非预期：$value');
      default:
        throw _protocolError('$context 的 $field 必须是字符串或 null');
    }
  }

  /// 校验可选字段只能是字符串或空值。
  static void _validateOptionalStringField(
      Map<String, dynamic> object, String field, String context) {
    final value = object[field];
    if (value == null || value is String) return;
    throw _protocolError('$context 的 $field 必须是字符串或 null');
  }

  /// 校验可选时间字段符合 RFC 3339。
  static void _validateOptionalRfc3339(
      Map<String, dynamic> object, String field, String context) {
    final value = object[field];
    if (value == null) return;
    if (value is String) {
      if (_rfc3339Pattern.hasMatch(value) &&
          DateTime.tryParse(value) != null) {
        return;
      }
      throw _protocolError('$context 的 $field 不是 RFC3339 时间');
    }
    throw _protocolError('$context 的 $field 必须是 RFC3339 字符串或 null');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 通用辅助
  // ═══════════════════════════════════════════════════════════════════

  /// GET 并解析 JSON 对象响应。
  Future<Map<String, dynamic>> _getJson(String url, String ctx) async {
    final raw = (await _client.get<String>(url)).unwrap();
    final Object? value;
    try {
      value = jsonDecode(raw);
    } catch (_) {
      throw _protocolError('$ctx 顶层响应不是对象');
    }
    if (value is! Map<String, dynamic>) {
      throw _protocolError('$ctx 顶层响应不是对象');
    }
    return value;
  }

  /// 构造带 Changes API 上下文的协议错误（对齐 Rust protocol_error）。
  static AppError _protocolError(String message) {
    return AppError.generic('华为 Changes API 协议错误：$message');
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

/// change.file 的中间解析结果（tombstone 可只含 id）
class _ParsedChangeFile {
  final String id;
  final bool recycled;
  final DriveFile? file;

  const _ParsedChangeFile(
      {required this.id, required this.recycled, this.file});
}
