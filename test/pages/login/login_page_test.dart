import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/auth/auth_state.dart';
import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/login/login_page.dart';
import 'package:petal_link/service/auth/auth_secrets.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/types/enums.dart';

// =============================================================================
// LoginPage 测试：登录状态流转渲染（对标 CMP LoginScreen.kt 三态）。
// =============================================================================

/// 假认证控制器：跳过 restoreSession，login 只置 authorizing 不走真实 OAuth。
class _FakeAuthController extends AuthController {
  int loginCalls = 0;

  @override
  // ignore: must_call_super — 有意跳过 restoreSession（避免触达 token store）
  void onInit() {}

  @override
  Future<bool> login([String server = '']) async {
    loginCalls++;
    state.value = const AuthState(status: AuthStatus.Authorizing);
    return true;
  }
}

void main() {
  late _FakeAuthController auth;

  /// 注册依赖：AuthService（secrets 决定 secretConfigured）+ 假 AuthController
  void registerDeps({required bool secretConfigured}) {
    Get.put<AuthService>(AuthService(
      secrets: secretConfigured
          ? const AuthSecrets(clientId: 'id', clientSecret: 'secret')
          : const AuthSecrets(),
    ));
    auth = _FakeAuthController();
    Get.put<AuthController>(auth);
  }

  Widget wrap() {
    return MaterialApp(
      home: MateLinkTheme(child: const LoginPage()),
    );
  }

  tearDown(() {
    Get.reset();
  });

  testWidgets('secret 未配置：显示警告横幅且登录按钮禁用', (tester) async {
    registerDeps(secretConfigured: false);
    await tester.pumpWidget(wrap());
    await tester.pump(); // 等待 isSecretConfigured 检查完成

    expect(find.textContaining('尚未配置 OAuth 凭据'), findsOneWidget);
    expect(find.text('使用华为账号登录'), findsOneWidget);

    // 禁用态点击不触发登录
    await tester.tap(find.text('使用华为账号登录'));
    await tester.pump();
    expect(auth.loginCalls, 0);
    expect(find.text('请在浏览器中完成授权...'), findsNothing);
  });

  testWidgets('已配置：点击登录 → 授权中面板 → 取消回到登录按钮', (tester) async {
    registerDeps(secretConfigured: true);
    await tester.pumpWidget(wrap());
    await tester.pump();

    expect(find.textContaining('尚未配置 OAuth 凭据'), findsNothing);

    // 点击登录 → 进入授权中
    await tester.tap(find.text('使用华为账号登录'));
    await tester.pump();
    expect(auth.loginCalls, 1);
    expect(find.text('请在浏览器中完成授权...'), findsOneWidget);
    expect(find.text('取消授权'), findsOneWidget);
    expect(find.text('使用华为账号登录'), findsNothing);

    // 取消授权 → 回到登录按钮
    await tester.tap(find.text('取消授权'));
    await tester.pump();
    expect(find.text('使用华为账号登录'), findsOneWidget);
    expect(find.text('请在浏览器中完成授权...'), findsNothing);
  });

  testWidgets('授权失败：错误横幅 + 重新授权回到初始态', (tester) async {
    registerDeps(secretConfigured: true);
    await tester.pumpWidget(wrap());
    await tester.pump();

    // 认证状态置为 Error → 页面显示错误横幅（带重新授权动作）
    auth.state.value = const AuthState(status: AuthStatus.Error);
    await tester.pump();
    expect(find.text('授权失败，请重试'), findsOneWidget);
    expect(find.text('重新授权'), findsOneWidget);

    // 点击重新授权 → 错误横幅消失，回到登录按钮
    await tester.tap(find.text('重新授权'));
    await tester.pump();
    expect(find.text('授权失败，请重试'), findsNothing);
    expect(find.text('使用华为账号登录'), findsOneWidget);
  });
}
