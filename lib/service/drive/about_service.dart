import 'dart:convert';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/drive/drive_endpoints.dart';

/// 配额 API 服务 —— `GET /about?fields=*`。
///
/// 严格对齐 Rust 原版 `src/drive/about_api.rs`。
/// 注意：`fields=*` 是**强制**参数，否则华为返回 400。
class AboutService {
  final MateHttpClient _client;

  AboutService(this._client);

  /// 获取配额信息（对齐 Rust `AboutApi::get`）。
  ///
  /// 配额字段在 `storageQuota` 子对象下且华为返回为 String（由
  /// [DriveAbout.fromJson] 容忍解析）。
  Future<AppResult<DriveAbout>> get() async {
    try {
      final result =
          await _client.get<String>('$driveApiBase/about?fields=*');
      final json = jsonDecode(result.unwrap()) as Map<String, dynamic>;
      return Ok(DriveAbout.fromJson(json));
    } on AppError catch (e) {
      return Err(e);
    } catch (e, st) {
      AppLogger.e('about.get 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 上传前配额校验（对齐 Rust `AboutApi::ensure_capacity`）。
  ///
  /// 剩余空间不足时返回 [QuotaExceededError]（携带所需与剩余字节数）。
  Future<AppResult<void>> ensureCapacity(int requiredBytes) async {
    final about = await get();
    if (about.isErr) return Err((about as Err<DriveAbout>).error);
    final value = (about as Ok<DriveAbout>).value;
    if (!value.canFit(requiredBytes)) {
      return Err(AppError.quotaExceeded(requiredBytes, value.remainingSpace));
    }
    return const Ok(null);
  }
}
