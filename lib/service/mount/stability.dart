/// 稳定性检查器 —— mtime > 5s + size 稳定 3s + lsof 双重检查。
///
/// 严格对齐 Rust 原版 `src/sync/stability.rs`。
///
/// 三阶段检查（F-MOUNT-10 / §2.8 第二阶段）：
/// 1. mtime 距今 > 5s（编辑已停止至少 5 秒）
/// 2. size 在 3s 窗口内不变（文件大小稳定）
/// 3. lsof 无进程以写模式打开（+ 双重检查 + 只读系统进程白名单）
///
/// 特殊场景（F-MOUNT-11）：持续编辑 > 5min → 标记「用户编辑中」暂停自动同步。
///
/// 时钟、睡眠与 lsof 均可注入（测试确定性）。
library;

import 'dart:async';
import 'dart:io';

/// 稳定性检查结果
enum StabilityResult {
  /// 文件稳定，可以传输
  stable,

  /// 文件仍在变化中（不稳定，延迟到下一周期）
  unstable,

  /// 有进程正在编辑（用户编辑中，标记暂停）
  editing,
}

/// 稳定性检查器
class StabilityChecker {
  /// 稳定性检查最低 mtime 滞后（秒）
  static const int minMtimeAgeSecs = 5;

  /// 大小稳定窗口（秒）
  static const int sizeStableWindowSecs = 3;

  /// 编辑阈值（秒）：超过此阈值 → 标记「用户编辑中」
  static const int editingThresholdSecs = 300; // 5 分钟

  /// lsof 双重检查间隔（秒）
  static const int lsofRecheckSecs = 1;

  /// lsof 只读系统进程白名单（对齐 Rust READONLY_PROCESSES）
  static const List<String> readOnlyProcesses = [
    'mds',
    'mdworker_shared',
    'mdimport',
    'mdflagworker',
    'QuickLookSatellite',
    'qlmanage',
    'corespotlightd',
    'secd',
    'bird',
    'CoreServicesUIAgent',
  ];

  /// 当前毫秒时钟（测试注入）
  final int Function() _nowMs;

  /// 睡眠（测试注入）
  final Future<void> Function(Duration) _sleep;

  /// lsof 命令名采集（测试注入；返回以写/读模式打开目标文件的进程命令名列表）
  final Future<List<String>> Function(String path) _lsofCommands;

  /// 追踪持续编辑的文件（path → 首次发现时间毫秒）
  final Map<String, int> _tracking = {};

  StabilityChecker({
    int Function()? nowMs,
    Future<void> Function(Duration)? sleep,
    Future<List<String>> Function(String path)? lsofCommands,
  })  : _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch),
        _sleep = sleep ?? Future.delayed,
        _lsofCommands = lsofCommands ?? defaultLsofCommands;

  /// 检查文件是否稳定（可传输）。
  /// 对齐 Rust `StabilityChecker::check`。
  Future<StabilityResult> check(String path) async {
    // 1. mtime 年龄检查
    final int mtimeAge;
    try {
      final stat = await FileStat.stat(path);
      // dart:io FileStat.stat 对缺失文件不抛异常而是返回 notFound 类型
      if (stat.type == FileSystemEntityType.notFound) {
        return StabilityResult.unstable;
      }
      final ageMs = _nowMs() - stat.modified.millisecondsSinceEpoch;
      // saturating_sub 语义：未来 mtime 视为 0 秒
      mtimeAge = ageMs <= 0 ? 0 : ageMs ~/ 1000;
    } catch (_) {
      return StabilityResult.unstable;
    }
    if (mtimeAge < minMtimeAgeSecs) {
      return StabilityResult.unstable;
    }

    // 2. 大小稳定性检查（3s 窗口）
    final size1 = await _fileSize(path);
    await _sleep(const Duration(seconds: sizeStableWindowSecs));
    final size2 = await _fileSize(path);
    if (size1 != size2) {
      // 对齐 Rust：size 不稳定时也检查编辑阈值（>5min 升级为 editing）
      return _trackUnstable(path);
    }

    // 3. lsof 检查
    if (await _isFileBusy(path)) {
      // 双重检查（1s 后重查，消除 Spotlight/QuickLook 误报）
      await _sleep(const Duration(seconds: lsofRecheckSecs));
      if (await _isFileBusy(path)) {
        return _trackUnstable(path);
      }
    }

    // 之前可能在 tracking 中（现在已稳定，移除追踪）
    _tracking.remove(path);
    return StabilityResult.stable;
  }

  /// 清除某路径的追踪状态（文件已被删除/不再同步时调用）。
  void clearTracking(String path) {
    _tracking.remove(path);
  }

  /// 记录首次发现时间并判定是否升级为「用户编辑中」。
  StabilityResult _trackUnstable(String path) {
    final now = _nowMs();
    final firstSeen = _tracking.putIfAbsent(path, () => now);
    final elapsedSecs = (now - firstSeen) ~/ 1000;
    if (elapsedSecs > editingThresholdSecs) {
      return StabilityResult.editing;
    }
    return StabilityResult.unstable;
  }

  /// 文件大小（字节）；读取失败或文件缺失返回 null（对齐 Rust Option 语义，
  /// 两次均失败视为「大小一致」进入 lsof 阶段）。
  Future<int?> _fileSize(String path) async {
    try {
      final stat = await FileStat.stat(path);
      if (stat.type == FileSystemEntityType.notFound) return null;
      return stat.size;
    } catch (_) {
      return null;
    }
  }

  /// lsof 检查：是否有非白名单进程打开文件。
  /// （白名单判定对齐 Rust：全部命令均在只读白名单内 → 不判 busy）
  Future<bool> _isFileBusy(String path) async {
    final commands = await _lsofCommands(path);
    if (commands.isEmpty) return false;
    if (commands.every(readOnlyProcesses.contains)) return false;
    return true;
  }

  /// 默认 lsof 采集：`lsof -nP -F pc <path>`，解析 c 行（command）。
  ///
  /// 进程退出码非 0 / 启动失败均返回空列表（视为无占用，
  /// 对齐 Rust `is_file_busy` 的失败分支）。
  static Future<List<String>> defaultLsofCommands(String path) async {
    final ProcessResult result;
    try {
      result = await Process.run('lsof', ['-nP', '-F', 'pc', path]);
    } catch (_) {
      return const [];
    }
    if (result.exitCode != 0) return const [];
    final stdout = result.stdout as String;
    // 解析 lsof -F pc 输出：p 行 = pid, c 行 = command
    return stdout
        .split('\n')
        .where((line) => line.startsWith('c'))
        .map((line) => line.substring(1))
        .toList();
  }
}
