import 'dart:convert';

import 'package:dio/dio.dart';

// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/auth_constants.dart';

/// 华为账号信息客户端（合并多端点）。
///
/// 严格对齐 Rust 原版 `src/auth/user_info_api.rs`：
/// 并行调三个端点（任一失败不影响其他），合并为单一 [UserInfo]：
/// 1. `POST rest.php?nsp_svc=GOpen.User.getInfo` → displayName / openID / headPictureURL（需 profile scope）
/// 2. `POST rest.php?nsp_svc=GOpen.User.getPhone` → 纯文本手机号（需 mobile scope，仅中国大陆）
/// 3. `GET /oauth2/v3/userinfo`（OIDC）→ sub 标识（尽力而为，华为该端点常 404）
///
/// 合并优先级：oidc < info < phone（phone 最优先，覆盖 info 的脱敏手机号）。
class UserInfoApi {
  final Future<String> Function() _tokenProvider;
  final Dio _http;

  UserInfoApi({
    required Future<String> Function() tokenProvider,
    Dio? http,
  })  : _tokenProvider = tokenProvider,
        _http = http ?? _createDefaultHttp();

  /// 默认 HTTP 客户端（30s 超时，对齐 Rust reqwest；任意状态码均读 body，
  /// 非 2xx 的错误体解析失败按空 map 处理，对齐 Rust 尽力而为语义）。
  static Dio _createDefaultHttp() {
    return Dio(BaseOptions(
      connectTimeout: AuthConstants.httpTimeout,
      sendTimeout: AuthConstants.httpTimeout,
      receiveTimeout: AuthConstants.httpTimeout,
      validateStatus: (_) => true,
    ));
  }

  /// 拉取完整账号信息（合并三端点）。任一端点失败不影响其他。
  Future<UserInfo> get() async {
    final token = await _tokenProvider();
    AppLogger.i('开始拉取账号信息');

    // 并行三端点（失败返回空 map，不影响其他）
    final results = await Future.wait([
      _getDisplayInfo(token),
      _getPhoneNumber(token),
      _getOidcUserInfo(token),
    ]);
    final info = results[0];
    final phone = results[1];
    final oidc = results[2];

    // 合并：oidc 先放，info 覆盖，phone 最后覆盖（最优先）
    final merged = <String, dynamic>{}
      ..addAll(oidc)
      ..addAll(info)
      ..addAll(phone);

    return UserInfo.fromJson(merged).resolveAnonymousAsMobile();
  }

  /// 请求账号展示名、开放标识与头像资料。
  ///
  /// 通过 POST GOpen.User.getInfo 读取 displayName / openID / headPictureURL / displayNameFlag。
  Future<Map<String, dynamic>> _getDisplayInfo(String token) async {
    try {
      final resp = await _http.post<String>(
        AuthConstants.restPhpUrl,
        queryParameters: {'nsp_svc': 'GOpen.User.getInfo'},
        data: {
          'access_token': token,
          'getNickName': '1', // 1=返回真实昵称；0=匿名化
        },
        options: Options(
          contentType: Headers.formUrlEncodedContentType,
          responseType: ResponseType.plain,
        ),
      );
      return _decodeObject(resp.data, 'GOpen.User.getInfo');
    } catch (e) {
      AppLogger.w('GOpen.User.getInfo 请求失败', e);
      return {};
    }
  }

  /// POST GOpen.User.getPhone → 纯文本手机号（无字段名），也可能在 JSON 字段中。
  Future<Map<String, dynamic>> _getPhoneNumber(String token) async {
    try {
      final resp = await _http.post<String>(
        AuthConstants.restPhpUrl,
        queryParameters: {'nsp_svc': 'GOpen.User.getPhone'},
        data: {'access_token': token},
        options: Options(
          contentType: Headers.formUrlEncodedContentType,
          responseType: ResponseType.plain,
        ),
      );
      final text = (resp.data ?? '').trim();
      if (text.isEmpty) return {};
      // 先尝试 JSON 解析
      try {
        final decoded = jsonDecode(text);
        if (decoded is Map<String, dynamic>) return decoded;
        if (decoded is Map) return Map<String, dynamic>.from(decoded);
      } catch (_) {
        // 非 JSON → 纯文本手机号
      }
      // 纯文本形式：包装为 {mobile: <text>}
      return {'mobile': text};
    } catch (e) {
      AppLogger.w('GOpen.User.getPhone 请求失败', e);
      return {};
    }
  }

  /// GET OIDC userinfo（尽力而为，常 404）。
  Future<Map<String, dynamic>> _getOidcUserInfo(String token) async {
    try {
      final resp = await _http.get<String>(
        AuthConstants.userInfoUrl,
        options: Options(
          headers: {'Authorization': 'Bearer $token'},
          responseType: ResponseType.plain,
        ),
      );
      return _decodeObject(resp.data, 'oidc userinfo');
    } catch (e) {
      AppLogger.w('oidc userinfo 请求失败', e);
      return {};
    }
  }

  /// 解析 JSON 对象响应体；非对象按空 map（对齐 Rust `is_object` 校验）。
  static Map<String, dynamic> _decodeObject(String? text, String tag) {
    if (text == null || text.isEmpty) return {};
    try {
      final decoded = jsonDecode(text);
      if (decoded is Map<String, dynamic>) return decoded;
      if (decoded is Map) return Map<String, dynamic>.from(decoded);
      AppLogger.w('$tag 返回非对象');
      return {};
    } catch (e) {
      AppLogger.w('$tag 响应解析失败', e);
      return {};
    }
  }
}
