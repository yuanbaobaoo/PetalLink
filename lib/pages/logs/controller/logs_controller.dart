import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:get/get.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/platform/platform_service.dart';

/// 日志级别过滤器（对标 CMP LogViewerScreen 的 LevelFilter）。
///
/// 语义为**精确级别匹配**（非「及以上」），与 CMP `records.filter` 一致。
enum LevelFilter {
  /// 全部级别
  all,

  /// 仅 INFO
  info,

  /// 仅 WARN
  warn,

  /// 仅 ERROR
  error,
}

/// 日志记录 UI 模型（对标 CMP LogRecordDisplay）。
///
/// 从 [AppLogger] 环形缓冲的结构化 [LogRecord] 转换；newest-first，
/// 最多 [LogsController.maxRecords] 条。
class LogRecordDisplay {
  /// 时间戳（毫秒，epoch）
  final int timestampMs;

  /// 日志级别
  final AppLogLevel level;

  /// logger 名称（模块名）
  final String target;

  /// 日志消息
  final String message;

  const LogRecordDisplay({
    required this.timestampMs,
    required this.level,
    required this.target,
    required this.message,
  });
}

/// 日志查看器状态
class LogsState {
  /// 当前过滤器
  final LevelFilter filter;

  /// 全部日志记录（newest-first）
  final List<LogRecordDisplay> records;

  /// 是否正在加载（仅首次加载为 true；轮询刷新不闪 loading）
  final bool loading;

  const LogsState({
    this.filter = LevelFilter.all,
    this.records = const [],
    this.loading = false,
  });

  /// 初始状态
  factory LogsState.initial() => const LogsState();

  /// 过滤后的记录（精确级别匹配，对齐 CMP LogViewerScreen）
  List<LogRecordDisplay> get filteredRecords {
    if (filter == LevelFilter.all) return records;
    return records.where((r) {
      return switch (filter) {
        LevelFilter.error => r.level == AppLogLevel.error,
        LevelFilter.warn => r.level == AppLogLevel.warn,
        LevelFilter.info => r.level == AppLogLevel.info,
        LevelFilter.all => true,
      };
    }).toList();
  }

  /// 深拷贝并替换指定字段
  LogsState copyWith({
    LevelFilter? filter,
    List<LogRecordDisplay>? records,
    bool? loading,
  }) {
    return LogsState(
      filter: filter ?? this.filter,
      records: records ?? this.records,
      loading: loading ?? this.loading,
    );
  }
}

/// 日志查看器控制器 — 日志查看页状态管理
///
/// 对标 CMP LogViewerScreen.kt（2s 轮询）与 Rust logs 命令面：
/// - 从 [AppLogger] 环形缓冲读取结构化记录（最多 [maxRecords] 条）
/// - 按级别过滤（all / info / warn / error，精确匹配）
/// - [startPolling] 每 2s 自动刷新（页面前台期间）
/// - 导出日志到文件（logs 目录全部滚动文件拼接）/ 清空环形缓冲
class LogsController extends GetxController {
  /// 列表最大条数（对齐 CMP 1000 条 ring buffer 上限）
  static const int maxRecords = 1000;

  /// 默认轮询间隔（对齐 CMP 2s）
  static const Duration defaultPollInterval = Duration(seconds: 2);

  /// 轮询间隔（测试可注入更大值）
  final Duration pollInterval;

  /// 日志查看器状态（响应式）
  final Rx<LogsState> state = LogsState.initial().obs;

  /// 轮询定时器
  Timer? _pollTimer;

  LogsController({this.pollInterval = defaultPollInterval});

  @override
  void onInit() {
    super.onInit();
    loadLogs(showLoading: true);
  }

  @override
  void onClose() {
    stopPolling();
    super.onClose();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 加载与轮询
  // ═══════════════════════════════════════════════════════════════════

  /// 从 [AppLogger] 环形缓冲加载日志（newest-first，最多 [maxRecords] 条）
  ///
  /// [showLoading] 仅首次加载为 true（显示全屏 spinner）；
  /// 轮询刷新传 false，避免列表闪烁。
  void loadLogs({bool showLoading = false}) {
    if (showLoading) {
      state.value = state.value.copyWith(loading: true);
    }
    try {
      final snapshot = AppLogger.instance.snapshot();
      final records = snapshot
          .take(maxRecords)
          .map((r) => LogRecordDisplay(
                timestampMs: r.timeMs,
                level: r.level,
                target: r.loggerName,
                message: r.message,
              ))
          .toList();
      state.value = state.value.copyWith(records: records, loading: false);
    } catch (e, st) {
      AppLogger.e('loadLogs 异常', e, st);
      state.value = state.value.copyWith(loading: false);
    }
  }

  /// 启动 2s 轮询（对齐 CMP LogViewerScreen 的 LaunchedEffect 轮询）
  void startPolling() {
    if (_pollTimer != null) return;
    _pollTimer = Timer.periodic(pollInterval, (_) => loadLogs());
  }

  /// 停止轮询
  void stopPolling() {
    _pollTimer?.cancel();
    _pollTimer = null;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 过滤
  // ═══════════════════════════════════════════════════════════════════

  /// 切换日志级别过滤器
  void setFilter(LevelFilter filter) {
    state.value = state.value.copyWith(filter: filter);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 导出与清空
  // ═══════════════════════════════════════════════════════════════════

  /// 导出日志到用户选择的路径（对齐 Rust `logs_export`）。
  ///
  /// [path] 为空时弹出 file_picker 保存对话框；导出内容为 logs 目录下
  /// 全部 `PetalLink.log*` 滚动文件的拼接（含分隔头），非当前过滤结果。
  Future<void> exportLogs([String path = '']) async {
    try {
      var target = path;
      if (target.isEmpty) {
        final picked = await FilePicker.platform.saveFile(
          dialogTitle: '导出日志',
          fileName: 'PetalLink-logs.txt',
        );
        if (picked == null) return; // 用户取消
        target = picked;
      }

      await Get.find<PlatformService>().logsExport(target);
      AppLogger.i('日志已导出: $target');
    } on AppError catch (e) {
      AppLogger.e('exportLogs 失败', e);
    } catch (e, st) {
      AppLogger.e('exportLogs 异常', e, st);
    }
  }

  /// 清空内存环形缓冲（对齐 Rust `logs_clear`；磁盘滚动日志由保留策略管）
  void clearLogs() {
    Get.find<PlatformService>().logsClear();
    state.value = state.value.copyWith(records: []);
  }
}
