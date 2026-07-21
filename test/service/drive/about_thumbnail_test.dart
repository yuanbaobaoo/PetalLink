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
  });
}
