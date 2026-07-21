import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:intl/intl.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/sync/conflict.dart';

/// ConflictResolver 测试（对齐 Rust conflict.rs：60s 规则 + 副本命名去重）。

DriveFile cloudEdited(int editedMs) {
  return DriveFile(
    id: 'f1',
    name: 'a.txt',
    editedTime:
        DateTime.fromMillisecondsSinceEpoch(editedMs, isUtc: true),
  );
}

void main() {
  late Directory dir;
  late ConflictResolver resolver;

  setUp(() {
    dir = Directory.systemTemp.createTempSync('conflict_test');
    resolver = ConflictResolver();
  });

  tearDown(() {
    if (dir.existsSync()) dir.deleteSync(recursive: true);
  });

  group('60s 规则', () {
    test('本地比云端晚 > 60s → 本地赢', () async {
      final cloudMs = 1700000000000;
      final localMs = cloudMs + 61 * 1000;
      final r = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), localMs);
      expect(r.winner, ConflictSide.local);
      // 副本标签来自败方（云端）
      expect(p.basename(r.copyPath), contains('云端副本'));
    });

    test('本地恰好晚 60s → 云端赢（必须严格大于）', () async {
      final cloudMs = 1700000000000;
      final localMs = cloudMs + 60 * 1000;
      final r = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), localMs);
      expect(r.winner, ConflictSide.cloud);
      expect(p.basename(r.copyPath), contains('本地副本'));
    });

    test('本地比云端早 → 云端赢', () async {
      final cloudMs = 1700000000000;
      final r = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), cloudMs - 5000);
      expect(r.winner, ConflictSide.cloud);
    });

    test('云端 editedTime 缺失 → 按现在处理（本地早 → 云端赢）', () async {
      const file = DriveFile(id: 'f1', name: 'a.txt');
      final r = await resolver.resolve(
          p.join(dir.path, 'a.txt'), file, 1000000000000);
      expect(r.winner, ConflictSide.cloud);
    });
  });

  group('副本命名与去重', () {
    test('命名格式：`stem (本地副本 YYYY-MM-DD HH-mm-ss).ext`', () async {
      final cloudMs = 1700000000000; // 2023-11-14T22:13:20Z
      final r = await resolver.resolve(
          p.join(dir.path, 'report.pdf'), cloudEdited(cloudMs), cloudMs - 1000);
      final name = p.basename(r.copyPath);
      expect(name, startsWith('report (本地副本 '));
      expect(name, endsWith(').pdf'));
      final expected = DateFormat('yyyy-MM-dd HH-mm-ss').format(
          DateTime.fromMillisecondsSinceEpoch(cloudMs - 1000, isUtc: true));
      expect(name, contains(expected));
    });

    test('时间戳来自败方（本地赢时用云端时间）', () async {
      final cloudMs = 1700000000000;
      final r = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), cloudMs + 120000);
      final expected = DateFormat('yyyy-MM-dd HH-mm-ss').format(
          DateTime.fromMillisecondsSinceEpoch(cloudMs, isUtc: true));
      expect(p.basename(r.copyPath), contains(expected));
    });

    test('撞名加序号 `(1)`，序号递增', () async {
      final cloudMs = 1700000000000;
      final localMs = cloudMs - 1000;
      final r1 = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), localMs);
      // 占用首选路径
      File(r1.copyPath).writeAsStringSync('x');
      final r2 = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), localMs);
      expect(p.basename(r2.copyPath), contains(' (1).txt'));
      File(r2.copyPath).writeAsStringSync('x');
      final r3 = await resolver.resolve(
          p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), localMs);
      expect(p.basename(r3.copyPath), contains(' (2).txt'));
    });

    test('无扩展名文件', () async {
      final cloudMs = 1700000000000;
      final r = await resolver.resolve(
          p.join(dir.path, 'Makefile'), cloudEdited(cloudMs), cloudMs - 1000);
      expect(p.basename(r.copyPath), startsWith('Makefile (本地副本 '));
      expect(p.basename(r.copyPath), endsWith(')'));
    });
  });

  test('冲突日志 newest-first 记录', () async {
    final cloudMs = 1700000000000;
    await resolver.resolve(
        p.join(dir.path, 'a.txt'), cloudEdited(cloudMs), cloudMs - 1000);
    await resolver.resolve(
        p.join(dir.path, 'b.txt'), cloudEdited(cloudMs), cloudMs + 120000);
    expect(resolver.log.length, 2);
    expect(resolver.log.first, contains('b.txt'));
    expect(resolver.log.first, contains('正本=本地'));
    expect(resolver.log.last, contains('正本=云端'));
  });
}
