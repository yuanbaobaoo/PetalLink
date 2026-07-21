import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/auth/auth_constants.dart';

/// OAuth 客户端凭据（client_id / client_secret）。
///
/// 严格对齐 Rust 原版 `src/constants.rs` 的解析语义：
/// 构建期注入值优先，`.env` 次之；无默认值，必须由用户显式提供。
///
/// Flutter 端加载来源（按优先级）：
/// 1. `--dart-define=HWCLOUD_CLIENT_ID=...`（构建期注入，对齐 Rust option_env）
/// 2. 应用包内 `.env` asset（构建期打入，发布版主要来源）
/// 3. 工作目录 `.env` 文件（开发期 flutter run）
/// 4. 进程环境变量（开发期兜底）
class AuthSecrets {
  /// OAuth client_id
  final String clientId;

  /// OAuth client_secret
  final String clientSecret;

  const AuthSecrets({
    this.clientId = '',
    this.clientSecret = '',
  });

  /// client_id 与 client_secret 是否均已配置（对齐 Rust `auth_check_secret`）
  bool get configured => clientId.isNotEmpty && clientSecret.isNotEmpty;

  /// 解析 .env 文本为键值对（纯函数，可测）。
  ///
  /// 规则：忽略空行与 `#` 注释行；按首个 `=` 切分；键值 trim；
  /// 值两端的成对引号（' 或 "）去除。
  static Map<String, String> parse(String content) {
    final result = <String, String>{};
    for (final rawLine in const LineSplitter().convert(content)) {
      final line = rawLine.trim();
      if (line.isEmpty || line.startsWith('#')) continue;
      final eq = line.indexOf('=');
      if (eq <= 0) continue;
      final key = line.substring(0, eq).trim();
      var value = line.substring(eq + 1).trim();
      if (value.length >= 2) {
        final first = value[0];
        final last = value[value.length - 1];
        if ((first == '"' && last == '"') || (first == "'" && last == "'")) {
          value = value.substring(1, value.length - 1);
        }
      }
      if (key.isNotEmpty) result[key] = value;
    }
    return result;
  }

  /// 按优先级加载凭据（见类注释）。
  static Future<AuthSecrets> load() async {
    // 1. 构建期注入（--dart-define）
    const defineId = String.fromEnvironment(AuthConstants.envClientIdKey);
    const defineSecret =
        String.fromEnvironment(AuthConstants.envClientSecretKey);
    if (defineId.isNotEmpty && defineSecret.isNotEmpty) {
      return const AuthSecrets(clientId: defineId, clientSecret: defineSecret);
    }

    // 2. 应用包内 .env asset
    final assetSecrets = await _loadFromAsset();
    if (assetSecrets != null && assetSecrets.configured) return assetSecrets;

    // 3. 工作目录 .env 文件（开发期）
    final fileSecrets = await _loadFromFile();
    if (fileSecrets != null && fileSecrets.configured) return fileSecrets;

    // 4. 进程环境变量兜底
    final envId = Platform.environment[AuthConstants.envClientIdKey] ?? '';
    final envSecret =
        Platform.environment[AuthConstants.envClientSecretKey] ?? '';
    if (envId.isNotEmpty && envSecret.isNotEmpty) {
      return AuthSecrets(clientId: envId, clientSecret: envSecret);
    }

    // 各来源均未配置完整：返回已解析到的部分值（configured=false 供 UI 提示）
    return assetSecrets ?? fileSecrets ?? const AuthSecrets();
  }

  /// 从打包 asset 读取 .env（发布版主要来源；加载失败静默降级）
  static Future<AuthSecrets?> _loadFromAsset() async {
    try {
      final content = await rootBundle.loadString('.env');
      return _fromMap(parse(content));
    } catch (_) {
      // asset 不存在或未注册（如纯 Dart 测试环境）→ 降级到下一来源
      return null;
    }
  }

  /// 从工作目录 .env 文件读取（开发期 flutter run；读取失败静默降级）
  static Future<AuthSecrets?> _loadFromFile() async {
    try {
      final file = File('.env');
      if (!file.existsSync()) return null;
      return _fromMap(parse(await file.readAsString()));
    } catch (e) {
      AppLogger.w('.env 文件读取失败', e);
      return null;
    }
  }

  static AuthSecrets _fromMap(Map<String, String> map) {
    return AuthSecrets(
      clientId: map[AuthConstants.envClientIdKey] ?? '',
      clientSecret: map[AuthConstants.envClientSecretKey] ?? '',
    );
  }
}
