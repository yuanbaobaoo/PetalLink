import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';

/// 进程执行结果（可注入抽象，对齐 dart:io ProcessResult 关键字段）
class ProcResult {
  /// 退出码
  final int exitCode;

  /// 标准输出
  final String stdout;

  /// 标准错误
  final String stderr;

  const ProcResult({
    required this.exitCode,
    this.stdout = '',
    this.stderr = '',
  });
}

/// 进程执行器（测试注入 fake；生产为 [defaultProcRunner]）
typedef ProcRunner = Future<ProcResult> Function(String exe, List<String> args);

/// 生产进程执行器（薄封装 [Process.run]）
Future<ProcResult> defaultProcRunner(String exe, List<String> args) async {
  final r = await Process.run(exe, args);
  return ProcResult(
    exitCode: r.exitCode,
    stdout: '${r.stdout}',
    stderr: '${r.stderr}',
  );
}

/// 开机启动服务 —— LaunchAgent 管理。
///
/// 严格对齐 Rust 原版 `src/platform/launch_at_login.rs`：
/// - plist 路径 `~/Library/LaunchAgents/<bundle-id>.plist`
/// - ProgramArguments 固定带 `--hidden`（后台静默启动）
/// - 启用：旧 plist 先 bootout → 写 plist → launchctl bootstrap
/// - 禁用：launchctl bootout → 删 plist
/// - is_enabled 仅判断 plist 文件存在（不验证 bootstrap 状态）
/// - 启用/禁用时经 osascript 移除 Login Items 同名项（防双重启动）
class LaunchAtLoginService {
  /// 进程执行器（launchctl / osascript / id）
  final ProcRunner _runner;

  /// 用户 home 目录（测试注入临时目录）
  final String? _homeDirOverride;

  /// bundle identifier（测试注入；默认按构建模式对齐 xcconfig）
  final String? _bundleIdOverride;

  /// 当前可执行文件路径（测试注入；默认 [Platform.resolvedExecutable]）
  final String? _executablePathOverride;

  LaunchAtLoginService({
    ProcRunner? runner,
    String? homeDir,
    String? bundleId,
    String? executablePath,
  })  : _runner = runner ?? defaultProcRunner,
        _homeDirOverride = homeDir,
        _bundleIdOverride = bundleId,
        _executablePathOverride = executablePath;

  /// bundle identifier（Debug 用 -dev 后缀，对齐 xcconfig 与 Rust dev conf）
  String get bundleId =>
      _bundleIdOverride ??
      (kDebugMode
          ? '${AppPaths.bundleIdentifier}-dev'
          : AppPaths.bundleIdentifier);

  /// plist 完整路径：`~/Library/LaunchAgents/<bundle-id>.plist`
  String get plistPath {
    final home = _homeDirOverride ?? Platform.environment['HOME'] ?? '';
    return p.join(home, 'Library', 'LaunchAgents', '$bundleId.plist');
  }

  /// 是否已启用（仅判断 plist 文件存在，对齐 Rust `is_enabled`）
  bool isEnabled() => File(plistPath).existsSync();

  /// 设置开机启动；失败仅记录日志并返回 false（对齐 Rust 不抛错语义）
  Future<bool> setEnabled(bool enabled) async {
    try {
      if (enabled) {
        await _enable();
      } else {
        await _disable();
      }
      return true;
    } catch (e, st) {
      AppLogger.e('设置开机启动失败: $enabled', e, st);
      return false;
    }
  }

  // ============================================================
  // 内部实现（对齐 Rust enable / disable）
  // ============================================================

  Future<void> _enable() async {
    final uid = await _currentUid();
    // 旧 plist 存在先 bootout（忽略失败）
    if (File(plistPath).existsSync()) {
      await _bootout(uid);
    }

    final (bundlePath, programPath) = resolvePaths(
      _executablePathOverride ?? Platform.resolvedExecutable,
    );

    final file = File(plistPath);
    await file.parent.create(recursive: true);
    await file.writeAsString(buildPlist(
      label: bundleId,
      bundlePath: bundlePath,
      programPath: programPath,
    ));
    AppLogger.i('LaunchAgent plist 已写入: $plistPath');

    // bootstrap；已加载视为成功，其余失败仅告警（plist 已写，下次登录生效）
    final r = await _runner('launchctl', ['bootstrap', 'gui/$uid', plistPath]);
    if (r.exitCode != 0) {
      final stderr = r.stderr;
      if (stderr.contains('already bootstrapped') ||
          stderr.contains('service already loaded')) {
        AppLogger.d('LaunchAgent 已加载，忽略 bootstrap 报错');
      } else {
        AppLogger.w('launchctl bootstrap 失败（下次登录生效）: $stderr');
      }
    }

    await removeFromLoginItems();
  }

