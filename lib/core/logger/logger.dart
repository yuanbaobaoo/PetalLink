import 'dart:io';

import 'package:intl/intl.dart';
import 'package:logger/logger.dart';
import 'package:path/path.dart' as p;

import 'package:petal_link/core/storage/app_paths.dart';

/// 日志级别（对齐 Rust 原版 `src/core/logging.rs` 的 LogLevel）。
///
/// 声明顺序即严重度：越靠前越严重；当前等级为 [AppLogLevel.info] 时
/// error/warn/info 输出，debug/trace 被过滤。
enum AppLogLevel {
  /// 错误
  error,

  /// 警告
  warn,

  /// 信息
  info,

  /// 调试
  debug,

  /// 跟踪（排查 HTTP 时在设置页临时调整）
  trace;

  /// 展示用大写名
  String get label => switch (this) {
        AppLogLevel.error => 'ERROR',
        AppLogLevel.warn => 'WARN',
        AppLogLevel.info => 'INFO',
        AppLogLevel.debug => 'DEBUG',
        AppLogLevel.trace => 'TRACE',
      };
}

/// 单条日志记录（对齐 Rust LogRecord）
class LogRecord {
  /// 级别
  final AppLogLevel level;

  /// logger 名称（模块名）
  final String loggerName;

  /// 消息内容
  final String message;

  /// 时间戳（毫秒，epoch）
  final int timeMs;

  const LogRecord({
    required this.level,
    required this.loggerName,
    required this.message,
    required this.timeMs,
  });
}

/// 应用全局日志工具（单例）。
///
/// 严格对齐 Rust 原版 `src/core/logging.rs` + `init_logger`：
/// - stdout 控制台输出
/// - 按天滚动文件 `<support>/logs/PetalLink.log.YYYY-MM-DD`，保留 30 天
/// - 1000 条内存环形缓冲（newest-first），供日志页读取
/// - 默认 INFO 等级，可运行时调整（排查 HTTP 时临时调到 trace）
///
/// 使用前需调用 [init] 初始化（通常在 `main()` 中）；
/// 未初始化时降级为 print + 环形缓冲，不阻断启动。
class AppLogger {
  AppLogger._internal();

  /// 单例
  factory AppLogger() => _instance;
  static final AppLogger _instance = AppLogger._internal();

  /// 单例实例（等价于 [AppLogger()] 构造）
  static AppLogger get instance => _instance;

  /// 环形缓冲最大条数（对齐 Rust MAX_BUFFER_SIZE）
  static const int maxBufferSize = 1000;

  /// 日志文件最大保留天数（对齐 Rust MAX_LOG_DAYS）
  static const int maxLogDays = 30;

  /// 内存环形缓冲（newest-first，对齐 Rust `buf.insert(0, record)`）
  final List<LogRecord> _buffer = [];

  /// 当前日志等级（可运行时调整）
  AppLogLevel _level = AppLogLevel.info;

  Logger? _logger;

  final DateFormat _lineDateFormat = DateFormat('yyyy-MM-dd HH:mm:ss');

  /// 初始化日志系统。
  ///
  /// 必须在应用启动早期调用（`main()` 或初始化阶段）。
  /// 日志目录不可用时降级为控制台 + 环形缓冲，不阻断启动。
  Future<void> init() async {
    if (_logger != null) return;

    LogOutput fileOutput;
    try {
      final logDir = await AppPaths.logDir();
      if (!logDir.existsSync()) {
        logDir.createSync(recursive: true);
      }
      fileOutput = DailyFileOutput(directory: logDir.path);
      _cleanupOldLogs(logDir);
    } catch (e) {
      // ignore: avoid_print
      print('日志目录不可用，跳过文件日志：$e');
      _logger = Logger(
        level: Level.trace,
        printer: Slf4jPrinter(),
        output: ConsoleOutput(),
      );
      return;
    }

    _logger = Logger(
      level: Level.trace, // 等级由本类 _level 统一门控
      printer: Slf4jPrinter(),
      output: MultiOutput([ConsoleOutput(), fileOutput]),
    );
  }

  // ============================================================
  // 日志方法
  // ============================================================

