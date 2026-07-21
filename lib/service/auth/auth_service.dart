// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

import 'dart:convert';

import 'package:dio/dio.dart';
import 'package:url_launcher/url_launcher.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/auth_constants.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/oauth_server.dart';
import 'package:petal_link/service/auth/pkce.dart';
import 'package:petal_link/service/auth/token_refresher.dart';
import 'package:petal_link/service/auth/token_store.dart';
import 'package:petal_link/service/auth/user_info_api.dart';

/// Auth 编排服务 —— OAuth 授权流程 + code 交换 + token 生命周期管理。
///
/// 严格对齐 Rust 原版 `src/auth/service.rs` 与 `src/commands/auth.rs`。
///
/// # 关键编码细节（华为 API 怪癖）
/// - scope 用空格分隔，空格替换为 `%20`（**不**用 URL 编码整个 scope，`/` 不编码）
/// - code 交换时手工拼接 form body，用 [Uri.encodeComponent] 精确编码每个值
///   （authorization_code 含 `+ / =`，form-urlencoded 会把 `+` 当空格 → invalid code 1101）
///
/// 对应 Rust 命令层方法映射：
/// - [restore] ↔ auth_restore（启动加载 + 临期刷新）
/// - [authorize] ↔ auth_login（完整登录流程）
/// - [cancelAuthorize] ↔ auth_cancel_login
/// - [logout] ↔ auth_logout（token 部分；DB 同步行/缓存清理待 sync 服务落地后接线）
/// - [getUserInfo] ↔ auth_get_user_info
/// - [isLoggedIn] ↔ auth_is_logged_in
/// - [isSecretConfigured] ↔ auth_check_secret
class AuthService {
  final TokenStore _tokenStore;
  final Dio _http;
  final Future<AuthSecrets> Function() _secretsProvider;
  final Future<bool> Function(String url) _browserLauncher;
  final Future<OauthServer> Function(int port) _oauthServerFactory;

  late final TokenRefresher _refresher;
  late final UserInfoApi _userInfoApi;

  /// 当前授权流程的 PKCE verifier（仅在 authorize 期间有效）
  String? _currentVerifier;

  /// 是否被用户取消授权
  bool _cancelled = false;

  /// 当前 OAuth 回调 server 停止句柄
  OauthServerStopHandle? _currentOauthStop;

  /// 最近一次拉取的账号信息缓存（登录成功后可用）
  UserInfo? _currentUserInfo;

  /// 构造 Auth 服务。各依赖均可注入（测试用），默认走生产实现：
  /// - token 存储：机器码绑定的 [EncryptedFileTokenStore]
  /// - 浏览器打开：url_launcher externalApplication
  /// - 回调服务：127.0.0.1 回环 [OauthServer]
  factory AuthService({
    TokenStore? tokenStore,
    TokenRefresher? refresher,
    UserInfoApi? userInfoApi,
    Dio? http,
    AuthSecrets? secrets,
    Future<bool> Function(String url)? browserLauncher,
    Future<OauthServer> Function(int port)? oauthServerFactory,
  }) {
    final store = tokenStore ?? EncryptedFileTokenStore();
    final dio = http ?? _createDefaultHttp();
    // 凭据加载 memo（override 优先，避免每次请求都读 .env / asset）
    Future<AuthSecrets>? cached;
    Future<AuthSecrets> secretsProvider() {
      final override = secrets;
      return cached ??=
          override != null ? Future.value(override) : AuthSecrets.load();
    }

    return AuthService._(
      tokenStore: store,
      http: dio,
      secretsProvider: secretsProvider,
      browserLauncher: browserLauncher ?? _defaultBrowserLauncher,
      oauthServerFactory: oauthServerFactory ?? OauthServer.start,
      refresher: refresher,
      userInfoApi: userInfoApi,
    );
  }

  AuthService._({
    required TokenStore tokenStore,
    required Dio http,
    required Future<AuthSecrets> Function() secretsProvider,
    required Future<bool> Function(String url) browserLauncher,
    required Future<OauthServer> Function(int port) oauthServerFactory,
    TokenRefresher? refresher,
    UserInfoApi? userInfoApi,
  })  : _tokenStore = tokenStore,
        _http = http,
        _secretsProvider = secretsProvider,
        _browserLauncher = browserLauncher,
        _oauthServerFactory = oauthServerFactory {
    // 默认实现共享同一 tokenStore / http / 凭据加载器
    _refresher = refresher ??
        TokenRefresher(
          tokenStore: tokenStore,
          secretsProvider: secretsProvider,
          http: http,
        );
    _userInfoApi = userInfoApi ??
        UserInfoApi(
          tokenProvider: ensureValidAccessToken,
          http: http,
        );
  }

