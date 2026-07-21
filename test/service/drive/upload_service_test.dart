import 'dart:io';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/types/enums.dart';

import '../auth/fake_http.dart';
import 'drive_test_util.dart';

void main() {
  late Directory tempDir;

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('upload_service_test');
  });

  tearDown(() {
    if (tempDir.existsSync()) tempDir.deleteSync(recursive: true);
  });

  const sessionUrl = 'https://driveapis.cloud.huawei.com.cn'
      '/upload/drive/v1/SESSION/files?uploadType=resume&uploadId=u1';

  /// 创建指定大小的测试文件（稀疏）。
  Future<File> createFile(String name, int size) async {
    final file = File('${tempDir.path}/$name');
    final raf = await file.open(mode: FileMode.write);
    await raf.truncate(size);
    await raf.close();
    return file;
  }

  /// 创建带内容的测试文件。
  Future<File> createFileWithContent(String name, List<int> content) async {
    final file = File('${tempDir.path}/$name');
    await file.writeAsBytes(content);
    return file;
  }

  /// 配额充足的 /about 响应。
  ResponseBody aboutResponse() {
    return jsonResponse(const {
      'storageQuota': {
        'userCapacity': '1099511627776',
        'usedSpace': '0',
      },
    });
  }

  /// 上传完成的 File 响应。
  Map<String, dynamic> uploadedFileJson(
      String id, String name, int size,
      {List<String>? parentFolder}) {
    return fileJson(id: id, name: name, size: size, parentFolder: parentFolder);
  }

  group('UploadService.uploadSmall（multipart/related）', () {
    test('multipart 体结构：两段 + boundary + 中文 metadata 不转义', () async {
      await createFileWithContent('你好.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('up1', '你好.txt', 5, parentFolder: ['p1']));
      });
      final service = UploadService(buildTestClient(adapter));

      final file =
          (await service.uploadSmall('${tempDir.path}/你好.txt', parentId: 'p1'))
              .unwrap();

      expect(file.id, 'up1');
      final req =
          adapter.requests.firstWhere((r) => r.method == 'POST');
      expect(req.uri.path, '/upload/drive/v1/files');
      expect(req.uri.query, 'uploadType=multipart');
      final contentType = req.headers['Content-Type'] as String;
      expect(contentType,
          startsWith('multipart/related; boundary=hwcloud_'));
      final boundary =
          contentType.substring('multipart/related; boundary='.length);

      // 体结构：metadata 段（普通 JSON，中文不转义）+ octet-stream 段
      final expected = '--$boundary\r\n'
          'Content-Type: application/json; charset=UTF-8\r\n\r\n'
          '{"fileName":"你好.txt","parentFolder":["p1"]}\r\n'
          '--$boundary\r\n'
          'Content-Type: application/octet-stream\r\n\r\n'
          'hello\r\n'
          '--$boundary--\r\n';
      expect(req.body, expected);
    });

    test('响应 size 与本地不一致 → 远端歧义错误', () async {
      await createFileWithContent('a.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('up1', 'a.txt', 999));
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadSmall('${tempDir.path}/a.txt');

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).requestMayHaveReachedServer, isTrue);
    });

    test('响应父目录与请求不一致 → 远端歧义错误（对齐 CMP 父目录核验）', () async {
      await createFileWithContent('a.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(
            uploadedFileJson('up1', 'a.txt', 5, parentFolder: ['other-parent']));
      });
      final service = UploadService(buildTestClient(adapter));

      final result =
          await service.uploadSmall('${tempDir.path}/a.txt', parentId: 'p1');

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).requestMayHaveReachedServer, isTrue);
    });

    test('请求指定 parentId 但响应缺父目录 → 远端歧义错误', () async {
      await createFileWithContent('a.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('up1', 'a.txt', 5));
      });
      final service = UploadService(buildTestClient(adapter));

      final result =
          await service.uploadSmall('${tempDir.path}/a.txt', parentId: 'p1');

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).requestMayHaveReachedServer, isTrue);
    });

    test('非 2xx → 结构化 HTTP 错误', () async {
      await createFileWithContent('a.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(const {'errorCode': '500'}, status: 500);
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadSmall('${tempDir.path}/a.txt');
      expect(result.isErr, isTrue);
      expect(
          ((result as Err).error as DriveApiError).statusCode, 500);
    });
  });

  group('UploadService.upload（大小路由）', () {
    test('≤ 20MB 走 multipart（不发 resume init）', () async {
      await createFileWithContent('small.bin', List.filled(100, 1));
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        expect(req.uri.query, 'uploadType=multipart');
        return jsonResponse(uploadedFileJson('up1', 'small.bin', 100));
      });
      final service = UploadService(buildTestClient(adapter));

      final file =
          (await service.upload('${tempDir.path}/small.bin')).unwrap();
      expect(file.id, 'up1');
    });

    test('> 20MB 走 resume：init 拿 Location → 单分片 PUT 到会话 URL', () async {
      await createFile('big.bin', 20 * 1024 * 1024 + 1);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.uri.query == 'uploadType=resume' && req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 67108864},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        // 分片 PUT 到会话 URL
        return jsonResponse(
            uploadedFileJson('up-big', 'big.bin', 20 * 1024 * 1024 + 1));
      });
      final resumeEvents = <List<Object>>[];
      final service = UploadService(buildTestClient(adapter));

      final file = (await service.upload(
        '${tempDir.path}/big.bin',
        onResumeProgress: (serverId, uploadId, offset, url) {
          resumeEvents.add([serverId, uploadId, offset, url]);
        },
      ))
          .unwrap();

      expect(file.id, 'up-big');
      // init 请求头
      final init = adapter.requests.firstWhere(
          (r) => r.method == 'POST' && r.uri.query == 'uploadType=resume');
      expect(init.headers['X-Upload-Content-Length'],
          (20 * 1024 * 1024 + 1).toString());
      // 分片 PUT：目标为 Location 会话 URL，Content-Range 全覆盖
      final put = adapter.requests.singleWhere((r) => r.method == 'PUT');
      expect(put.uri.path, '/upload/drive/v1/SESSION/files');
      expect(put.headers['Content-Range'],
          'bytes 0-${20 * 1024 * 1024}/${20 * 1024 * 1024 + 1}');
      // init 后回调持久化会话信息（offset 0 + sessionUrl）
      expect(resumeEvents, isNotEmpty);
      expect(resumeEvents.first[2], 0);
      expect(resumeEvents.first[3], sessionUrl);
    });
  });

  group('UploadService.uploadResume（分片协议）', () {
    test('308 rangeList 推进 → 最终 200 File 结算', () async {
      await createFile('mid.bin', 300000);
      final puts = <FakeRequest>[];
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 262144},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        puts.add(req);
        if (puts.length == 1) {
          return jsonResponse(
              const {'rangeList': ['0-262143']},
              status: 308);
        }
        return jsonResponse(uploadedFileJson('up1', 'mid.bin', 300000));
      });
      final resumeOffsets = <int>[];
      final service = UploadService(buildTestClient(adapter));

      final file = (await service.uploadResume(
        '${tempDir.path}/mid.bin',
        onResumeProgress: (serverId, uploadId, offset, url) {
          resumeOffsets.add(offset);
        },
      ))
          .unwrap();

      expect(file.id, 'up1');
      expect(puts[0].headers['Content-Range'], 'bytes 0-262143/300000');
      expect(puts[1].headers['Content-Range'], 'bytes 262144-299999/300000');
      // init 回调 offset 0，chunk1 确认后回调 262144
      expect(resumeOffsets.first, 0);
      expect(resumeOffsets.last, 262144);
    });

    test('rangeList 出现空洞 → 远端歧义错误（绝不本地推算）', () async {
      await createFile('mid.bin', 300000);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 262144},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        return jsonResponse(
            const {'rangeList': ['0-99', '200-299']},
            status: 308);
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadResume('${tempDir.path}/mid.bin');

      expect(result.isErr, isTrue);
      final error = (result as Err).error as DriveApiError;
      expect(error.requestMayHaveReachedServer, isTrue);
      expect(error.transportKind, DriveTransportKind.decode);
    });

    test('308 未确认当前分片 → 停止本地偏移推进', () async {
      await createFile('mid.bin', 300000);
      var putCount = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 262144},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        putCount++;
        // 第一次确认 100 字节；第二次仍只确认 100 → 未推进
        return jsonResponse(
            const {'rangeList': ['0-99']},
            status: 308);
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadResume('${tempDir.path}/mid.bin');

      expect(result.isErr, isTrue);
      expect(putCount, 2);
    });

    test('全部分片确认后轮询最终状态（308 → 200 File）', () async {
      await createFile('mid.bin', 300000);
      final statusQueries = <FakeRequest>[];
      var chunkCount = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 262144},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        final isStatusQuery =
            req.headers['Content-Range'] == 'bytes */300000';
        if (!isStatusQuery) {
          chunkCount++;
          return jsonResponse(
              chunkCount == 1
                  ? const {'rangeList': ['0-262143']}
                  : const {
                      'rangeList': ['0-262143', '262144-299999'],
                      'processTime': 250,
                    },
              status: 308);
        }
        statusQueries.add(req);
        if (statusQueries.length == 1) {
          // 服务端异步合并：第一次仍 308
          return jsonResponse(
              const {'rangeList': ['0-262143', '262144-299999']},
              status: 308);
        }
        return jsonResponse(uploadedFileJson('up1', 'mid.bin', 300000));
      });
      final service = UploadService(
        buildTestClient(adapter),
        finalPollInterval: const Duration(milliseconds: 250),
      );

      final file =
          (await service.uploadResume('${tempDir.path}/mid.bin')).unwrap();

      expect(file.id, 'up1');
      expect(statusQueries.length, 2);
      expect(statusQueries.every((r) => r.headers['Content-Length'] == '0'),
          isTrue);
    });

    test('恢复持久化会话：先状态查询确认偏移，再续传（不重新 init）', () async {
      await createFile('mid.bin', 300000);
      final sequence = <String>[];
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        sequence.add('${req.method} ${req.headers['Content-Range']}');
        if (req.headers['Content-Range'] == 'bytes */300000') {
          return jsonResponse(
              const {'rangeList': ['0-262143']},
              status: 308);
        }
        return jsonResponse(uploadedFileJson('up1', 'mid.bin', 300000));
      });
      final service = UploadService(buildTestClient(adapter));

      final file = (await service.uploadResume(
        '${tempDir.path}/mid.bin',
        resume: ResumeSession(sessionUrl: sessionUrl, startOffset: 262144),
      ))
          .unwrap();

      expect(file.id, 'up1');
      // 无 init POST；首请求为状态查询
      expect(adapter.requests.any((r) => r.method == 'POST'), isFalse);
      expect(sequence.first, 'PUT bytes */300000');
      expect(sequence.last, 'PUT bytes 262144-299999/300000');
    });

    test('状态查询 404 → 会话已失效错误（upload_session_expired）', () async {
      await createFile('mid.bin', 300000);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(const {'errorCode': '404'}, status: 404);
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadResume(
        '${tempDir.path}/mid.bin',
        resume: ResumeSession(sessionUrl: sessionUrl),
      );

      expect(result.isErr, isTrue);
      expect(((result as Err).error as DriveApiError).errorCode,
          'upload_session_expired');
    });

    test('init 缺 Location 且无 serverId → 远端歧义错误', () async {
      await createFile('mid.bin', 100);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(const {'sliceSize': 262144});
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadResume('${tempDir.path}/mid.bin');

      expect(result.isErr, isTrue);
      expect(((result as Err).error as DriveApiError)
          .requestMayHaveReachedServer, isTrue);
    });

    test('连接失败本地重试：reconcile 未推进后重发分片成功', () async {
      await createFile('mid.bin', 300000);
      var putAttempts = 0;
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        if (req.method == 'POST') {
          return jsonResponseWithHeaders(
            const {'sliceSize': 262144},
            headers: {
              'location': [sessionUrl],
            },
          );
        }
        if (req.headers['Content-Range'] == 'bytes */300000') {
          // reconcile：服务端未确认任何字节
          return jsonResponse(const {'rangeList': []}, status: 308);
        }
        putAttempts++;
        if (putAttempts == 1) {
          throw DioException(
            requestOptions: RequestOptions(path: req.uri.path),
            type: DioExceptionType.connectionError,
            error: const SocketException('no route to host'),
          );
        }
        if (putAttempts == 2) {
          return jsonResponse(
              const {'rangeList': ['0-262143']},
              status: 308);
        }
        return jsonResponse(uploadedFileJson('up1', 'mid.bin', 300000));
      });
      final service = UploadService(
        buildTestClient(adapter),
        chunkRetryDelayUnit: Duration.zero,
      );

      final file =
          (await service.uploadResume('${tempDir.path}/mid.bin')).unwrap();

      expect(file.id, 'up1');
      expect(putAttempts, 3);
    });
  });

  group('UploadService.uploadForTask（执行器级）', () {
    /// 构造与本地文件快照一致的任务行。
    Future<TransferTask> buildTask(
      File file, {
      TransferOperation operation = TransferOperation.create,
      String? sessionUrl,
      int resumeOffset = 0,
    }) async {
      final stat = await file.stat();
      return TransferTask(
        id: 1,
        direction: TransferDirection.upload,
        fileId: 'fid1',
        localPath: file.path,
        name: file.uri.pathSegments.last,
        totalSize: stat.size,
        operation: operation,
        sourceMtime: stat.modified.millisecondsSinceEpoch,
        sourceSize: stat.size,
        sessionUrl: sessionUrl,
        resumeOffset: resumeOffset,
        createdAt: DateTime.now().millisecondsSinceEpoch,
      );
    }

    test('任务行带 sessionUrl → 走续传恢复路径', () async {
      final file = await createFile('task.bin', 300000);
      final task = await buildTask(file, sessionUrl: sessionUrl);
      final sequence = <String>[];
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        sequence.add(req.headers['Content-Range'] ?? req.method);
        if (req.headers['Content-Range'] == 'bytes */300000') {
          return jsonResponse(
              const {'rangeList': ['0-262143']},
              status: 308);
        }
        return jsonResponse(uploadedFileJson('up1', 'task.bin', 300000));
      });
      final progress = <int>[];
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadForTask(
        task,
        onProgress: progress.add,
      );

      expect(result.isOk, isTrue);
      // 状态查询先行，然后续传剩余分片
      expect(sequence.first, 'bytes */300000');
      // 对齐 Rust：resume 路径最后一次进度为服务端确认的断点偏移，
      // 最终 100% 由任务层结算而非进度回调
      expect(progress.last, inInclusiveRange(262143, 262144));
    });

    test('operation 非 Create/Update → 拒绝执行', () async {
      final file = await createFile('task.bin', 100);
      final task = await buildTask(file, operation: TransferOperation.delete);
      final service = UploadService(
          buildTestClient(FakeHttpAdapter((req) => aboutResponse())));

      expect((await service.uploadForTask(task)).isErr, isTrue);
    });

    test('入队源快照不一致 → 拒绝执行（本地源已变更）', () async {
      final file = await createFile('task.bin', 100);
      final task = (await buildTask(file))
          .copyWith(sourceSize: 999, totalSize: 999);
      final service = UploadService(
          buildTestClient(FakeHttpAdapter((req) => aboutResponse())));

      final result = await service.uploadForTask(task);

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
    });

    test('网络门控：离线 → 网络错误拒绝执行', () async {
      final file = await createFile('task.bin', 100);
      final task = await buildTask(file);
      final service = UploadService(
          buildTestClient(FakeHttpAdapter((req) => aboutResponse())));

      final result =
          await service.uploadForTask(task, isOnline: () => false);

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<DriveApiError>());
    });

    test('无会话小文件任务 → multipart 上传并回报字节进度', () async {
      final file = await createFileWithContent('task.txt', 'hello'.codeUnits);
      final task = await buildTask(file);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('up1', 'task.txt', 5));
      });
      final progress = <int>[];
      final service = UploadService(buildTestClient(adapter));

      final result =
          await service.uploadForTask(task, onProgress: progress.add);

      expect(result.isOk, isTrue);
      expect(progress, [5]);
    });
  });

  group('UploadService.uploadUpdate（覆盖已有文件）', () {
    test('返回不同 fileId → 错误（对齐 CMP uploadSmallUpdate）', () async {
      await createFileWithContent('u.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('other-id', 'u.txt', 5));
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadUpdate('fid1', '${tempDir.path}/u.txt');

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).requestMayHaveReachedServer, isTrue);
    });

    test('PATCH multipart 到 /files/{fileId}?uploadType=multipart', () async {
      await createFileWithContent('u.txt', 'hello'.codeUnits);
      final adapter = FakeHttpAdapter((req) {
        if (req.uri.path == '/drive/v1/about') return aboutResponse();
        return jsonResponse(uploadedFileJson('fid1', 'u.txt', 5));
      });
      final service = UploadService(buildTestClient(adapter));

      final file =
          (await service.uploadUpdate('fid1', '${tempDir.path}/u.txt'))
              .unwrap();

      expect(file.id, 'fid1');
      final req = adapter.requests
          .firstWhere((r) => r.method == 'PATCH');
      expect(req.uri.path, '/upload/drive/v1/files/fid1');
      expect(req.uri.query, 'uploadType=multipart');
    });

    test('> 20MiB 覆盖明确拒绝（禁止退化为新建）', () async {
      await createFile('big.bin', 20 * 1024 * 1024 + 1);
      final adapter = FakeHttpAdapter((req) {
        // 不应发出任何写请求
        return aboutResponse();
      });
      final service = UploadService(buildTestClient(adapter));

      final result = await service.uploadUpdate(
          'fid1', '${tempDir.path}/big.bin');

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
      expect(adapter.requests, isEmpty);
    });
  });
}
