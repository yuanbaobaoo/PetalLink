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

  /// 获取云盘文件缩略图二进制内容（对齐 Rust `ThumbnailApi::get`）。
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
      return Ok(response.data ?? Uint8List(0));
    } on AppError catch (e) {
      return Err(e);
    } on DioException catch (e) {
      return Err(AppError.fromDioException(e));
    } catch (e, st) {
      AppLogger.e('getThumbnail 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }
}
