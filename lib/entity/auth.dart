/// 认证数据模型 —— TokenPair / UserInfo / AuthState。
///
/// 严格对齐 Rust 原版 `src/auth/models.rs`（TokenPair、UserInfo）与
/// `src/commands/auth.rs`（AuthState）。JSON 字段名为 snake_case（对齐 serde）。
library;

/// copyWith 的「保持原值」哨兵（区分「不传」与「显式置 null」）
const Object _keep = Object();

/// OAuth Token 对（对齐 Rust `TokenPair`）。
///
/// access_token + refresh_token + 过期时间，加密持久化到安全存储。
class TokenPair {
  /// 访问令牌
  final String accessToken;

  /// 刷新令牌（用于获取新 accessToken）
  final String refreshToken;

  /// access_token 过期时间（毫秒 epoch，对齐 Rust `expires_at: i64`）
  final int expiresAt;

  /// Token 类型（默认 "Bearer"）
  final String tokenType;

  /// 授权范围（可选）
  final String? scope;

  const TokenPair({
    required this.accessToken,
    required this.refreshToken,
    required this.expiresAt,
    this.tokenType = 'Bearer',
    this.scope,
  });

  /// 是否已过期（对齐 Rust `is_expired`）
  bool get isExpired => DateTime.now().millisecondsSinceEpoch >= expiresAt;

  /// 距过期是否小于 [buffer]（用于提前刷新，默认 60 秒）。
  ///
  /// 对齐 Rust `will_expire_within(buffer_secs)`。
  bool willExpireWithin([Duration buffer = const Duration(seconds: 60)]) {
    final threshold =
        DateTime.now().millisecondsSinceEpoch + buffer.inMilliseconds;
    return threshold >= expiresAt;
  }

  /// 从华为 token 端点响应构造（expires_in 为**秒**，容忍 String 数字）。
  ///
  /// 对齐 Rust `from_token_response`：缺少 access_token 时返回 null。
  static TokenPair? fromTokenResponse(Map<String, dynamic> json) {
    final accessToken = json['access_token'];
    if (accessToken is! String || accessToken.isEmpty) return null;

    final refreshToken = json['refresh_token'];
    final expiresInSec = _tolerantInt(json['expires_in']) ?? 3600;
    final expiresAt =
        DateTime.now().millisecondsSinceEpoch + expiresInSec * 1000;
    final tokenType = json['token_type'];

    return TokenPair(
      accessToken: accessToken,
      refreshToken: refreshToken is String ? refreshToken : '',
      expiresAt: expiresAt,
      tokenType: tokenType is String && tokenType.isNotEmpty
          ? tokenType
          : 'Bearer',
      scope: json['scope'] is String ? json['scope'] as String : null,
    );
  }

  /// 从 JSON 构造（snake_case 键，expires_at 容忍 String 数字）
  factory TokenPair.fromJson(Map<String, dynamic> json) {
    return TokenPair(
      accessToken: json['access_token'] as String? ?? '',
      refreshToken: json['refresh_token'] as String? ?? '',
      expiresAt: _tolerantInt(json['expires_at']) ?? 0,
      tokenType: json['token_type'] as String? ?? 'Bearer',
      scope: json['scope'] as String?,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'access_token': accessToken,
      'refresh_token': refreshToken,
      'expires_at': expiresAt,
      'token_type': tokenType,
      'scope': scope,
    };
  }

  /// 深拷贝并替换指定字段（scope 传 null 显式清空）
  TokenPair copyWith({
    String? accessToken,
    String? refreshToken,
    int? expiresAt,
    String? tokenType,
    Object? scope = _keep,
  }) {
    return TokenPair(
      accessToken: accessToken ?? this.accessToken,
      refreshToken: refreshToken ?? this.refreshToken,
      expiresAt: expiresAt ?? this.expiresAt,
      tokenType: tokenType ?? this.tokenType,
      scope: identical(scope, _keep) ? this.scope : scope as String?,
    );
  }
}

