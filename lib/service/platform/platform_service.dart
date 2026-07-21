import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/service/platform/launch_at_login.dart';
import 'package:petal_link/service/sync/cloud_tree.dart';

/// macOS 平台服务
///
/// 严格对齐 Rust 原版 `src/commands/platform.rs` + `src/platform/`
/// 的平台命令面：
/// - `openInFinder` / `revealInFinder`：Finder 集成（`open` / `open -R`）
/// - `launchAtLoginIsEnabled` / `setLaunchAtLoginEnabled`：开机启动
///   （委托 [LaunchAtLoginService]，LaunchAgent plist + launchctl）
/// - `getFreeSpace` / `getInodeInfo`：statfs / lstat（原生 MethodChannel）
/// - `logsList` / `logsExport` / `logsClear`：日志命令面
/// - `appClearCache` + `relaunchApp`：全量清理并重启
/// - 激活策略切换（regular/accessory）与 `--hidden` 启动检测
class PlatformService {
  /// 平台通道名（与原生侧约定一致，复用 xattr 通道）
  static const MethodChannel _defaultChannel =
      MethodChannel('com.petallink/platform');

  final MethodChannel _channel;
  final ProcRunner _runner;
  final LaunchAtLoginService _launchAtLogin;

  /// appClearCache：停止同步运行时（引擎/挂载管理器）
  final Future<void> Function()? _teardownRuntime;

  /// appClearCache：清除登录态（token.bin）
  final Future<void> Function()? _clearAuth;

  /// appClearCache：清业务表并删除数据库文件
  final Future<void> Function()? _clearDatabase;

  /// appClearCache：清安全存储中的敏感配置
  final Future<void> Function()? _clearSecureConfig;

  /// 当前是否处于 accessory 模式（对齐 Rust `IS_ACCESSORY`）
  bool _isAccessory = false;

  PlatformService({
    LaunchAtLoginService? launchAtLogin,
    MethodChannel? channel,
    ProcRunner? runner,
    Future<void> Function()? onTeardownRuntime,
    Future<void> Function()? onClearAuth,
    Future<void> Function()? onClearDatabase,
    Future<void> Function()? onClearSecureConfig,
  })  : _launchAtLogin = launchAtLogin ?? LaunchAtLoginService(),
        _channel = channel ?? _defaultChannel,
        _runner = runner ?? defaultProcRunner,
        _teardownRuntime = onTeardownRuntime,
        _clearAuth = onClearAuth,
        _clearDatabase = onClearDatabase,
        _clearSecureConfig = onClearSecureConfig;

  // ═══════════════════════════════════════════════════════════════════
  // Finder 集成
  // ═══════════════════════════════════════════════════════════════════

