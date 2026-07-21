import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'package:petal_link/core/logger/logger.dart';

/// 稳定网络状态发生的真实转换。
enum NetworkTransition {
  /// 离线 → 在线
  online,

  /// 在线 → 离线
  offline,
}

/// 网络守卫（单例）—— 断网/睡眠时暂停一切同步操作。
///
/// 严格对齐 Rust 原版 `src/core/net_guard.rs`：
/// - 维护全局 Online/Offline 状态，同步引擎各入口通过 [isOnline] 快速查询；
///   定时器循环通过 [waitUntilOnline] 阻塞等待网络恢复
/// - 网络判定：每 30s 向华为 API 域名做轻量 TCP connect 探测（443 端口，3s 超时）
/// - 迟滞切换：一次探测失败即离线；恢复在线要求连续两次探测成功，
///   避免弱网抖动导致传输队列频繁启停
///
/// 睡眠/唤醒采用纯探测降级方案（无 NSWorkspace 通知监听）：
/// 睡眠期间 TCP 探测必然超时 → 自动离线；唤醒后探测恢复 → 在线。
/// 即时性损失最多 30s（一个探测周期）。
class NetGuard {
  NetGuard._internal();

  /// 单例
  factory NetGuard() => _instance;
  static final NetGuard _instance = NetGuard._internal();

  /// 单例实例（等价于 [NetGuard()] 构造）
  static NetGuard get instance => _instance;

  /// 探测目标主机（华为 Drive API 域名）
  static const String probeHost = 'driveapis.cloud.huawei.com.cn';

  /// 探测目标端口
  static const int probePort = 443;

  /// 探测间隔
  static const Duration probeInterval = Duration(seconds: 30);

  /// 单次探测超时
  static const Duration probeTimeout = Duration(seconds: 3);

  /// waitUntilOnline 轮询间隔
  static const Duration pollWait = Duration(seconds: 2);

  /// 恢复在线所需的连续成功次数（迟滞）
  static const int _requiredSuccesses = 2;

  /// 当前稳定在线状态
  bool _online = true;

  /// 连续探测成功次数（离线恢复计数）
  int _consecutiveSuccesses = 0;

  /// 探测任务是否运行中
  bool _running = false;

  /// 探测代次：start 递增，用于拒绝陈旧任务回写
  int _generation = 0;

  /// 稳定状态转换广播
  final StreamController<NetworkTransition> _transitions =
      StreamController<NetworkTransition>.broadcast();

  /// 探测函数（默认 TCP connect，可注入便于测试）
  Future<bool> Function() _probe = _tcpProbe;

  /// 探测间隔（可注入便于测试）
  Duration _interval = probeInterval;

  /// 查询当前是否在线（零开销，供同步引擎各入口快速判断）。
  bool get isOnline => _online;

  /// 探测任务是否运行中。
  bool get isRunning => _running;

  /// 订阅稳定网络转换；只会收到真实 offline/online 状态变化。
  Stream<NetworkTransition> get transitions => _transitions.stream;

  /// 启动后台探测任务（幂等，重复调用安全）。
  void start() {
    if (_running) return;
    _running = true;
    _generation++;
    final generation = _generation;
    // 重置迟滞计数，以当前状态为新基线
    _consecutiveSuccesses = 0;

    AppLogger.i('网络探测任务已启动（间隔 ${_interval.inSeconds}s）；'
        '睡眠/唤醒监听：采用纯探测方案（依赖周期探测）');

    unawaited(_probeLoop(generation));
  }

  /// 通知探测任务退出（应用关闭时调用）。
  void shutdown() {
    if (!_running) return;
    _running = false;
    AppLogger.i('网络探测任务已请求停止');
  }

  /// 探测循环：周期性探测 → 更新稳定状态 → 间隔休眠。
  Future<void> _probeLoop(int generation) async {
    while (_running && _generation == generation) {
      final succeeded = await _probe();
      if (!_running || _generation != generation) return;
      observeProbeResult(succeeded);
      await Future<void>.delayed(_interval);
    }
  }

  /// 接收探测样本，更新稳定状态机；状态真实变化时广播转换。
  ///
  /// 迟滞逻辑（对齐 Rust NetworkStateMachine.observe）：
  /// - 失败样本：立即离线（在线时发布 Offline 边沿）
  /// - 成功样本：离线状态下累计连续成功，达 [_requiredSuccesses] 次才恢复在线
  @visibleForTesting
  void observeProbeResult(bool probeSucceeded) {
    final transition = _observe(probeSucceeded);
    if (transition == null) return;
    if (transition == NetworkTransition.online) {
      AppLogger.i('网络状态：在线（恢复同步）');
    } else {
      AppLogger.w('网络状态：离线（探测失败，暂停同步）');
    }
    _transitions.add(transition);
  }

  /// 将真实请求层传输失败送入 TCP 探测共用的稳定状态机。
  ///
  /// 最多发布一次 online→offline 边沿；恢复仍要求连续两次探测成功，
  /// 避免等待网络的任务热循环重试。返回是否发布了离线转换。
  bool reportRequestNetworkFailure() {
    if (!_online) return false;
    observeProbeResult(false);
    return true;
  }

  /// 阻塞等待网络恢复（供定时器循环使用）。
  ///
  /// [isShutdown] 返回 true 时立即返回（引擎停止）。
  Future<void> waitUntilOnline({bool Function()? isShutdown}) async {
    while (!_online) {
      if (isShutdown?.call() ?? false) return;
      await Future<void>.delayed(pollWait);
    }
  }

  /// 状态机核心：接收样本，仅在稳定状态真实改变时返回转换。
  NetworkTransition? _observe(bool probeSucceeded) {
    if (!probeSucceeded) {
      _consecutiveSuccesses = 0;
      if (_online) {
        _online = false;
        return NetworkTransition.offline;
      }
      return null;
    }

    if (_online) {
      _consecutiveSuccesses = 0;
      return null;
    }

    _consecutiveSuccesses++;
    if (_consecutiveSuccesses < _requiredSuccesses) return null;
    _online = true;
    _consecutiveSuccesses = 0;
    return NetworkTransition.online;
  }

  /// 单次 TCP 探测：connect 到目标主机 443 端口。
  static Future<bool> _tcpProbe() async {
    try {
      final socket = await Socket.connect(
        probeHost,
        probePort,
        timeout: probeTimeout,
      );
      socket.destroy();
      return true;
    } catch (e) {
      AppLogger.d('网络探测连接失败: $e');
      return false;
    }
  }

  // ============================================================
  // 测试钩子
  // ============================================================

  /// 测试用：注入探测函数与探测间隔。
  @visibleForTesting
  void debugConfigure({
    Future<bool> Function()? probe,
    Duration? interval,
  }) {
    if (probe != null) _probe = probe;
    if (interval != null) _interval = interval;
  }

  /// 测试用：重置全部状态（在线、停止探测、清空计数）。
  @visibleForTesting
  void debugReset() {
    _running = false;
    _generation++;
    _online = true;
    _consecutiveSuccesses = 0;
    _probe = _tcpProbe;
    _interval = probeInterval;
  }
}
