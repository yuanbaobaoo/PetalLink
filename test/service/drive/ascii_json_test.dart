import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/drive/ascii_json.dart';

void main() {
  group('asciiJsonEncode（对齐 Rust ascii_json.rs）', () {
    test('纯 ASCII 内容保持不变', () {
      expect(asciiJsonEncode({'fileName': 'hello.txt', 'size': 5}),
          '{"fileName":"hello.txt","size":5}');
    });

    test('中文转义为 \\uXXXX（小写 hex）', () {
      expect(asciiJsonEncode({'fileName': '那你'}),
          '{"fileName":"\\u90a3\\u4f60"}');
    });

    test('BMP 边界：U+007F 不转义，U+0080 转义', () {
      expect(escapeNonAscii('\u007F'), '\u007F');
      expect(escapeNonAscii('\u0080'), '\\u0080');
      // é（U+00E9）→ 小写 hex
      expect(escapeNonAscii('é'), '\\u00e9');
    });

    test('辅助平面字符（emoji）转义为 UTF-16 代理对', () {
      // 👍 U+1F44D → 高代理 D83D + 低代理 DC4D
      expect(asciiJsonEncode({'fileName': 'a👍b'}),
          '{"fileName":"a\\ud83d\\udc4db"}');
    });

    test('混合中英文与特殊字符', () {
      expect(asciiJsonEncode({'fileName': '新建 文件夹(1).txt'}),
          '{"fileName":"\\u65b0\\u5efa \\u6587\\u4ef6\\u5939(1).txt"}');
    });

    test('控制字符仍由 jsonEncode 标准转义（\\n），不受影响', () {
      expect(asciiJsonEncode({'a': 'x\ny'}), '{"a":"x\\ny"}');
    });

    test('escapeNonAscii 只转义 > 0x7F，JSON 结构字符保留', () {
      expect(escapeNonAscii('{"k":"值"}'), '{"k":"\\u503c"}');
    });

    test('数组与嵌套对象中的中文全部转义', () {
      expect(asciiJsonEncode({'parentFolder': ['根目录']}),
          '{"parentFolder":["\\u6839\\u76ee\\u5f55"]}');
    });
  });
}
