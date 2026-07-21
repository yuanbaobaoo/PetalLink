import 'dart:async';

import 'package:system_tray/system_tray.dart';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/types/enums.dart';

/// 托盘菜单项（纯 Dart 模型；separator 项忽略 id/label）
class TrayMenuItem {
  /// 菜单项 id（事件路由用；对齐 Rust：`version` / `show_window` /
  /// `quit` / `transfer_name_{id}` / `transfer_status_{id}`）
  final String id;

  /// 展示文案
  final String label;

  /// 是否可点击
  final bool enabled;

  /// 是否为分隔线
  final bool separator;

  const TrayMenuItem({
    required this.id,
    required this.label,
    this.enabled = true,
  }) : separator = false;

  /// 分隔线
  const TrayMenuItem.divider()
      : id = '',
        label = '',
        enabled = false,
        separator = true;
}

/// 托盘后端抽象（测试注入 fake；生产为 [SystemTrayBackend]）
abstract class TrayBackend {
  /// 初始化状态栏图标（模板图标 + tooltip）
  Future<void> init({
    required String iconPath,
    required String tooltip,
    required bool isTemplate,
  });

  /// 重建上下文菜单
  Future<void> setMenu(List<TrayMenuItem> items);

  /// 更新 tooltip
  Future<void> setToolTip(String tooltip);

  /// 销毁托盘
  Future<void> destroy();
}

/// 基于 system_tray 插件的生产托盘后端（macOS NSStatusItem）
class SystemTrayBackend implements TrayBackend {
  final SystemTray _tray = SystemTray();

  /// 菜单项点击路由（入参为 [TrayMenuItem.id]）
  final void Function(String id) _selectionHandler;

  SystemTrayBackend({required void Function(String id) onSelected})
      : _selectionHandler = onSelected;

  @override
  Future<void> init({
    required String iconPath,
    required String tooltip,
    required bool isTemplate,
  }) async {
    await _tray.initSystemTray(
      iconPath: iconPath,
      toolTip: tooltip,
      isTemplate: isTemplate,
    );
    // 左键点击也弹出菜单（对齐 Rust show_menu_on_left_click）
    _tray.registerSystemTrayEventHandler((event) {
      if (event == 'click') {
        _tray.popUpContextMenu();
      }
    });
  }

  @override
  Future<void> setMenu(List<TrayMenuItem> items) async {
    final menu = Menu();
    await menu.buildFrom(items.map((item) {
      if (item.separator) return MenuSeparator();
      return MenuItemLabel(
        label: item.label,
        enabled: item.enabled,
        onClicked: (_) => _selectionHandler(item.id),
      );
    }).toList());
    await _tray.setContextMenu(menu);
  }

  @override
  Future<void> setToolTip(String tooltip) => _tray.setToolTip(tooltip);

  @override
  Future<void> destroy() => _tray.destroy();
}

/// 托盘服务 —— macOS 菜单栏图标与菜单。
///
/// 严格对齐 Rust 原版 `src/platform/tray.rs`：
/// - 模板图标 `assets/menubar-icon.png`（明暗自动着色）
/// - 菜单结构：版本标签（禁用）/ 显示主窗口 / 活动传输（每任务两行）/
///   退出 PetalLink
/// - 菜单重建：`transfer_update` 与 `sync_state` 触发；签名判等跳过；
///   有活动传输时 5s 节流；无活动传输立即重建（清场）
class TrayService {
  /// 托盘 id（对齐 Rust TRAY_ID）
  static const String trayId = 'PetalLink-tray';

  /// 菜单文件名最大字符数（超出截断加 …；对齐 Rust MAX_NAME_CHARS）
  static const int maxNameChars = 20;

  /// 菜单重建节流（有活动传输时；对齐 Rust MENU_REBUILD_INTERVAL_MS）
  static const Duration rebuildThrottle = Duration(seconds: 5);

  /// 初始 tooltip（对齐 Rust）
  static const String defaultTooltip = 'PetalLink — 后台同步中';

  /// 版本标签（对齐 Rust build_menu 首项，禁用展示）
  static const String versionLabel = 'PetalLink - 华为云盘 Mac 客户端开源版';

  /// 模板图标 asset 路径（pubspec 已声明）
  static const String iconAsset = 'assets/menubar-icon.png';

  late final TrayBackend _backend;
  final Future<List<TransferTask>> Function() _transfersProvider;
  final Future<void> Function()? _showWindowHandler;
  final Future<void> Function()? _quitHandler;
  final int Function() _nowMs;
  final Duration _rebuildThrottleInterval;

  StreamSubscription<dynamic>? _transferSub;
  StreamSubscription<dynamic>? _syncSub;

  /// 上次重建时间（毫秒 epoch）
  int _lastRebuildMs = 0;

  /// 上次已发布菜单的传输签名
  int? _lastSignature;

  /// 是否已初始化
  bool _initialized = false;