  /// 默认 HTTP 客户端（30s 超时，对齐 Rust reqwest；任意状态码均读 body，
  /// 华为错误体在 body 里由解析逻辑判定）。
  static Dio _createDefaultHttp() {
    return Dio(BaseOptions(
      connectTimeout: AuthConstants.httpTimeout,
      sendTimeout: AuthConstants.httpTimeout,
      receiveTimeout: AuthConstants.httpTimeout,
      validateStatus: (_) => true,
    ));
  }

  /// 默认浏览器打开方式（url_launcher 外部浏览器）。
  static Future<bool> _defaultBrowserLauncher(String url) {
    return launchUrl(Uri.parse(url), mode: LaunchMode.externalApplication);
  }

  /// 获取 token refresher（供 MateHttpClient 401 重放用）。
  TokenRefresher get refresher => _refresher;

  /// 最近一次拉取的账号信息（登录成功后可用）。
  UserInfo? get currentUserInfo => _currentUserInfo;

  // ═══════════════════════════════════════════════════════════════════
  // auth_restore：启动时恢复登录态
  // ═══════════════════════════════════════════════════════════════════

  /// 启动时恢复登录态：加载 token，若临期则刷新。
  ///
  /// 对齐 Rust `auth_restore`：返回登录/凭据/回调端口快照。
  /// 刷新失败的处置（对齐 `restore_refresh_failure_action`）：
  /// - 网络故障（DriveApi network）→ 保留 token，向上抛错
  /// - 其余失败（token 被拒绝等）→ 登出清理，快照 loggedIn=false
  Future<AuthState> restore({
    int callbackPort = AuthConstants.defaultCallbackPort,
  }) async {
    final token = await _tokenStore.load();
    var loggedIn = false;
    if (token != null) {
      _refresher.setCurrent(token);
      if (token.willExpireWithin(AuthConstants.tokenExpiryBuffer)) {
        try {
          await _refresher.refresh();
          loggedIn = true;
        } catch (e) {
          if (_isNetworkFailure(e)) {
            AppLogger.w('恢复登录态时刷新失败，保留本地 token 并返回错误', e);
            rethrow;
          }
          AppLogger.w('恢复登录态时 token 被拒绝，登出', e);
          await logout();
          loggedIn = false;
        }
      } else {
        loggedIn = true;
      }
    }
    return AuthState(
      loggedIn: loggedIn,
      secretConfigured: await isSecretConfigured(),
      callbackPort: callbackPort,
    );
  }

  /// 网络故障保留 token，其余刷新失败清理失效登录态（对齐 Rust 匹配分支）。
  static bool _isNetworkFailure(Object e) {
    return e is DriveApiError && e.driveCode == DriveApiErrorCode.network;
  }

  // ═══════════════════════════════════════════════════════════════════
  // auth_login：完整 OAuth 授权流程
  // ═══════════════════════════════════════════════════════════════════

