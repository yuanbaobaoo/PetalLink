import 'package:petal_link/types/enums.dart';

/// 认证状态
class AuthState {
  /// 当前认证状态
  final AuthStatus status;

  /// 访问令牌
  final String? accessToken;

  /// 刷新令牌
  final String? refreshToken;

  /// 账号名称（手机号）
  final String? accountName;

  /// 令牌过期时间（毫秒时间戳）
  final int? expiresAt;

  const AuthState({
    this.status = AuthStatus.Init,
    this.accessToken,
    this.refreshToken,
    this.accountName,
    this.expiresAt,
  });

  /// 初始状态
  factory AuthState.init() => const AuthState(status: AuthStatus.Init);

  /// 已登录状态
  factory AuthState.authorized({
    required String accessToken,
    required String refreshToken,
    String? accountName,
    int? expiresAt,
  }) {
    return AuthState(
      status: AuthStatus.Authorized,
      accessToken: accessToken,
      refreshToken: refreshToken,
      accountName: accountName,
      expiresAt: expiresAt,
    );
  }

  /// 未登录状态
  factory AuthState.unauthorized() =>
      const AuthState(status: AuthStatus.Unauthorized);

  /// 是否已登录
  bool get isAuthorized => status == AuthStatus.Authorized;

  /// 从 JSON 恢复
  factory AuthState.fromJson(Map<String, dynamic> json) {
    return AuthState(
      status: AuthStatus.Authorized,
      accessToken: json['accessToken'] as String?,
      refreshToken: json['refreshToken'] as String?,
      accountName: json['accountName'] as String?,
      expiresAt: json['expiresAt'] as int?,
    );
  }

  /// 转为 JSON
  Map<String, dynamic> toJson() {
    return {
      'accessToken': accessToken,
      'refreshToken': refreshToken,
      'accountName': accountName,
      'expiresAt': expiresAt,
    };
  }
}
