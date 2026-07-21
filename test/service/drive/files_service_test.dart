import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/service/drive/files_service.dart';

import '../auth/fake_http.dart';
import 'drive_test_util.dart';

void main() {
  group('FilesService.list（查询拼接与严格解析）', () {
    test('根目录：queryParam=\'root\' in parentFolder，fields=*&pageSize=100',
        () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson(const []));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.list();

      expect(result.isOk, isTrue);
      final req = adapter.requests.single;
      expect(req.method, 'GET');
      expect(req.uri.host, 'driveapis.cloud.huawei.com.cn');
      expect(req.uri.path, '/drive/v1/files');
      expect(req.uri.query,
          'fields=*&pageSize=100&queryParam=%27root%27%20in%20parentFolder');
      expect(req.headers['Authorization'], 'Bearer test-token');
    });

    test('子目录 + cursor：queryParam 与 cursor 正确编码拼接', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson(const []));
      });
      final service = FilesService(buildTestClient(adapter));

      await service.list(parentId: 'fld/er 1', cursor: 'cur/sor 2');

      final req = adapter.requests.single;
      expect(
          req.uri.query,
          'fields=*&pageSize=100&queryParam=%27fld%2Fer%201%27'
          '%20in%20parentFolder&cursor=cur%2Fsor%202');
    });

    test('解析文件列表与 nextCursor', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson([
          fileJson(
              id: 'f1',
              name: 'a.txt',
              size: 3,
              parentFolder: ['root'],
              editedTime: '2026-07-01T00:00:00.000Z'),
        ], nextCursor: 'next1'));
      });
      final service = FilesService(buildTestClient(adapter));

      final page = (await service.list()).unwrap();

      expect(page.files.single.id, 'f1');
      expect(page.files.single.name, 'a.txt');
      expect(page.nextCursor, 'next1');
      expect(page.hasNext, isTrue);
    });

    test('空 nextCursor 按终页处理', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#fileList',
          'files': const [],
          'nextCursor': '',
        });
      });
      final service = FilesService(buildTestClient(adapter));

      final page = (await service.list()).unwrap();
      expect(page.nextCursor, isNull);
      expect(page.hasNext, isFalse);
    });

    test('条目缺 mimeType → 整页失败（DriveApi decode 协议错误）', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson([
          {'id': 'f1', 'fileName': 'a.txt'},
        ]));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.list();

      expect(result.isErr, isTrue);
      final error = (result as Err).error;
      expect(error, isA<DriveApiError>());
      expect((error as DriveApiError).transportKind,
          DriveTransportKind.decode);
    });

    test('pageSize 越界 → Generic 错误', () async {
      final service = FilesService(
          buildTestClient(FakeHttpAdapter((req) => jsonResponse(const {}))));
      expect((await service.list(pageSize: 0)).isErr, isTrue);
      expect((await service.list(pageSize: 101)).isErr, isTrue);
    });
  });

  group('FilesService.listAll（自动翻页）', () {
    test('连续翻页直到 nextCursor 缺失', () async {
      var call = 0;
      final adapter = FakeHttpAdapter((req) {
        call++;
        if (!req.uri.query.contains('cursor=')) {
          return jsonResponse(fileListPageJson(
              [fileJson(id: 'f1', name: 'a')],
              nextCursor: 'c1'));
        }
        return jsonResponse(
            fileListPageJson([fileJson(id: 'f2', name: 'b')]));
      });
      final service = FilesService(buildTestClient(adapter));

      final files = (await service.listAll()).unwrap();

      expect(files.map((f) => f.id), ['f1', 'f2']);
      expect(call, 2);
      // 第二页请求携带 cursor=c1
      expect(adapter.requests[1].uri.query, contains('cursor=c1'));
    });

    test('空的中间页仍按 nextCursor 继续', () async {
      var call = 0;
      final adapter = FakeHttpAdapter((req) {
        call++;
        if (call == 1) {
          return jsonResponse(fileListPageJson(const [], nextCursor: 'c1'));
        }
        return jsonResponse(
            fileListPageJson([fileJson(id: 'f1', name: 'a')]));
      });
      final service = FilesService(buildTestClient(adapter));

      final files = (await service.listAll()).unwrap();
      expect(files.single.id, 'f1');
      expect(call, 2);
    });

    test('nextCursor 循环 → 协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson(const [], nextCursor: 'c1'));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.listAll();
      expect(result.isErr, isTrue);
    });

    test('达到分页上限仍有 nextCursor → 失败（不返回部分树）', () async {
      var call = 0;
      final adapter = FakeHttpAdapter((req) {
        call++;
        return jsonResponse(
            fileListPageJson(const [], nextCursor: 'c$call'));
      });
      final service =
          FilesService(buildTestClient(adapter), maxPages: 2);

      final result = await service.listAll();
      expect(result.isErr, isTrue);
      expect(call, 2);
    });
  });

  group('FilesService.get / search', () {
    test('get：路径编码 + fields=*，严格解析', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileJson(id: 'fid 1', name: 'a.txt'));
      });
      final service = FilesService(buildTestClient(adapter));

      final file = (await service.get('fid 1')).unwrap();

      expect(file.id, 'fid 1');
      final req = adapter.requests.single;
      expect(req.uri.path, '/drive/v1/files/fid%201');
      expect(req.uri.query, 'fields=*');
    });

    test('search：contains DSL 整段只编码一次', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson(const []));
      });
      final service = FilesService(buildTestClient(adapter));

      await service.search('报告', parentId: 'p1');

      expect(
          adapter.requests.single.uri.query,
          'fields=*&pageSize=100&queryParam=fileName%20contains'
          '%20%27%E6%8A%A5%E5%91%8A%27%20and%20%27p1%27%20in%20parentFolder');
    });

    test('search：单引号/反斜线 fail closed', () async {
      final service = FilesService(
          buildTestClient(FakeHttpAdapter((req) => jsonResponse(const {}))));
      expect((await service.search("it's")).isErr, isTrue);
      expect((await service.search('a\\b')).isErr, isTrue);
    });
  });

  group('FilesService.createFolder', () {
    test('body ASCII 转义 + root 省略 parentFolder + 响应核验', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'GET') {
          // 查重：父目录为空
          return jsonResponse(fileListPageJson(const []));
        }
        return jsonResponse(folderJson(
            id: 'new1', name: '新建文件夹', parentFolder: ['root']));
      });
      final service = FilesService(buildTestClient(adapter));

      final folder = (await service.createFolder('新建文件夹')).unwrap();

      expect(folder.id, 'new1');
      final post = adapter.requests.firstWhere((r) => r.method == 'POST');
      expect(post.uri.path, '/drive/v1/files');
      expect(post.uri.query, 'fields=*');
      expect(post.body,
          '{"fileName":"\\u65b0\\u5efa\\u6587\\u4ef6\\u5939",'
          '"mimeType":"application/vnd.huawei-apps.folder"}');
      expect(post.headers['Content-Type'], contains('application/json'));
    });

    test('非 root parentFolder 写入 body', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'GET') {
          return jsonResponse(fileListPageJson(const []));
        }
        return jsonResponse(
            folderJson(id: 'new1', name: 'sub', parentFolder: ['p1']));
      });
      final service = FilesService(buildTestClient(adapter));

      await service.createFolder('sub', parentId: 'p1');

      final post = adapter.requests.firstWhere((r) => r.method == 'POST');
      expect(post.body, contains('"parentFolder":["p1"]'));
    });

    test('创建前查重命中唯一同名目录 → 跳过 POST', () async {
      final adapter = FakeHttpAdapter((req) {
        expect(req.method, 'GET');
        return jsonResponse(fileListPageJson([
          folderJson(id: 'exist', name: 'dup', parentFolder: ['root']),
        ]));
      });
      final service = FilesService(buildTestClient(adapter));

      final folder = (await service.createFolder('dup')).unwrap();

      expect(folder.id, 'exist');
      expect(adapter.requests.every((r) => r.method == 'GET'), isTrue);
    });

    test('POST 失败后父目录唯一核验确认已提交 → 返回已存在目录', () async {
      var postSeen = false;
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'POST') {
          postSeen = true;
          return jsonResponse(const {'errorCode': '500'}, status: 500);
        }
        if (postSeen) {
          return jsonResponse(fileListPageJson([
            folderJson(id: 'exist', name: 'dup', parentFolder: ['root']),
          ]));
        }
        return jsonResponse(fileListPageJson(const []));
      });
      final service = FilesService(buildTestClient(adapter));

      final folder = (await service.createFolder('dup')).unwrap();
      expect(folder.id, 'exist');
    });

    test('POST 失败且核验零匹配 → 返回原始错误', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'POST') {
          return jsonResponse(const {'errorCode': '500'}, status: 500);
        }
        return jsonResponse(fileListPageJson(const []));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.createFolder('dup');

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<DriveApiError>());
      expect(((result as Err).error as DriveApiError).statusCode, 500);
    });

    test('父目录存在多个同名文件夹 → 歧义错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileListPageJson([
          folderJson(id: 'd1', name: 'dup', parentFolder: ['root']),
          folderJson(id: 'd2', name: 'dup', parentFolder: ['root']),
        ]));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.createFolder('dup');

      expect(result.isErr, isTrue);
      expect((result as Err).error, isA<GenericError>());
    });

    test('写响应非 200（201）→ 写协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'GET') {
          return jsonResponse(fileListPageJson(const []));
        }
        return jsonResponse(
            folderJson(id: 'new1', name: 'sub', parentFolder: ['root']),
            status: 201);
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.createFolder('sub');

      expect(result.isErr, isTrue);
    });

    test('响应 fileName 与请求不一致 → 拒绝', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'GET') {
          return jsonResponse(fileListPageJson(const []));
        }
        return jsonResponse(
            folderJson(id: 'new1', name: 'other', parentFolder: ['root']));
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.createFolder('sub')).isErr, isTrue);
    });
  });

  group('FilesService.update / rename / moveFile', () {
    test('rename：PATCH body ASCII 转义并核验最终名称', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileJson(id: 'f1', name: '新名字'));
      });
      final service = FilesService(buildTestClient(adapter));

      final file = (await service.rename('f1', '新名字')).unwrap();

      expect(file.name, '新名字');
      final req = adapter.requests.single;
      expect(req.method, 'PATCH');
      expect(req.uri.path, '/drive/v1/files/f1');
      expect(req.uri.query, 'fields=*');
      expect(req.body, '{"fileName":"\\u65b0\\u540d\\u5b57"}');
    });

    test('rename：响应名称不一致 → 写协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileJson(id: 'f1', name: '旧名字'));
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.rename('f1', '新名字')).isErr, isTrue);
    });

    test('update 移动：preflight GET + 成对 parent 参数 + 目标父核验', () async {
      final adapter = FakeHttpAdapter((req) {
        if (req.method == 'GET') {
          return jsonResponse(
              fileJson(id: 'f1', name: 'a', parentFolder: ['oldP']));
        }
        return jsonResponse(
            fileJson(id: 'f1', name: 'a', parentFolder: ['newP']));
      });
      final service = FilesService(buildTestClient(adapter));

      final file =
          (await service.update('f1', newParentFolder: 'newP')).unwrap();

      expect(file.parentId, 'newP');
      final patch = adapter.requests.firstWhere((r) => r.method == 'PATCH');
      expect(patch.uri.query,
          'fields=*&addParentFolder=newP&removeParentFolder=oldP');
      expect(patch.body, '{}');
    });

    test('update 移动：已在目标位置时不发送 PATCH（幂等）', () async {
      final adapter = FakeHttpAdapter((req) {
        expect(req.method, 'GET');
        return jsonResponse(
            fileJson(id: 'f1', name: 'a', parentFolder: ['sameP']));
      });
      final service = FilesService(buildTestClient(adapter));

      final file =
          (await service.update('f1', newParentFolder: 'sameP')).unwrap();

      expect(file.parentId, 'sameP');
      expect(adapter.requests.every((r) => r.method == 'GET'), isTrue);
    });

    test('moveFile：新旧 parent 相同 → GET 核验后直接返回', () async {
      final adapter = FakeHttpAdapter((req) {
        expect(req.method, 'GET');
        return jsonResponse(
            fileJson(id: 'f1', name: 'a', parentFolder: ['p1']));
      });
      final service = FilesService(buildTestClient(adapter));

      final file = (await service.moveFile('f1', 'p1', 'p1')).unwrap();
      expect(file.parentId, 'p1');
    });

    test('moveFile：响应 parent 未生效 → 写协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(
            fileJson(id: 'f1', name: 'a', parentFolder: ['oldP']));
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.moveFile('f1', 'oldP', 'newP');
      expect(result.isErr, isTrue);
    });

    test('响应 File.id 与请求不一致 → 写协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileJson(id: 'other', name: 'x'));
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.rename('f1', 'x')).isErr, isTrue);
    });
  });

  group('FilesService.delete / verifyDeleted', () {
    test('软删除：PATCH recycled:true + 200 + recycled=true 核验', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          ...fileJson(id: 'f1', name: 'a'),
          'recycled': true,
        });
      });
      final service = FilesService(buildTestClient(adapter));

      final result = await service.delete('f1');

      expect(result.isOk, isTrue);
      final req = adapter.requests.single;
      expect(req.method, 'PATCH');
      expect(req.uri.path, '/drive/v1/files/f1');
      expect(req.body, '{"recycled":true}');
    });

    test('响应未明确确认 recycled=true → 写协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(fileJson(id: 'f1', name: 'a'));
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.delete('f1')).isErr, isTrue);
    });

    test('verifyDeleted：GET 404 → true', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {'errorCode': '404'}, status: 404);
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.verifyDeleted('f1')).unwrap(), isTrue);
    });

    test('verifyDeleted：GET 200 且 recycled=true → true', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          ...fileJson(id: 'f1', name: 'a'),
          'recycled': true,
        });
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.verifyDeleted('f1')).unwrap(), isTrue);
    });

    test('verifyDeleted：recycled=false → false', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          ...fileJson(id: 'f1', name: 'a'),
          'recycled': false,
        });
      });
      final service = FilesService(buildTestClient(adapter));

      expect((await service.verifyDeleted('f1')).unwrap(), isFalse);
    });
  });
}
