/// Token 存储 —— 机器码绑定的加密二进制文件。
///
/// 严格对齐 Rust 原版 `src/auth/token_store.rs`（方案 C）：
/// - `<Application Support>/token.bin`，自定义二进制格式，ChaCha20-Poly1305 AEAD 加密
/// - 加密密钥由本机 **IOPlatformUUID**（经 MethodChannel 从原生 IOKit 读取）
///   经 SHA-256 派生 → 绑定本机硬件
/// - 安全边界：
///   - 防跨机器复制：token.bin 拷到别的机器 → UUID 不同 → AEAD 解密失败 → 视为未登录
///   - 防篡改：AEAD 自带 Poly1305 完整性校验
///   - 不防本机攻击：本机任何进程可读同样的 UUID（IOPlatformUUID 非秘密）
/// - 文件权限 0600（仅 owner 读写）
/// - 失败行为：UUID 取不到/文件不存在/损坏/跨机器 → load 返回 null（未登录）
/// - token 绝不日志输出
library;

import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:cryptography/cryptography.dart';
import 'package:flutter/services.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/entity/auth.dart';

/// token 加密文件名（.bin，与旧版明文 token.json 区分 → 自动忽略旧文件）
const String tokenFileName = 'token.bin';

/// 文件格式魔数（版本标识，便于未来格式迁移）
final Uint8List tokenFileMagic = Uint8List.fromList('PTL1'.codeUnits);

/// ChaCha20-Poly1305 nonce 长度（12 字节）
const int tokenNonceLength = 12;

/// Token 存储接口（对齐 Rust `TokenStore` trait）。
abstract class TokenStore {
  /// 读取并解密已持久化的 token；不存在或解密失败时返回 null（视为未登录）。
  Future<TokenPair?> load();

  /// 加密并原子保存 token。
  Future<void> save(TokenPair token);

  /// 删除已持久化的 token（幂等）。
  Future<void> clear();
}

/// 本机 IOPlatformUUID 获取（macOS 原生 MethodChannel）。
///
/// 原生侧经 IOKit 读取 `kIOPlatformUUIDKey`（对齐 Rust `ioreg` 解析语义）。
class PlatformUuid {
  PlatformUuid._();

  /// 平台通道名（与 PlatformService 约定的通道一致）
  static const MethodChannel channel = MethodChannel('com.petallink/platform');

  /// 取本机 IOPlatformUUID。失败抛 [AppError]（调用方按未登录处理）。
  static Future<String> fetch() async {
    try {
      final uuid = await channel.invokeMethod<String>('getPlatformUUID');
      if (uuid == null || uuid.isEmpty) {
        throw AppError.generic('IOPlatformUUID 为空');
      }
      return uuid;
    } on AppError {
      rethrow;
    } catch (e) {
      throw AppError.generic('获取 IOPlatformUUID 失败：$e');
    }
  }
}

/// 加密文件存储：token.bin，机器码绑定的 ChaCha20-Poly1305 加密。
///
/// [directoryProvider] / [uuidProvider] 可注入，默认走
/// [AppPaths.supportDir] 与 [PlatformUuid.fetch]（测试时替换）。
class EncryptedFileTokenStore implements TokenStore {
  final Future<Directory> Function() _directoryProvider;
  final Future<String> Function() _uuidProvider;

  EncryptedFileTokenStore({
    Future<Directory> Function()? directoryProvider,
    Future<String> Function()? uuidProvider,
  })  : _directoryProvider = directoryProvider ?? AppPaths.supportDir,
        _uuidProvider = uuidProvider ?? PlatformUuid.fetch;

  /// token.bin 完整路径（`Application Support / <bundle_id> / token.bin`）
  Future<File> _file() async =>
      File('${(await _directoryProvider()).path}/$tokenFileName');

  /// 读取 token 文件；文件不可读或认证失败均按未登录处理。
  @override
  Future<TokenPair?> load() async {
    final file = await _file();
    if (!file.existsSync()) return null;
    final Uint8List raw;
    try {
      raw = await file.readAsBytes();
    } catch (e) {
      AppLogger.w('token 文件读取失败', e);
      return null;
    }
    // 解密失败一律视为未登录（损坏/跨机器/UUID 变更）
    try {
      final token = await decryptToken(raw, uuidProvider: _uuidProvider);
      AppLogger.i('从加密 token 文件恢复登录态');
      return token;
    } catch (e) {
      AppLogger.w('token 解密失败（损坏/跨机器/UUID 变更？），视为未登录', e);
      return null;
    }
  }