/// 华为账号信息（对齐 Rust `UserInfo`，合并自多个端点响应）。
///
/// 字段全部可空：不同端点返回的子集不同，由上游合并后统一构造。
class UserInfo {
  /// OIDC sub（用户唯一标识）
  final String? sub;

  /// 华为账号 openId
  final String? openId;

  /// 华为账号 unionId
  final String? unionId;

  /// 展示名（可能为匿名账号名，见 [isAnonymized]）
  final String? displayName;

  /// OIDC name
  final String? name;

  /// 昵称
  final String? nickname;

  /// 邮箱
  final String? email;

  /// 手机号
  final String? mobile;

  /// 头像 URL
  final String? avatarUrl;

  /// displayName 是否为匿名账号（displayNameFlag=1）
  final bool isAnonymized;

  const UserInfo({
    this.sub,
    this.openId,
    this.unionId,
    this.displayName,
    this.name,
    this.nickname,
    this.email,
    this.mobile,
    this.avatarUrl,
    this.isAnonymized = false,
  });

  /// 用户主要展示名（按优先级）：displayName > 手机号 > name/nickname/openId/sub。
  ///
  /// 对齐 Rust `primary_label`。
  String? get primaryLabel {
    final d = _nonEmpty(displayName);
    if (d != null) return d;
    final m = _nonEmpty(mobile);
    if (m != null) return m;
    for (final c in [name, nickname, openId, sub]) {
      final v = _nonEmpty(c);
      if (v != null) return v;
    }
    return null;
  }

  /// 副标题：邮箱（与主标不同且非空）→ 手机号（同样不重复）→ 匿名账号提示 / null。
  ///
  /// 对齐 Rust `secondary_label`。
  String? get secondaryLabel {
    final pri = primaryLabel;
    final e = _nonEmpty(email);
    if (e != null && e != pri) return e;
    final m = _nonEmpty(mobile);
    if (m != null && m != pri) return m;
    if (isAnonymized) return '匿名账号';
    return null;
  }

  /// 头像首字符（取主标第一个字符，对齐 Rust `initial`：首个 Unicode scalar）
  String? get initial {
    final label = primaryLabel;
    if (label == null || label.isEmpty) return null;
    return String.fromCharCode(label.runes.first);
  }

  /// 把「匿名 displayName + 真实手机号」合并为最优展示：
  /// 清空匿名名让 primaryLabel 走 mobile。对齐 Rust `resolve_anonymous_as_mobile`。
  UserInfo resolveAnonymousAsMobile() {
    if (!isAnonymized) return this;
    if (_nonEmpty(mobile) == null) return this;
    return copyWith(displayName: null);
  }

  /// 从合并后的 JSON 构造（兼容多端点字段命名，对齐 Rust `from_json`）
  factory UserInfo.fromJson(Map<String, dynamic> json) {
    // displayNameFlag=1 表示匿名账号（容忍 String 数字）
    final flag = _tolerantInt(json['displayNameFlag']);

    return UserInfo(
      sub: _pick(json, const ['sub', 'user_id', 'userId']),
      openId: _pick(json, const ['openID', 'openId', 'open_id']),
      unionId: _pick(json, const ['unionID', 'unionId', 'union_id']),
      displayName: _pick(json, const ['displayName', 'display_name']),
      name: _pick(json, const ['name']),
      nickname:
          _pick(json, const ['nickname', 'nick_name', 'preferred_username']),
      email: _pick(json, const ['email']),
      mobile:
          _pick(json, const ['mobile', 'phone', 'phone_number', 'mobile_number']),
      avatarUrl:
          _pick(json, const ['headPictureURL', 'picture', 'avatar', 'avatar_url']),
      isAnonymized: flag == 1,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'sub': sub,
      'open_id': openId,
      'union_id': unionId,
      'display_name': displayName,
      'name': name,
      'nickname': nickname,
      'email': email,
      'mobile': mobile,
      'avatar_url': avatarUrl,
      'is_anonymized': isAnonymized,
    };
  }

