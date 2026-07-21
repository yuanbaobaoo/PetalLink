import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/pages/update/update_page.dart';
import 'package:petal_link/service/update/update_service.dart';

// =============================================================================
// UpdatePage 测试：9 阶段状态机 UI 渲染（对标 CMP UpdateDialogScreen.kt）。
// =============================================================================

/// 假更新控制器：跳过启动/周期定时器；下载/重启只计数不走真实流程。
class _FakeUpdateController extends UpdateController {
  _FakeUpdateController()
      : super(
          updateService: UpdateService(
            httpClient: MockClient((_) async => http.Response('{}', 404)),
            currentVersion: '1.0.0',
          ),
          hasActiveTransfersProvider: () async => false,
        );

  int installCalls = 0;
  int relaunchCalls = 0;

  @override
  // ignore: must_call_super — 有意跳过 3s 启动检查与每小时定期检查定时器
  void onReady() {}

  @override
  Future<void> downloadAndInstall() async {
    installCalls++;
  }

  @override
  Future<void> relaunch() async {
    relaunchCalls++;
  }
}

void main() {
  const manifest = UpdateManifest(
    version: '1.1.0',
    url: 'https://example.com/PetalLink.dmg',
    sha256: 'abc123',
    notes: '修复若干问题\n优化同步性能',
  );

  late _FakeUpdateController updater;

  setUp(() {
    updater = _FakeUpdateController();
    Get.put<UpdateController>(updater);
  });

  tearDown(() {
    Get.reset();
  });

  Widget wrap() {
    return MaterialApp(
      home: MateLinkTheme(
        child: const Scaffold(
          body: Stack(
            children: [
              SizedBox.expand(),
              UpdatePage(),
            ],
          ),
        ),
      ),
    );
  }

  /// 设置更新状态机阶段
  void setPhase(
    UpdatePhase phase, {
    UpdateManifest? manifest,
    double progress = 0,
    String? error,
    bool hasActive = false,
    bool dialogVisible = true,
  }) {
    updater.state.value = UpdateUIState(
      phase: phase,
      manifest: manifest,
      downloadProgress: progress,
      errorMessage: error,
      hasActiveTransfers: hasActive,
      dialogVisible: dialogVisible,
    );
  }

  testWidgets('idle / 弹窗不可见：不渲染任何对话框内容', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    expect(find.text('发现新版本'), findsNothing);
    expect(find.text('立即更新'), findsNothing);
  });

  testWidgets('available：标题/版本/更新日志/按钮组，点击立即更新', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(UpdatePhase.available, manifest: manifest);
    await tester.pump();

    expect(find.text('发现新版本'), findsOneWidget);
    expect(find.text('v1.1.0'), findsOneWidget);
    expect(find.text('更新内容'), findsOneWidget);
    expect(find.text('修复若干问题\n优化同步性能'), findsOneWidget);
    expect(find.text('稍后提醒'), findsOneWidget);

    await tester.tap(find.text('立即更新'));
    await tester.pump();
    expect(updater.installCalls, 1);
  });

  testWidgets('available 无更新日志：显示占位提示', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(
      UpdatePhase.available,
      manifest: const UpdateManifest(
        version: '1.1.0',
        url: 'https://example.com/PetalLink.dmg',
        sha256: 'abc123',
      ),
    );
    await tester.pump();

    expect(find.text('暂无更新日志。是否下载并安装此更新？'), findsOneWidget);
  });

  testWidgets('downloading：进度百分比 + 后台下载按钮', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(UpdatePhase.downloading, manifest: manifest, progress: 0.42);
    await tester.pump();

    expect(find.text('正在下载更新…'), findsOneWidget);
    expect(find.text('42%'), findsOneWidget);
    expect(find.text('后台下载'), findsOneWidget);
  });

  testWidgets('waitingTransfers 有活跃传输：立即重启禁用', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(
      UpdatePhase.waitingTransfers,
      manifest: manifest,
      hasActive: true,
    );
    await tester.pump();

    expect(find.text('下载完成'), findsOneWidget);
    expect(find.text('下载完成。等待所有文档上传/下载完成后自动重启…'), findsOneWidget);
    expect(find.text('后台等待'), findsOneWidget);

    // 禁用态「等待传输完成…」点击不触发重启
    await tester.tap(find.text('等待传输完成…'));
    await tester.pump();
    expect(updater.relaunchCalls, 0);
  });

  testWidgets('ready：就绪文案 + 立即重启', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(UpdatePhase.ready, manifest: manifest);
    await tester.pump();

    expect(find.text('更新就绪'), findsOneWidget);
    expect(find.text('更新已准备就绪，重启即可生效。'), findsOneWidget);
    expect(find.text('稍后'), findsOneWidget);

    await tester.tap(find.text('立即重启'));
    await tester.pump();
    expect(updater.relaunchCalls, 1);
  });

  testWidgets('failed：错误文案 + 重试 / 关闭', (tester) async {
    await tester.pumpWidget(wrap());
    setPhase(UpdatePhase.failed, manifest: manifest, error: '网络错误');
    await tester.pump();

    expect(find.text('更新失败'), findsOneWidget);
    expect(find.text('网络错误'), findsOneWidget);

    // 重试 → 重新下载
    await tester.tap(find.text('重试'));
    await tester.pump();
    expect(updater.installCalls, 1);

    // 关闭 → dismiss 回到 idle，对话框消失
    await tester.tap(find.text('关闭'));
    await tester.pump();
    expect(updater.state.value.phase, UpdatePhase.idle);
    expect(find.text('更新失败'), findsNothing);
  });
}
