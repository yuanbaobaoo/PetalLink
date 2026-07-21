import 'dart:io';

import '../core/error/app_error.dart';

/// 同步状态展示排序字段（对齐 Rust `SortField`）
enum SortField {
  /// 按文件名
  Name('name'),

  /// 按大小
  Size('size'),

  /// 按修改时间
  ModifiedTime('modifiedTime');

  /// 序列化值（camelCase，对齐 Rust serde rename_all=camelCase）
  final String wireName;

  const SortField(this.wireName);

  /// 从序列化值解析；未知值返回 null
  static SortField? fromWireName(String? name) {
    if (name == null) return null;
    for (final f in values) {
      if (f.wireName == name) return f;
    }
    return null;
  }
}

/// 列表排序方向（对齐 Rust `SortOrder`）
enum SortOrder {
  /// 升序
  Ascending('ascending'),

  /// 降序
  Descending('descending');

  /// 序列化值（camelCase，对齐 Rust serde rename_all=camelCase）
  final String wireName;

  const SortOrder(this.wireName);

  /// 从序列化值解析；未知值返回 null
  static SortOrder? fromWireName(String? name) {
    if (name == null) return null;
    for (final o in values) {
      if (o.wireName == name) return o;
    }
    return null;
  }
}

/// 应用配置（对齐 Rust `AppConfig`，不可变值对象）。
///
/// 所有可配置项集中在此，不含 token。
/// 持久化为 config 表中的 JSON（对应 Rust config.json）。
class AppConfig {
  /// 默认 OAuth 回调 URI（必须与 AGC 后台配置一致）
  static const String defaultRedirectUri = 'http://127.0.0.1:9999/oauth/callback';

  /// 默认 OAuth 回调端口
  static const int defaultCallbackPort = 9999;

  /// 默认跳过文件列表（通配符，名称匹配）
  static const List<String> defaultSkipPatterns = [
    '.DS_Store',
    '.tmp',
    '~\$*',
    '.Trash',
  ];

  /// OAuth 回调 URI（必须与 AGC 后台一致）
  final String oauthRedirectUri;

  /// OAuth 回调端口
  final int oauthCallbackPort;

  /// 本地挂载目录（可能含 ~ 前缀；空串表示未配置）
  final String mountDir;

  /// 用户是否已显式配置过挂载目录（首次同步引导用，F-MOUNT-13）。
  ///
  /// 区分「默认值」与「用户已确认」，避免未选目录就自动同步覆盖本地已有内容。
  final bool mountConfigured;

  /// 并发传输数，范围 1-20（默认 6）
  final int concurrency;

  /// 云端定时刷新间隔（秒）。0 = 关闭自动刷新；开启时最小 60 秒。
  final int pollIntervalSec;

  /// 变更 debounce 时长（秒，默认 3，F-MOUNT-09）
  final int debounceSec;

  /// 跳过文件列表（通配符）
  final List<String> skipPatterns;

  /// 排序字段
  final SortField sortField;

  /// 排序方向
  final SortOrder sortOrder;

  const AppConfig({
    this.oauthRedirectUri = defaultRedirectUri,
    this.oauthCallbackPort = defaultCallbackPort,
    this.mountDir = '',
    this.mountConfigured = false,
    this.concurrency = 6,
    this.pollIntervalSec = 60,
    this.debounceSec = 3,
    this.skipPatterns = defaultSkipPatterns,
    this.sortField = SortField.Name,
    this.sortOrder = SortOrder.Ascending,
  });

  /// 展开 ~ 为真实 home 路径（对齐 Rust `expanded_mount_dir`）
  String get expandedMountDir {
    if (mountDir.startsWith('~/')) {
      final home = Platform.environment['HOME'] ?? '/';
      return '$home/${mountDir.substring(2)}';
    }
    return mountDir;
  }