  TrayService({
    TrayBackend? backend,
    required Future<List<TransferTask>> Function() activeTransfersProvider,
    Future<void> Function()? onShowWindow,
    Future<void> Function()? onQuit,
    int Function()? nowMs,
    Duration throttle = rebuildThrottle,
  })  : _transfersProvider = activeTransfersProvider,
        _showWindowHandler = onShowWindow,
        _quitHandler = onQuit,
        _nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch),
        _rebuildThrottleInterval = throttle {
    _backend = backend ?? SystemTrayBackend(onSelected: _dispatch);
  }

  /// 初始化托盘：建图标 → 首建菜单
  Future<void> init() async {
    if (_initialized) return;
    _initialized = true;
    try {
      await _backend.init(
        iconPath: iconAsset,
        tooltip: defaultTooltip,
        isTemplate: true,
      );
      await refreshMenu();
      AppLogger.i('托盘已初始化');
    } catch (e, st) {
      AppLogger.e('托盘初始化失败', e, st);
    }
  }

  /// 订阅刷新触发源（对齐 Rust：`transfer_update` 广播与每次
  /// `sync_state` 桥接都会调 refresh_menu；签名判等挡掉无变化重建）
  void bindRefreshTriggers({
    Stream<dynamic>? transferUpdates,
    Stream<dynamic>? syncStates,
  }) {
    _transferSub = transferUpdates?.listen((_) => _safeRefresh());
    _syncSub = syncStates?.listen((_) => _safeRefresh());
  }

  void _safeRefresh() {
    unawaited(refreshMenu().catchError((Object e) {
      AppLogger.w('托盘菜单刷新失败: $e');
    }));
  }

  /// 刷新菜单（对齐 Rust `refresh_menu` 的三段逻辑）：
  /// 1. 签名与上次相同 → 跳过
  /// 2. 有活动传输且距上次重建 <5s → 跳过（节流）
  /// 3. 无活动传输 → 不节流立即重建（清场）
  Future<void> refreshMenu() async {
    final active = await _transfersProvider();
    final signature = transferSignature(active);
    if (signature == _lastSignature) return;

    if (active.isNotEmpty) {
      final elapsed = _nowMs() - _lastRebuildMs;
      if (elapsed < _rebuildThrottleInterval.inMilliseconds) return;
    }

    await _backend.setMenu(buildMenu(active));
    _lastSignature = signature;
    _lastRebuildMs = _nowMs();
  }

  /// 更新 tooltip（对齐 Rust `update_tooltip`）
  Future<void> updateTooltip(String tooltip) => _backend.setToolTip(tooltip);

  /// 菜单项点击路由
  void _dispatch(String id) {
    switch (id) {
      case 'show_window':
        if (_showWindowHandler != null) unawaited(_showWindowHandler());
      case 'quit':
        if (_quitHandler != null) unawaited(_quitHandler());
      default:
        // version / transfer_* 均为禁用展示项，不会触发
        break;
    }
  }

  /// 释放资源（应用退出/测试收尾）
  Future<void> dispose() async {
    await _transferSub?.cancel();
    await _syncSub?.cancel();
  }

  // ============================================================
  // 纯逻辑（可测试）：菜单构建 / 签名 / 文案
  // ============================================================

  /// 构建菜单（对齐 Rust `build_menu` 的顺序与结构）：
  /// 版本标签（禁用）→ 分隔线 → 显示主窗口 →
  /// [有传输时：分隔线 + 每任务两行禁用项] → 底部分隔线（无条件）→ 退出
  static List<TrayMenuItem> buildMenu(List<TransferTask> active) {
    final items = <TrayMenuItem>[
      const TrayMenuItem(id: 'version', label: versionLabel, enabled: false),
      const TrayMenuItem.divider(),
      const TrayMenuItem(id: 'show_window', label: '显示主窗口'),
    ];

    if (active.isNotEmpty) {
      items.add(const TrayMenuItem.divider());
      for (final task in active) {
        items.add(TrayMenuItem(
          id: 'transfer_name_${task.id}',
          label: truncateName(task.name),
          enabled: false,
        ));
        items.add(TrayMenuItem(
          id: 'transfer_status_${task.id}',
          label: transferStatusLine(task),
          enabled: false,
        ));
      }
    }

    // 底部分隔线无条件保留（对齐 Rust sep_bottom）
    items.add(const TrayMenuItem.divider());
    items.add(const TrayMenuItem(id: 'quit', label: '退出 PetalLink'));
    return items;
  }

  /// 传输签名（对齐 Rust `transfer_signature`）：
  /// 覆盖任务数 + 每任务 id/state/transferred/totalSize，
  /// 相同则跳过重建（防高频重建闪烁）。
  static int transferSignature(List<TransferTask> active) {
    var hash = active.length;
    for (final t in active) {
      hash = Object.hash(hash, t.id, t.state.code, t.transferred, t.totalSize);
    }
    return hash;
  }

  /// 文件名按字符截断（20 字符 + …；对齐 Rust truncate_name，
  /// Rust chars() 即 Unicode 标量值 = Dart runes）
  static String truncateName(String name) {
    final runes = name.runes.toList();
    if (runes.length <= maxNameChars) return name;
    return '${String.fromCharCodes(runes.take(maxNameChars))}…';
  }

  /// 状态行文案：`{方向标签}…{pct}%`（U+2026；对齐 Rust build_menu）
  static String transferStatusLine(TransferTask task) {
    final label = switch (task.direction) {
      TransferDirection.Upload => '正在上传',
      TransferDirection.Download => '正在下载',
      TransferDirection.DownloadUpdate => '正在更新',
      TransferDirection.Delete => '正在删除',
    };
    final pct = task.totalSize > 0
        ? (task.transferred * 100 ~/ task.totalSize).clamp(0, 100)
        : 0;
    return '$label…$pct%';
  }
}
