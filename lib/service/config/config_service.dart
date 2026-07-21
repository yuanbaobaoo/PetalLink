import 'dart:convert';
import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/config_entry.dart';

/// 配置服务
///
/// 底层为 config 表键值存储（通用项）+ FlutterSecureStorage（敏感项）；
/// 上层对齐 Rust 原版 `src/commands/config.rs` 的命令面：
/// - [configLoad]：装配 [AppConfig]（缺失字段取默认，对齐 serde default）
/// - [configSave]：校验 + 落盘；挂载目录变更/取消配置 → 清运行时与缓存
///   并重启 app；首次配置 → 启动引擎（均经构造注入回调编排）
/// - [configExportJson]：camelCase 键的 pretty JSON（不含 token）
/// - [configImportJson]：仅解析 + 校验，不回写
class ConfigService {
  final DatabaseService _db;
  final FlutterSecureStorage _storage;

  /// 引擎配置变更回调（首次配置/常规保存 → SyncService.onMountConfigChanged）
  final Future<void> Function() _engineConfigChanged;

  /// 挂载目录变更回调（停运行时 + 清两侧缓存 + 重启 app）
  final Future<void> Function(String? oldAbs, String? newAbs) _mountDirChanged;

  ConfigService(
    this._db,
    this._storage, {
    Future<void> Function()? onEngineConfigChanged,
    Future<void> Function(String? oldAbs, String? newAbs)? onMountDirChanged,
  })  : _engineConfigChanged = onEngineConfigChanged ?? _noopCallback,
        _mountDirChanged = onMountDirChanged ?? _noopDirCallback;

  /// 默认空回调（未注入编排时降级为仅落盘）
  static Future<void> _noopCallback() async {}
  static Future<void> _noopDirCallback(String? oldAbs, String? newAbs) async {}

  // ═══════════════════════════════════════════════════════════════════
  // 底层键值原语（现有调用方保持兼容）
  // ═══════════════════════════════════════════════════════════════════

  /// 保存配置值
  ///
  /// [secure]=true 时写入安全存储（密钥、token 等），否则写入 SQLite。
  Future<void> set(String key, String value, {bool secure = false}) async {
    if (secure) {
      await _storage.write(key: key, value: value);
    } else {
      final db = await _db.database;
      await db.rawInsert(
        'INSERT OR REPLACE INTO config(key, value) VALUES(?, ?)',
        [key, value],
      );
    }
  }

  /// 获取配置值
  ///
  /// [secure]=true 时从安全存储读取，否则从 SQLite 读取。
  Future<String?> get(String key, {bool secure = false}) async {
    if (secure) {
      return _storage.read(key: key);
    } else {
      final db = await _db.database;
      final rows = await db.query(
        'config',
        columns: ['value'],
        where: 'key = ?',
        whereArgs: [key],
        limit: 1,
      );
      if (rows.isEmpty) return null;
      return rows.first['value'] as String?;
    }
  }

  /// 移除配置值
  Future<void> remove(String key, {bool secure = false}) async {
    if (secure) {
      await _storage.delete(key: key);
    } else {
      final db = await _db.database;
      await db.delete('config', where: 'key = ?', whereArgs: [key]);
    }
  }

  /// 服务器地址
  Future<String?> get serverUrl => get('server_url');

  /// 本地挂载/同步目录
  Future<String?> get mountPath => get('mount_path');

  /// OAuth client_id（安全存储）
  Future<String?> get clientId => get('client_id', secure: true);

  /// OAuth client_secret（安全存储）
  Future<String?> get clientSecret => get('client_secret', secure: true);

  /// 获取所有配置（用于调试/导出）
  Future<Map<String, String>> getAll() async {
    final db = await _db.database;
    final rows = await db.query('config');
    final result = <String, String>{};
    for (final row in rows) {
      final key = row['key'] as String?;
      final value = row['value'] as String?;
      if (key != null && value != null) {
        result[key] = value;
      }
    }
    return result;
  }

