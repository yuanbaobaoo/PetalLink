import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/mount/stability.dart';

void main() {
  late Directory tempDir;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_stability_test');
  });

  tearDown(() {
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  String tempPath(String name) => '${tempDir.path}/$name';

  /// 创建 mtime 为 [age] 秒前的文件。
  String createAgedFile(String name, int ageSecs, [String content = 'data']) {
    final path = tempPath(name);
    File(path).writeAsStringSync(content);
    File(path).setLastModifiedSync(
        DateTime.now().subtract(Duration(seconds: ageSecs)));
    return path;
  }

  /// 无操作睡眠（跳过 3s/1s 窗口等待）。
  Future<void> noopSleep(Duration _) async {}

  group('StabilityChecker 三阶段判定', () {
    test('mtime 年龄不足 5s → unstable', () async {
      final path = createAgedFile('fresh.txt', 0);
      var slept = false;
      final checker = StabilityChecker(
        sleep: (d) async {
          slept = true;
        },
        lsofCommands: (_) async => const [],
      );
      expect(await checker.check(path), StabilityResult.unstable);
      // mtime 阶段即返回，不应进入 size/lsof 窗口
      expect(slept, isFalse);
    });

    test('老文件 + 无占用 → stable', () async {
      final path = createAgedFile('old.txt', 10);
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async => const [],
      );
      expect(await checker.check(path), StabilityResult.stable);
    });

    test('size 在窗口内变化 → unstable', () async {
      final path = createAgedFile('growing.txt', 10);
      final checker = StabilityChecker(
        sleep: (d) async {
          if (d.inSeconds == StabilityChecker.sizeStableWindowSecs) {
            File(path).writeAsStringSync('more data', mode: FileMode.append);
          }
        },
        lsofCommands: (_) async => const [],
      );
      expect(await checker.check(path), StabilityResult.unstable);
    });

    test('size 变化持续超 5min → editing', () async {
      final path = createAgedFile('editing.txt', 10);
      var now = DateTime.now().millisecondsSinceEpoch;
      final checker = StabilityChecker(
        nowMs: () => now,
        sleep: (d) async {
          if (d.inSeconds == StabilityChecker.sizeStableWindowSecs) {
            File(path).writeAsStringSync('x', mode: FileMode.append);
          }
        },
        lsofCommands: (_) async => const [],
      );
      // 首次：记录 firstSeen，unstable
      expect(await checker.check(path), StabilityResult.unstable);
      // 推进时钟超过编辑阈值（300s）
      now += (StabilityChecker.editingThresholdSecs + 1) * 1000;
      expect(await checker.check(path), StabilityResult.editing);
    });

    test('lsof 两次均占用 → unstable', () async {
      final path = createAgedFile('busy.txt', 10);
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async => const ['TextEdit'],
      );
      expect(await checker.check(path), StabilityResult.unstable);
    });

    test('lsof 占用持续超 5min → editing', () async {
      final path = createAgedFile('busy2.txt', 10);
      var now = DateTime.now().millisecondsSinceEpoch;
      final checker = StabilityChecker(
        nowMs: () => now,
        sleep: noopSleep,
        lsofCommands: (_) async => const ['TextEdit'],
      );
      expect(await checker.check(path), StabilityResult.unstable);
      now += (StabilityChecker.editingThresholdSecs + 1) * 1000;
      expect(await checker.check(path), StabilityResult.editing);
    });

    test('lsof 首次占用、复查已释放 → stable（消除 Spotlight 误报）', () async {
      final path = createAgedFile('flaky.txt', 10);
      var calls = 0;
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async => calls++ == 0 ? const ['mds_stores'] : const [],
      );
      expect(await checker.check(path), StabilityResult.stable);
      expect(calls, 2); // 双重检查确实执行
    });

    test('白名单只读进程不判 busy', () async {
      final path = createAgedFile('spotlight.txt', 10);
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async =>
            const ['mds', 'mdworker_shared', 'QuickLookSatellite'],
      );
      expect(await checker.check(path), StabilityResult.stable);
    });

    test('白名单混入非白名单进程 → busy', () async {
      final path = createAgedFile('mixed.txt', 10);
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async => const ['mds', 'TextEdit'],
      );
      expect(await checker.check(path), StabilityResult.unstable);
    });

    test('稳定后清除编辑追踪（重新不稳定重新计时）', () async {
      final path = createAgedFile('track.txt', 10);
      var now = DateTime.now().millisecondsSinceEpoch;
      var busy = true;
      final checker = StabilityChecker(
        nowMs: () => now,
        sleep: noopSleep,
        lsofCommands: (_) async => busy ? const ['TextEdit'] : const [],
      );
      expect(await checker.check(path), StabilityResult.unstable);
      // 稳定一次 → tracking 清除
      busy = false;
      expect(await checker.check(path), StabilityResult.stable);
      // 再次不稳定：firstSeen 重置，推进 299s 仍非 editing
      busy = true;
      expect(await checker.check(path), StabilityResult.unstable);
      now += (StabilityChecker.editingThresholdSecs - 1) * 1000;
      expect(await checker.check(path), StabilityResult.unstable);
    });

    test('文件不存在 → unstable', () async {
      final checker = StabilityChecker(
        sleep: noopSleep,
        lsofCommands: (_) async => const [],
      );
      expect(await checker.check(tempPath('missing.txt')),
          StabilityResult.unstable);
    });

    test('clearTracking 清除追踪状态', () async {
      final path = createAgedFile('clear.txt', 10);
      var now = DateTime.now().millisecondsSinceEpoch;
      final checker = StabilityChecker(
        nowMs: () => now,
        sleep: noopSleep,
        lsofCommands: (_) async => const ['TextEdit'],
      );
      expect(await checker.check(path), StabilityResult.unstable);
      checker.clearTracking(path);
      now += (StabilityChecker.editingThresholdSecs + 1) * 1000;
      // 追踪已清除 → 重新计时，不升级 editing
      expect(await checker.check(path), StabilityResult.unstable);
    });
  });

  group('StabilityChecker.defaultLsofCommands', () {
    test('无占用文件返回空列表', () async {
      final path = createAgedFile('idle.txt', 0);
      expect(await StabilityChecker.defaultLsofCommands(path), isEmpty);
    });

    test('解析 c 行命令名', () async {
      // 以读模式持续打开文件，lsof 应能列出当前进程
      final path = createAgedFile('opened.txt', 0);
      final raf = await File(path).open();
      try {
        final commands = await StabilityChecker.defaultLsofCommands(path);
        expect(commands, isNotEmpty);
      } finally {
        await raf.close();
      }
    });
  });
}
