/// 本地文件监视 —— FSEvents + 防抖 + 历史回放防护。
///
/// 严格对齐 Rust 原版 `src/mount/local_watcher.rs`（notify crate / FSEvents）：
/// - 3s 防抖（可配置）：窗口内持续变化则重置计时器
/// - 2s 预热窗口（可配置）：吞掉 FSEvents 注册时的历史事件回放
/// - 跳过 `.hwcloud_` 前缀 / `.tmp` 后缀及用户配置模式
/// - **必须在 BFS 完成后才启动**（否则云端树为空 → 误删本地文件）
///
/// 技术选型说明：macOS 上 dart:io `Directory.watch(recursive: true)` 即基于
/// FSEvents（与 notify crate 同源），`watcher` package 在 macOS 上也是对
/// dart:io watch 的薄封装，因此直接使用 dart:io，不引入额外依赖。
///
/// # FSEvents 历史回放防护
/// macOS FSEvents 在新 watcher 注册时会**回放**「自进程启动以来」的历史事件——
/// 含本次 BFS / 首次 sync cycle 在本地建的目录/占位符。这些非用户改动一旦
/// 触发 sync cycle，planner 会把它们误判为「本地新建 → 重复上传」。
/// 防护：注册后整个预热窗口内的事件全部丢弃，窗口结束发出空变更集
/// （表示请求一次全量重扫，补偿扫描与监视启动间隙）。
library;

import 'dart:async';
import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/mount/skip.dart';

/// 本地文件监视器。
///
/// 输出事件流供 sync 引擎订阅：[changes] 为广播流，每批是去重后的
/// 相对路径集合；空列表表示「主动请求全量重扫」（预热窗口结束信号）。
class LocalWatcher {
  /// 挂载根目录（绝对路径）
  final String mountDir;

  /// 用户配置的跳过模式（glob）
  final List<String> skipPatterns;

  /// 防抖窗口（对齐 Rust debounce_secs，默认 3s）
  final Duration debounce;

  /// 预热窗口（对齐 Rust WARMUP_SECS=2；仅需覆盖 FSEvents 历史回放）
  final Duration warmup;

  /// 原始事件源工厂（测试注入；默认 dart:io FSEvents 递归监视）
  final Stream<FileSystemEvent> Function()? _eventSourceFactory;

  final StreamController<List<String>> _changes =
      StreamController<List<String>>.broadcast();

  StreamSubscription<FileSystemEvent>? _subscription;
  Timer? _debounceTimer;
  Timer? _warmupTimer;

  /// 当前待冲刷的路径集合（插入序去重）
  final Set<String> _pending = <String>{};

  bool _warmingUp = false;
  bool _running = false;

  /// 每次 start/stop 都推进 generation；旧 timer 在发布前必须匹配当前代。
  int _generation = 0;

  /// 创建新监视器（未启动）。
  LocalWatcher({
    required this.mountDir,
    this.skipPatterns = const [],
    this.debounce = const Duration(seconds: 3),
    this.warmup = const Duration(seconds: 2),
    Stream<FileSystemEvent> Function()? eventSource,
  }) : _eventSourceFactory = eventSource;

  /// 变更事件流（广播）：每批为相对挂载目录的路径集合；空批 = 全量重扫请求。
  Stream<List<String>> get changes => _changes.stream;

  /// 是否正在运行
  bool get isRunning => _running;

  /// 启动 watcher。**必须在 BFS 完成后才调用**。
  Future<void> start() async {
    if (_running) return;
    _running = true;
    final generation = ++_generation;

    final source = _eventSourceFactory?.call() ??
        Directory(mountDir).watch(recursive: true);
    _subscription = source.listen(
      _onEvent,
      onError: (Object e) => AppLogger.e('本地文件监视器事件流异常', e),
    );

    // 预热窗口：吞掉 FSEvents 历史回放
    if (warmup > Duration.zero) {
      _warmingUp = true;
      _warmupTimer = Timer(warmup, () {
        if (!_running || _generation != generation) return;
        _warmingUp = false;
        // 空变更集表示主动请求全量重扫，用于补偿扫描与监视启动间隙。
        _changes.add(const []);
      });
    }

    AppLogger.i('本地文件监视器已启动：$mountDir（防抖 ${debounce.inSeconds}s）');
  }

  /// 处理单条原始事件。
  void _onEvent(FileSystemEvent event) {
    if (!_running) return;
    if (_warmingUp) {
      AppLogger.d('watcher warmup: 丢弃历史事件 ${event.path}');
      return;
    }
    final paths = _extractRelativePaths(event);
    if (paths.isEmpty) return;
    _pending.addAll(paths);
    // 防抖：窗口内持续变化则重置计时器
    _debounceTimer?.cancel();
    _debounceTimer = Timer(debounce, _flush);
  }

  /// 冲刷待发布路径集合。
  void _flush() {
    if (!_running || _pending.isEmpty) return;
    final paths = _pending.toList();
    _pending.clear();
    _changes.add(paths);
  }

  /// 从系统事件中提取相对路径（跳过应排除的文件）。
  ///
  /// 移动事件同时计入目标路径（对齐 Rust notify 事件的多 paths 语义）。
  List<String> _extractRelativePaths(FileSystemEvent event) {
    final rawPaths = <String>[event.path];
    if (event is FileSystemMoveEvent) {
      final destination = event.destination;
      if (destination != null) rawPaths.add(destination);
    }
    final paths = <String>[];
    for (final raw in rawPaths) {
      final name = p.basename(raw);
      // 跳过应排除的文件
      if (MountSkip.shouldSkip(name, skipPatterns)) {
        AppLogger.d('watcher: 跳过排除文件 $raw');
        continue;
      }
      final rel = p.relative(raw, from: mountDir);
      if (rel == '.' || rel == '..' || rel.startsWith('../')) {
        AppLogger.d('watcher: 路径不在挂载目录下，跳过 $raw');
        continue;
      }
      paths.add(rel);
    }
    return paths;
  }

  /// 停止监视：释放事件订阅，清空 pending。
  Future<void> stop() async {
    if (!_running) return;
    _running = false;
    _generation++;
    await _subscription?.cancel();
    _subscription = null;
    _debounceTimer?.cancel();
    _debounceTimer = null;
    _warmupTimer?.cancel();
    _warmupTimer = null;
    _pending.clear();
    _warmingUp = false;
    AppLogger.i('本地文件监视器已停止');
  }

  /// 释放全部资源（应用退出时调用）。
  Future<void> dispose() async {
    await stop();
    await _changes.close();
  }
}
