import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/pages/files/widgets/file_list_view.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// FileListView 交互测试：列表渲染 / 排序表头 / 多选批量条 / 右键菜单 / 删除确认。
// =============================================================================

/// 最小测试环境（MateLinkTheme + MaterialApp + Dialog/Toast 宿主）
Widget _wrap(Widget child) {
  return MateLinkTheme(
    child: MaterialApp(
      home: Scaffold(
        body: Stack(
          children: [
            child,
            const MateDialogHost(),
            const MateToastHost(),
          ],
        ),
      ),
    ),
  );
}

DriveFile _folder(String id, String name) =>
    DriveFile(id: id, name: name, category: FileCategory.folder);

DriveFile _file(String id, String name,
        {int size = 0, String? mimeType, DateTime? edited}) =>
    DriveFile(id: id, name: name, size: size, mimeType: mimeType,
        editedTime: edited);

/// 通用回调记录
class _Callbacks {
  final List<SortField> sorts = [];
  final List<List<DriveFile>> deletes = [];
  final List<DriveFile> enters = [];
  final List<DriveFile> opens = [];
  int downloads = 0;
  int syncs = 0;
  bool Function(DriveFile) canFree = (_) => false;
}

Widget _listView(
  _Callbacks cb, {
  FileBrowserState? browser,
  Map<String, String> statuses = const {},
  bool mountConfigured = true,
  bool isIndexing = false,
}) {
  return FileListView(
    browser: browser ?? const FileBrowserState(),
    fileStatuses: statuses,
    thumbnails: const {},
    mountConfigured: mountConfigured,
    isIndexing: isIndexing,
    onSort: cb.sorts.add,
    onEnterFolder: cb.enters.add,
    onOpenItem: cb.opens.add,
    onThumbnailNeeded: (_) {},
    onDelete: cb.deletes.add,
    onPreviewFreeUp: (files, onResult) => onResult(const []),
    onFreeUp: (_) {},
    onDownload: (_) => cb.downloads++,
    onSyncFolder: (f) => cb.syncs++,
    onRename: (_, _) {},
    onMove: (_, _) {},
    onCanFreeUp: (f, onResult) => onResult(cb.canFree(f)),
  );
}

