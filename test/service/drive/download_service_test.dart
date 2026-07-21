import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/types/enums.dart';

import 'package:petal_link/service/mount/manager.dart';

import '../auth/fake_http.dart';
import '../mount/proc_xattr.dart';
import 'drive_test_util.dart';

void main() {
  late Directory tempDir;
  late String destPath;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('download_service_test');
    destPath = '${tempDir.path}/file.bin';
  });

  tearDown(() {
    if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
  });

  const fileId = 'fid1';
  const editedTime = '2026-07-01T00:00:00.000Z';
  final editedTimeMs =
      DateTime.parse(editedTime).toUtc().millisecondsSinceEpoch;

  /// 云端元数据响应（id/size/editedTime + 可选 etag 头/sha256）。
  ResponseBody metadataResponse(
    int size, {
    String? etag,
    String? sha256Hash,
    String edited = editedTime,
  }) {
    return jsonResponseWithHeaders(
      {
        'id': fileId,
        'fileName': 'file.bin',
        'mimeType': 'application/octet-stream',
        'size': size,
        'editedTime': edited,
        'sha256': ?sha256Hash,
      },
      headers: {
        if (etag != null) 'etag': [etag],
      },
    );
  }

  /// 二进制内容响应。
  ResponseBody contentResponse(
    List<int> bytes, {
    int status = 200,
    String? contentRange,
  }) {
    return ResponseBody.fromBytes(
      Uint8List.fromList(bytes),
      status,
      headers: {
        Headers.contentTypeHeader: ['application/octet-stream'],
        if (contentRange != null) 'content-range': [contentRange],
      },
    );
  }

  /// 写入与云端版本匹配的断点 sidecar。
  Future<void> writeSidecar(
    String dest,
    int size, {
    int? editedMs,
  }) async {
    final sidecar = {
      'file_id': fileId,
      'size': size,
      'revision': null,
      'edited_time_ms': editedMs ?? editedTimeMs,
      'etag': null,
      'sha256': null,
      'content_hash': null,
    };
    await File(resumeMetadataPath(dest)).writeAsString(jsonEncode(sidecar));
  }

  /// 统计各类请求。
  List<FakeRequest> requestsTo(FakeHttpAdapter adapter, String queryPart) {
    return adapter.requests
        .where((r) => r.uri.query.contains(queryPart))
        .toList();
  }

  group('DownloadService.download（全新下载）', () {
    test('元数据 → 内容 → 原子安装；tmp 与 sidecar 清除', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
      expect(File(tmpPath(destPath)).existsSync(), isFalse);
      expect(File(resumeMetadataPath(destPath)).existsSync(), isFalse);
      // 内容请求无 Range 头；元数据 GET 两次（下载前 + 安装前复核）
      final contents = requestsTo(adapter, 'form=content');
      expect(contents.single.headers['Range'], isNull);
      expect(requestsTo(adapter, 'fields=*').length, 2);
    });

    test('etag 头存在时内容请求携带 If-Match', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') {
          return metadataResponse(5, etag: 'W/"abc123"');
        }
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      await service.download(fileId, destPath);

      final content = requestsTo(adapter, 'form=content').single;
      expect(content.headers['If-Match'], 'W/"abc123"');
    });

    test('空文件直接落盘（无内容请求）', () async {
      final adapter = FakeHttpAdapter((req) {
        return metadataResponse(0);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(await File(destPath).length(), 0);
      expect(requestsTo(adapter, 'form=content'), isEmpty);
    });
  });

  group('DownloadService Range 断点续传', () {
    test('sidecar 版本匹配 → Range 续传追加完成', () async {
      // 已下载前 3 字节
      await File(tmpPath(destPath)).writeAsString('hel');
      await writeSidecar(destPath, 5);

      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        final range = req.headers['Range'];
        if (range == 'bytes=3-') {
          return contentResponse('lo'.codeUnits,
              status: 206, contentRange: 'bytes 3-4/5');
        }
        fail('应发送 Range: bytes=3-，实际: $range');
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('sidecar 版本漂移 → 丢弃断点重新全量下载', () async {
      await File(tmpPath(destPath)).writeAsString('XXX');
      await writeSidecar(destPath, 5, editedMs: 1); // 过期版本

      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        expect(req.headers['Range'], isNull);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('tmp 长度超过云端 size → 丢弃断点重新下载', () async {
      await File(tmpPath(destPath)).writeAsString('toolong!!');
      await writeSidecar(destPath, 5);

      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        expect(req.headers['Range'], isNull);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      expect((await service.download(fileId, destPath)).isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('服务端忽略 Range 返回 200 → 截断从 0 写', () async {
      await File(tmpPath(destPath)).writeAsString('hel');
      await writeSidecar(destPath, 5);

      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        // 无视 Range，直接返回全量 200
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('206 Content-Range 不匹配 → 从 0 安全重启一次', () async {
      await File(tmpPath(destPath)).writeAsString('hel');
      await writeSidecar(destPath, 5);

      var contentCalls = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        contentCalls++;
        if (contentCalls == 1) {
          // Range 请求得到不匹配的 206
          expect(req.headers['Range'], 'bytes=3-');
          return contentResponse('xx'.codeUnits,
              status: 206, contentRange: 'bytes 0-1/5');
        }
        // 重启后全量
        expect(req.headers['Range'], isNull);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(contentCalls, 2);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('416 → 丢弃断点从 0 重启一次', () async {
      await File(tmpPath(destPath)).writeAsString('hel');
      await writeSidecar(destPath, 5);

      var contentCalls = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        contentCalls++;
        if (contentCalls == 1) {
          return contentResponse(const [], status: 416);
        }
        expect(req.headers['Range'], isNull);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isOk, isTrue);
      expect(contentCalls, 2);
    });
  });

  group('DownloadService 完成核验', () {
    test('sha256 校验失败 → 错误并丢弃断点', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') {
          return metadataResponse(5, sha256Hash: '0' * 64);
        }
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
      expect(File(tmpPath(destPath)).existsSync(), isFalse);
    });

    test('sha256 校验通过 → 正常安装', () async {
      final helloSha256 = sha256.convert('hello'.codeUnits).toString();
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') {
          return metadataResponse(5, sha256Hash: helloSha256);
        }
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      expect((await service.download(fileId, destPath)).isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });

    test('下载期间云端版本变化 → 错误并丢弃断点', () async {
      var metadataCalls = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') {
          metadataCalls++;
          // 安装前复核返回不同 editedTime
          return metadataResponse(5,
              edited: metadataCalls == 1
                  ? editedTime
                  : '2026-07-02T00:00:00.000Z');
        }
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
      expect(File(tmpPath(destPath)).existsSync(), isFalse);
      expect(File(destPath).existsSync(), isFalse);
    });

    test('长度不足（响应提前结束）→ 保留部分文件报网络错误', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hel'.codeUnits); // 只给 3 字节
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.download(fileId, destPath);

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<DriveApiError>());
      // 暂态失败保留断点供下次续传
      expect(File(tmpPath(destPath)).existsSync(), isTrue);
    });

    test('期望约束不匹配 → 任务过期错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return metadataResponse(5);
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.downloadWithExpectation(
        fileId,
        destPath,
        expectation: const DownloadExpectation(size: 999),
      );

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
    });
  });

  group('DownloadService.downloadForTask（执行器级）', () {
    TransferTask buildTask({
      TransferOperation operation = TransferOperation.download,
      int totalSize = 5,
    }) {
      return TransferTask(
        id: 1,
        direction: TransferDirection.download,
        fileId: fileId,
        localPath: destPath,
        name: 'file.bin',
        totalSize: totalSize,
        operation: operation,
        expectedCloudEditedTime: editedTimeMs,
        createdAt: DateTime.now().millisecondsSinceEpoch,
      );
    }

    test('Download 任务按期望约束下载完成', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hello'.codeUnits);
      });
      final service = DownloadService(buildTestClient(adapter));
      final progress = <int>[];

      final result = await service.downloadForTask(
        buildTask(),
        onProgress: progress.add,
      );

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
      expect(progress.last, 5);
    });

    test('期望 editedTime 不匹配 → 任务过期', () async {
      final adapter = FakeHttpAdapter((req) {
        return metadataResponse(5,
            edited: '2026-07-02T00:00:00.000Z');
      });
      final service = DownloadService(buildTestClient(adapter));

      final result = await service.downloadForTask(buildTask());
      expect(result.isErr, isTrue);
    });

    test('operation 非下载类 → 拒绝执行', () async {
      final service = DownloadService(
          buildTestClient(FakeHttpAdapter((req) => metadataResponse(5))));
      final result = await service.downloadForTask(
          buildTask(operation: TransferOperation.create));
      expect(result.isErr, isTrue);
    });

    test('网络门控：离线 → 网络错误拒绝执行', () async {
      final service = DownloadService(
          buildTestClient(FakeHttpAdapter((req) => metadataResponse(5))));
      final result = await service
          .downloadForTask(buildTask(), isOnline: () => false);
      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<DriveApiError>());
    });
  });

  group('DownloadService 占位属主核验（对齐 Rust verify_local_destination）', () {
    DownloadService serviceWithXattr(FakeHttpAdapter adapter, ProcXattrService x) {
      return DownloadService(buildTestClient(adapter), xattr: x);
    }

    test('占位目标缺 state xattr（用户 0 字节文件）→ 拒绝覆盖', () async {
      // 目标：0 字节普通文件但无占位标记（用户文件，如 .gitkeep）
      await File(destPath).writeAsBytes(const []);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hello'.codeUnits);
      });
      final x = ProcXattrService();
      final service = serviceWithXattr(adapter, x);

      final result = await service.downloadWithExpectation(
        fileId,
        destPath,
        expectation: const DownloadExpectation(placeholderFileId: fileId),
      );

      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('用户内容'));
      // 用户文件未被覆盖
      expect(await File(destPath).readAsBytes(), isEmpty);
    });

    test('占位属主 fileId 不匹配 → 拒绝覆盖', () async {
      await File(destPath).writeAsBytes(const []);
      final x = ProcXattrService();
      await x.set(destPath, xattrState, statePlaceholder);
      await x.set(destPath, xattrFileId, 'other-fid');
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hello'.codeUnits);
      });
      final service = serviceWithXattr(adapter, x);

      final result = await service.downloadWithExpectation(
        fileId,
        destPath,
        expectation: const DownloadExpectation(placeholderFileId: fileId),
      );

      expect(result.isErr, isTrue);
      expect((result as Err).error.message, contains('用户内容'));
    });

    test('合法占位（state + owner 匹配）→ 正常安装', () async {
      await File(destPath).writeAsBytes(const []);
      final x = ProcXattrService();
      await x.set(destPath, xattrState, statePlaceholder);
      await x.set(destPath, xattrFileId, fileId);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.query == 'fields=*') return metadataResponse(5);
        return contentResponse('hello'.codeUnits);
      });
      final service = serviceWithXattr(adapter, x);

      final result = await service.downloadWithExpectation(
        fileId,
        destPath,
        expectation: const DownloadExpectation(placeholderFileId: fileId),
      );

      expect(result.isOk, isTrue);
      expect(await File(destPath).readAsString(), 'hello');
    });
  });
}
