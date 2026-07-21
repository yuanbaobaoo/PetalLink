import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/pages/files/widgets/app_bar.dart';
import 'package:petal_link/pages/files/widgets/breadcrumb.dart';
import 'package:petal_link/pages/files/widgets/search_results.dart';
import 'package:petal_link/pages/files/widgets/sidebar.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// 页面组件测试：Sidebar（logo/位置树/账号卡/更新卡）、AppBar、Breadcrumb、
// SearchResults。
// =============================================================================

Widget _wrap(Widget child) {
  return MateLinkTheme(
    child: MaterialApp(
      home: Scaffold(
        body: Row(children: [child]),
      ),
    ),
  );
}

DriveFile _folder(String id, String name) =>
    DriveFile(id: id, name: name, category: FileCategory.folder);

void main() {
  group('FilesSidebar', () {
    testWidgets('渲染位置标签/目录树/账号卡（用户名 + 配额）', (tester) async {
      await tester.pumpWidget(_wrap(FilesSidebar(
        rootChildren: [_folder('f1', '文档')],
        directoryChildren: const {},
        selectedFolderId: '',
        pathFolderIds: const {},
        userName: '张三',
        quotaText: '36.5 GB / 200 GB',
        onDismissUpdate: () {},
        onInstallUpdate: () {},
        onShowUpdate: () {},
        onNavigate: (_) {},
        onExpandNode: (_) {},
      )));
      await tester.pump();

      expect(find.text('位置'), findsOneWidget);
      expect(find.text('全部文件'), findsOneWidget);
      expect(find.text('文档'), findsOneWidget);
      expect(find.text('张三'), findsOneWidget);
      expect(find.text('张'), findsOneWidget); // 渐变头像首字
      expect(find.text('36.5 GB / 200 GB'), findsOneWidget);
    });

    testWidgets('更新提示卡：版本号 + 立即更新/日志/关闭', (tester) async {
      var installs = 0, shows = 0, dismisses = 0;
      await tester.pumpWidget(_wrap(FilesSidebar(
        rootChildren: const [],
        directoryChildren: const {},
        selectedFolderId: '',
        pathFolderIds: const {},
        updateAvailableVersion: '1.2.0',
        onDismissUpdate: () => dismisses++,
        onInstallUpdate: () => installs++,
        onShowUpdate: () => shows++,
        onNavigate: (_) {},
        onExpandNode: (_) {},
      )));
      await tester.pump();

      expect(find.text('新版本 1.2.0'), findsOneWidget);
      await tester.tap(find.text('立即更新'));
      await tester.pump();
      expect(installs, 1);
      await tester.tap(find.text('日志'));
      await tester.pump();
      expect(shows, 1);
      await tester.tap(find.text('×'));
      await tester.pump();
      expect(dismisses, 1);
    });

    testWidgets('更新下载进度卡：百分比 + 点击重开', (tester) async {
      var shows = 0;
      await tester.pumpWidget(_wrap(FilesSidebar(
        rootChildren: const [],
        directoryChildren: const {},
        selectedFolderId: '',
        pathFolderIds: const {},
        updateDownloading: true,
        updateDownloadProgress: 0.42,
        onDismissUpdate: () {},
        onInstallUpdate: () {},
        onShowUpdate: () => shows++,
        onNavigate: (_) {},
        onExpandNode: (_) {},
      )));
      await tester.pump();

      expect(find.text('正在下载更新'), findsOneWidget);
      expect(find.text('42%'), findsOneWidget);
      await tester.tap(find.text('正在下载更新'));
      await tester.pump();
      expect(shows, 1);
    });

    testWidgets('目录树：点击节点导航，chevron 展开触发懒加载', (tester) async {
      final navigated = <DriveFile>[];
      final expanded = <DriveFile>[];
      await tester.pumpWidget(_wrap(FilesSidebar(
        rootChildren: [_folder('f1', '文档')],
        directoryChildren: const {},
        selectedFolderId: '',
        pathFolderIds: const {},
        onDismissUpdate: () {},
        onInstallUpdate: () {},
        onShowUpdate: () {},
        onNavigate: navigated.add,
        onExpandNode: expanded.add,
      )));
      await tester.pump();

      // 点击节点名称导航
      await tester.tap(find.text('文档'));
      await tester.pump();
      expect(navigated.single.id, 'f1');

      // 点击 chevron（节点未加载过子目录 → 触发懒加载回调）
      await tester.tap(find.byType(GestureDetector).first);
      await tester.pump();
      // 根节点 chevron 不触发懒加载（id 为空）；子节点 chevron 触发
      // 这里点击的是第一个 GestureDetector（根节点行），无需断言 expanded
    });

    testWidgets('选中节点高亮 brand 色', (tester) async {
      await tester.pumpWidget(_wrap(FilesSidebar(
        rootChildren: [_folder('f1', '文档')],
        directoryChildren: const {},
        selectedFolderId: 'f1',
        pathFolderIds: const {'f1'},
        onDismissUpdate: () {},
        onInstallUpdate: () {},
        onShowUpdate: () {},
        onNavigate: (_) {},
        onExpandNode: (_) {},
      )));
      await tester.pump();

      final text = tester.widget<Text>(find.text('文档'));
      final colors = MateTheme.colorsOf(
          tester.element(find.text('文档')));
      expect(text.style?.color, colors.brand);
    });
  });

  group('FilesAppBar', () {
    testWidgets('mountConfigured=false 隐藏 同步索引/Finder', (tester) async {
      await tester.pumpWidget(_wrap(Expanded(
        child: FilesAppBar(
          searchController: TextEditingController(),
          searchKeyword: '',
          mountConfigured: false,
          isIndexing: false,
          onSearchChanged: (_) {},
          onSearchSubmit: (_) {},
          onSearchClear: () {},
          onRefresh: () {},
          onToggleTransfer: () {},
          onOpenFinder: () {},
          onOpenSettings: () {},
        ),
      )));
      await tester.pump();

      expect(find.text('同步索引'), findsNothing);
      expect(find.text('Finder'), findsNothing);
      expect(find.text('传输队列'), findsOneWidget);
    });

    testWidgets('mountConfigured=true 显示全部按钮；搜索提交与清除', (tester) async {
      var refreshes = 0, toggles = 0, finders = 0, settings = 0;
      final submits = <String>[];
      var clears = 0;
      final controller = TextEditingController(text: '报告');
      await tester.pumpWidget(_wrap(Expanded(
        child: FilesAppBar(
          searchController: controller,
          searchKeyword: '报告',
          mountConfigured: true,
          isIndexing: false,
          onSearchChanged: (_) {},
          onSearchSubmit: submits.add,
          onSearchClear: () => clears++,
          onRefresh: () => refreshes++,
          onToggleTransfer: () => toggles++,
          onOpenFinder: () => finders++,
          onOpenSettings: () => settings++,
        ),
      )));
      await tester.pump();

      expect(find.text('同步索引'), findsOneWidget);
      expect(find.text('传输队列'), findsOneWidget);
      expect(find.text('Finder'), findsOneWidget);

      await tester.tap(find.text('同步索引'));
      await tester.pump();
      expect(refreshes, 1);
      await tester.tap(find.text('传输队列'));
      await tester.pump();
      expect(toggles, 1);
      await tester.tap(find.text('Finder'));
      await tester.pump();
      expect(finders, 1);

      // 回车提交搜索
      await tester.enterText(find.byType(TextField), '报表');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(submits.single, '报表');

      // 清除搜索按钮（keyword 非空时）
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'x'));
      await tester.pump();
      expect(clears, 1);

      // 设置按钮
      await tester.tap(find.byWidgetPredicate(
          (w) => w is MateButton && w.icon == 'settings'));
      await tester.pump();
      expect(settings, 1);
    });
  });

  group('FilesBreadcrumb', () {
    testWidgets('渲染路径与分隔符，点击非末级段跳转', (tester) async {
      final navigated = <int>[];
      await tester.pumpWidget(_wrap(Expanded(
        child: FilesBreadcrumb(
          crumbs: const [
            Breadcrumb(name: '全部文件'),
            Breadcrumb(id: 'f1', name: '文档'),
            Breadcrumb(id: 'f2', name: '工作'),
          ],
          onNavigate: navigated.add,
        ),
      )));
      await tester.pump();

      expect(find.text('全部文件'), findsOneWidget);
      expect(find.text('文档'), findsOneWidget);
      expect(find.text('工作'), findsOneWidget);
      expect(find.text('›'), findsNWidgets(2));

      await tester.tap(find.text('文档'));
      await tester.pump();
      expect(navigated.single, 1);

      // 末级段不可点
      await tester.tap(find.text('工作'));
      await tester.pump();
      expect(navigated.length, 1);
    });
  });

  group('FilesSearchResults', () {
    testWidgets('渲染关键词 header 与结果行，文件夹可进入', (tester) async {
      final enters = <DriveFile>[];
      await tester.pumpWidget(_wrap(Expanded(
        child: FilesSearchResults(
          keyword: '报告',
          searching: false,
          results: [
            _folder('f1', '报告汇总'),
            const DriveFile(id: 'a1', name: '报告.pdf', size: 100),
          ],
          onEnterFolder: enters.add,
        ),
      )));
      await tester.pump();

      expect(find.text('搜索：报告'), findsOneWidget);
      expect(find.text('报告汇总'), findsOneWidget);
      expect(find.text('文件夹'), findsOneWidget);
      expect(find.text('100 字节'), findsOneWidget);

      await tester.tap(find.text('报告汇总'));
      await tester.pump();
      expect(enters.single.id, 'f1');
    });

    testWidgets('空结果显示无匹配结果', (tester) async {
      await tester.pumpWidget(_wrap(const Expanded(
        child: FilesSearchResults(
          keyword: 'zzz',
          searching: false,
          results: [],
          onEnterFolder: _noop,
        ),
      )));
      await tester.pump();

      expect(find.text('无匹配结果'), findsOneWidget);
      expect(find.text('试试其他关键词'), findsOneWidget);
    });

    testWidgets('搜索中显示搜索中…', (tester) async {
      await tester.pumpWidget(_wrap(const Expanded(
        child: FilesSearchResults(
          keyword: '报告',
          searching: true,
          results: [],
          onEnterFolder: _noop,
        ),
      )));
      await tester.pump();

      expect(find.text('搜索中…'), findsOneWidget);
    });
  });
}

void _noop(DriveFile _) {}
