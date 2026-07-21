import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/service/auth/token_store.dart';

void main() {
  const testUuid = 'TEST-UUID-0000-1111-2222';
  Future<String> fixedUuid() async => testUuid;

  final sampleToken = TokenPair(
    accessToken: 'access-令牌',
    refreshToken: 'refresh-token',
    expiresAt: 1700000000000,
    tokenType: 'Bearer',
    scope: 'openid profile',
  );

  group('serializeToken / deserializeToken', () {
    test('序列化布局与 Rust 字节级一致（length-prefixed 小端）', () {
      const token = TokenPair(
        accessToken: 'at',
        refreshToken: 'rt',
        expiresAt: 0x0102030405060708,
        tokenType: 'Bearer',
        scope: 's',
      );
      final bytes = serializeToken(token);

      final expected = BytesBuilder();
      expected.add(_u64Le(2));
      expected.add(utf8.encode('at'));
      expected.add(_u64Le(2));
      expected.add(utf8.encode('rt'));
      expected.add(_i64Le(0x0102030405060708));
      expected.add(_u32Le(6));
      expected.add(utf8.encode('Bearer'));
      expected.addByte(1);
      expected.add(_u64Le(1));
      expected.add(utf8.encode('s'));

      expect(bytes, expected.toBytes());
    });

    test('scope 缺省时仅写 present=0（对齐 Rust None 分支）', () {
      const token = TokenPair(
        accessToken: 'a',
        refreshToken: 'r',
        expiresAt: 1,
        tokenType: 'Bearer',
      );
      final bytes = serializeToken(token);
      expect(bytes.last, 0);
      final restored = deserializeToken(bytes);
      expect(restored.scope, isNull);
    });

    test('往返一致（含中文 / scope）', () {
      final restored = deserializeToken(serializeToken(sampleToken));
      expect(restored.accessToken, sampleToken.accessToken);
      expect(restored.refreshToken, sampleToken.refreshToken);
      expect(restored.expiresAt, sampleToken.expiresAt);
      expect(restored.tokenType, sampleToken.tokenType);
      expect(restored.scope, sampleToken.scope);
    });

    test('截断数据反序列化抛 AppError', () {
      final bytes = serializeToken(sampleToken);
      expect(
        () => deserializeToken(Uint8List.fromList(bytes.sublist(0, 5))),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('ChaCha20-Poly1305 AEAD（cryptography 用法守卫）', () {
    test('RFC 8439 §2.8.2 已知向量', () async {
      final key = _hex(
          '808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f');
      final nonce = _hex('070000004041424344454647');
      final aad = _hex('50515253c0c1c2c3c4c5c6c7');
      final plaintext = utf8.encode(
          "Ladies and Gentlemen of the class of '99: If I could offer you "
          'only one tip for the future, sunscreen would be it.');
      const expectedCipher =
          'd31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6'
          '3dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b36'
          '92ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc'
          '3ff4def08e4b7a9de576d26586cec64b6116';
      const expectedTag = '1ae10b594f09e26a7e902ecbd0600691';

      final algorithm = Chacha20.poly1305Aead();
      final box = await algorithm.encrypt(
        plaintext,
        secretKey: SecretKey(key),
        nonce: nonce,
        aad: aad,
      );
      expect(_bytesToHex(box.cipherText), expectedCipher);
      expect(_bytesToHex(box.mac.bytes), expectedTag);

      final decrypted = await algorithm.decrypt(
        SecretBox(_hex(expectedCipher), nonce: nonce, mac: Mac(_hex(expectedTag))),
        secretKey: SecretKey(key),
        aad: aad,
      );
      expect(decrypted, plaintext);
    });
  });

  group('encryptToken / decryptToken', () {
    test('往返一致，文件格式为 [PTL1][nonce 12B][密文+tag]', () async {
      final encrypted = await encryptToken(sampleToken, uuidProvider: fixedUuid);

      // 魔数
      expect(utf8.decode(encrypted.sublist(0, 4)), 'PTL1');
      // 总长度 = 4 + 12 + 明文长度 + 16
      expect(
        encrypted.length,
        4 + tokenNonceLength + serializeToken(sampleToken).length + 16,
      );

      final restored =
          await decryptToken(encrypted, uuidProvider: fixedUuid);
      expect(restored.accessToken, sampleToken.accessToken);
      expect(restored.refreshToken, sampleToken.refreshToken);
      expect(restored.expiresAt, sampleToken.expiresAt);
      expect(restored.tokenType, sampleToken.tokenType);
      expect(restored.scope, sampleToken.scope);
    });

    test('每次加密使用随机 nonce（两次密文不同）', () async {
      final a = await encryptToken(sampleToken, uuidProvider: fixedUuid);
      final b = await encryptToken(sampleToken, uuidProvider: fixedUuid);
      expect(a, isNot(b));
      // nonce 段（4..16）不同
      expect(a.sublist(4, 16), isNot(b.sublist(4, 16)));
    });

    test('魔数不匹配抛 AppError', () async {
      final encrypted = await encryptToken(sampleToken, uuidProvider: fixedUuid);
      encrypted[0] = 0x58; // 'X'
      expect(
        () => decryptToken(encrypted, uuidProvider: fixedUuid),
        throwsA(isA<AppError>()),
      );
    });

    test('文件过短抛 AppError', () async {
      expect(
        () => decryptToken(Uint8List.fromList('PTL1'.codeUnits),
            uuidProvider: fixedUuid),
        throwsA(isA<AppError>()),
      );
    });

    test('UUID 不同（跨机器）AEAD 解密失败', () async {
      final encrypted = await encryptToken(sampleToken, uuidProvider: fixedUuid);
      Future<String> otherUuid() async => 'OTHER-MACHINE-UUID';
      expect(
        () => decryptToken(encrypted, uuidProvider: otherUuid),
        throwsA(isA<AppError>()),
      );
    });

    test('篡改任一密文字节 → Poly1305 校验失败', () async {
      final encrypted = await encryptToken(sampleToken, uuidProvider: fixedUuid);
      encrypted[encrypted.length - 3] ^= 0xFF;
      expect(
        () => decryptToken(encrypted, uuidProvider: fixedUuid),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('EncryptedFileTokenStore', () {
    late Directory tempDir;
    late EncryptedFileTokenStore store;

    setUp(() {
      tempDir = Directory.systemTemp.createTempSync('token_store_test');
      store = EncryptedFileTokenStore(
        directoryProvider: () async => tempDir,
        uuidProvider: fixedUuid,
      );
    });

    tearDown(() {
      if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
    });

    test('save/load 往返一致，文件以 PTL1 开头且权限为 0600', () async {
      await store.save(sampleToken);

      final file = File('${tempDir.path}/$tokenFileName');
      expect(file.existsSync(), isTrue);
      final raw = await file.readAsBytes();
      expect(utf8.decode(raw.sublist(0, 4)), 'PTL1');

      // 文件权限 0600（仅 owner 读写）
      final mode = file.statSync().mode & 0x1FF;
      expect(mode, 0x180);

      final loaded = await store.load();
      expect(loaded, isNotNull);
      expect(loaded!.accessToken, sampleToken.accessToken);
      expect(loaded.refreshToken, sampleToken.refreshToken);
      expect(loaded.expiresAt, sampleToken.expiresAt);
      expect(loaded.scope, sampleToken.scope);
    });

    test('文件不存在时 load 返回 null（未登录）', () async {
      expect(await store.load(), isNull);
    });

    test('损坏文件 load 返回 null（视为未登录，对齐 Rust）', () async {
      final file = File('${tempDir.path}/$tokenFileName');
      await file.writeAsBytes(utf8.encode('garbage-not-encrypted'));
      expect(await store.load(), isNull);
    });

    test('UUID 变更（跨机器）load 返回 null', () async {
      await store.save(sampleToken);
      final otherStore = EncryptedFileTokenStore(
        directoryProvider: () async => tempDir,
        uuidProvider: () async => 'OTHER-MACHINE-UUID',
      );
      expect(await otherStore.load(), isNull);
    });

    test('clear 删除文件且幂等', () async {
      await store.save(sampleToken);
      await store.clear();
      expect(await store.load(), isNull);
      // 幂等：再次 clear 不抛错
      await store.clear();
    });

    test('save 为原子写（临时文件不残留）', () async {
      await store.save(sampleToken);
      final entries = tempDir.listSync().map((e) => e.path).toList();
      expect(entries, hasLength(1));
      expect(entries.single, endsWith(tokenFileName));
    });
  });
}

// ===== 测试辅助（独立实现小端编码，交叉校验生产布局） =====

Uint8List _u64Le(int v) =>
    (ByteData(8)..setUint64(0, v, Endian.little)).buffer.asUint8List();

Uint8List _u32Le(int v) =>
    (ByteData(4)..setUint32(0, v, Endian.little)).buffer.asUint8List();

Uint8List _i64Le(int v) =>
    (ByteData(8)..setInt64(0, v, Endian.little)).buffer.asUint8List();

Uint8List _hex(String hex) {
  final out = Uint8List(hex.length ~/ 2);
  for (var i = 0; i < out.length; i++) {
    out[i] = int.parse(hex.substring(i * 2, i * 2 + 2), radix: 16);
  }
  return out;
}

String _bytesToHex(List<int> bytes) {
  return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}
