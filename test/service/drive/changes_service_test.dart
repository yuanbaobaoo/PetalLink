import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/service/drive/changes_service.dart';

import '../auth/fake_http.dart';
import 'drive_test_util.dart';

void main() {
  /// 构造一条 change JSON。
  Map<String, dynamic> changeJson({
    required String fileId,
    bool deleted = false,
    String? changeType,
    Map<String, dynamic>? file,
    bool includeFile = true,
  }) {
    return {
      'category': 'drive#change',
      'type': 'File',
      'time': '2026-07-06T05:51:13.053Z',
      'fileId': fileId,
      'deleted': deleted,
      'changeType': ?changeType,
      if (includeFile) 'file': ?file,    };
  }

  Map<String, dynamic> modifiedFile(String id,
      {List<String> parentFolder = const ['root']}) {
    return fileJson(id: id, name: 'a.txt', parentFolder: parentFolder);
  }

  group('ChangesService.getStartCursor', () {
    test('解析 startCursor 并校验 category', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#startCursor',
          'startCursor': '311296',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final cursor = (await service.getStartCursor()).unwrap();

      expect(cursor, '311296');
      expect(adapter.requests.single.uri.path,
          '/drive/v1/changes/getStartCursor');
      expect(adapter.requests.single.uri.query, 'fields=*');
    });

    test('category 非预期 → 协议错误', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#other',
          'startCursor': '311296',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.getStartCursor()).isErr, isTrue);
    });
  });

  group('ChangesService.listChanges（单页游标语义）', () {
    test('URL 拼接：pageSize=100&includeDeleted=true&cursor=', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#changeList',
          'changes': [],
          'newStartCursor': '300',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      await service.listChanges('200');

      expect(
          adapter.requests.single.uri.query,
          'fields=*&pageSize=100&includeDeleted=true&cursor=200');
    });

    test('nextCursor 与 newStartCursor 分开解析，禁止合并', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#changeList',
          'changes': [],
          'nextCursor': '201',
          'newStartCursor': '299',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final page = (await service.listChanges('200')).unwrap();

      expect(page.nextCursor, '201');
      expect(page.newStartCursor, '299');
    });

    test('空 cursor → 协议错误（华为强制要求 cursor）', () async {
      final service = ChangesService(
          buildTestClient(FakeHttpAdapter((req) => jsonResponse(const {}))));
      expect((await service.listChanges('')).isErr, isTrue);
      expect((await service.listChanges('  ')).isErr, isTrue);
    });

    test('deleted=true 无 file 的 tombstone → Removed', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', deleted: true, includeFile: false),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final page = (await service.listChanges('300')).unwrap();

      final change = page.changes.single;
      expect(change.kind, ChangeKind.Removed);
      expect(change.fileId, 'f1');
      expect(change.file, isNull);
    });

    test('changeType=trashDone 软删除 → Removed（真机兼容）', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(
                fileId: 'f1',
                changeType: 'trashDone',
                file: modifiedFile('f1')),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final change =
          (await service.listChanges('300')).unwrap().changes.single;
      expect(change.kind, ChangeKind.Removed);
    });

    test('file.recycled=true 软删除 → Removed', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', file: {
              ...modifiedFile('f1'),
              'recycled': true,
            }),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final change =
          (await service.listChanges('300')).unwrap().changes.single;
      expect(change.kind, ChangeKind.Removed);
    });

    test('非删除 change 携带完整 file → Modified', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', file: modifiedFile('f1')),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final change =
          (await service.listChanges('300')).unwrap().changes.single;
      expect(change.kind, ChangeKind.Modified);
      expect(change.file!.id, 'f1');
    });

    test('非删除 change 缺 file → 整页失败', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', includeFile: false),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listChanges('300')).isErr, isTrue);
    });

    test('非删除 change 多 parent → 整页失败', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(
                fileId: 'f1',
                file: modifiedFile('f1', parentFolder: ['p1', 'p2'])),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listChanges('300')).isErr, isTrue);
    });

    test('change.file.id 与 fileId 不一致 → 整页失败', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', file: modifiedFile('f2')),
          ],
          'newStartCursor': '301',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listChanges('300')).isErr, isTrue);
    });
  });

  group('ChangesService.listAllChanges（追平语义）', () {
    test('空的中间页继续翻页，末页提交 newStartCursor', () async {
      var call = 0;
      final adapter = FakeHttpAdapter((req) {
        call++;
        if (call == 1) {
          return jsonResponse(const {
            'category': 'drive#changeList',
            'changes': [],
            'nextCursor': '201',
          });
        }
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', file: modifiedFile('f1')),
          ],
          'newStartCursor': '299',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final result = (await service.listAllChanges('200')).unwrap();

      expect(result.checkpoint, '299');
      expect(result.changes.single.fileId, 'f1');
      expect(call, 2);
      expect(adapter.requests[1].uri.query, contains('cursor=201'));
    });

    test('终页缺 newStartCursor → 失败（不提交 checkpoint）', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#changeList',
          'changes': [],
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listAllChanges('200')).isErr, isTrue);
    });

    test('nextCursor 循环 → 失败', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#changeList',
          'changes': [],
          'nextCursor': '200',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listAllChanges('200')).isErr, isTrue);
    });

    test('有变更但 newStartCursor 未推进 → 失败', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': [
            changeJson(fileId: 'f1', file: modifiedFile('f1')),
          ],
          'newStartCursor': '200',
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      expect((await service.listAllChanges('200')).isErr, isTrue);
    });

    test('达到页数上限仍有 nextCursor → 拒绝部分结果', () async {
      var call = 0;
      final adapter = FakeHttpAdapter((req) {
        call++;
        return jsonResponse({
          'category': 'drive#changeList',
          'changes': const [],
          'nextCursor': '2$call',
        });
      });
      final service =
          ChangesService(buildTestClient(adapter), maxPages: 2);

      expect((await service.listAllChanges('200')).isErr, isTrue);
      expect(call, 2);
    });

    test('协议错误为 GenericError（对齐 Rust AppError::generic）', () async {
      final adapter = FakeHttpAdapter((req) {
        return jsonResponse(const {
          'category': 'drive#changeList',
          'changes': [],
        });
      });
      final service = ChangesService(buildTestClient(adapter));

      final result = await service.listAllChanges('200');
      expect((result as Err).error, isA<GenericError>());
    });
  });
}