  // ═══════════════════════════════════════════════════════════════════
  // Rust 命令面：config_load / config_save
  // ═══════════════════════════════════════════════════════════════════

  /// 装配应用配置（对齐 `config_load`；缺失字段取 [AppConfig] 默认值）
  Future<AppConfig> configLoad() async {
    final all = await getAll();
    final mountDir = all['mount_path'] ?? '';
    return AppConfig(
      oauthRedirectUri: all['oauth_redirect_uri'] ??
          const AppConfig().oauthRedirectUri,
      oauthCallbackPort: _parseInt(all['oauth_callback_port']) ??
          AppConfig.defaultCallbackPort,
      mountDir: mountDir,
      // 用户显式配置过目录即视为已确认（对齐 F-MOUNT-13 语义）
      mountConfigured: mountDir.isNotEmpty,
      concurrency:
          _parseInt(all['concurrency']) ?? const AppConfig().concurrency,
      pollIntervalSec: _parseInt(all['poll_interval_sec']) ??
          const AppConfig().pollIntervalSec,
      debounceSec:
          _parseInt(all['debounce_sec']) ?? const AppConfig().debounceSec,
      skipPatterns: _parsePatterns(all['skip_patterns']),
      sortField:
          SortField.fromWireName(all['sort_field']) ?? SortField.Name,
      sortOrder:
          SortOrder.fromWireName(all['sort_order']) ?? SortOrder.Ascending,
    );
  }

  /// 保存应用配置（对齐 `config_save`）。
  ///
  /// 1. 校验（[AppConfig.validate] + 挂载目录可写探测）
  /// 2. 落盘 config 表
  /// 3. 分支（对齐 Rust）：
  ///    - 取消配置或目录变更 → 停运行时 + 清两侧缓存 + 重启 app
  ///    - 首次配置 → 启动引擎；其他常规修改 → 引擎按新配置刷新
  ///
  /// 校验失败抛 [ConfigError]。
  Future<void> configSave(AppConfig config) async {
    final invalid = config.validate();
    if (invalid != null) throw invalid;

    // 挂载目录可写探测（对齐 Rust validate_configured_mount_dir_access）
    if (config.mountConfigured) {
      await _probeMountDirWritable(config.expandedMountDir);
    }

    final old = await configLoad();
    final oldAbs = old.mountConfigured ? old.expandedMountDir : null;
    final newAbs = config.mountConfigured ? config.expandedMountDir : null;
    final dirChanged = old.mountConfigured &&
        config.mountConfigured &&
        oldAbs != newAbs;

    // 落盘
    await _persistConfig(config);
    AppLogger.i('配置已保存（挂载目录: ${config.mountConfigured ? config.mountDir : '未配置'}）');

    // 分支编排
    if (old.mountConfigured && (!config.mountConfigured || dirChanged)) {
      AppLogger.w('挂载目录变更或取消配置：清理运行时与缓存并重启');
      await _mountDirChanged(oldAbs, newAbs);
      return;
    }
    // 首次配置 → 启动引擎；常规修改 → 引擎按新配置刷新
    await _engineConfigChanged();
  }

  /// 持久化到 config 表（snake_case 键）
  Future<void> _persistConfig(AppConfig config) async {
    await set('oauth_redirect_uri', config.oauthRedirectUri);
    await set('oauth_callback_port', '${config.oauthCallbackPort}');
    if (config.mountConfigured) {
      await set('mount_path', config.mountDir);
    } else {
      await remove('mount_path');
    }
    await set('concurrency', '${config.concurrency}');
    await set('poll_interval_sec', '${config.pollIntervalSec}');
    await set('debounce_sec', '${config.debounceSec}');
    await set('skip_patterns', config.skipPatterns.join(','));
    await set('sort_field', config.sortField.wireName);
    await set('sort_order', config.sortOrder.wireName);
  }

