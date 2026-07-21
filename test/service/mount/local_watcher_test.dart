import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/mount/local_watcher.dart';

void main() {
  late Directory tempDir;
  late StreamController<FileSystemEvent> events;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_watcher_test');
    events = StreamController<FileSystemEvent>();
  });

  tearDown(() async {
    await events.close();
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  String abs(String rel) => '${tempDir.path}/$rel';

  LocalWatcher buildWatcher({
    Duration warmup = const Duration(milliseconds: 80),
    Duration debounce = const Duration(milliseconds: 60),
    List<String> skipPatterns = const [],
  }) {
    return LocalWatcher(
      mountDir: tempDir.path,
      skipPatterns: skipPatterns,
      debounce: debounce,
      warmup: warmup,
      eventSource: () => events.stream,
    );
  }

  /// 收集 watcher 输出，直到 [duration] 超时。
  Future<List<List<String>>> collect(
      LocalWatcher watcher, Duration duration) async {
    final batches = <List<String>>[];
    final sub = watcher.changes.listen(batches.add);
    await Future<void>.delayed(duration);
    await sub.cancel();
    return batches;
  }

  group('LocalWatcher 预热窗口', () {
    test('预热窗口内事件被吞掉，窗口结束发出全量重扫信号', () async {
      final watcher = buildWatcher();
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 250));

      // 预热窗口内的事件（FSEvents 历史回放）
      events.add(FileSystemCreateEvent(abs('replay.txt'), false));
      await Future<void>.delayed(const Duration(milliseconds: 150));

      final batches = await collectFuture;
      // 仅一批：空变更集（全量重扫请求）
      expect(batches, hasLength(1));
      expect(batches.single, isEmpty);
      await watcher.dispose();
    });

    test('预热结束后正常投递', () async {
      final watcher = buildWatcher();
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 400));

      // 等预热窗口结束
      await Future<void>.delayed(const Duration(milliseconds: 120));
      events.add(FileSystemCreateEvent(abs('new.txt'), false));

      final batches = await collectFuture;
      expect(batches, hasLength(2));
      expect(batches[0], isEmpty); // 预热结束信号
      expect(batches[1], ['new.txt']);
      await watcher.dispose();
    });
  });

  group('LocalWatcher 防抖', () {
    test('窗口内连续事件合并为一批并去重', () async {
      final watcher = buildWatcher(
          warmup: Duration.zero, debounce: const Duration(milliseconds: 100));
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 350));

      events.add(FileSystemCreateEvent(abs('a.txt'), false));
      await Future<void>.delayed(const Duration(milliseconds: 30));
      events.add(FileSystemModifyEvent(abs('b.txt'), false, true));
      await Future<void>.delayed(const Duration(milliseconds: 30));
      events.add(FileSystemModifyEvent(abs('a.txt'), false, true));

      final batches = await collectFuture;
      expect(batches, hasLength(1));
      expect(batches.single, containsAll(['a.txt', 'b.txt']));
      expect(batches.single, hasLength(2)); // a.txt 去重
      await watcher.dispose();
    });

    test('静默期超过防抖窗口后各自成批', () async {
      final watcher = buildWatcher(
          warmup: Duration.zero, debounce: const Duration(milliseconds: 60));
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 450));

      events.add(FileSystemCreateEvent(abs('a.txt'), false));
      await Future<void>.delayed(const Duration(milliseconds: 180));
      events.add(FileSystemCreateEvent(abs('b.txt'), false));

      final batches = await collectFuture;
      expect(batches, hasLength(2));
      expect(batches[0], ['a.txt']);
      expect(batches[1], ['b.txt']);
      await watcher.dispose();
    });
  });

  group('LocalWatcher 路径过滤', () {
    test('内部文件 / .tmp / 用户模式被跳过', () async {
      final watcher = buildWatcher(
        warmup: Duration.zero,
        skipPatterns: const ['.DS_Store', '~\$*'],
      );
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 300));

      events.add(FileSystemCreateEvent(abs('.hwcloud_cache'), false));
      events.add(FileSystemCreateEvent(abs('download.tmp'), false));
      events.add(FileSystemCreateEvent(abs('.DS_Store'), false));
      events.add(FileSystemCreateEvent(abs('~\$draft.docx'), false));
      events.add(FileSystemCreateEvent(abs('keep.txt'), false));

      final batches = await collectFuture;
      expect(batches, hasLength(1));
      expect(batches.single, ['keep.txt']);
      await watcher.dispose();
    });

    test('相对路径含子目录', () async {
      final watcher = buildWatcher(warmup: Duration.zero);
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 300));

      events.add(FileSystemCreateEvent(abs('sub/deep/f.txt'), false));

      final batches = await collectFuture;
      expect(batches.single, ['sub/deep/f.txt']);
      await watcher.dispose();
    });

    test('移动事件同时计入目标路径', () async {
      final watcher = buildWatcher(warmup: Duration.zero);
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 300));

      events.add(FileSystemMoveEvent(abs('old.txt'), false, abs('new.txt')));

      final batches = await collectFuture;
      expect(batches.single, containsAll(['old.txt', 'new.txt']));
      await watcher.dispose();
    });

    test('挂载目录外的路径被忽略', () async {
      final watcher = buildWatcher(warmup: Duration.zero);
      await watcher.start();
      final collectFuture = collect(watcher, const Duration(milliseconds: 300));

      events.add(FileSystemCreateEvent('/etc/hosts', false));
      events.add(FileSystemCreateEvent(abs('in.txt'), false));

      final batches = await collectFuture;
      expect(batches.single, ['in.txt']);
      await watcher.dispose();
    });
  });

  group('LocalWatcher 生命周期', () {
    test('stop 后不再投递（pending 被清空）', () async {
      final watcher = buildWatcher(
          warmup: Duration.zero, debounce: const Duration(milliseconds: 80));
      await watcher.start();

      final batches = <List<String>>[];
      final sub = watcher.changes.listen(batches.add);
      events.add(FileSystemCreateEvent(abs('a.txt'), false));
      await watcher.stop();
      await Future<void>.delayed(const Duration(milliseconds: 150));
      expect(batches, isEmpty);
      await sub.cancel();
      await watcher.dispose();
    });

    test('重复 start 幂等', () async {
      final watcher = buildWatcher();
      await watcher.start();
      await watcher.start();
      expect(watcher.isRunning, isTrue);
      await watcher.dispose();
      expect(watcher.isRunning, isFalse);
    });
  });
}
