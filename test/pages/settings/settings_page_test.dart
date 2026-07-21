import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/auth/auth_state.dart';
import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/config_entry.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/settings/settings_page.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/about_service.dart';
import 'package:petal_link/service/platform/platform_service.dart';
import 'package:petal_link/service/update/update_service.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// SettingsPage 测试：分区渲染 / 保存流程 / 校验失败 / 退出登录确认。
// =============================================================================

/// 假配置服务：内存加载/保存，记录保存入参。
class _FakeConfigService extends ConfigService {
  _FakeConfigService()
      : super(DatabaseService.instance, const FlutterSecureStorage());

  final AppConfig _loaded = const AppConfig(
    mountDir: '/tmp/petal-mount',
    mountConfigured: true,
  );

  int saveCalls = 0;
  AppConfig? savedConfig;

  @override
  Future<AppConfig> configLoad() async => _loaded;

  @override
  Future<void> configSave(AppConfig config) async {
    saveCalls++;
    savedConfig = config;
  }
}

/// 假平台服务：开机启动内存态。
class _FakePlatformService extends PlatformService {
  bool launchEnabled = false;

  @override
  bool launchAtLoginIsEnabled() => launchEnabled;

  @override
  Future<bool> setLaunchAtLoginEnabled(bool enabled) async {
    launchEnabled = enabled;
    return true;
  }
}

/// 假配额服务：返回 36.5 GB / 200 GB。
class _FakeAboutService extends AboutService {
  _FakeAboutService()
      : super(MateHttpClient(
          baseUrl: '',
          tokenProvider: () async => '',
          refreshTokenProvider: () async => null,
          onAuthExpired: () {},
        ));

  @override
  Future<AppResult<DriveAbout>> get() async {
    return const Ok(DriveAbout(
      userCapacity: 214748364800, // 200 GB
      usedSpace: 39191576576, // 36.5 GB
    ));
  }
}

/// 假认证控制器：跳过 restoreSession，logout 只计数。
class _FakeAuthController extends AuthController {
  int logoutCalls = 0;

  @override
  // ignore: must_call_super — 有意跳过 restoreSession（避免触达 token store）
  void onInit() {}

  @override
  Future<void> logout() async {
    logoutCalls++;
    state.value = AuthState.unauthorized();
  }
}

/// 假更新控制器：跳过启动/周期定时器。
class _FakeUpdateController extends UpdateController {
  _FakeUpdateController()
      : super(
          updateService: UpdateService(
            httpClient: MockClient((_) async => http.Response('{}', 404)),
            currentVersion: '1.0.0',
          ),
          hasActiveTransfersProvider: () async => false,
        );

  @override
  // ignore: must_call_super — 有意跳过启动/周期检查定时器
  void onReady() {}
}

