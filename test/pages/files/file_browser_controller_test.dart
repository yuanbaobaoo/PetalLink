import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/service/drive/files_service.dart';

// =============================================================================
// FileBrowserController 单测：requestId 乱序防护 / 导航 / 目录树懒加载 /
// enterFolderFromTree 路径替换 / 排序与过滤。
// =============================================================================

/// 可控的 FilesService 假实现：按 parentId 返回预置页，支持延迟完成。
class FakeFilesService implements FilesService {
  /// 各目录预置文件（key 为 parentId，null 表示根目录）
  final Map<String?, List<DriveFile>> pages = {};

  /// 延迟加载的 completer（key 为 parentId；设置后 list 挂起直到 complete）
  final Map<String?, Completer<AppResult<FileListResult>>> gates = {};

  /// list 调用记录
  final List<String?> listCalls = [];

  @override
  Future<AppResult<FileListResult>> list({
    String? parentId,
    String? cursor,
    int pageSize = 100,
  }) {
    listCalls.add(parentId);
    final gate = gates[parentId];
    if (gate != null) return gate.future;
    return Future.value(
        Ok(FileListResult(files: pages[parentId] ?? const [])));
  }

  @override
  Future<AppResult<FileListResult>> search(
    String keyword, {
    String? parentId,
    int pageSize = 100,
  }) {
    return Future.value(Ok(FileListResult(
      files: (pages[parentId] ?? const [])
          .where((f) => f.name.contains(keyword))
          .toList(),
    )));
  }

  // ---- 以下方法本测试不触达 ----

  @override
  Future<AppResult<List<DriveFile>>> listAll({String? parentId}) =>
      throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> get(String id) => throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> createFolder(String name,
          {String? parentId}) =>
      throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> update(String id,
          {String? newName, String? newParentFolder, String? description}) =>
      throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> rename(String id, String newName) =>
      throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> moveFile(
          String id, String oldParentFolder, String newParentFolder) =>
      throw UnimplementedError();

  @override
  Future<AppResult<void>> delete(String id) => throw UnimplementedError();

  @override
  Future<AppResult<DriveFile>> deleteVerified(String id) =>
      throw UnimplementedError();

  @override
  Future<AppResult<bool>> verifyDeleted(String id) =>
      throw UnimplementedError();
}

DriveFile _folder(String id, String name) =>
    DriveFile(id: id, name: name, category: FileCategory.folder);

DriveFile _file(String id, String name, {int size = 0, DateTime? edited}) =>
    DriveFile(id: id, name: name, size: size, editedTime: edited);

