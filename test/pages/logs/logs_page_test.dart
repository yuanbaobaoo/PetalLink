import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/pages/logs/controller/logs_controller.dart';
import 'package:petal_link/pages/logs/logs_page.dart';
import 'package:petal_link/service/platform/platform_service.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// LogsPage 测试：级别筛选 / 空态 / 清空 / 轮询刷新 / 导出。
// =============================================================================

/// 假平台服务：记录 logsClear / logsExport 调用。
class _FakePlatformService extends PlatformService {
  int clearCalls = 0;
  String? exportPath;

  @override
  void logsClear() {
    clearCalls++;
  }

  @override
  Future<void> logsExport(String path) async {
    exportPath = path;
  }
}

void main() {
  late _FakePlatformService platform;

  setUp(() {
    platform = _FakePlatformService();
    Get.put<PlatformService>(platform);
  });

  tearDown(() {
    Get.reset();
  });

  Widget wrap() {
    return MaterialApp(
      home: MateLinkTheme(child: const LogsPage()),
    );
  }

  /// 卸载页面（触发 dispose → Get.delete → onClose，取消 2s 轮询定时器）
  Future<void> unmount(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  }

  /// 直接向控制器注入三条测试记录（error / warn / info）
  void seedRecords() {
    final controller = Get.find<LogsController>();
    controller.state.value = controller.state.value.copyWith(records: [
      const LogRecordDisplay(
        timestampMs: 1752731528000,
        level: AppLogLevel.error,
        target: 'petallink::sync',
        message: '上传失败：配额不足',
      ),
      const LogRecordDisplay(
        timestampMs: 1752731461000,
        level: AppLogLevel.warn,
        target: 'petallink::watcher',
        message: '文件被持续编辑超过 5 分钟',
      ),
      const LogRecordDisplay(
        timestampMs: 1752731400000,
        level: AppLogLevel.info,
        target: 'petallink::engine',
        message: '同步完成：处理 24 项变更',
      ),
    ]);
  }

  testWidgets('级别标签渲染与精确过滤', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();
    seedRecords();
    await tester.pump();

    // 工具栏 4 个过滤标签 + 三条记录全部展示
    expect(find.text('ALL'), findsOneWidget);
    expect(find.text('INFO'), findsWidgets);
    expect(find.text('WARN'), findsWidgets);
    expect(find.text('ERROR'), findsWidgets);
    expect(find.text('上传失败：配额不足'), findsOneWidget);
    expect(find.text('文件被持续编辑超过 5 分钟'), findsOneWidget);
    expect(find.text('同步完成：处理 24 项变更'), findsOneWidget);

    // 选中 ERROR（工具栏第一个 ERROR 标签）→ 仅错误条目
    await tester.tap(find.text('ERROR').first);
    await tester.pump();
    expect(find.text('上传失败：配额不足'), findsOneWidget);
    expect(find.text('文件被持续编辑超过 5 分钟'), findsNothing);
    expect(find.text('同步完成：处理 24 项变更'), findsNothing);

    // 选中 WARN → 仅警告条目
    await tester.tap(find.text('WARN').first);
    await tester.pump();
    expect(find.text('文件被持续编辑超过 5 分钟'), findsOneWidget);
    expect(find.text('上传失败：配额不足'), findsNothing);

    // 回到 ALL → 全部恢复
    await tester.tap(find.text('ALL'));
    await tester.pump();
    expect(find.text('上传失败：配额不足'), findsOneWidget);
    expect(find.text('文件被持续编辑超过 5 分钟'), findsOneWidget);
    expect(find.text('同步完成：处理 24 项变更'), findsOneWidget);

    await unmount(tester);
  });

  testWidgets('清空按钮：调用 logsClear 并展示空态', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();
    seedRecords();
    await tester.pump();
    expect(find.text('上传失败：配额不足'), findsOneWidget);

    // 点击工具栏 trash 图标按钮
    final clearButton = tester
        .widgetList<MateButton>(find.byType(MateButton))
        .firstWhere((b) => b.icon == 'trash');
    await tester.tap(find.byWidget(clearButton));
    await tester.pump();

    expect(platform.clearCalls, 1);
    expect(find.text('暂无日志'), findsOneWidget);

    await unmount(tester);
  });

  testWidgets('2s 轮询：新日志自动出现在列表顶部', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    // 写入一条新日志到环形缓冲（唯一标记，避免与其他用例串扰）
    AppLogger.i('轮询标记-8f3d2c');
    await tester.pump(
      const Duration(seconds: 2, milliseconds: 100),
    ); // 推进一个轮询周期

    expect(find.text('轮询标记-8f3d2c'), findsOneWidget);

    await unmount(tester);
  });

  testWidgets('导出：显式路径走 PlatformService.logsExport', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    final controller = Get.find<LogsController>();
    await controller.exportLogs('/tmp/petal-logs.txt');

    expect(platform.exportPath, '/tmp/petal-logs.txt');

    await unmount(tester);
  });
}