void main() {
  late _FakeConfigService config;
  late _FakePlatformService platform;
  late _FakeAuthController auth;

  setUp(() {
    PackageInfo.setMockInitialValues(
      appName: 'PetalLink',
      packageName: 'dev.petallink',
      version: '1.0.0',
      buildNumber: '1',
      buildSignature: '',
    );

    config = _FakeConfigService();
    platform = _FakePlatformService();
    auth = _FakeAuthController();

    Get.put<ConfigService>(config);
    Get.put<PlatformService>(platform);
    Get.put<AboutService>(_FakeAboutService());
    Get.put<AuthService>(AuthService(
      secrets: const AuthSecrets(clientId: 'id', clientSecret: 'secret'),
    ));
    Get.put<AuthController>(auth);
    Get.put<UpdateController>(_FakeUpdateController());
  });

  tearDown(() {
    Get.reset();
  });

  /// 挂载设置页（含 Dialog/Toast 宿主，对齐 app.dart builder 结构）
  Widget wrap() {
    return MaterialApp(
      home: MateLinkTheme(
        child: const Stack(
          children: [
            SettingsPage(),
            MateDialogHost(),
            MateToastHost(),
          ],
        ),
      ),
    );
  }

  testWidgets('默认渲染：导航分组 + 同步目录分区 + 保存底栏', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump(); // 等待 loadSettings 完成

    expect(find.text('设置'), findsOneWidget);
    expect(find.text('通用'), findsOneWidget);
    expect(find.text('其他'), findsOneWidget);
    expect(find.text('当前同步目录'), findsOneWidget);
    expect(find.text('/tmp/petal-mount'), findsOneWidget);
    expect(find.text('保存设置'), findsOneWidget);
    expect(find.text('重置默认'), findsOneWidget);
  });

  testWidgets('保存流程：修改并发数 → 保存 → 已保存态 + toast', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    // 切到「传输设置」，步进器 +1（6 → 7）
    await tester.tap(find.text('传输设置'));
    await tester.pump();
    expect(find.text('并发上传数'), findsOneWidget);

    await tester.tap(find.text('+'));
    await tester.pump();
    expect(find.text('7'), findsOneWidget);

    // 保存 → configSave 收到 concurrency=7；按钮转「已保存」+ 绿色指示 + toast
    await tester.tap(find.text('保存设置'));
    await tester.pump();
    expect(config.saveCalls, 1);
    expect(config.savedConfig?.concurrency, 7);
    expect(find.text('已保存'), findsOneWidget);
    // 绿色指示 + Toast 两处「配置已保存」
    expect(find.text('配置已保存'), findsNWidgets(2));
  });

  testWidgets('校验失败：轮询间隔 30 不落盘并显示错误文案', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    await tester.tap(find.text('传输设置'));
    await tester.pump();

    // 自动同步间隔输入 30（违反「0 或 ≥ 60」校验）
    final fields =
        tester.widgetList<TextField>(find.byType(TextField)).toList();
    final pollField =
        fields.firstWhere((f) => f.controller?.text == '60');
    await tester.enterText(find.byWidget(pollField), '30');
    await tester.pump();

    await tester.tap(find.text('保存设置'));
    await tester.pump();

    expect(config.saveCalls, 0);
    expect(find.textContaining('云端刷新间隔必须为 0（关闭）或 ≥ 60 秒'), findsOneWidget);
  });

  testWidgets('账号管理：配额展示 + 退出登录确认对话框', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();
    await tester.pump(); // 等待配额加载

    await tester.tap(find.text('账号管理'));
    await tester.pump();

    expect(find.text('未获取到'), findsOneWidget); // userInfo 为 null
    expect(find.text('存储配额'), findsOneWidget);
    expect(find.text('36.5 GB'), findsOneWidget);
    expect(find.text('200.0 GB'), findsOneWidget);

    // 退出登录 → 确认对话框 → 确认（按钮与行标签同名，精确定位 MateButton；
    // 按钮位于滚动列表底部，先滚动到可见再点击）
    final logoutButton = tester
        .widgetList<MateButton>(find.byType(MateButton))
        .firstWhere((b) => b.label == '退出登录');
    await tester.ensureVisible(find.byWidget(logoutButton));
    await tester.pump();
    await tester.tap(find.byWidget(logoutButton));
    await tester.pump();
    expect(find.text('将清除本地 token 并返回登录页，后台同步会停止。确定退出登录？'),
        findsOneWidget);

    // 对话框确认按钮（页面按钮 + 对话框按钮同名，取后者）
    final confirmButton = tester
        .widgetList<MateButton>(find.byType(MateButton))
        .lastWhere((b) => b.label == '退出登录');
    await tester.tap(find.byWidget(confirmButton));
    await tester.pump();
    expect(auth.logoutCalls, 1);
  });

  testWidgets('关于：版本号 + 检查更新 + 外链', (tester) async {
    await tester.pumpWidget(wrap());
    await tester.pump();

    await tester.tap(find.text('关于'));
    await tester.pump();

    expect(find.text('版本 1.0.0'), findsOneWidget);
    expect(find.text('检查更新'), findsOneWidget);
    expect(find.text('一款开源免费的华为云盘客户端'), findsOneWidget);
    expect(find.text('GitHub'), findsOneWidget);
    expect(find.text('GitCode'), findsOneWidget);
  });
}
