import 'dart:convert';
import 'dart:typed_data';

import 'package:dio/dio.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';
import 'package:petal_link/service/drive/drive_http.dart';

/// 缩略图 API 服务 —— 图片/视频缩略图二进制下载。
///
/// 严格对齐 Rust 原版 `src/drive/thumbnail_api.rs`：
/// `GET /thumbnails/{fileId}?form=content`，Bearer 由拦截器注入；
/// 单次请求不做 401 重放，非 2xx 归一化为结构化 [DriveApiError]。
class ThumbnailService {
  final MateHttpClient _client;

  ThumbnailService(this._client);

  /// 获取云盘文件缩略图二进制内容（对齐 Rust `ThumbnailApi::get_data_url`
  /// 的字节部分；非图片格式拒绝，避免错误页被当作图片解码）。
  Future<AppResult<Uint8List>> getThumbnail(String fileId) async {
    try {
      final response = await _client.requestRaw<Uint8List>(
        'GET',
        '$driveApiBase/thumbnails/$fileId?form=content',
        responseType: ResponseType.bytes,
      );
      final status = response.statusCode ?? 0;
      if (status < 200 || status >= 300) {
        return Err(httpErrorFromResponse(
            response, RequestSemantics.read, false));
      }
      final bytes = response.data ?? Uint8List(0);
      if (bytes.isEmpty) {
        return Err(GenericError(message: '缩略图响应为空'));
      }
      // 校验为支持的图片格式（对齐 Rust thumbnail_media_type）：
      // 通用二进制/错误页拒绝，防 Image.memory 解码失败
      final mediaTypeResult = thumbnailMediaType(
        response.headers.value('content-type'),
        bytes,
      );
      if (mediaTypeResult.isErr) {
        return Err((mediaTypeResult as Err).error);
      }
      return Ok(bytes);
    } on AppError catch (e) {
      return Err(e);
    } on DioException catch (e) {
      return Err(AppError.fromDioException(e));
    } catch (e, st) {
      AppLogger.e('getThumbnail 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 获取缩略图并编码为保留真实 MIME 的 data URL
  /// （对齐 Rust `get_data_url`，供 WebView/HTML 内嵌场景使用）。
  Future<AppResult<String>> getThumbnailDataUrl(String fileId) async {
    final result = await getThumbnail(fileId);
    if (result.isErr) {
      return Err((result as Err).error);
    }
    final bytes = (result as Ok<Uint8List>).value;
    final mediaTypeResult = thumbnailMediaType(null, bytes);
    if (mediaTypeResult.isErr) {
      return Err((mediaTypeResult as Err).error);
    }
    final mediaType = (mediaTypeResult as Ok<String>).value;
    final encoded = base64Encode(bytes);
    return Ok('data:$mediaType;base64,$encoded');
  }
}

/// 解析服务端 MIME，并在通用二进制响应时按文件签名识别图片格式
/// （对齐 Rust `thumbnail_media_type`）。
///
/// 优先用响应 Content-Type（以 `image/` 开头才采纳）；否则按魔数判定
/// PNG/JPEG/GIF/WEBP。非图片格式返回错误，拒绝传给渲染层。
AppResult<String> thumbnailMediaType(String? contentType, Uint8List bytes) {
  // 1. 服务端 Content-Type 为具体图片类型时直接采纳
  if (contentType != null) {
    final mime = contentType.split(';').first.trim().toLowerCase();
    if (mime.startsWith('image/')) {
      return Ok(mime);
    }
  }
  // 2. 按文件头魔数识别（服务端常返回通用二进制）
  if (bytes.length >= 8 &&
      bytes[0] == 0x89 &&
      bytes[1] == 0x50 &&
      bytes[2] == 0x4E &&
      bytes[3] == 0x47 &&
      bytes[4] == 0x0D &&
      bytes[5] == 0x0A &&
      bytes[6] == 0x1A &&
      bytes[7] == 0x0A) {
    return const Ok('image/png');
  }
  if (bytes.length >= 3 &&
      bytes[0] == 0xFF &&
      bytes[1] == 0xD8 &&
      bytes[2] == 0xFF) {
    return const Ok('image/jpeg');
  }
  if (_startsWith(bytes, 'GIF87a') || _startsWith(bytes, 'GIF89a')) {
    return const Ok('image/gif');
  }
  if (bytes.length >= 12 &&
      _startsWith(bytes, 'RIFF') &&
      bytes[8] == 0x57 && // 'W'
      bytes[9] == 0x45 && // 'E'
      bytes[10] == 0x42 && // 'B'
      bytes[11] == 0x50) {
    // 'P'
    return const Ok('image/webp');
  }
  return Err(GenericError(message: '缩略图响应不是支持的图片格式'));
}

/// 判断 bytes 是否以给定 ASCII 前缀开头
bool _startsWith(Uint8List bytes, String prefix) {
  if (bytes.length < prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (bytes[i] != prefix.codeUnitAt(i)) return false;
  }
  return true;
}
