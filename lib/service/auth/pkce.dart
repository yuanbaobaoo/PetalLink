/// PKCE (Proof Key for Code Exchange) 生成器（OAuth 2.0 增强安全）。
///
/// 严格对齐 Rust 原版 `src/auth/pkce.rs`：
/// 华为 OAuth 支持授权码 + PKCE。code_verifier 随机生成，code_challenge
/// 为其 SHA256 的 base64url。授权请求带 challenge，换 token 时带 verifier。
library;

import 'dart:convert';
import 'dart:math';

import 'package:crypto/crypto.dart';

/// PKCE 对（对齐 Rust `PkcePair`）。
class PkcePair {
  /// 原始随机串，换 token 时回传给华为（仅本次会话使用）
  final String codeVerifier;

  /// codeVerifier 的 S256 摘要 base64url（去 = 填充），授权请求携带
  final String codeChallenge;

  const PkcePair({
    required this.codeVerifier,
    required this.codeChallenge,
  });

  /// 以不含 verifier 的摘要形式输出，避免泄露授权凭据（对齐 Rust Display）。
  @override
  String toString() =>
      'PkcePair(challenge=$codeChallenge, verifier=<hidden>)';
}

/// 生成随机 state（防 CSRF），返回 32 字节 hex（64 字符）。
///
/// 对齐 Rust `generate_state`（hex 编码，注意不是 base64url）。
String generateState() {
  final random = Random.secure();
  final bytes = List<int>.generate(32, (_) => random.nextInt(256));
  return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}

/// 生成 PKCE 对。code_verifier 为 64 字节随机数的 base64url（去 = 填充），
/// 长度约 86 字符（在 RFC 43-128 范围内）。method = S256。
///
/// 对齐 Rust `generate_pkce`。
PkcePair generatePkce() {
  final random = Random.secure();
  final bytes = List<int>.generate(64, (_) => random.nextInt(256));
  // base64url 去 = 填充（对齐 Rust URL_SAFE_NO_PAD）
  final verifier = base64Url.encode(bytes).replaceAll('=', '');

  // SHA256(verifier) → base64url 去 =
  final digest = sha256.convert(utf8.encode(verifier));
  final challenge = base64Url.encode(digest.bytes).replaceAll('=', '');

  return PkcePair(codeVerifier: verifier, codeChallenge: challenge);
}
