import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/sync/cloud_tree.dart';

import '../auth/fake_http.dart';
import '../drive/drive_test_util.dart';

/// cloud_tree 测试：checkpoint 校验/原子持久化/加载、BFS（fake HTTP）、
/// Changes 增量回放、缓存路径转义（对齐 Rust cloud_tree.rs / cache_paths.rs）。

DriveFile file(String id, String name,
        {String? parentId, int editedMs = 1700000000000}) =>
    DriveFile(
      id: id,
      name: name,
      parentFolder: parentId != null ? [parentId] : null,
      editedTime:
          DateTime.fromMillisecondsSinceEpoch(editedMs, isUtc: true),
    );

DriveFile folder(String id, String name, {String? parentId}) =>
    DriveFile(
      id: id,
      name: name,
      category: FileCategory.Folder,
      parentFolder: parentId != null ? [parentId] : null,
    );

void main() {
  late Directory supportRoot;

  setUp(() {
    supportRoot = Directory.systemTemp.createTempSync('cloud_tree_test');
    AppPaths.debugSupportRoot = supportRoot.path;
  });

  tearDown(() {
    AppPaths.debugSupportRoot = null;
    if (supportRoot.existsSync()) supportRoot.deleteSync(recursive: true);
  });

  group('缓存路径转义（对齐 cache_paths.rs）', () {
    test('保留 [A-Za-z0-9._-]，其余替换为 _', () {
      expect(CachePaths.escapeMountPath('/Users/me/hwcloud-drive'),
          '_Users_me_hwcloud-drive');
      expect(CachePaths.escapeMountPath('/数据/同 步'), '_______');
      expect(CachePaths.escapeMountPath('a.b_c-d'), 'a.b_c-d');
    });

    test('缓存文件名形态', () async {
      final f = await CachePaths.cloudTreeCacheFile('/mnt/x');
      expect(p.basename(f.path), 'cloudtree__mnt_x.json');
      final s = await CachePaths.syncStateCacheFile('/mnt/x');
      expect(p.basename(s.path), 'syncstate__mnt_x.json');
    });
  });

  group('detectRootFolderId', () {
    test('最高频 parentFolder 胜出', () {
      final id = detectRootFolderId([
        file('a', 'a', parentId: 'root1'),
        file('b', 'b', parentId: 'root1'),
        file('c', 'c', parentId: 'root2'),
      ]);
      expect(id, 'root1');
    });

    test('最高频并列 → null（fail closed）', () {
      final id = detectRootFolderId([
        file('a', 'a', parentId: 'root1'),
        file('b', 'b', parentId: 'root2'),
      ]);
      expect(id, isNull);
    });

    test('空列表 → null', () {
      expect(detectRootFolderId(const []), isNull);
    });
  });

  group('CloudTreeCache.validateTrusted', () {
    test('合法 checkpoint 通过', () {
      final cache = CloudTreeCache.newTrusted(
        'root',
        {'a.txt': file('f1', 'a.txt', parentId: 'root')},
        {'a.txt': 'f1'},
        'cursor-1',
      );
      expect(cache.complete, isTrue);
      expect(cache.pathToId[''], 'root'); // rootId → "" 反查
    });

    test('未完整提交 → 拒绝', () {
      final cache = CloudTreeCache(
        tree: const {},
        pathToId: const {},
        cursor: 'c',
      );
      expect(() => cache.validateTrusted(), throwsA(isA<AppError>()));
    });

    test('缺 cursor → 拒绝', () {
      final cache = CloudTreeCache(
        tree: const {},
        pathToId: const {},
        complete: true,
      );
      expect(() => cache.validateTrusted(), throwsA(isA<AppError>()));
    });

    test('fileId 重复 → 拒绝', () {
      final cache = CloudTreeCache(
        tree: {
          'a': file('f1', 'a'),
          'b': file('f1', 'b'),
        },
        pathToId: {'a': 'f1', 'b': 'f1'},
        cursor: 'c',
        complete: true,
      );
      expect(() => cache.validateTrusted(), throwsA(isA<AppError>()));
    });

    test('孤立路径索引 → 拒绝', () {
      final cache = CloudTreeCache(
        tree: {'a': file('f1', 'a')},
        pathToId: {'a': 'f1', 'ghost': 'f2'},
        cursor: 'c',
        complete: true,
      );
      expect(() => cache.validateTrusted(), throwsA(isA<AppError>()));
    });

    test('根目录索引不一致 → 拒绝', () {
      final cache = CloudTreeCache(
        rootFolderId: 'root',
        tree: const {},
        pathToId: const {'': 'other'},
        cursor: 'c',
        complete: true,
      );
      expect(() => cache.validateTrusted(), throwsA(isA<AppError>()));
    });
  });

  group('checkpoint 原子持久化与加载', () {
    const mount = '/mnt/sync';

    test('persist → load 完整往返', () async {
      final cache = CloudTreeCache.newTrusted(
        'root',
        {
          'a.txt': file('f1', 'a.txt', parentId: 'root'),
          'dir': folder('f2', 'dir', parentId: 'root'),
        },
        {'a.txt': 'f1', 'dir': 'f2'},
        'cursor-9',
      );
      await persistCloudCheckpoint(mount, cache);
      final loaded = await loadPersistedCloudTree(mount);
      expect(loaded, isNotNull);
      expect(loaded!.cursor, 'cursor-9');
      expect(loaded.tree.keys, containsAll(['a.txt', 'dir']));
      expect(loaded.pathToId[''], 'root');
      // 正式提交后无 .tmp/.bak 残留
      final cacheFile = await CachePaths.cloudTreeCacheFile(mount);
      expect(File('${cacheFile.path}.tmp').existsSync(), isFalse);
      expect(File('${cacheFile.path}.bak').existsSync(), isFalse);
    });

    test('覆盖写保留崩溃一致性（旧版本在 rename 前备份）', () async {
      final v1 = CloudTreeCache.newTrusted(
          'root', {'a': file('f1', 'a')}, {'a': 'f1'}, 'c1');
      await persistCloudCheckpoint(mount, v1);
      final v2 = CloudTreeCache.newTrusted(
          'root', {'b': file('f2', 'b')}, {'b': 'f2'}, 'c2');
      await persistCloudCheckpoint(mount, v2);
      final loaded = await loadPersistedCloudTree(mount);
      expect(loaded!.tree.keys, ['b']);
      expect(loaded.cursor, 'c2');
    });

    test('损坏 JSON → null（触发全量）', () async {
      final cacheFile = await CachePaths.cloudTreeCacheFile(mount);
      await Directory(cacheFile.parent.path).create(recursive: true);
      await cacheFile.writeAsString('{not json');
      expect(await loadPersistedCloudTree(mount), isNull);
    });

    test('不可信内容（complete=false）→ null', () async {
      final cacheFile = await CachePaths.cloudTreeCacheFile(mount);
      await Directory(cacheFile.parent.path).create(recursive: true);
      await cacheFile.writeAsString(jsonEncode({
        'root_folder_id': null,
        'tree': <String, dynamic>{},
        'path_to_id': <String, String>{},
        'cursor': 'c',
        'complete': false,
      }));
      expect(await loadPersistedCloudTree(mount), isNull);
    });

    test('残留 .tmp 不影响正式 checkpoint 加载', () async {
      final cache =
          CloudTreeCache.newTrusted('root', const {}, const {}, 'c1');
      await persistCloudCheckpoint(mount, cache);
      final cacheFile = await CachePaths.cloudTreeCacheFile(mount);
      await File('${cacheFile.path}.tmp').writeAsString('partial');
      expect(await loadPersistedCloudTree(mount), isNotNull);
      await markCloudCacheIncompleteIfExists(mount);
      expect(File('${cacheFile.path}.tmp').existsSync(), isFalse);
    });

    test('clearForMount 清理全部缓存文件', () async {
      final cache =
          CloudTreeCache.newTrusted('root', const {}, const {}, 'c1');
      await persistCloudCheckpoint(mount, cache);
      await SyncStateStore(mount).save(const {});
      await CachePaths.clearForMount(mount);
      expect(
          (await CachePaths.cloudTreeCacheFile(mount)).existsSync(), isFalse);
      expect(
          (await CachePaths.syncStateCacheFile(mount)).existsSync(), isFalse);
    });
  });

  group('Changes 增量回放（applyChangesToCandidate）', () {
    test('Removed 删除整棵子树', () {
      final tree = {
        'dir': folder('d1', 'dir', parentId: 'root'),
        'dir/a.txt': file('f1', 'a.txt', parentId: 'd1'),
        'b.txt': file('f2', 'b.txt', parentId: 'root'),
      };
      final index = {'dir': 'd1', 'dir/a.txt': 'f1', 'b.txt': 'f2', '': 'root'};
      applyChangesToCandidate(
        [const DriveChange(kind: ChangeKind.Removed, fileId: 'd1')],
        tree,
        index,
        'root',
      );
      expect(tree.keys, ['b.txt']);
      expect(index.keys, containsAll(['b.txt', '']));
    });

    test('Modified 新增插入', () {
      final tree = <String, DriveFile>{};
      final index = <String, String>{'': 'root'};
      applyChangesToCandidate(
        [
          DriveChange(
            kind: ChangeKind.Modified,
            fileId: 'f1',
            file: file('f1', 'a.txt', parentId: 'root'),
          ),
        ],
        tree,
        index,
        'root',
      );
      expect(tree['a.txt']?.id, 'f1');
      expect(index['a.txt'], 'f1');
    });

    test('Modified 改名重键子树', () {
      final tree = {
        'old': folder('d1', 'old', parentId: 'root'),
        'old/a.txt': file('f1', 'a.txt', parentId: 'd1'),
      };
      final index = {'old': 'd1', 'old/a.txt': 'f1', '': 'root'};
      applyChangesToCandidate(
        [
          DriveChange(
            kind: ChangeKind.Modified,
            fileId: 'd1',
            file: folder('d1', 'new', parentId: 'root'),
          ),
        ],
        tree,
        index,
        'root',
      );
      expect(tree.keys, containsAll(['new', 'new/a.txt']));
      expect(index['new'], 'd1');
    });

    test('目标路径冲突 → fail-closed 抛错', () {
      final tree = {'a.txt': file('f1', 'a.txt', parentId: 'root')};
      final index = {'a.txt': 'f1', '': 'root'};
      expect(
        () => applyChangesToCandidate(
          [
            DriveChange(
              kind: ChangeKind.Modified,
              fileId: 'f2',
              file: file('f2', 'a.txt', parentId: 'root'),
            ),
          ],
          tree,
          index,
          'root',
        ),
        throwsA(isA<AppError>()),
      );
    });

    test('父目录未知 → fail-closed 抛错', () {
      final tree = <String, DriveFile>{};
      final index = <String, String>{'': 'root'};
      expect(
        () => applyChangesToCandidate(
          [
            DriveChange(
              kind: ChangeKind.Modified,
              fileId: 'f1',
              file: file('f1', 'a.txt', parentId: 'ghost'),
            ),
          ],
          tree,
          index,
          'root',
        ),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('BFS（fake HTTP）', () {
    test('嵌套树构建 + root 动态发现', () async {
      final adapter = FakeHttpAdapter((request) {
        final qp = request.uri.queryParameters['queryParam'] ?? '';
        if (qp.contains("'root'")) {
          return jsonResponse(fileListPageJson([
            folderJson(id: 'd1', name: 'docs', parentFolder: ['root']),
            fileJson(id: 'f1', name: 'a.txt', parentFolder: ['root']),
          ]));
        }
        if (qp.contains("'d1'")) {
          return jsonResponse(fileListPageJson([
            fileJson(id: 'f2', name: 'b.txt', parentFolder: ['d1']),
          ]));
        }
        throw StateError('未知请求: $qp');
      });
      final files = FilesService(buildTestClient(adapter));
      final result = await refreshCloudTree(files);
      expect(result.rootFolderId, 'root');
      expect(result.tree.keys,
          containsAll(['docs', 'a.txt', 'docs/b.txt']));
      expect(result.pathToId['docs'], 'd1');
    });

    test('单目录失败重试 2 次后成功', () async {
      var attempts = 0;
      final adapter = FakeHttpAdapter((request) {
        final qp = request.uri.queryParameters['queryParam'] ?? '';
        if (qp.contains("'root'")) {
          return jsonResponse(fileListPageJson([
            folderJson(id: 'd1', name: 'docs', parentFolder: ['root']),
          ]));
        }
        attempts++;
        if (attempts < 3) {
          return jsonResponse({'error': 'boom'}, status: 500);
        }
        return jsonResponse(fileListPageJson(const []));
      });
      final files = FilesService(buildTestClient(adapter));
      final result = await refreshCloudTree(files);
      expect(attempts, 3);
      expect(result.tree.keys, ['docs']);
    });

    test('重试耗尽 → 失败（子树缺失不可接受）', () async {
      final adapter = FakeHttpAdapter((request) {
        final qp = request.uri.queryParameters['queryParam'] ?? '';
        if (qp.contains("'root'")) {
          return jsonResponse(fileListPageJson([
            folderJson(id: 'd1', name: 'docs', parentFolder: ['root']),
          ]));
        }
        return jsonResponse({'error': 'boom'}, status: 500);
      });
      final files = FilesService(buildTestClient(adapter));
      expect(() => refreshCloudTree(files), throwsA(isA<AppError>()));
    });

    test('.hwcloud_ 前缀内部文件被跳过', () async {
      final adapter = FakeHttpAdapter((request) {
        return jsonResponse(fileListPageJson([
          fileJson(id: 'f1', name: 'a.txt', parentFolder: ['root']),
          fileJson(id: 'f2', name: '.hwcloud_cache', parentFolder: ['root']),
        ]));
      });
      final files = FilesService(buildTestClient(adapter));
      final result = await refreshCloudTree(files);
      expect(result.tree.keys, ['a.txt']);
    });
  });

  group('SyncStateStore（syncstate 快照）', () {
    test('save/load/clear 往返', () async {
      const mount = '/mnt/state';
      final store = SyncStateStore(mount);
      expect(await store.exists(), isFalse);
      await store.save({
        'a.txt': const LocalSnapshotEntry(mtime: 1, size: 2, sha256: 'ab'),
      });
      expect(await store.exists(), isTrue);
      final loaded = await store.load();
      expect(loaded['a.txt']?.size, 2);
      expect(loaded['a.txt']?.sha256, 'ab');
      await store.clear();
      expect(await store.exists(), isFalse);
    });

    test('损坏内容 → 空 map（不报错）', () async {
      const mount = '/mnt/state2';
      final cacheFile = await CachePaths.syncStateCacheFile(mount);
      await Directory(cacheFile.parent.path).create(recursive: true);
      await cacheFile.writeAsString('!!');
      expect(await SyncStateStore(mount).load(), isEmpty);
    });
  });
}