  /// 跟踪日志
  void trace(String message, [Object? error, StackTrace? stackTrace]) =>
      _log(AppLogLevel.trace, message, error, stackTrace);

  /// 调试日志
  void debug(String message, [Object? error, StackTrace? stackTrace]) =>
      _log(AppLogLevel.debug, message, error, stackTrace);

  /// 信息日志
  void info(String message, [Object? error, StackTrace? stackTrace]) =>
      _log(AppLogLevel.info, message, error, stackTrace);

  /// 警告日志
  void warn(String message, [Object? error, StackTrace? stackTrace]) =>
      _log(AppLogLevel.warn, message, error, stackTrace);

  /// 错误日志
  void error(String message, [Object? error, StackTrace? stackTrace]) =>
      _log(AppLogLevel.error, message, error, stackTrace);

  /// 记录一条业务日志：入环形缓冲 + 输出到控制台/文件。
  void _log(AppLogLevel level, String message,
      [Object? error, StackTrace? stackTrace]) {
    // 等级门控：声明顺序越靠后越不严重
    if (level.index > _level.index) return;

    _push(LogRecord(
      level: level,
      loggerName: 'PetalLink',
      message: message,
      timeMs: DateTime.now().millisecondsSinceEpoch,
    ));

    final logger = _logger;
    if (logger == null) {
      // 未初始化时降级到 print
      // ignore: avoid_print
      print(_formatLine(_buffer.first));
      return;
    }
    logger.log(
      _toLoggerLevel(level),
      message,
      error: error,
      stackTrace: stackTrace,
    );
  }

  // ---- 静态快捷方法 ----

  /// 快捷跟踪日志（需先 [init]）
  static void t(String message, [Object? error, StackTrace? stackTrace]) =>
      _instance.trace(message, error, stackTrace);

  /// 快捷调试日志（需先 [init]）
  static void d(String message, [Object? error, StackTrace? stackTrace]) =>
      _instance.debug(message, error, stackTrace);

  /// 快捷信息日志（需先 [init]）
  static void i(String message, [Object? error, StackTrace? stackTrace]) =>
      _instance.info(message, error, stackTrace);

  /// 快捷警告日志（需先 [init]）
  static void w(String message, [Object? error, StackTrace? stackTrace]) =>
      _instance.warn(message, error, stackTrace);

  /// 快捷错误日志（需先 [init]）
  static void e(String message, [Object? error, StackTrace? stackTrace]) =>
      _instance.error(message, error, stackTrace);

  // ============================================================
  // 环形缓冲（供日志查看页）
  // ============================================================

  /// 返回缓冲中所有日志的快照（newest-first）。
  List<LogRecord> snapshot() => List.unmodifiable(_buffer);

  /// 按 level 过滤后的快照（日志查看页筛选用）。
  List<LogRecord> snapshotFiltered(AppLogLevel? level) {
    if (level == null) return snapshot();
    return _buffer.where((r) => r.level == level).toList();
  }

  /// 获取内存中最近 [count] 条格式化日志行（oldest-first，供日志查看器）。
  ///
  /// 行格式：`[yyyy-MM-dd HH:mm:ss] [LEVEL] message`。
  List<String> recentLogs({int count = 100}) {
    final recent = _buffer.take(count).toList().reversed;
    return recent.map(_formatLine).toList();
  }

  /// 清空内存环形缓冲（日志查看页「清空」按钮）。
  void clearRingBuffer() => _buffer.clear();

  /// 获取当前生效的日志等级。
  AppLogLevel get currentLevel => _level;

  /// 运行时调整日志等级（设置页临时调到 trace 排查 HTTP）。
  void updateLevel(AppLogLevel level) {
    _level = level;
    info('日志等级已调整: ${level.label}');
  }

  // ============================================================
  // 内部
  // ============================================================

  /// 追加一条日志到缓冲（newest-first，溢出裁剪尾部）。
  void _push(LogRecord record) {
    _buffer.insert(0, record);
    if (_buffer.length > maxBufferSize) {
      _buffer.removeRange(maxBufferSize, _buffer.length);
    }
  }