  /// 启动 OAuth 授权流程：检查凭据 → 开回环服务 → 打开授权页 → 等回调
  /// → 换 token → 存 token.bin → 拉取合并 UserInfo（尽力而为，不阻塞登录）。
  ///
  /// 对齐 Rust `authorize`。成功后 token 已存存储，currentToken 可用。
  /// 抛 [AppError]（取消 / 超时 / state 不匹配 / 换 token 失败等）。
  Future<TokenPair> authorize({
    int port = AuthConstants.defaultCallbackPort,
  }) async {
    AppLogger.i('开始 OAuth 授权流程（port=$port）');

    // 0. 检查凭据配置（对齐 auth_check_secret 前置校验）
    final secrets = await _secretsProvider();
    if (!secrets.configured) {
      throw AppError.config(
          '尚未配置 OAuth 凭据，请在 .env 中配置 HWCLOUD_CLIENT_ID / HWCLOUD_CLIENT_SECRET');
    }

    final state = generateState();
    final pkce = generatePkce();
    _currentVerifier = pkce.codeVerifier;
    final redirectUri = AuthConstants.buildRedirectUri(port);
    _cancelled = false;

    // 1. 启动 loopback 监听
    final server = await _oauthServerFactory(port);
    _currentOauthStop = server.stopHandle;

    // 2. 构造授权 URL 并打开浏览器
    final authUrl =
        buildAuthorizeUrl(secrets.clientId, redirectUri, state, pkce);
    AppLogger.i('打开授权页');
    final launched = await _browserLauncher(authUrl);
    if (!launched) {
      _currentOauthStop = null;
      await server.stop();
      throw AppError.authBrowserLaunchFailed();
    }

    // 3. 等待回调（finally：server 已在 waitForCallback 内 stop）
    AppError? waitError;
    OauthCallbackResult? callback;
    try {
      callback = await server.waitForCallback();
    } on AppError catch (e) {
      waitError = e;
    } finally {
      _currentOauthStop = null;
    }

    // 4. 用户取消检测（先于错误传播，对齐 Rust 顺序）
    if (_cancelled) throw AppError.authCancelled();
    if (waitError != null) throw waitError;
    callback!;

    // 5. 校验回调
    validateCallback(callback, state);

    // 6. 换 token（带 PKCE code_verifier）
    AppLogger.i('收到授权码，换取 token...');
    final token = await exchangeCodeForToken(
      secrets: secrets,
      code: callback.code!,
      redirectUri: redirectUri,
      codeVerifier: _currentVerifier,
    );

    // 7. 持久化
    await _tokenStore.save(token);
    _refresher.setCurrent(token);

    // 8. 拉取合并账号信息（尽力而为，失败不阻塞登录）
    try {
      _currentUserInfo = await _userInfoApi.get();
    } catch (e) {
      AppLogger.w('登录后拉取账号信息失败（非致命）', e);
    }

    AppLogger.i('OAuth 授权流程完成 ✓');
    return token;
  }

  /// 取消正在进行的授权流程。对齐 Rust `cancel_authorize`。
  void cancelAuthorize() {
    _cancelled = true;
    _currentOauthStop?.stop();
    _currentOauthStop = null;
    AppLogger.i('用户取消授权');
  }

  // ═══════════════════════════════════════════════════════════════════
  // auth_logout / auth_get_user_info / auth_is_logged_in
  // ═══════════════════════════════════════════════════════════════════

  /// 退出登录：清空存储 + 内存（F-AUTH-05）。对齐 Rust `logout`。
  ///
  /// 注：Rust 命令层的「清 DB 同步行 / 缓存文件 / 目录配置」依赖 sync/transfer
  /// 服务，待对应服务重写落地后在调用方接线（不在本服务范围内）。
  Future<void> logout() async {
    await _tokenStore.clear();
    _refresher.clearCurrent();
    _currentUserInfo = null;
    AppLogger.i('已退出登录');
  }

  /// 拉取当前账号信息（3 端点合并）。对齐 Rust `auth_get_user_info`。
  Future<UserInfo> getUserInfo() async {
    final userInfo = await _userInfoApi.get();
    _currentUserInfo = userInfo;
    return userInfo;
  }

  /// 以本地 token store 是否存在有效记录判断登录状态。
  ///
  /// 对齐 Rust `auth_is_logged_in`。
  Future<bool> isLoggedIn() async {
    return await _tokenStore.load() != null;
  }

  /// client_id 与 client_secret 是否均已配置。对齐 Rust `auth_check_secret`。
  Future<bool> isSecretConfigured() async {
    return (await _secretsProvider()).configured;
  }