  /// 深拷贝并替换指定字段（可空字段传 null 显式清空）
  UserInfo copyWith({
    Object? sub = _keep,
    Object? openId = _keep,
    Object? unionId = _keep,
    Object? displayName = _keep,
    Object? name = _keep,
    Object? nickname = _keep,
    Object? email = _keep,
    Object? mobile = _keep,
    Object? avatarUrl = _keep,
    bool? isAnonymized,
  }) {
    return UserInfo(
      sub: identical(sub, _keep) ? this.sub : sub as String?,
      openId: identical(openId, _keep) ? this.openId : openId as String?,
      unionId: identical(unionId, _keep) ? this.unionId : unionId as String?,
      displayName: identical(displayName, _keep)
          ? this.displayName
          : displayName as String?,
      name: identical(name, _keep) ? this.name : name as String?,
      nickname:
          identical(nickname, _keep) ? this.nickname : nickname as String?,
      email: identical(email, _keep) ? this.email : email as String?,
      mobile: identical(mobile, _keep) ? this.mobile : mobile as String?,
      avatarUrl: identical(avatarUrl, _keep)
          ? this.avatarUrl
          : avatarUrl as String?,
      isAnonymized: isAnonymized ?? this.isAnonymized,
    );
  }

  /// 取 trim 后非空的字符串，否则 null（对齐 Rust `non_empty_trimmed`）
  static String? _nonEmpty(String? s) {
    if (s == null) return null;
    final t = s.trim();
    return t.isEmpty ? null : t;
  }

  /// 从 JSON 按 keys 顺序取首个非空 trim 字符串（对齐 Rust `pick`）
  static String? _pick(Map<String, dynamic> json, List<String> keys) {
    for (final k in keys) {
      final v = json[k];
      if (v is String) {
        final t = v.trim();
        if (t.isNotEmpty) return t;
      }
    }
    return null;
  }
}

/// 认证配置快照（对齐 Rust `AuthState`，登录页恢复用）。
///
/// 注意：这是后端认证配置快照 DTO，与 `app/auth/auth_state.dart` 的
/// UI 状态机类同名但职责不同；同时引用两者时请使用 import hide/prefix。
class AuthState {
  /// 是否已登录（token store 中存在有效凭据）
  final bool loggedIn;

  /// OAuth client_id / client_secret 是否均已配置
  final bool secretConfigured;

  /// OAuth 回调端口
  final int callbackPort;

  const AuthState({
    this.loggedIn = false,
    this.secretConfigured = false,
    this.callbackPort = 9999,
  });

  /// 从 JSON 构造（snake_case 键）
  factory AuthState.fromJson(Map<String, dynamic> json) {
    return AuthState(
      loggedIn: json['logged_in'] == true,
      secretConfigured: json['secret_configured'] == true,
      callbackPort: _tolerantInt(json['callback_port']) ?? 9999,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'logged_in': loggedIn,
      'secret_configured': secretConfigured,
      'callback_port': callbackPort,
    };
  }

  /// 深拷贝并替换指定字段
  AuthState copyWith({
    bool? loggedIn,
    bool? secretConfigured,
    int? callbackPort,
  }) {
    return AuthState(
      loggedIn: loggedIn ?? this.loggedIn,
      secretConfigured: secretConfigured ?? this.secretConfigured,
      callbackPort: callbackPort ?? this.callbackPort,
    );
  }
}

/// 容忍解析 int：接受 int / num / String（华为部分数值字段返回 String）。
int? _tolerantInt(Object? v) {
  if (v is int) return v;
  if (v is num) return v.toInt();
  if (v is String) return int.tryParse(v.trim());
  return null;
}