void main() {
  tearDown(() {
    MateDialog.close();
    MateToast.dismiss();
  });

  group('列表渲染', () {
    testWidgets('渲染文件行名称/大小/时间与底部统计', (tester) async {
      final cb = _Callbacks();
      final browser = FileBrowserState(files: [
        _folder('f1', '文档'),
        _file('a1', '报告.pdf', size: 2048, mimeType: 'application/pdf',
            edited: DateTime.utc(2026, 7, 20, 10, 30)),
      ]);

      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      expect(find.text('文档'), findsOneWidget);
      expect(find.text('报告.pdf'), findsOneWidget);
      // 文件夹大小列显示 —
      expect(find.text('—'), findsOneWidget);
      expect(find.text('2.0 KB'), findsOneWidget);
      expect(find.text('2026-07-20 10:30'), findsOneWidget);
      expect(find.text('2 项 · 已全部加载'), findsOneWidget);
      // 表头
      expect(find.text('名称'), findsOneWidget);
      expect(find.text('大小'), findsOneWidget);
      expect(find.text('修改时间'), findsOneWidget);
      expect(find.text('状态'), findsOneWidget);
    });

    testWidgets('空目录显示空状态', (tester) async {
      final cb = _Callbacks();
      await tester.pumpWidget(_wrap(_listView(cb)));
      await tester.pump();

      expect(find.text('此文件夹为空'), findsOneWidget);
    });

    testWidgets('双击文件夹进入，双击文件打开', (tester) async {
      final cb = _Callbacks();
      final browser = FileBrowserState(files: [
        _folder('f1', '文档'),
        _file('a1', '报告.pdf'),
      ]);
      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      await tester.tap(find.text('文档'));
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(find.text('文档'));
      // 消化单 tap 的 doubleTapTimeout 计时器
      await tester.pump(const Duration(milliseconds: 350));
      expect(cb.enters.single.name, '文档');

      await tester.tap(find.text('报告.pdf'));
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tap(find.text('报告.pdf'));
      await tester.pump(const Duration(milliseconds: 350));
      expect(cb.opens.single.name, '报告.pdf');
    });
  });

  group('排序表头', () {
    testWidgets('点击表头触发对应排序字段', (tester) async {
      final cb = _Callbacks();
      final browser = FileBrowserState(files: [_file('a1', 'a.txt')]);
      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      await tester.tap(find.text('名称'));
      await tester.pump();
      expect(cb.sorts, [SortField.name]);

      await tester.tap(find.text('大小'));
      await tester.pump();
      expect(cb.sorts, [SortField.name, SortField.size]);

      await tester.tap(find.text('修改时间'));
      await tester.pump();
      expect(cb.sorts, [SortField.name, SortField.size, SortField.modifiedTime]);
    });
  });

  group('多选批量条', () {
    testWidgets('开启多选勾选后出现批量条，关闭后消失', (tester) async {
      final cb = _Callbacks();
      final browser = FileBrowserState(files: [
        _file('a1', 'a.txt'),
        _file('b1', 'b.txt'),
      ]);
      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      // 初始无批量条
      expect(find.textContaining('已选'), findsNothing);

      // 表头 check 按钮开启多选
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'check'));
      await tester.pump();

      // 勾选第一行（checkbox 列表：表头 1 个 + 行 2 个）
      await tester.tap(find.byType(MateCheckbox).at(1));
      await tester.pump();

      expect(find.text('已选 1 项'), findsOneWidget);
      expect(find.text('批量下载'), findsOneWidget);
      expect(find.text('释放空间'), findsOneWidget);
      expect(find.text('批量删除'), findsOneWidget);

      // 批量下载回调
      await tester.tap(find.text('批量下载'));
      await tester.pump();
      expect(cb.downloads, 1);

      // 关闭批量条
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateIcon && w.name == 'x'));
      await tester.pump();
      expect(find.textContaining('已选'), findsNothing);
    });

    testWidgets('mountConfigured=false 时批量条无批量删除', (tester) async {
      final cb = _Callbacks();
      final browser = FileBrowserState(files: [_file('a1', 'a.txt')]);
      await tester.pumpWidget(
          _wrap(_listView(cb, browser: browser, mountConfigured: false)));
      await tester.pump();

      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'check'));
      await tester.pump();
      await tester.tap(find.byType(MateCheckbox).at(1));
      await tester.pump();

      expect(find.text('已选 1 项'), findsOneWidget);
      expect(find.text('批量下载'), findsOneWidget);
      expect(find.text('批量删除'), findsNothing);
    });
  });

  group('右键菜单与删除确认', () {
    testWidgets('操作按钮打开菜单，条件项按 mountConfigured 渲染', (tester) async {
      final cb = _Callbacks()..canFree = (_) => true;
      final browser = FileBrowserState(files: [_file('a1', 'a.txt')]);
      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      // 行尾操作按钮（icon=list）
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'list'));
      await tester.pumpAndSettle();

      expect(find.text('执行双端对齐'), findsOneWidget);
      expect(find.text('释放空间'), findsOneWidget);
      expect(find.text('重命名'), findsOneWidget);
      expect(find.text('移动到…'), findsOneWidget);
      expect(find.text('属性'), findsOneWidget);
      expect(find.text('删除'), findsOneWidget);

      // mountConfigured=false 时只剩 属性（+ 可释放时 释放空间）
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();
      await tester.pumpWidget(_wrap(
          _listView(cb, browser: browser, mountConfigured: false)));
      await tester.pump();
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'list'));
      await tester.pumpAndSettle();

      expect(find.text('属性'), findsOneWidget);
      expect(find.text('执行双端对齐'), findsNothing);
      expect(find.text('重命名'), findsNothing);
      expect(find.text('删除'), findsNothing);
    });

    testWidgets('菜单删除 → 确认对话框 → 确认后回调 onDelete', (tester) async {
      final cb = _Callbacks();
      final file = _file('a1', '报告.pdf');
      final browser = FileBrowserState(files: [file]);
      await tester.pumpWidget(_wrap(_listView(cb, browser: browser)));
      await tester.pump();

      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'list'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('删除'));
      await tester.pumpAndSettle();

      // 删除确认对话框（MateDialog 命令式）
      expect(find.text('删除文件'), findsOneWidget);
      expect(find.text('确定删除「报告.pdf」吗？删除后进入回收站。'), findsOneWidget);

      await tester.tap(find.text('删除').last);
      await tester.pumpAndSettle();
      expect(cb.deletes.single.single.id, 'a1');
    });

    testWidgets('已同步文件删除警告包含双端同删提示', (tester) async {
      final cb = _Callbacks();
      final file = _file('a1', '报告.pdf');
      final browser = FileBrowserState(files: [file]);
      await tester.pumpWidget(_wrap(_listView(
        cb,
        browser: browser,
        statuses: const {'a1': 'synced'},
      )));
      await tester.pump();

      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'list'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('删除'));
      await tester.pumpAndSettle();

      expect(find.textContaining('已双端对齐到本地'), findsOneWidget);
    });
  });
}
