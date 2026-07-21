import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/mount/skip.dart';

void main() {
  group('MountSkip.shouldSkip 硬编码规则', () {
    test('.hwcloud_ 前缀一律跳过', () {
      expect(MountSkip.shouldSkip('.hwcloud_cache', []), isTrue);
      expect(MountSkip.shouldSkip('.hwcloud_freeup-1-abc', []), isTrue);
      // 中间出现不算前缀
      expect(MountSkip.shouldSkip('a.hwcloud_x', []), isFalse);
    });

    test('旧版占位符后缀跳过', () {
      expect(MountSkip.shouldSkip('a.txt.hwcloud_placeholder', []), isTrue);
      expect(MountSkip.shouldSkip('a.hwcloud_placeholder.bak', []), isFalse);
    });

    test('.tmp 后缀跳过', () {
      expect(MountSkip.shouldSkip('download.bin.tmp', []), isTrue);
      expect(MountSkip.shouldSkip('tmp.txt', []), isFalse);
    });

    test('普通文件不跳过', () {
      expect(MountSkip.shouldSkip('合同.docx', []), isFalse);
      expect(MountSkip.shouldSkip('photo.jpg', []), isFalse);
    });
  });

  group('MountSkip.globMatches 简化 glob', () {
    test('星号匹配任意串', () {
      expect(MountSkip.globMatches('~\$*', '~\$合同.docx'), isTrue);
      expect(MountSkip.globMatches('~\$*', '~\$'), isTrue);
      expect(MountSkip.globMatches('~\$*', '合同.docx'), isFalse);
      expect(MountSkip.globMatches('*.tmp', 'a.tmp'), isTrue);
      expect(MountSkip.globMatches('*.tmp', 'a.tmp.bak'), isFalse);
    });

    test('问号匹配单字符', () {
      expect(MountSkip.globMatches('fo?', 'foo'), isTrue);
      expect(MountSkip.globMatches('fo?', 'fo'), isFalse);
      expect(MountSkip.globMatches('fo?', 'fooo'), isFalse);
    });

    test('正则特殊字符被转义', () {
      expect(MountSkip.globMatches('.DS_Store', '.DS_Store'), isTrue);
      expect(MountSkip.globMatches('.DS_Store', 'xDS_Store'), isFalse);
      expect(MountSkip.globMatches('a+b', 'a+b'), isTrue);
      expect(MountSkip.globMatches('a+b', 'ab'), isFalse);
      expect(MountSkip.globMatches('a(b)', 'a(b)'), isTrue);
      expect(MountSkip.globMatches('[ab]', '[ab]'), isTrue);
      expect(MountSkip.globMatches('[ab]', 'a'), isFalse);
    });

    test('精确匹配默认模式', () {
      expect(MountSkip.globMatches('.Trash', '.Trash'), isTrue);
      expect(MountSkip.globMatches('.Trash', 'Trash'), isFalse);
    });
  });

  group('MountSkip.shouldSkip 用户模式', () {
    test('默认模式集', () {
      const patterns = MountSkip.defaultPatterns;
      expect(MountSkip.shouldSkip('.DS_Store', patterns), isTrue);
      expect(MountSkip.shouldSkip('~\$report.docx', patterns), isTrue);
      expect(MountSkip.shouldSkip('.Trash', patterns), isTrue);
      expect(MountSkip.shouldSkip('report.docx', patterns), isFalse);
    });
  });

  group('MountSkip.shouldSkipRelativePath', () {
    test('任意层级命中即跳过（对齐 Rust 路径级合同）', () {
      const patterns = ['.DS_Store', '~\$*'];
      expect(
          MountSkip.shouldSkipRelativePath('projects/legal/.DS_Store', patterns),
          isTrue);
      expect(MountSkip.shouldSkipRelativePath('projects/~\$contract.docx', patterns),
          isTrue);
      expect(MountSkip.shouldSkipRelativePath('projects/cache.tmp', patterns),
          isTrue);
      expect(MountSkip.shouldSkipRelativePath('projects/contract.docx', patterns),
          isFalse);
    });

    test('空段被忽略', () {
      expect(MountSkip.shouldSkipRelativePath('a//b.txt', []), isFalse);
    });
  });
}