  /// 在 Finder 中打开路径（`open <path>`）。
  ///
  /// 对齐 Rust `open_in_finder`：目录在 Finder 窗口打开，文件用默认
  /// 应用打开。
  Future<AppResult<void>> openInFinder(String path) async {
    try {
      final r = await _runner('open', [path]);
      if (r.exitCode != 0) {
        return Err(GenericError(message: '打开 Finder 失败：${r.stderr.trim()}'));
      }
      return const Ok(null);
    } catch (e, st) {
      AppLogger.e('openInFinder 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 在 Finder 中显示并选中文件（`open -R <path>`）。
  Future<AppResult<void>> revealInFinder(String path) async {
    try {
      final r = await _runner('open', ['-R', path]);
      if (r.exitCode != 0) {
        return Err(
            GenericError(message: '在 Finder 中显示失败：${r.stderr.trim()}'));
      }
      return const Ok(null);
    } catch (e, st) {
      AppLogger.e('revealInFinder 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 开机启动（对齐 launch_at_login_is_enabled / set_enabled）
  // ═══════════════════════════════════════════════════════════════════

  /// 开机启动是否已启用（仅判断 plist 存在）
  bool launchAtLoginIsEnabled() => _launchAtLogin.isEnabled();

  /// 设置开机启动；失败仅记录日志并返回 false（对齐 Rust 不抛错语义）
  Future<bool> setLaunchAtLoginEnabled(bool enabled) =>
      _launchAtLogin.setEnabled(enabled);

  // ═══════════════════════════════════════════════════════════════════
  // statfs / inode（原生通道；Rust 版无对应命令，为 Flutter 侧新增）
  // ═══════════════════════════════════════════════════════════════════

  /// 获取路径所在卷的可用空间（字节，statfs `f_bavail * f_bsize`）
  Future<AppResult<int>> getFreeSpace(String path) async {
    try {
      final bytes = await _channel
          .invokeMethod<int>('getFreeSpace', {'path': path});
      if (bytes == null) {
        return const Err(GenericError(message: '获取磁盘剩余空间失败'));
      }
      return Ok(bytes);
    } on PlatformException catch (e) {
      return Err(GenericError(message: '获取磁盘剩余空间失败：${e.message}'));
    } catch (e, st) {
      AppLogger.e('getFreeSpace 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 获取文件 inode 与元数据（lstat；不跟随符号链接）。
  ///
  /// 返回键：`ino` / `dev` / `mode` / `nlink` / `size` / `mtimeMs`。
  Future<AppResult<Map<String, dynamic>>> getInodeInfo(String path) async {
    try {
      final info = await _channel.invokeMapMethod<String, dynamic>(
          'getInodeInfo', {'path': path});
      if (info == null) {
        return const Err(GenericError(message: '获取 inode 信息失败'));
      }
      return Ok(info);
    } on PlatformException catch (e) {
      return Err(GenericError(message: '获取 inode 信息失败：${e.message}'));
    } catch (e, st) {
      AppLogger.e('getInodeInfo 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 激活策略（对齐 src/platform/activation.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 切换到 accessory 模式（隐藏 Dock 图标，仅托盘；对齐 `set_accessory`）
  Future<void> setAccessoryMode() async {
    try {
      await _channel
          .invokeMethod<void>('setActivationPolicy', {'policy': 'accessory'});
      _isAccessory = true;
      AppLogger.d('已切换到 accessory 模式');
    } catch (e) {
      AppLogger.w('切换 accessory 模式失败: $e');
    }
  }

  /// 切换到 regular 模式（恢复 Dock 图标并激活；对齐 `set_regular`）
  Future<void> setRegularMode() async {
    try {
      await _channel
          .invokeMethod<void>('setActivationPolicy', {'policy': 'regular'});
      _isAccessory = false;
      AppLogger.d('已切换到 regular 模式');
    } catch (e) {
      AppLogger.w('切换 regular 模式失败: $e');
    }
  }

  /// 窗口聚焦时若处于 accessory 则恢复 regular
  /// （对齐 `ensure_regular_if_was_accessory`）
  Future<void> ensureRegularIfWasAccessory() async {
    if (_isAccessory) {
      await setRegularMode();
    }
  }

  /// 当前是否 accessory 模式
  bool get isAccessory => _isAccessory;

  /// 是否以 `--hidden` 参数启动（自启静默模式，对齐 Rust
  /// `is_launched_manually` 的判定依据）
  Future<bool> wasLaunchedHidden() async {
    try {
      final args =
          await _channel.invokeMethod<List<Object?>>('getLaunchArgs');
      return (args ?? const []).whereType<String>().contains('--hidden');
    } catch (e) {
      AppLogger.w('读取启动参数失败: $e');
      return false;
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 日志命令面（对齐 logs_list / logs_export / logs_clear）
  // ═══════════════════════════════════════════════════════════════════

  /// 内存环形缓冲日志快照（newest-first，上限 1000 条；对齐 `logs_list`）
  List<LogRecord> logsList() => AppLogger.instance.snapshot();

  /// 导出全部滚动日志到指定路径（对齐 `logs_export`）。
  ///
  /// 拼接 `<support>/logs` 下所有 `PetalLink.log*` 文件（按文件名升序），
  /// 每段带 `===== <完整路径> =====` 头；目录无内容时抛 [GenericError]。
  Future<void> logsExport(String path) async {
    final logDir = await AppPaths.logDir();
    final out = await _concatLogFiles(logDir);
    if (out.isEmpty) {
      throw GenericError(message: '日志目录为空，无可导出内容');
    }
    await File(path).writeAsString(out);
    AppLogger.i('日志已导出: $path');
  }

  /// 拼接日志目录内容（纯逻辑，测试直连）。
  ///
  /// 对齐 Rust：只取文件名以 `PetalLink.log` 开头的文件，按文件名升序，
  /// 内容为 UTF-8 容错解码，保证每段结尾有换行。
  static Future<String> concatLogFiles(Directory logDir) =>
      _concatLogFiles(logDir);

  static Future<String> _concatLogFiles(Directory logDir) async {
    if (!await logDir.exists()) return '';
    final files = await logDir
        .list()
        .where((e) => e is File)
        .cast<File>()
        .where((f) => p.basename(f.path).startsWith('PetalLink.log'))
        .toList();
    files.sort((a, b) => p.basename(a.path).compareTo(p.basename(b.path)));

    final out = StringBuffer();
    for (final file in files) {
      final bytes = await file.readAsBytes();
      if (bytes.isEmpty) continue;
      out.write('===== ${file.path} =====\n');
      out.write(utf8.decode(bytes, allowMalformed: true));
      if (!out.toString().endsWith('\n')) out.write('\n');
    }
    return out.toString();
  }

  /// 清空内存环形缓冲（对齐 `logs_clear`；磁盘滚动日志由保留策略管）
  void logsClear() {
    AppLogger.instance.clearRingBuffer();
    AppLogger.i('日志缓冲已清空');
  }

  // ═══════════════════════════════════════════════════════════════════
  // 全量清理与重启（对齐 app_clear_cache / relaunch）
  // ═══════════════════════════════════════════════════════════════════

  /// 全量清理并重启应用（对齐 Rust `app_clear_cache`）。
  ///
  /// 顺序：停运行时（引擎/挂载）→ 清 token → 清业务表并删库文件 →
  /// 清缓存文件（syncstate_/cloudtree_/changes_cursor_）→ 清安全存储 →
  /// 重启 app。任一步失败则中止并返回错误。
  Future<AppResult<void>> appClearCache() async {
    try {
      if (_teardownRuntime != null) {
        await _teardownRuntime();
        AppLogger.i('清缓存：运行时已停止');
      }
      if (_clearAuth != null) {
        await _clearAuth();
        AppLogger.i('清缓存：登录态已清除');
      }
      if (_clearDatabase != null) {
        await _clearDatabase();
        AppLogger.i('清缓存：数据库已清除');
      }
      await CachePaths.clearAll();
      if (_clearSecureConfig != null) {
        await _clearSecureConfig();
        AppLogger.i('清缓存：敏感配置已清除');
      }
      AppLogger.i('清缓存完成，即将重启应用');
      await relaunchApp();
      return const Ok(null);
    } catch (e, st) {
      AppLogger.e('appClearCache 异常', e, st);
      return Err(GenericError(message: '清除缓存失败：$e'));
    }
  }

  /// 重启应用（对齐 Rust `relaunch`）：
  /// 派生独立子进程 `sleep 0.5; open -n <bundle>` 后立即退出当前进程。
  ///
  /// dev 裸二进制模式（无法定位 .app）仅告警不退出，避免数据已清而
  /// 进程无法恢复。
  Future<void> relaunchApp() async {
    final (bundle, _) =
        LaunchAtLoginService.resolvePaths(Platform.resolvedExecutable);
    if (bundle == null) {
      AppLogger.w('dev 裸二进制模式，跳过自动重启（请手动重启应用）');
      return;
    }
    AppLogger.i('重启应用: $bundle');
    // 脱离当前进程：等本进程退出后由 launchd 语义重新打开 .app
    await Process.start(
      '/bin/sh',
      ['-c', 'sleep 0.5; open -n "$bundle"'],
      mode: ProcessStartMode.detached,
    );
    exit(0);
  }
}
