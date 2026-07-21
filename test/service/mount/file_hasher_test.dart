import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/service/mount/file_hasher.dart';

void main() {
  late Directory tempDir;
  late FileHasher hasher;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_hasher_test');
    hasher = FileHasher();
  });

  tearDown(() {
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  String tempPath(String name) => '${tempDir.path}/$name';

  group('FileHasher 已知向量', () {
    test('abc', () async {
      File(tempPath('a')).writeAsStringSync('abc');
      expect(
        await hasher.hashFile(tempPath('a')),
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
      );
    });

    test('空文件', () async {
      File(tempPath('empty')).writeAsBytesSync(const []);
      expect(
        await hasher.hashFile(tempPath('empty')),
        'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      );
    });

    test('一百万个 a（流式多块读取）', () async {
      final file = File(tempPath('big'));
      final raf = file.openSync(mode: FileMode.write);
      final chunk = List<int>.filled(97, 0x61); // 'a'
      for (var i = 0; i < 1000000 ~/ 97; i++) {
        raf.writeFromSync(chunk);
      }
      raf.writeFromSync(List<int>.filled(1000000 % 97, 0x61));
      raf.closeSync();
      expect(file.lengthSync(), 1000000);
      expect(
        await hasher.hashFile(file.path),
        'cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0',
      );
    });

    test('不存在的文件抛 AppError', () async {
      expect(
        () => hasher.hashFile(tempPath('missing')),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('FileHasher 缓存', () {
    /// dart:io setLastModified 仅秒级精度，统一用整秒 mtime 做缓存键。
    final wholeSecond =
        DateTime.fromMillisecondsSinceEpoch(1700000000000);

    test('mtime+size 未变时返回缓存（不重算）', () async {
      final path = tempPath('cached');
      File(path).writeAsStringSync('abc');
      File(path).setLastModifiedSync(wholeSecond);
      final first = await hasher.hashFile(path);

      // 同尺寸内容替换并恢复 mtime → 缓存命中，返回旧哈希
      File(path).writeAsStringSync('abd');
      File(path).setLastModifiedSync(wholeSecond);

      expect(await hasher.hashFile(path), first);
    });

    test('mtime 变化后重算', () async {
      final path = tempPath('changed');
      File(path).writeAsStringSync('abc');
      File(path).setLastModifiedSync(wholeSecond);
      await hasher.hashFile(path);

      File(path).writeAsStringSync('abd');
      File(path).setLastModifiedSync(
          wholeSecond.add(const Duration(seconds: 2)));

      expect(
        await hasher.hashFile(path),
        // sha256('abd')
        FileHasher.sha256OfString('abd'),
      );
    });

    test('invalidate 强制重算', () async {
      final path = tempPath('inv');
      File(path).writeAsStringSync('abc');
      File(path).setLastModifiedSync(wholeSecond);
      await hasher.hashFile(path);

      File(path).writeAsStringSync('abd');
      File(path).setLastModifiedSync(wholeSecond);
      hasher.invalidate(path);

      expect(await hasher.hashFile(path), FileHasher.sha256OfString('abd'));
    });

    test('clear 清空全部缓存', () async {
      final path = tempPath('clr');
      File(path).writeAsStringSync('abc');
      File(path).setLastModifiedSync(wholeSecond);
      await hasher.hashFile(path);
      hasher.clear();

      File(path).writeAsStringSync('abd');
      File(path).setLastModifiedSync(wholeSecond);

      expect(await hasher.hashFile(path), FileHasher.sha256OfString('abd'));
    });
  });

  group('FileHasher.hashRange', () {
    test('区间哈希对齐整段内容哈希', () async {
      File(tempPath('range')).writeAsStringSync('hello world');
      expect(
        await hasher.hashRange(tempPath('range'), 0, 5),
        FileHasher.sha256OfString('hello'),
      );
      expect(
        await hasher.hashRange(tempPath('range'), 6, 5),
        FileHasher.sha256OfString('world'),
      );
    });

    test('区间超出文件末尾按实际可读计算', () async {
      File(tempPath('short')).writeAsStringSync('abc');
      expect(
        await hasher.hashRange(tempPath('short'), 0, 100),
        FileHasher.sha256OfString('abc'),
      );
    });
  });

  group('FileHasher.sha256OfString', () {
    test('对齐已知向量', () {
      expect(
        FileHasher.sha256OfString('abc'),
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
      );
      expect(
        FileHasher.sha256OfString(''),
        'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      );
    });
  });
}
