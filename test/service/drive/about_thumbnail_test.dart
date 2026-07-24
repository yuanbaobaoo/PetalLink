import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/service/drive/about_service.dart';
import 'package:petal_link/service/drive/thumbnail_service.dart';

import '../auth/fake_http.dart';
import 'drive_test_util.dart';

void main() {
  group('AboutService（GET /about?fields=*）', () {
    test('配额字段为 String 时容忍解析', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#about',
          'storageQuota': {
            'userCapacity': '16106127360',
            'usedSpace': '1073741824',
          },
          'user': {'displayName': '测试用户'},
        });
      });
      final service = AboutService(buildTestClient(adapter));

      final about = (await service.get()).unwrap();

      expect(about.userCapacity, 16106127360);
      expect(about.usedSpace, 1073741824);
      expect(about.userDisplayName, '测试用户');
      expect(about.remainingSpace, 16106127360 - 1073741824);
      final req = adapter.requests.single;
      expect(req.uri.path, '/drive/v1/about');
      expect(req.uri.query, 'fields=*');
    });

    test('缺失 storageQuota 时回退顶层字段', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'userCapacity': 100,
          'usedSpace': 40,
        });
      });
      final service = AboutService(buildTestClient(adapter));

      final about = (await service.get()).unwrap();
      expect(about.userCapacity, 100);
      expect(about.usedSpace, 40);
    });

    test('ensureCapacity：空间足够 → Ok', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'storageQuota': {'userCapacity': '100', 'usedSpace': '40'},
        });
      });
      final service = AboutService(buildTestClient(adapter));

      expect((await service.ensureCapacity(60)).isOk, isTrue);
    });

    test('ensureCapacity：空间不足 → QuotaExceededError 携带所需/剩余',
        () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'storageQuota': {'userCapacity': '100', 'usedSpace': '40'},
        });
      });
      final service = AboutService(buildTestClient(adapter));

      final result = await service.ensureCapacity(61);

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<QuotaExceededError>());
      expect((error as QuotaExceededError).required, 61);
      expect(error.remaining, 60);
    });
  });

  group('ThumbnailService（GET /thumbnails/{id}?form=content）', () {
    test('200 → 返回二进制字节', () async {
      final adapter = FakeHttpAdapter((req) {
        return ResponseBody.fromBytes(
          Uint8List.fromList(const [1, 2, 3, 255]),
          200,
          headers: {
            Headers.contentTypeHeader: ['image/jpeg'],
          },
        );
      });
      final service = ThumbnailService(buildTestClient(adapter));

      final bytes = (await service.getThumbnail('f1')).unwrap();

      expect(bytes, Uint8List.fromList(const [1, 2, 3, 255]));
      final req = adapter.requests.single;
      expect(req.uri.path, '/drive/v1/thumbnails/f1');
      expect(req.uri.query, 'form=content');
      expect(req.headers['Authorization'], 'Bearer test-token');
    });

    test('404 → 结构化 DriveApiError（对齐 Rust，不吞错）', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {'errorCode': '404'}, status: 404);
      });
      final service = ThumbnailService(buildTestClient(adapter));

      final result = await service.getThumbnail('missing');

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).statusCode, 404);
    });

    test('通用二进制响应 + PNG 魔数 → 按 MIME 签名识别通过', () async {
      // 服务端常返回 application/octet-stream，需按文件头识别真实格式
      final pngBytes = Uint8List.fromList(
          [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]);
      final adapter = FakeHttpAdapter((req) {
        return ResponseBody.fromBytes(pngBytes, 200,
            headers: {
              Headers.contentTypeHeader: ['application/octet-stream'],
            });
      });
      final service = ThumbnailService(buildTestClient(adapter));

      final result = await service.getThumbnail('f1');

      expect(result.isOk, isTrue);
      expect((result as Ok<Uint8List>).value, pngBytes);
    });

    test('非图片二进制（如错误页 HTML）→ 拒绝，防 Image.memory 解码失败', () async {
      final htmlBytes = Uint8List.fromList('<html>err</html>'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        return ResponseBody.fromBytes(htmlBytes, 200,
            headers: {
              Headers.contentTypeHeader: ['text/html'],
            });
      });
      final service = ThumbnailService(buildTestClient(adapter));

      final result = await service.getThumbnail('f1');

      expect(result.isErr, isTrue);
    });

    test('getThumbnailDataUrl → 生成 data:image/...;base64,...', () async {
      final pngBytes = Uint8List.fromList(
          [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2]);
      final adapter = FakeHttpAdapter((req) {
        return ResponseBody.fromBytes(pngBytes, 200,
            headers: {
              Headers.contentTypeHeader: ['image/png'],
            });
      });
      final service = ThumbnailService(buildTestClient(adapter));

      final result = await service.getThumbnailDataUrl('f1');

      expect(result.isOk, isTrue);
      final url = (result as Ok<String>).value;
      expect(url, startsWith('data:image/png;base64,'));
    });
  });

  group('thumbnailMediaType（对齐 Rust thumbnail_media_type 魔数识别）', () {
    test('Content-Type 为 image/* → 直接采纳', () {
      final r = thumbnailMediaType('image/jpeg', Uint8List(0));
      expect(r.isOk, isTrue);
      expect((r as Ok<String>).value, 'image/jpeg');
    });

    test('Content-Type 带参数 → 截取主类型', () {
      final r = thumbnailMediaType('image/png; charset=utf-8', Uint8List(0));
      expect((r as Ok<String>).value, 'image/png');
    });

    test('PNG 魔数 → image/png', () {
      final r = thumbnailMediaType(null,
          Uint8List.fromList([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]));
      expect((r as Ok<String>).value, 'image/png');
    });

    test('JPEG 魔数 → image/jpeg', () {
      final r = thumbnailMediaType(null, Uint8List.fromList([0xFF, 0xD8, 0xFF]));
      expect((r as Ok<String>).value, 'image/jpeg');
    });

    test('GIF89a 魔数 → image/gif', () {
      final r = thumbnailMediaType(
          null, Uint8List.fromList([0x47, 0x49, 0x46, 0x38, 0x39, 0x61]));
      expect((r as Ok<String>).value, 'image/gif');
    });

    test('WEBP 魔数 → image/webp', () {
      final r = thumbnailMediaType(null,
          Uint8List.fromList([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50]));
      expect((r as Ok<String>).value, 'image/webp');
    });

    test('非图片格式 → Err（拒绝渲染）', () {
      final r = thumbnailMediaType(
          'text/html', Uint8List.fromList('<html>'.codeUnits));
      expect(r.isErr, isTrue);
    });
  });
}