  Future<void> _disable() async {
    final uid = await _currentUid();
    await _bootout(uid);
    final file = File(plistPath);
    if (file.existsSync()) {
      await file.delete();
      AppLogger.i('LaunchAgent plist 已删除');
    }
    await removeFromLoginItems();
  }

  /// bootout（忽略失败；服务未加载时 launchctl 返回非零属正常）
  Future<void> _bootout(int uid) async {
    try {
      await _runner('launchctl', ['bootout', 'gui/$uid/$bundleId']);
    } catch (_) {
      // 忽略：服务可能不存在
    }
  }

  /// 当前 uid（`id -u`；失败兜底 501，对齐 Rust `current_uid`）
  Future<int> _currentUid() async {
    try {
      final r = await _runner('id', ['-u']);
      if (r.exitCode == 0) {
        return int.tryParse(r.stdout.trim()) ?? 501;
      }
    } catch (_) {
      // 兜底
    }
    return 501;
  }

  /// 移除 Login Items 同名项（防「LaunchAgent + Login Items」双重启动；
  /// 失败静默，对齐 Rust `remove_from_login_items`）
  Future<void> removeFromLoginItems() async {
    try {
      await _runner('osascript', [
        '-e',
        'tell application "System Events" to delete every login item '
            'whose name is "PetalLink"',
      ]);
    } catch (_) {
      // 静默
    }
  }

  // ============================================================
  // 纯函数（可测试）
  // ============================================================

  /// 解析可执行路径为 (bundlePath, programPath)。
  ///
  /// 对齐 Rust `resolve_paths`：可执行文件上溯三级且扩展名为 `.app`
  /// 视为 bundle 模式（programPath 为 bundle 内相对路径
  /// `Contents/MacOS/<exe名>`），否则为 dev 裸二进制模式（programPath
  /// 为可执行文件绝对路径，bundlePath 为 null）。
  static (String?, String) resolvePaths(String executablePath) {
    // exe = <App>.app/Contents/MacOS/<exe名> → 上溯三级 = <App>.app
    final bundle = p.dirname(p.dirname(p.dirname(executablePath)));
    if (p.extension(bundle) == '.app') {
      final exeName = p.basename(executablePath);
      return (bundle, p.join('Contents', 'MacOS', exeName));
    }
    return (null, executablePath);
  }

  /// 生成 LaunchAgent plist 内容（对齐 Rust plist 模板）。
  ///
  /// bundle 模式 ProgramArguments 第一项为 `<bundle>/<programPath>`；
  /// dev 裸二进制模式额外注入 PATH 环境变量。`--hidden` 固定第二参数。
  static String buildPlist({
    required String label,
    required String? bundlePath,
    required String programPath,
  }) {
    final program =
        bundlePath != null ? '$bundlePath/$programPath' : programPath;
    final buffer = StringBuffer()
      ..writeln('<?xml version="1.0" encoding="UTF-8"?>')
      ..writeln('<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
          '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">')
      ..writeln('<plist version="1.0">')
      ..writeln('<dict>')
      ..writeln('    <key>Label</key>')
      ..writeln('    <string>$label</string>')
      ..writeln('    <key>ProgramArguments</key>')
      ..writeln('    <array>')
      ..writeln('        <string>$program</string>')
      ..writeln('        <string>--hidden</string>')
      ..writeln('    </array>');
    if (bundlePath == null) {
      buffer
        ..writeln('    <key>EnvironmentVariables</key>')
        ..writeln('    <dict>')
        ..writeln('        <key>PATH</key>')
        ..writeln('        <string>/usr/bin:/bin:/usr/sbin:/sbin'
            ':/usr/local/bin</string>')
        ..writeln('    </dict>');
    }
    buffer
      ..writeln('    <key>RunAtLoad</key>')
      ..writeln('    <true/>')
      ..writeln('    <key>KeepAlive</key>')
      ..writeln('    <false/>')
      ..writeln('</dict>')
      ..writeln('</plist>');
    return buffer.toString();
  }
}