  /// 加密 token 并通过临时文件替换完成原子写入。
  @override
  Future<void> save(TokenPair token) async {
    final file = await _file();
    final parent = file.parent;
    if (!parent.existsSync()) {
      await parent.create(recursive: true);
    }
    final encrypted = await encryptToken(token, uuidProvider: _uuidProvider);
    // 原子写：先写临时文件再重命名，避免中途崩溃产生半截文件
    final tmp = File('${file.parent.path}/token.tmp');
    await tmp.writeAsBytes(encrypted, flush: true);
    // 收紧权限 0600（仅 owner 读写）
    try {
      final result = await Process.run('chmod', ['600', tmp.path]);
      if (result.exitCode != 0) {
        throw AppError.generic('收紧 token 文件权限失败：${result.stderr}');
      }
    } catch (e) {
      if (e is AppError) rethrow;
      throw AppError.generic('收紧 token 文件权限失败：$e');
    }
    await tmp.rename(file.path);
    AppLogger.i('token 已加密保存到本地文件（机器码绑定，权限 600）');
  }

  /// 删除本机 token 文件；文件不存在视为成功（幂等）。
  @override
  Future<void> clear() async {
    final file = await _file();
    if (!file.existsSync()) return;
    try {
      await file.delete();
    } catch (e) {
      throw AppError.generic('清除 token 文件失败：$e');
    }
    AppLogger.i('已清除 token 文件');
  }
}

// ===== 机器码 + 密钥派生 =====

/// 密钥派生：SHA-256(machine_uuid) → 32 字节。
///
/// UUID 本身高熵，无需慢哈希；不加 salt（salt 会随文件走，失去绑机器意义）。
Uint8List deriveTokenKey(String uuid) {
  return Uint8List.fromList(sha256.convert(utf8.encode(uuid)).bytes);
}

// ===== 加密 / 解密 =====

/// 加密 token：序列化明文 → 随机 nonce → ChaCha20-Poly1305 加密 → 拼装文件格式。
///
/// 文件格式：`[魔数 4B][nonce 12B][密文+tag]`（对齐 Rust `encrypt_token`）。
Future<Uint8List> encryptToken(
  TokenPair token, {
  required Future<String> Function() uuidProvider,
}) async {
  // 密钥派生（UUID 取不到则无法加密）
  final uuid = await uuidProvider();
  final key = deriveTokenKey(uuid);

  // 随机 nonce（每次保存重新生成，AEAD 安全性靠 nonce 不重用）
  final random = Random.secure();
  final nonce =
      Uint8List.fromList(List<int>.generate(tokenNonceLength, (_) => random.nextInt(256)));

  // 序列化明文（紧凑二进制，length-prefixed 小端）
  final plaintext = serializeToken(token);

  // 加密（密文含 16B Poly1305 tag）
  final box = await Chacha20.poly1305Aead().encrypt(
    plaintext,
    secretKey: SecretKey(key),
    nonce: nonce,
  );

  // 拼装文件格式
  final builder = BytesBuilder()
    ..add(tokenFileMagic)
    ..add(nonce)
    ..add(box.cipherText)
    ..add(box.mac.bytes);
  return builder.toBytes();
}

/// 解密 token：校验魔数 → 取 nonce → AEAD 解密 → 反序列化。
///
/// 任何步骤失败抛 [AppError]（调用方据此判定未登录）。
/// 对齐 Rust `decrypt_token`。
Future<TokenPair> decryptToken(
  Uint8List raw, {
  required Future<String> Function() uuidProvider,
}) async {
  // 校验最小长度：魔数 + nonce + 至少 16B tag
  if (raw.length < tokenFileMagic.length + tokenNonceLength + 16) {
    throw AppError.generic('token 文件长度异常');
  }

  // 校验魔数
  for (var i = 0; i < tokenFileMagic.length; i++) {
    if (raw[i] != tokenFileMagic[i]) {
      throw AppError.generic('token 文件魔数不匹配');
    }
  }

  // 读取 nonce，剩余为密文 + tag
  final nonce = Uint8List.fromList(
      raw.sublist(tokenFileMagic.length, tokenFileMagic.length + tokenNonceLength));
  final ciphertextWithTag = raw.sublist(tokenFileMagic.length + tokenNonceLength);
  final tagStart = ciphertextWithTag.length - 16;
  final ciphertext = ciphertextWithTag.sublist(0, tagStart);
  final tag = ciphertextWithTag.sublist(tagStart);

  // 派生本机密钥并解密（UUID 变化/跨机器 → AEAD 失败）
  final uuid = await uuidProvider();
  final key = deriveTokenKey(uuid);
  final List<int> plaintext;
  try {
    plaintext = await Chacha20.poly1305Aead().decrypt(
      SecretBox(ciphertext, nonce: nonce, mac: Mac(tag)),
      secretKey: SecretKey(key),
    );
  } catch (e) {
    throw AppError.generic('token 解密失败：$e');
  }

  return deserializeToken(Uint8List.fromList(plaintext));
}