void main() {
  late FakeFilesService filesService;
  late FileBrowserController controller;

  setUp(() {
    Get.testMode = true;
    filesService = FakeFilesService();
    controller = FileBrowserController(filesService: filesService);
  });

  tearDown(() {
    controller.onClose();
  });

  group('文件加载与乱序防护', () {
    test('loadFiles 应用当前目录页并缓存子目录', () async {
      filesService.pages[null] = [_folder('f1', '文档'), _file('a1', 'a.txt')];

      await controller.loadFiles();

      final state = controller.state.value;
      expect(state.loading, isFalse);
      expect(state.files.length, 2);
      expect(
        state.directoryChildren[FileBrowserController.rootKey]!.single.id,
        'f1',
      );
    });

    test('过期请求的晚到结果被拒绝（requestId 乱序防护）', () async {
      // 根目录加载挂起
      filesService.gates[null] = Completer<AppResult<FileListResult>>();
      filesService.pages[null] = [_file('old', '旧.txt')];
      filesService.pages['f1'] = [_file('new', '新.txt')];

      final pending = controller.loadFiles(); // requestId=1，挂起
      // 进入 f1（requestSequence 递增 → requestId=1 过期）
      controller.enterFolder(_folder('f1', '文档'));
      await pumpEventQueue();

      // 晚到的根目录结果完成
      filesService.gates[null]!
          .complete(Ok(FileListResult(files: filesService.pages[null]!)));
      await pending;

      // f1 的加载（requestId=3）正常完成
      await pumpEventQueue();

      final state = controller.state.value;
      expect(state.folderId, 'f1');
      expect(state.files.single.id, 'new');
    });
  });

  group('导航', () {
    test('enterFolder 追加面包屑并清空列表', () async {
      filesService.pages[null] = [_folder('f1', '文档')];
      await controller.loadFiles();

      controller.enterFolder(_folder('f1', '文档'));
      await pumpEventQueue();

      final state = controller.state.value;
      expect(state.folderId, 'f1');
      expect(state.breadcrumbs.map((b) => b.name), ['全部文件', '文档']);
    });

    test('enterFolder 忽略非文件夹', () async {
      controller.enterFolder(_file('a1', 'a.txt'));
      expect(controller.state.value.folderId, isNull);
    });

    test('navigateToBreadcrumb 截断路径', () async {
      filesService.pages[null] = [_folder('f1', '文档')];
      await controller.loadFiles();
      controller.enterFolder(_folder('f1', '文档'));
      await pumpEventQueue();

      controller.navigateToBreadcrumb(0);
      await pumpEventQueue();

      final state = controller.state.value;
      expect(state.folderId, isNull);
      expect(state.breadcrumbs.single.name, '全部文件');
    });
  });

  group('目录树', () {
    test('loadTreeChildren 写入缓存并清除 loading 标记', () async {
      filesService.pages['f1'] = [_folder('f2', '子目录'), _file('a1', 'a.txt')];

      expect(controller.state.value.treeLoadingIds, isEmpty);
      final future = controller.loadTreeChildren(_folder('f1', '文档'));
      expect(controller.state.value.treeLoadingIds, contains('f1'));
      await future;

      final state = controller.state.value;
      expect(state.treeLoadingIds, isEmpty);
      // 仅缓存文件夹（文件不进目录树）
      expect(state.directoryChildren['f1']!.single.id, 'f2');
    });

    test('enterFolderFromTree 用完整路径替换面包屑', () async {
      filesService.pages[null] = [_folder('f1', '文档')];
      await controller.loadFiles();
      // f1 下的子目录 f2
      filesService.pages['f1'] = [_folder('f2', '工作')];
      await controller.loadTreeChildren(_folder('f1', '文档'));

      controller.enterFolderFromTree(_folder('f2', '工作'));
      await pumpEventQueue();

      final state = controller.state.value;
      expect(state.folderId, 'f2');
      expect(state.breadcrumbs.map((b) => b.name), ['全部文件', '文档', '工作']);
    });
  });

  group('排序与过滤', () {
    test('sort 同字段反转升降序，异字段重置升序', () {
      controller.sort(SortField.size);
      expect(controller.state.value.sortField, SortField.size);
      expect(controller.state.value.ascending, isTrue);

      controller.sort(SortField.size);
      expect(controller.state.value.ascending, isFalse);

      controller.sort(SortField.name);
      expect(controller.state.value.sortField, SortField.name);
      expect(controller.state.value.ascending, isTrue);
    });

    test('visibleFiles 文件夹优先 + 名称排序 + query 过滤', () {
      final state = FileBrowserState(
        files: [
          _file('b1', 'b.txt'),
          _folder('z1', 'z目录'),
          _file('a1', 'a.txt'),
          _folder('y1', 'y目录'),
        ],
        query: '',
      );
      expect(
        state.visibleFiles.map((f) => f.name),
        ['y目录', 'z目录', 'a.txt', 'b.txt'],
      );

      final searched = state.copyWith(query: 'a.');
      expect(searched.visibleFiles.map((f) => f.name), ['a.txt']);
    });
  });

  group('搜索', () {
    test('searchFiles 替换列表并设置 query', () async {
      filesService.pages[null] = [
        _file('a1', '报告.pdf'),
        _file('b1', '笔记.txt'),
      ];
      await controller.loadFiles();

      await controller.searchFiles('报告');

      final state = controller.state.value;
      expect(state.query, '报告');
      expect(state.files.single.name, '报告.pdf');
    });

    test('clearSearch 清空 query 并重载', () async {
      filesService.pages[null] = [_file('a1', '报告.pdf')];
      await controller.searchFiles('报告');
      await controller.clearSearch();

      expect(controller.state.value.query, isEmpty);
      expect(controller.state.value.files.single.name, '报告.pdf');
    });
  });
}