  /// 确保拥有有效 token，必要时刷新（供 MateHttpClient tokenProvider 用）。
  ///
  /// 对齐 Rust `ensure_valid_access_token`。返回 access_token 字符串。
  Future<String> ensureValidAccessToken() async {
    final token = await _refresher.currentToken();
    if (token == null) throw AppError.tokenNotLoggedIn();
    if (token.willExpireWithin(AuthConstants.tokenExpiryBuffer)) {
      final refreshed = await _refresher.refresh();
      return refreshed.accessToken;
    }
    return token.accessToken;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 回调校验与 code 交换
  // ═══════════════════════════════════════════════════════════════════

  /// 校验回调结果。对齐 Rust `validate_callback`。
  static void validateCallback(
    OauthCallbackResult callback,
    String expectedState,
  ) {
    final error = callback.error;
    if (error != null) {
      // 华为 OAuth 错误码识别（1101 + invalid scope → 明确指引）
      if (error == '1101') {
        final descLower = (callback.errorDescription ?? '').toLowerCase();
        if (descLower.contains('scope')) {
          final scopeList = AuthConstants.scopes.join(', ');
          throw AuthError(
            authCode: AuthErrorCode.denied,
            message: '授权失败：scope 未在 AppGallery Connect 后台授权\n'
                '错误码：error=1101 sub_error=${callback.subError ?? 'N/A'}\n'
                '当前请求的 scope：$scopeList\n\n'
                '请在 AGC 后台「API 管理」和「OAuth 2.0 凭据 → 作用域」两处'
                '勾选上述所有 scope 后重试。',
          );
        }
      }
      throw AppError.authDenied(callback.errorDescription ?? error);
    }
    if (callback.code == null) throw AppError.authInvalidCode();
    if (callback.state != expectedState) {
      AppLogger.w('state 不匹配（期望 $expectedState，实收 ${callback.state}）');
      throw AppError.authStateMismatch();
    }
  }

  /// 用授权码换 token。手工拼接 form body 防止 `+` 被当空格。
  ///
  /// 对齐 Rust `exchange_code_for_token`。
  Future<TokenPair> exchangeCodeForToken({
    required AuthSecrets secrets,
    required String code,
    required String redirectUri,
    String? codeVerifier,
  }) async {
    // 关键：authorization_code 含 '+' '/' '='，form-urlencoded 会把 '+' 当空格。
    // 手工拼接 form body，用 Uri.encodeComponent（RFC 3986 unreserved，
    // '+' → %2B）对每个值精确编码。
    final enc = Uri.encodeComponent;
    final parts = <String>[
      'grant_type=${enc('authorization_code')}',
      'code=${enc(code)}',
      'client_id=${enc(secrets.clientId)}',
      'client_secret=${enc(secrets.clientSecret)}',
      'redirect_uri=${enc(redirectUri)}',
    ];
    final verifier = codeVerifier;
    if (verifier != null) {
      parts.add('code_verifier=${enc(verifier)}');
    }
    final body = parts.join('&');

    final Response<String> resp;
    try {
      resp = await _http.post<String>(
        AuthConstants.tokenUrl,
        data: body,
        options: Options(
          contentType: Headers.formUrlEncodedContentType,
          responseType: ResponseType.plain,
        ),
      );
    } on DioException catch (e) {
      throw AppError.generic('换 token 失败：${e.message ?? e}');
    }

    final text = resp.data ?? '';
    final Map<String, dynamic> data;
    try {
      final decoded = jsonDecode(text);
      if (decoded is! Map) throw const FormatException('非 JSON 对象');
      data = Map<String, dynamic>.from(decoded);
    } catch (_) {
      throw AppError.authTokenResponseInvalid();
    }

    if (data['access_token'] == null) {
      final desc = data['error_description'] ?? data['error'] ?? text;
      AppLogger.e('换 token 失败（status=${resp.statusCode}）：$desc');
      throw AppError.generic('换 token 失败：$desc');
    }

    final token = TokenPair.fromTokenResponse(data);
    if (token == null) throw AppError.authTokenResponseInvalid();
    return token;
  }
}

/// 构造授权 URL。
///
/// 关键：scope 用空格分隔，空格替换为 `%20`（不整体编码，`/` 保留）。
/// 对齐 Rust `build_authorize_url`。
String buildAuthorizeUrl(
  String clientId,
  String redirectUri,
  String state,
  PkcePair pkce,
) {
  final enc = Uri.encodeComponent;
  // 其余参数用 enc（RFC 3986 unreserved）编码
  final query = [
    'response_type=${enc('code')}',
    'client_id=${enc(clientId)}',
    'redirect_uri=${enc(redirectUri)}',
    'state=${enc(state)}',
    'access_type=${enc('offline')}',
    'code_challenge=${enc(pkce.codeChallenge)}',
    'code_challenge_method=${enc('S256')}',
  ].join('&');
  // scope 不整体编码，空格用 %20（华为接受）
  final scopeEncoded = AuthConstants.scopes.join(' ').replaceAll(' ', '%20');
  return '${AuthConstants.authorizeUrl}?$query&scope=$scopeEncoded';
}
