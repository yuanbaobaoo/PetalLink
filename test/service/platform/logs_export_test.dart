import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/service/platform/platform_service.dart';

void main() {
  late Directory tempDir;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('logs_export_test');
    AppPaths.debugSupportRoot = tempDir.path;
  });

  tearDown(() {
    AppPaths.debugSupportRoot = null;
    if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
  });

  /// 在 logs 目录造滚动日志文件
  Future<Directory> seedLogFiles(Map<String, String> files) async {
    final logDir = Directory('${tempDir.path}/${AppPaths.bundleIdentifier}/logs');
    await logDir.create(recursive: true);
    for (final entry in files.entries) {
      await File('${logDir.path}/${entry.key}').writeAsString(entry.value);
    }
    return logDir;
  }

  group('PlatformService.concatLogFiles（对齐 logs_export 拼接段）', () {
    test('按文件名升序拼接，带 ===== 分隔头与结尾换行', () async {
      final logDir = await seedLogFiles({
        'PetalLink.log.2026-07-20': 'line-c\n',
        'PetalLink.log.2026-07-18': 'line-a',
        'PetalLink.log.2026-07-19': 'line-b\n',
        'other.txt': 'ignored\n',
      });

      final out = await PlatformService.concatLogFiles(logDir);

      // 仅 PetalLink.log* 文件，按文件名升序
      final aIdx = out.indexOf('line-a');
      final bIdx = out.indexOf('line-b');
      final cIdx = out.indexOf('line-c');
      expect(aIdx, greaterThanOrEqualTo(0));
      expect(aIdx, lessThan(bIdx));
      expect(bIdx, lessThan(cIdx));
      expect(out.contains('other.txt'), isFalse);
      expect(out.contains('ignored'), isFalse);

      // 分隔头格式 ===== <完整路径> =====，缺换行的内容补换行
      expect(
        out.contains('===== ${logDir.path}/PetalLink.log.2026-07-18 =====\n'),
        isTrue,
      );
      expect(out.contains('line-a\n'), isTrue);
    });

    test('目录不存在 → 空串', () async {
      final out = await PlatformService.concatLogFiles(
          Directory('${tempDir.path}/no-such-dir'));
      expect(out, '');
    });

    test('全部文件为空 → 空串', () async {
      final logDir = await seedLogFiles({'PetalLink.log.2026-07-20': ''});
      final out = await PlatformService.concatLogFiles(logDir);
      expect(out, '');
    });
  });

  group('PlatformService.logsExport（对齐 logs_export 命令）', () {
    test('写出拼接内容到指定路径', () async {
      await seedLogFiles({'PetalLink.log.2026-07-20': 'hello\n'});
      final target = '${tempDir.path}/export.txt';

      await PlatformService().logsExport(target);

      final content = await File(target).readAsString();
      expect(content, contains('hello'));
      expect(content, contains('===== '));
    });

    test('日志目录为空 → GenericError（对齐 Rust 错误文案）', () async {
      // 不造任何日志文件
      expect(
        () => PlatformService().logsExport('${tempDir.path}/export.txt'),
        throwsA(isA<GenericError>().having(
          (e) => e.message,
          'message',
          '日志目录为空，无可导出内容',
        )),
      );
    });
  });
}