// ===== 二进制序列化（length-prefixed，小端） =====

/// 序列化 token 为紧凑二进制（对齐 Rust `serialize_token`）。
///
/// 明文布局（小端）：
/// - `[u64 access_len][access_bytes]`
/// - `[u64 refresh_len][refresh_bytes]`
/// - `[i64 expires_at]`（毫秒）
/// - `[u32 token_type_len][token_type_bytes]`
/// - `[u8 scope_present][u64 scope_len][scope_bytes]`（present=0 时后续省略）
Uint8List serializeToken(TokenPair token) {
  final builder = BytesBuilder();
  final access = utf8.encode(token.accessToken);
  final refresh = utf8.encode(token.refreshToken);
  final type = utf8.encode(token.tokenType);

  builder.add(_u64Le(access.length));
  builder.add(access);
  builder.add(_u64Le(refresh.length));
  builder.add(refresh);
  builder.add(_i64Le(token.expiresAt));
  builder.add(_u32Le(type.length));
  builder.add(type);

  final scopeText = token.scope;
  if (scopeText != null) {
    final scope = utf8.encode(scopeText);
    builder.addByte(1);
    builder.add(_u64Le(scope.length));
    builder.add(scope);
  } else {
    builder.addByte(0);
  }
  return builder.toBytes();
}

/// 反序列化紧凑二进制为 token（对齐 Rust `deserialize_token`）。
///
/// 任何截断/越界/UTF-8 非法均抛 [AppError]。
TokenPair deserializeToken(Uint8List data) {
  var pos = 0;

  String readStringU64() {
    final len = _readU64Le(data, pos);
    pos += 8;
    final end = pos + len;
    if (len < 0 || end > data.length) {
      throw AppError.generic('读取字符串内容失败：长度越界');
    }
    final bytes = data.sublist(pos, end);
    pos = end;
    try {
      return utf8.decode(bytes);
    } catch (e) {
      throw AppError.generic('UTF-8 解码失败：$e');
    }
  }

  String readStringU32() {
    final len = _readU32Le(data, pos);
    pos += 4;
    final end = pos + len;
    if (end > data.length) {
      throw AppError.generic('读取字符串内容失败：长度越界');
    }
    final bytes = data.sublist(pos, end);
    pos = end;
    try {
      return utf8.decode(bytes);
    } catch (e) {
      throw AppError.generic('UTF-8 解码失败：$e');
    }
  }

  final accessToken = readStringU64();
  final refreshToken = readStringU64();
  if (pos + 8 > data.length) {
    throw AppError.generic('读取 expires_at 失败');
  }
  final expiresAt = _readI64Le(data, pos);
  pos += 8;
  final tokenType = readStringU32();
  if (pos + 1 > data.length) {
    throw AppError.generic('读取 scope 标志失败');
  }
  final present = data[pos];
  pos += 1;
  final scope = present == 1 ? readStringU64() : null;

  return TokenPair(
    accessToken: accessToken,
    refreshToken: refreshToken,
    expiresAt: expiresAt,
    tokenType: tokenType,
    scope: scope,
  );
}

// ===== 小端编码/解码辅助 =====

Uint8List _u64Le(int v) {
  final data = ByteData(8)..setUint64(0, v, Endian.little);
  return data.buffer.asUint8List();
}

Uint8List _u32Le(int v) {
  final data = ByteData(4)..setUint32(0, v, Endian.little);
  return data.buffer.asUint8List();
}

Uint8List _i64Le(int v) {
  final data = ByteData(8)..setInt64(0, v, Endian.little);
  return data.buffer.asUint8List();
}

int _readU64Le(Uint8List data, int offset) {
  if (offset + 8 > data.length) {
    throw AppError.generic('读取长度失败');
  }
  return ByteData.sublistView(data, offset, offset + 8)
      .getUint64(0, Endian.little);
}

int _readU32Le(Uint8List data, int offset) {
  if (offset + 4 > data.length) {
    throw AppError.generic('读取长度失败');
  }
  return ByteData.sublistView(data, offset, offset + 4)
      .getUint32(0, Endian.little);
}

int _readI64Le(Uint8List data, int offset) {
  return ByteData.sublistView(data, offset, offset + 8)
      .getInt64(0, Endian.little);
}
