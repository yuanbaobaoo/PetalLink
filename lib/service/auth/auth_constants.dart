/// Auth 域常量（严格对齐 Rust 原版 `src/constants.rs`）。
library;

/// 华为 OAuth 与回调相关常量。
class AuthConstants {
  AuthConstants._();

  /// 授权页地址
  static const String authorizeUrl =
      'https://oauth-login.cloud.huawei.com/oauth2/v3/authorize';

  /// code 换 token / 刷新 token 地址
  static const String tokenUrl =
      'https://oauth-login.cloud.huawei.com/oauth2/v3/token';

  /// OIDC userinfo 地址（尽力而为，华为该端点常 404）
  static const String userInfoUrl =
      'https://oauth-login.cloud.huawei.com/oauth2/v3/userinfo';

  /// 华为账号扩展资料接口（GOpen.User.getInfo / getPhone）
  static const String restPhpUrl = 'https://account.cloud.huawei.com/rest.php';

  /// 授权 scope（空格分隔，对齐 Rust `SCOPES`）
  static const List<String> scopes = [
    'openid',
    'profile',
    'https://www.huawei.com/auth/drive',
  ];

  /// 回环监听地址（仅绑定 IPv4 loopback，不监听 0.0.0.0）
  static const String loopbackHost = '127.0.0.1';

  /// OAuth 回调默认端口
  static const int defaultCallbackPort = 9999;

  /// OAuth 回调路径
  static const String callbackPath = '/oauth/callback';

  /// 等待 OAuth 回调超时（对齐 Rust `OAUTH_TIMEOUT_SECS = 5 * 60`）
  static const Duration oauthTimeout = Duration(minutes: 5);

  /// token 临期提前刷新窗口（对齐 Rust `TOKEN_EXPIRY_BUFFER_SECS = 60`）
  static const Duration tokenExpiryBuffer = Duration(seconds: 60);

  /// auth 域 HTTP 请求超时（对齐 Rust reqwest 30s）
  static const Duration httpTimeout = Duration(seconds: 30);

  /// .env 中的 client_id 键名
  static const String envClientIdKey = 'HWCLOUD_CLIENT_ID';

  /// .env 中的 client_secret 键名
  static const String envClientSecretKey = 'HWCLOUD_CLIENT_SECRET';

  /// 构造 redirect_uri：`http://127.0.0.1:<port>/oauth/callback`。
  ///
  /// 对齐 Rust `build_redirect_uri`。
  static String buildRedirectUri(int port) {
    return 'http://$loopbackHost:$port$callbackPath';
  }
}