  /// 挂载目录可写探测：create_dir_all + 写测试文件再删
  /// （对齐 Rust `validate_configured_mount_dir_access`）
  static Future<void> _probeMountDirWritable(String expandedDir) async {
    try {
      await Directory(expandedDir).create(recursive: true);
      final probe = File(
          p.join(expandedDir, '.petallink-write-test-$pid'));
      await probe.writeAsString('');
      await probe.delete();
    } catch (e) {
      throw ConfigError(message: '同步目录不可写：$expandedDir（$e）');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // Rust 命令面：config_export_json / config_import_json
  // ═══════════════════════════════════════════════════════════════════

  /// 导出配置为 camelCase 键的 pretty JSON（对齐 `config_export_json`，
  /// 不含 token）
  Future<String> configExportJson() async {
    final config = await configLoad();
    const encoder = JsonEncoder.withIndent('  ');
    return encoder.convert(_toCamelJson(config));
  }

  /// 解析并校验导入的配置 JSON（对齐 `config_import_json` 的校验段；
  /// 仅校验不回写）。
  ///
  /// 合法返回解析出的 [AppConfig]；非法抛 [ConfigError]。
  AppConfig configImportJson(String jsonStr) {
    final Object? decoded;
    try {
      decoded = jsonDecode(jsonStr);
    } catch (e) {
      throw ConfigError(message: '配置 JSON 解析失败：$e');
    }
    if (decoded is! Map<String, dynamic>) {
      throw const ConfigError(message: '配置 JSON 必须是对象');
    }
    final config = AppConfig.fromJson(_fromCamelJson(decoded));
    final invalid = config.validate();
    if (invalid != null) throw invalid;
    return config;
  }

  // ============================================================
  // 内部：解析与序列化
  // ============================================================

  /// camelCase 导出（对齐 Rust export_to_json 键名）
  static Map<String, dynamic> _toCamelJson(AppConfig config) {
    return {
      'oauthRedirectUri': config.oauthRedirectUri,
      'oauthCallbackPort': config.oauthCallbackPort,
      'mountDir': config.mountDir,
      'mountConfigured': config.mountConfigured,
      'concurrency': config.concurrency,
      'pollIntervalSec': config.pollIntervalSec,
      'debounceSec': config.debounceSec,
      'skipPatterns': config.skipPatterns,
      'sortField': config.sortField.wireName,
      'sortOrder': config.sortOrder.wireName,
    };
  }

  /// camelCase 导入映射为 AppConfig.fromJson 的 snake_case 键
  static Map<String, dynamic> _fromCamelJson(Map<String, dynamic> json) {
    return {
      if (json.containsKey('oauthRedirectUri'))
        'oauth_redirect_uri': json['oauthRedirectUri'],
      if (json.containsKey('oauthCallbackPort'))
        'oauth_callback_port': json['oauthCallbackPort'],
      if (json.containsKey('mountDir')) 'mount_dir': json['mountDir'],
      if (json.containsKey('mountConfigured'))
        'mount_configured': json['mountConfigured'],
      if (json.containsKey('concurrency')) 'concurrency': json['concurrency'],
      if (json.containsKey('pollIntervalSec'))
        'poll_interval_sec': json['pollIntervalSec'],
      if (json.containsKey('debounceSec'))
        'debounce_sec': json['debounceSec'],
      if (json.containsKey('skipPatterns'))
        'skip_patterns': json['skipPatterns'],
      if (json.containsKey('sortField')) 'sort_field': json['sortField'],
      if (json.containsKey('sortOrder')) 'sort_order': json['sortOrder'],
    };
  }

  static int? _parseInt(String? raw) =>
      raw == null ? null : int.tryParse(raw.trim());

  static List<String> _parsePatterns(String? raw) {
    if (raw == null) return AppConfig.defaultSkipPatterns;
    final list =
        raw.split(',').map((s) => s.trim()).where((s) => s.isNotEmpty).toList();
    return list.isEmpty ? AppConfig.defaultSkipPatterns : list;
  }
}