  /// 校验配置合法性（范围、非空等），对齐 Rust `validate`。
  ///
  /// 合法返回 null；非法返回 [ConfigError]（中文消息与 Rust 一致）。
  ConfigError? validate() {
    if (oauthCallbackPort < 1 || oauthCallbackPort > 65535) {
      return ConfigError(message: '回调端口越界：$oauthCallbackPort');
    }
    if (concurrency < 1 || concurrency > 20) {
      return ConfigError(message: '并发数必须在 1-20 之间：$concurrency');
    }
    // 云端定时刷新间隔：0 = 关闭；开启时最小 60 秒（防止误设过小拖垮大网盘）
    if (pollIntervalSec != 0 && pollIntervalSec < 60) {
      return ConfigError(
          message: '云端刷新间隔必须为 0（关闭）或 ≥ 60 秒：$pollIntervalSec');
    }
    if (debounceSec < 1) {
      return const ConfigError(message: 'debounce 时长必须 ≥ 1 秒');
    }
    if (mountConfigured) {
      if (mountDir.trim().isEmpty) {
        return const ConfigError(message: '同步目录不能为空');
      }
      final expanded = expandedMountDir;
      if (!expanded.startsWith('/')) {
        return ConfigError(message: '同步目录必须是绝对路径：$mountDir');
      }
      if (expanded == '/') {
        return const ConfigError(message: '不能把系统根目录作为同步目录');
      }
      final home = Platform.environment['HOME'];
      if (home != null && home.isNotEmpty && expanded == home) {
        return const ConfigError(message: '不能把用户 Home 目录作为同步目录');
      }
      if (home != null && home.isNotEmpty) {
        final dataDir = '$home/Library/Application Support';
        if (expanded == dataDir || expanded.startsWith('$dataDir/')) {
          return const ConfigError(
              message: '不能把 Application Support 目录作为同步目录');
        }
      }
    }
    return null;
  }

  /// 从 JSON 构造（snake_case 键；缺失字段取默认值，对齐 Rust serde(default)）
  factory AppConfig.fromJson(Map<String, dynamic> json) {
    const defaults = AppConfig();

    final rawPatterns = json['skip_patterns'];
    final List<String> skipPatterns;
    if (rawPatterns is List) {
      skipPatterns = rawPatterns.map((e) => e.toString()).toList();
    } else {
      skipPatterns = defaults.skipPatterns;
    }

    return AppConfig(
      oauthRedirectUri:
          json['oauth_redirect_uri'] as String? ?? defaults.oauthRedirectUri,
      oauthCallbackPort: _tolerantInt(json['oauth_callback_port']) ??
          defaults.oauthCallbackPort,
      mountDir: json['mount_dir'] as String? ?? defaults.mountDir,
      mountConfigured: json['mount_configured'] == true,
      concurrency: _tolerantInt(json['concurrency']) ?? defaults.concurrency,
      pollIntervalSec:
          _tolerantInt(json['poll_interval_sec']) ?? defaults.pollIntervalSec,
      debounceSec: _tolerantInt(json['debounce_sec']) ?? defaults.debounceSec,
      skipPatterns: skipPatterns,
      sortField:
          SortField.fromWireName(json['sort_field'] as String?) ??
              defaults.sortField,
      sortOrder:
          SortOrder.fromWireName(json['sort_order'] as String?) ??
              defaults.sortOrder,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'oauth_redirect_uri': oauthRedirectUri,
      'oauth_callback_port': oauthCallbackPort,
      'mount_dir': mountDir,
      'mount_configured': mountConfigured,
      'concurrency': concurrency,
      'poll_interval_sec': pollIntervalSec,
      'debounce_sec': debounceSec,
      'skip_patterns': skipPatterns,
      'sort_field': sortField.wireName,
      'sort_order': sortOrder.wireName,
    };
  }

  /// 深拷贝并替换指定字段（对齐 Rust `with` 链式构造）
  AppConfig copyWith({
    String? oauthRedirectUri,
    int? oauthCallbackPort,
    String? mountDir,
    bool? mountConfigured,
    int? concurrency,
    int? pollIntervalSec,
    int? debounceSec,
    List<String>? skipPatterns,
    SortField? sortField,
    SortOrder? sortOrder,
  }) {
    return AppConfig(
      oauthRedirectUri: oauthRedirectUri ?? this.oauthRedirectUri,
      oauthCallbackPort: oauthCallbackPort ?? this.oauthCallbackPort,
      mountDir: mountDir ?? this.mountDir,
      mountConfigured: mountConfigured ?? this.mountConfigured,
      concurrency: concurrency ?? this.concurrency,
      pollIntervalSec: pollIntervalSec ?? this.pollIntervalSec,
      debounceSec: debounceSec ?? this.debounceSec,
      skipPatterns: skipPatterns ?? this.skipPatterns,
      sortField: sortField ?? this.sortField,
      sortOrder: sortOrder ?? this.sortOrder,
    );
  }
}

/// 容忍解析 int：接受 int / num / String（int 字段兼容 "123"）。
int? _tolerantInt(Object? v) {
  if (v is int) return v;
  if (v is num) return v.toInt();
  if (v is String) return int.tryParse(v.trim());
  return null;
}
