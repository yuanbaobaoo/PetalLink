/// minisign 签名验签（对齐 Tauri updater 的更新包签名校验）。
///
/// Tauri updater 用 [minisign](https://jedisct1.github.io/minisign/) 对
/// `.app.tar.gz` 更新包做 Ed25519 签名。本模块在 Dart 侧实现验签，
/// 使 Flutter 客户端能与 Tauri 客户端共用同一套更新签名体系。
///
/// ## 签名格式（minisign .sig 文本，4 行）
/// - 第 1 行：`untrusted comment: ...`（不参与验签）
/// - 第 2 行：base64，解码后 74 字节 = sig_alg(2) + key_id(8) + signature(64)
/// - 第 3 行：`trusted comment: ...`
/// - 第 4 行：base64，解码后 64 字节 = global_signature（trusted comment 的签名）
///
/// ## sig_alg 取值（决定 prehash 模式）
/// - `0x45 0x44`（ASCII "ED"）= **prehashed**：对文件 BLAKE2b-512 哈希后验签
/// - `0x45 0x64`（ASCII "Ed"）= **legacy**：对原始文件字节直接验签
///
/// Tauri 默认用 prehashed 模式（trusted comment 含 `hashed` 标记）。
/// 本模块两种模式都支持。
///
/// ## 公钥格式（minisign .pub 第 2 行 base64）
/// 解码后 42 字节 = sig_alg(2) + key_id(8) + public_key(32)。
/// 公钥的 sig_alg 仅标识密钥类型（Ed25519），不代表签名模式。
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';

/// minisign 公钥（解析自 base64 串）。
class MinisignPublicKey {
  /// 原始 42 字节：sig_alg(2) + key_id(8) + ed25519_pubkey(32)
  final Uint8List bytes;

  /// Ed25519 公钥（32 字节）
  final Uint8List ed25519PublicKey;

  MinisignPublicKey._(this.bytes, this.ed25519PublicKey);

  /// 从 base64 编码的公钥串解析（minisign .pub 文件第 2 行）。
  ///
  /// 抛 [FormatException]：长度非 42 或 sig_alg 非 Ed25519。
  factory MinisignPublicKey.fromBase64(String base64Str) {
    final decoded = base64.decode(base64Str.trim());
    if (decoded.length != 42) {
      throw FormatException(
          'minisign 公钥长度非法（期望 42 字节，实际 ${decoded.length}）');
    }
    // sig_alg[0..2]：公钥仅接受 Ed25519（0x45 0x64 "Ed" 或 0x45 0x44 "ED"）
    if (decoded[0] != 0x45 ||
        (decoded[1] != 0x64 && decoded[1] != 0x44)) {
      throw FormatException('minisign 公钥 sig_alg 非 Ed25519');
    }
    final pubkey = Uint8List.fromList(decoded.sublist(10, 42));
    return MinisignPublicKey._(Uint8List.fromList(decoded), pubkey);
  }
}

/// minisign 签名（解析自 .sig 文本）。
class MinisignSignature {
  /// sig_alg 标识是否 prehashed
  final bool isPrehashed;

  /// Ed25519 签名（64 字节）
  final Uint8List signature;

  MinisignSignature._(this.isPrehashed, this.signature);

  /// 从 minisign .sig 文本（4 行）解析签名块。
  ///
  /// 抛 [FormatException]：行数不足、第 2 行解码非 74 字节、sig_alg 非法。
  factory MinisignSignature.fromSigText(String sigText) {
    final lines = sigText.trim().split('\n');
    if (lines.length < 2) {
      throw FormatException('minisign 签名行数不足（期望至少 2 行）');
    }
    final sigBlock = base64.decode(lines[1].trim());
    if (sigBlock.length != 74) {
      throw FormatException(
          'minisign 签名块长度非法（期望 74 字节，实际 ${sigBlock.length}）');
    }
    // sig_alg[0..2]
    final alg0 = sigBlock[0];
    final alg1 = sigBlock[1];
    final bool isPrehashed;
    if (alg0 == 0x45 && alg1 == 0x44) {
      // "ED" = prehashed（BLAKE2b）
      isPrehashed = true;
    } else if (alg0 == 0x45 && alg1 == 0x64) {
      // "Ed" = legacy（raw）
      isPrehashed = false;
    } else {
      throw FormatException('minisign 签名 sig_alg 非法（0x${alg0.toRadixString(16)}${alg1.toRadixString(16)}）');
    }
    // signature[10..74]
    final signature = Uint8List.fromList(sigBlock.sublist(10, 74));
    return MinisignSignature._(isPrehashed, signature);
  }
}

/// 验证 minisign 签名（对齐 Tauri updater 的验签逻辑）。
///
/// [publicKey] minisign 公钥；[signature] 签名块；[fileBytes] 被签名的文件内容。
/// 签名 prehashed 时先对文件算 BLAKE2b-512 再验签；legacy 时直接对文件验签。
/// 返回 true 表示验签通过。
Future<bool> verifyMinisign({
  required MinisignPublicKey publicKey,
  required MinisignSignature signature,
  required List<int> fileBytes,
}) async {
  final ed25519 = Ed25519();
  final pubKey = SimplePublicKey(
    publicKey.ed25519PublicKey,
    type: KeyPairType.ed25519,
  );
  final sig = Signature(signature.signature, publicKey: pubKey);

  if (signature.isPrehashed) {
    // prehashed：BLAKE2b-512(文件) 作为验签消息
    final hash = await Blake2b(hashLengthInBytes: 64).hash(fileBytes);
    return ed25519.verify(hash.bytes, signature: sig);
  } else {
    // legacy：原始文件字节作为验签消息
    return ed25519.verify(fileBytes, signature: sig);
  }
}