  /// 格式化单条记录为标准日志行。
  String _formatLine(LogRecord record) {
    final time = _lineDateFormat
        .format(DateTime.fromMillisecondsSinceEpoch(record.timeMs));
    return '[$time] [${record.level.label}] ${record.message}';
  }

  /// 映射到 logger 包的 Level。
  static Level _toLoggerLevel(AppLogLevel level) => switch (level) {
        AppLogLevel.error => Level.error,
        AppLogLevel.warn => Level.warning,
        AppLogLevel.info => Level.info,
        AppLogLevel.debug => Level.debug,
        AppLogLevel.trace => Level.trace,
      };

  /// 清理超期日志文件（保留最近 [maxLogDays] 天），在 [init] 时调用。
  void _cleanupOldLogs(Directory logDir) {
    try {
      final now = DateTime.now();
      final today = DateTime(now.year, now.month, now.day);
      var removed = 0;
      for (final entity in logDir.listSync()) {
        if (entity is! File) continue;
        final name = p.basename(entity.path);
        // 只处理 PetalLink.log.YYYY-MM-DD 格式
        if (!name.startsWith('PetalLink.log.')) continue;
        final dateStr = name.substring('PetalLink.log.'.length);
        final date = DateTime.tryParse(dateStr);
        if (date == null) continue;
        final age = today.difference(DateTime(date.year, date.month, date.day)).inDays;
        if (age > maxLogDays) {
          entity.deleteSync();
          removed++;
        }
      }
      if (removed > 0) {
        info('清理超期日志文件: $removed 个（保留 $maxLogDays 天）');
      }
    } catch (_) {
      // 清理失败不影响主流程
    }
  }
}

// ============================================================
// SLF4J 风格 Printer
// ============================================================

/// 无颜色的 SLF4J 风格 Printer。
///
/// 输出格式：`[yyyy-MM-dd HH:mm:ss] [LEVEL] message`，
/// 错误与堆栈各占后续缩进行。
class Slf4jPrinter extends LogPrinter {
  final DateFormat _dateFormat = DateFormat('yyyy-MM-dd HH:mm:ss');

  @override
  List<String> log(LogEvent event) {
    final time = _dateFormat.format(DateTime.now());
    final level = _levelString(event.level);
    final message = event.message;

    final lines = <String>[];

    // 主日志行
    lines.add('[$time] [$level] $message');

    // 错误详情
    final error = event.error;
    if (error != null) {
      lines.add('  Error: $error');
    }

    // 堆栈跟踪
    final stackTrace = event.stackTrace;
    if (stackTrace != null && stackTrace.toString().isNotEmpty) {
      final stackLines = stackTrace.toString().split('\n');
      lines.addAll(stackLines.map((line) => '  $line'));
    }

    return lines;
  }

  String _levelString(Level level) => switch (level) {
        Level.error => 'ERROR',
        Level.warning => 'WARN',
        Level.info => 'INFO',
        Level.debug => 'DEBUG',
        Level.trace => 'TRACE',
        _ => level.name.toUpperCase(),
      };
}

// ============================================================
// 文件输出：按天滚动
// ============================================================

/// 文件输出（按天滚动）。
///
/// 文件名 `PetalLink.log.YYYY-MM-DD`（对齐 Rust tracing_appender 的
/// daily rolling），当天日志追加写入同一文件。
class DailyFileOutput extends LogOutput {
  /// 日志目录
  final String directory;

  File? _currentFile;
  String? _currentDate;

  DailyFileOutput({required this.directory});

  final DateFormat _fileDateFormat = DateFormat('yyyy-MM-dd');

  @override
  void output(OutputEvent event) {
    try {
      final dateStr = _fileDateFormat.format(DateTime.now());

      // 日期变更，切换到新文件
      if (_currentDate != dateStr) {
        _currentDate = dateStr;
        _currentFile = null;
      }

      _currentFile ??=
          File(p.join(directory, 'PetalLink.log.$dateStr'));

      final content = StringBuffer();
      for (final line in event.lines) {
        content.writeln(line);
      }
      _currentFile!.writeAsStringSync(content.toString(), mode: FileMode.append);
    } catch (e) {
      // 日志持久化失败不应影响主流程
      // ignore: avoid_print
      print('日志文件写入失败: $e');
    }
  }
}
