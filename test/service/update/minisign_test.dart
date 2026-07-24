import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/update/minisign.dart';

/// minisign 验签测试（对齐 Tauri updater 的更新包签名校验）。
///
/// 测试向量由 minisign 0.12 真实生成（prehashed/BLAKE2b 模式）：
/// ```
/// echo "" | minisign -G -p test_pub.key -s test_sec.key -W
/// echo "hello petallink update package" > test_payload.bin
/// minisign -S -s test_sec.key -m test_payload.bin -W
/// ```
/// 官方 `minisign -V` 已验证该向量有效。

/// 测试用公钥（base64，42 字节）
const _testPublicKeyBase64 = 'RWQumH1FCnRspPE49wySomZGx2w80Y4Yrt785hrQR5duOAcdhgVVb1Pu';

/// 测试用签名（minisign .sig 全文，4 行）
const _testSigText = '''untrusted comment: signature from minisign secret key
RUQumH1FCnRspNU2xY6zOPq5vZK16g5cmp0e3GzqTIkSkUQyH+zRTPuLJ/k/AkWByliYulOrRJzsPAsSaYubxLDtOxuWgXryDQM=
trusted comment: timestamp:1784872454\tfile:test_payload.bin\thashed
x7N6m+v81toEeyF+y5tuYgmgE3WGczy15xzF1EtRw4J/ln//l4eVGEqkRbaj5SlTaOtbuD6mTabudP4R6Y2/Dg==''';

/// 被签名的文件内容
final _testPayload =
    Uint8List.fromList('hello petallink update package\n'.codeUnits);

void main() {
  group('MinisignPublicKey.fromBase64', () {
    test('解析 42 字节公钥 → 拆出 32 字节 Ed25519 公钥', () {
      final pk = MinisignPublicKey.fromBase64(_testPublicKeyBase64);
      expect(pk.bytes.length, 42);
      expect(pk.ed25519PublicKey.length, 32);
    });

    test('长度非法 → 抛 FormatException', () {
      expect(
        () => MinisignPublicKey.fromBase64('dGVzdA=='), // 4 字节
        throwsA(isA<FormatException>()),
      );
    });

    test('sig_alg 非 Ed25519 → 抛 FormatException', () {
      // 构造 sig_alg 为 0x00 0x00 的非法公钥
      final bad = Uint8List(42)..[0] = 0x00..[1] = 0x00;
      expect(
        () => MinisignPublicKey.fromBase64(base64.encode(bad)),
        throwsA(isA<FormatException>()),
      );
    });
  });

  group('MinisignSignature.fromSigText', () {
    test('解析 4 行 .sig → prehashed 模式 + 64 字节签名', () {
      final sig = MinisignSignature.fromSigText(_testSigText);
      expect(sig.isPrehashed, isTrue); // "ED" = prehashed
      expect(sig.signature.length, 64);
    });

    test('行数不足 → 抛 FormatException', () {
      expect(
        () => MinisignSignature.fromSigText('only one line'),
        throwsA(isA<FormatException>()),
      );
    });
  });

  group('verifyMinisign（端到端验签）', () {
    test('真实 minisign 签名 + 原始文件 → 验签通过', () async {
      final pk = MinisignPublicKey.fromBase64(_testPublicKeyBase64);
      final sig = MinisignSignature.fromSigText(_testSigText);
      final ok = await verifyMinisign(
        publicKey: pk,
        signature: sig,
        fileBytes: _testPayload,
      );
      expect(ok, isTrue);
    });

    test('文件被篡改 → 验签失败', () async {
      final pk = MinisignPublicKey.fromBase64(_testPublicKeyBase64);
      final sig = MinisignSignature.fromSigText(_testSigText);
      // 篡改：改一个字节
      final tampered = Uint8List.fromList(_testPayload)..[0] ^= 0xFF;
      final ok = await verifyMinisign(
        publicKey: pk,
        signature: sig,
        fileBytes: tampered,
      );
      expect(ok, isFalse);
    });

    test('用错误的公钥验签 → 失败', () async {
      // 另一把无关公钥（合法格式但不同密钥）
      const wrongKey = 'RWRFuz2UYehJmK1q/bUx6XfRv3RnCYmMnX6rYK4l/Odxf96Y5XLi4MHt';
      final pk = MinisignPublicKey.fromBase64(wrongKey);
      final sig = MinisignSignature.fromSigText(_testSigText);
      final ok = await verifyMinisign(
        publicKey: pk,
        signature: sig,
        fileBytes: _testPayload,
      );
      expect(ok, isFalse);
    });
  });
}
