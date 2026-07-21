import 'dart:async';

import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/auth/auth_state.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/types/enums.dart';

/// 登录页状态
///
/// 对标 CMP LoginScreen 的三个入参：
/// `loggingIn` / `secretConfigured` / `errorMessage`（这里合并为状态机字段）。
class LoginState {
  /// 认证状态
  final AuthStatus status;

  /// OAuth 密钥是否已配置（client_id + client_secret）
  final bool secretConfigured;

  /// 错误消息（null 时不显示错误横幅）
  final String? errorMessage;

  const LoginState({
    this.status = AuthStatus.init,
    this.secretConfigured = false,
    this.errorMessage,
  });

  /// 初始状态
  factory LoginState.initial() => const LoginState();

  /// 深拷贝并替换指定字段
  LoginState copyWith({
    AuthStatus? status,
    bool? secretConfigured,
    String? errorMessage,
    bool clearError = false,
  }) {
    return LoginState(
      status: status ?? this.status,
      secretConfigured: secretConfigured ?? this.secretConfigured,
      errorMessage: clearError ? null : (errorMessage ?? this.errorMessage),
    );
  }

  /// 是否正在授权中（对标 CMP `loggingIn`）
  bool get isAuthorizing => status == AuthStatus.authorizing;

  /// 是否可发起登录
  bool get canLogin => secretConfigured && !isAuthorizing;
}

/// 登录页控制器 — 登录页 UI 状态与 OAuth PKCE 流程编排
///
/// 职责：
/// - 管理登录页 UI 状态（[LoginState]）
/// - 检查 OAuth 密钥配置状态（[AuthService.isSecretConfigured]）
/// - 委托 [AuthController] 执行完整 OAuth PKCE 流程
/// - 监听 [AuthController.state] 变化同步更新页面状态
///
/// 对标 CMP LoginScreen.kt 的 `onLogin` / `onCancel` / `onDismissError` 三回调。
class LoginController extends GetxController {
  final AuthController _authController = Get.find<AuthController>();
  final AuthService _authService = Get.find<AuthService>();

  /// 页面状态（响应式）
  final Rx<LoginState> state = LoginState.initial().obs;

  /// 监听 AuthController 状态变化的订阅
  StreamSubscription<AuthState>? _authSub;

  @override
  void onInit() {
    super.onInit();
    unawaited(refreshSecretConfig());
    _subscribeToAuthChanges();
  }

  @override
  void onClose() {
    _authSub?.cancel();
    super.onClose();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 初始化
  // ═══════════════════════════════════════════════════════════════════

  /// 检查 OAuth 密钥是否已配置（client_id + client_secret，来自 .env）
  Future<void> refreshSecretConfig() async {
    try {
      final configured = await _authService.isSecretConfigured();
      state.value = state.value.copyWith(secretConfigured: configured);
      if (!configured) {
        AppLogger.d('OAuth 密钥未配置（client_id / client_secret 缺失）');
      }
    } catch (e) {
      AppLogger.e('检查密钥配置失败', e);
    }
  }

  /// 订阅 [AuthController.state] 变化，同步更新登录页状态
  void _subscribeToAuthChanges() {
    _authSub = _authController.state.listen((authState) {
      switch (authState.status) {
        case AuthStatus.authorized:
          state.value = state.value.copyWith(status: AuthStatus.authorized);
          // 授权成功 → 跳转文件页（对齐 Vue App.vue 的条件渲染切换；
          // GetX 中间件 redirect 只在路由变化时触发，状态变化需显式导航）
          Get.offAllNamed('/files');
        case AuthStatus.error:
          state.value = state.value.copyWith(
            status: AuthStatus.error,
            errorMessage: '授权失败，请重试',
          );
        case AuthStatus.authorizing:
          state.value = state.value.copyWith(
            status: AuthStatus.authorizing,
            clearError: true,
          );
        case AuthStatus.unauthorized:
          // 授权中被打回未登录 → 用户取消，回到可点击登录的初始态
          if (state.value.status == AuthStatus.authorizing) {
            state.value = state.value.copyWith(status: AuthStatus.init);
          }
        case AuthStatus.init:
          if (state.value.status != AuthStatus.authorized) {
            state.value = state.value.copyWith(status: AuthStatus.init);
          }
      }
    });
  }

  // ═══════════════════════════════════════════════════════════════════
  // 登录操作
  // ═══════════════════════════════════════════════════════════════════

  /// 发起登录：委托 [AuthController] 执行完整 OAuth PKCE 流程
  ///
  /// 流程（对标 CMP LoginScreen `onLogin`）：
  /// 1. 再次检查密钥配置（防页面停留期间配置被清除）
  /// 2. 设置页面状态为 authorizing
  /// 3. AuthController 内部完成：PKCE 生成 → 打开浏览器 → 启动回调服务器 → 换 token
  /// 4. 后续状态流转由 [_subscribeToAuthChanges] 自动同步
  Future<void> login() async {
    // 防重复（对齐 CMP login() 防重复）
    if (state.value.isAuthorizing) {
      AppLogger.d('login 防重复：已在授权中');
      return;
    }

    await refreshSecretConfig();
    if (!state.value.secretConfigured) {
      state.value = state.value.copyWith(
        status: AuthStatus.error,
        errorMessage: '请先配置 OAuth 密钥（client_id / client_secret）',
      );
      return;
    }

    state.value = state.value.copyWith(
      status: AuthStatus.authorizing,
      clearError: true,
    );

    await _authController.login();
    // 成功/失败/取消均由订阅同步；此处无需再处理返回值
  }

  /// 取消登录：停止 OAuth 流程，回到初始状态（对标 CMP `onCancel`）
  void cancelLogin() {
    AppLogger.i('用户取消登录');
    _authController.cancelLogin();
    state.value = state.value.copyWith(
      status: AuthStatus.init,
      clearError: true,
    );
  }

  /// 关闭错误横幅（重新授权入口），回到初始状态（对标 CMP `onDismissError`）
  void dismissError() {
    _authController.dismissError();
    state.value = state.value.copyWith(
      status: AuthStatus.init,
      clearError: true,
    );
  }
}
