import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:logger/logger.dart';
import 'package:petal_link/core/logger/logger.dart';

void main() {
  setUp(() {
    AppLogger.instance.updateLevel(AppLogLevel.info);
    AppLogger.instance.clearRingBuffer();
  });

  group('AppLogger 环形缓冲', () {
    test('记录按 newest-first 存储', () {
      AppLogger.instance.info('第一条');
      AppLogger.instance.info('第二条');

      final snapshot = AppLogger.instance.snapshot();
      expect(snapshot, hasLength(2));
      expect(snapshot.first.message, '第二条');
      expect(snapshot.last.message, '第一条');
    });

    test('溢出裁剪保留最新 1000 条', () {
      for (var i = 0; i < AppLogger.maxBufferSize + 5; i++) {
        AppLogger.instance.info('消息 $i');
      }

      final snapshot = AppLogger.instance.snapshot();
      expect(snapshot, hasLength(AppLogger.maxBufferSize));
      expect(snapshot.first.message, '消息 ${AppLogger.maxBufferSize + 4}');
      expect(snapshot.last.message, '消息 5');
    });

    test('记录携带级别与毫秒时间戳', () {
      final before = DateTime.now().millisecondsSinceEpoch;
      AppLogger.instance.warn('警告消息');
      final after = DateTime.now().millisecondsSinceEpoch;

      final record = AppLogger.instance.snapshot().single;
      expect(record.level, AppLogLevel.warn);
      expect(record.message, '警告消息');
      expect(record.timeMs, greaterThanOrEqualTo(before));
      expect(record.timeMs, lessThanOrEqualTo(after));
    });

    test('snapshotFiltered 按级别过滤', () {
      AppLogger.instance.info('信息');
      AppLogger.instance.warn('警告');
      AppLogger.instance.error('错误');

      final errors =
          AppLogger.instance.snapshotFiltered(AppLogLevel.error);
      expect(errors, hasLength(1));
      expect(errors.single.message, '错误');

      expect(AppLogger.instance.snapshotFiltered(null), hasLength(3));
    });

    test('clearRingBuffer 清空缓冲', () {
      AppLogger.instance.info('待清空');
      AppLogger.instance.clearRingBuffer();
      expect(AppLogger.instance.snapshot(), isEmpty);
    });
  });

  group('AppLogger 等级门控', () {
    test('默认 INFO：debug/trace 被过滤', () {
      AppLogger.instance.debug('调试');
      AppLogger.instance.trace('跟踪');
      AppLogger.instance.info('信息');

      final snapshot = AppLogger.instance.snapshot();
      expect(snapshot, hasLength(1));
      expect(snapshot.single.message, '信息');
    });

    test('updateLevel 调整后生效', () {
      AppLogger.instance.updateLevel(AppLogLevel.trace);
      AppLogger.instance.debug('调试');

      expect(
        AppLogger.instance.snapshot().map((r) => r.message),
        containsAll(['调试', '日志等级已调整: TRACE']),
      );
    });

    test('updateLevel 到 error 后 info 被过滤', () {
      AppLogger.instance.updateLevel(AppLogLevel.error);
      AppLogger.instance.info('信息');
      AppLogger.instance.error('错误');

      final messages =
          AppLogger.instance.snapshot().map((r) => r.message).toList();
      expect(messages, contains('错误'));
      expect(messages, isNot(contains('信息')));
    });
  });

  group('AppLogger.recentLogs', () {
    test('返回 oldest-first 格式化日志行', () {
      AppLogger.instance.info('第一条');
      AppLogger.instance.error('第二条');

      final lines = AppLogger.instance.recentLogs();
      expect(lines, hasLength(2));
      expect(lines.first, contains('第一条'));
      expect(lines.last, contains('第二条'));
      // 格式：[yyyy-MM-dd HH:mm:ss] [LEVEL] message
      expect(
        lines.first,
        matches(r'^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\] \[INFO\] 第一条$'),
      );
      expect(lines.last, contains('[ERROR] 第二条'));
    });

    test('count 限制返回条数（取最新）', () {
      for (var i = 0; i < 10; i++) {
        AppLogger.instance.info('消息 $i');
      }

      final lines = AppLogger.instance.recentLogs(count: 3);
      expect(lines, hasLength(3));
      expect(lines.first, contains('消息 7'));
      expect(lines.last, contains('消息 9'));
    });
  });

  group('Slf4jPrinter', () {
    test('主日志行格式', () {
      final printer = Slf4jPrinter();
      final lines = printer.log(LogEvent(Level.info, 'hello'));

      expect(lines.single,
          matches(r'^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\] \[INFO\] hello$'));
    });

    test('错误与堆栈各占缩进行', () {
      final printer = Slf4jPrinter();
      final lines = printer.log(LogEvent(
        Level.error,
        'boom',
        error: StateError('bad'),
        stackTrace: StackTrace.fromString('frame1\nframe2'),
      ));

      expect(lines.first, contains('[ERROR] boom'));
      expect(lines[1], contains('Error: Bad state: bad'));
      expect(lines[2], '  frame1');
      expect(lines[3], '  frame2');
    });
  });

  group('DailyFileOutput（按天滚动）', () {
    test('写入 PetalLink.log.YYYY-MM-DD 并追加', () {
      final dir = Directory.systemTemp.createTempSync('petal_link_log_test');
      try {
        final output = DailyFileOutput(directory: dir.path);
        output.output(OutputEvent(LogEvent(Level.info, '第一行'), ['第一行']));
        output.output(OutputEvent(LogEvent(Level.info, '第二行'), ['第二行']));

        final files = dir.listSync().whereType<File>().toList();
        expect(files, hasLength(1));
        expect(
          files.single.path.split('/').last,
          matches(r'^PetalLink\.log\.\d{4}-\d{2}-\d{2}$'),
        );

        final content = files.single.readAsStringSync();
        expect(content, contains('第一行'));
        expect(content, contains('第二行'));
      } finally {
        dir.deleteSync(recursive: true);
      }
    });
  });
}
