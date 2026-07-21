import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/auth/pkce.dart';

void main() {
  group('generateState', () {
    test('返回 32 字节 hex（64 字符，对齐 Rust hex::encode）', () {
      final state = generateState();
      expect(state.length, 64);
      expect(RegExp(r'^[0-9a-f]{64}$').hasMatch(state), isTrue);
    });

    test('两次生成不相同（随机性）', () {
      expect(generateState(), isNot(generateState()));
    });
  });

  group('generatePkce', () {
    test('code_verifier 为 64 字节随机数的 base64url 无填充（86 字符）', () {
      final pair = generatePkce();
      // 64 字节 → base64 88 字符去 2 个 = → 86 字符（RFC 7636 43-128 范围内）
      expect(pair.codeVerifier.length, 86);
      expect(pair.codeVerifier, isNot(contains('=')));
      expect(
        RegExp(r'^[A-Za-z0-9\-_]+$').hasMatch(pair.codeVerifier),
        isTrue,
      );
    });

    test('code_challenge = base64url(SHA256(verifier)) 无填充（S256）', () {
      final pair = generatePkce();
      final digest = sha256.convert(utf8.encode(pair.codeVerifier));
      final expected = base64Url.encode(digest.bytes).replaceAll('=', '');
      expect(pair.codeChallenge, expected);
      // SHA256 32 字节 → base64url 43 字符
      expect(pair.codeChallenge.length, 43);
    });

    test('RFC 7636 Appendix B 已知向量', () {
      // 对齐 RFC 7636 附录 B 的 S256 示例
      const verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
      const expectedChallenge = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';
      final digest = sha256.convert(utf8.encode(verifier));
      final challenge = base64Url.encode(digest.bytes).replaceAll('=', '');
      expect(challenge, expectedChallenge);
    });

    test('两次生成不相同（随机性）', () {
      final a = generatePkce();
      final b = generatePkce();
      expect(a.codeVerifier, isNot(b.codeVerifier));
      expect(a.codeChallenge, isNot(b.codeChallenge));
    });
  });

  group('PkcePair.toString', () {
    test('隐藏 verifier，仅展示 challenge（对齐 Rust Display）', () {
      const pair = PkcePair(
        codeVerifier: 'secret-verifier',
        codeChallenge: 'public-challenge',
      );
      final text = pair.toString();
      expect(text, contains('public-challenge'));
      expect(text, isNot(contains('secret-verifier')));
      expect(text, contains('<hidden>'));
    });
  });
}
