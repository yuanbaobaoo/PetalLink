import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

/// 应用数据目录工具（静态方法类）。
///
/// 严格对齐 Rust 原版 `src/core/config_store.rs` 的 support_dir()：
/// macOS 路径为 `~/Library/Application Support/io.github.yuanbaobaoo.PetalLink`。
///
/// 注意：不直接使用 getApplicationSupportDirectory() 的返回值 —
/// 它由 bundle id 派生（骨架工程为 com.example.petalLink），
/// 与 Rust 原版数据目录不一致；这里取其父目录（Application Support 根）
/// 再拼接原版 bundle identifier，保证与 Tauri 版共用同一份数据。
class AppPaths {
  AppPaths._();

  /// 应用包标识（对齐 Rust constants::BUNDLE_IDENTIFIER 正式版）
  static const String bundleIdentifier = 'io.github.yuanbaobaoo.PetalLink';

  /// 测试用：覆盖 Application Support 根目录
  @visibleForTesting
  static String? debugSupportRoot;

  /// Application Support 下的 PetalLink 工作目录。
  static Future<Directory> supportDir() async {
    final root = debugSupportRoot ?? await _applicationSupportRoot();
    return Directory(p.join(root, bundleIdentifier));
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
