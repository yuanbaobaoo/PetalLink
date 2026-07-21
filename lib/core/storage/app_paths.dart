import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

/// 应用数据目录工具（静态方法类）。
///
/// 对齐 Rust 原版 `src/core/config_store.rs` 的 support_dir()：
/// macOS 路径为 `~/Library/Application Support/<bundle id>`。
///
/// dev/release 数据目录隔离（对齐 tauri.dev.conf.json 与 xcconfig）：
/// debug 构建（kDebugMode）→ `io.github.yuanbaobaoo.PetalLink-dev`；
/// release 构建 → `io.github.yuanbaobaoo.PetalLink`。
class AppPaths {
  AppPaths._();

  /// 应用包标识（对齐 Rust constants::BUNDLE_IDENTIFIER 正式版）
  static const String bundleIdentifier = 'io.github.yuanbaobaoo.PetalLink';

  /// dev 版包标识（对齐 tauri.dev.conf.json / Debug.xcconfig）
  static const String devBundleIdentifier = '$bundleIdentifier-dev';

  /// 测试用：覆盖 Application Support 根目录
  @visibleForTesting
  static String? debugSupportRoot;

  /// Application Support 下的 PetalLink 工作目录。
  static Future<Directory> supportDir() async {
    final override = debugSupportRoot;
    if (override != null) {
      return Directory(p.join(override, bundleIdentifier));
    }
    final root = await _applicationSupportRoot();
    // dev/release 隔离：debug 构建一律使用 -dev 数据目录
    final id = kDebugMode ? devBundleIdentifier : bundleIdentifier;
    return Directory(p.join(root, id));
  }

  /// 日志目录：`<support>/logs`（与 DB/config 同目录，不污染同步目录）。
  static Future<Directory> logDir() async {
    return Directory(p.join((await supportDir()).path, 'logs'));
  }

  /// 数据库文件路径：`<support>/petal_link.db`。
  static Future<String> databasePath() async {
    return p.join((await supportDir()).path, 'petal_link.db');
  }

  /// 获取 Application Support 根目录（macOS: ~/Library/Application Support）。
  static Future<String> _applicationSupportRoot() async {
    final appDir = await getApplicationSupportDirectory();
    // getApplicationSupportDirectory 返回 <root>/<bundle id>，取父目录得到根
    return p.dirname(appDir.path);
  }
}
