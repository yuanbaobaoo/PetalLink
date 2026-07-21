import 'dart:async';

import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_state.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/auth/auth_constants.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/types/enums.dart';

/// 认证控制器 — 全局认证状态管理
///
/// 职责：
/// - 维护 UI 认证状态机（AuthState）
/// - 委托 [AuthService] 执行完整 OAuth PKCE 流程 / 恢复 / 刷新 / 登出
/// - token 持久化由 AuthService 内部的 token.bin 加密存储负责
///
/// 对标 CMP auth store（docs/08 §2.1），状态机：
///   init --restore()--> authorized | unauthorized | error
///   unauthorized --login()--> authorizing --> authorized | error | unauthorized(用户取消)
///   authorized --logout()--> unauthorized
///   error --dismissError()--> unauthorized
class AuthController extends GetxController {
  /// OAuth 回调端口（对齐 Rust `DEFAULT_CALLBACK_PORT`）
  static const int callbackPort = AuthConstants.defaultCallbackPort;

  late final AuthService _authService = Get.find<AuthService>();

  /// 认证状态（响应式）
  final Rx<AuthState> state = AuthState.init().obs;

  @override
  void onInit() {
    super.onInit();
    restoreSession();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 登录态恢复
  // ═══════════════════════════════════════════════════════════════════

  /// 恢复登录态：加载 token.bin，临期则刷新（对齐 Rust auth_restore）
  Future<void> restoreSession() async {
    try {
      final snapshot = await _authService.restore(callbackPort: callbackPort);
      if (snapshot.loggedIn) {
        final token = await _authService.refresher.currentToken();
        if (token != null) {
          state.value = AuthState.authorized(
            accessToken: token.accessToken,
            refreshToken: token.refreshToken,
            accountName: _authService.currentUserInfo?.primaryLabel,
            expiresAt: token.expiresAt,
          );
          AppLogger.i('登录态已恢复');
          // 已登录 → 确保同步引擎启动（对齐 Rust ensure_engine_started）
          unawaited(Get.find<SyncService>().ensureEngineStarted());
          return;
        }
      }
      state.value = AuthState.unauthorized();
    } catch (e, st) {
      AppLogger.e('恢复登录态失败', e, st);
      state.value = AuthState.unauthorized();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // OAuth PKCE 登录流程
  // ═══════════════════════════════════════════════════════════════════

  /// 发起 OAuth PKCE 登录（委托 [AuthService.authorize] 完整流程）
  ///
  /// 防重复：authorizing 中 return（对标 CMP login() 防重复）。
  /// [server] 参数保留兼容（华为端点为固定常量，不再使用）。
  Future<bool> login([String server = '']) async {
    if (state.value.status == AuthStatus.authorizing) {
      AppLogger.d('login 防重复：已在授权流程中');
      return false;
    }

    state.value = const AuthState(status: AuthStatus.authorizing);

    try {
      final token = await _authService.authorize(port: callbackPort);

      state.value = AuthState.authorized(
        accessToken: token.accessToken,
        refreshToken: token.refreshToken,
        accountName: _authService.currentUserInfo?.primaryLabel,
        expiresAt: token.expiresAt,
      );
      AppLogger.i('OAuth 登录成功');
      // 登录成功 → 确保同步引擎启动（对齐 Rust ensure_engine_started）
      unawaited(Get.find<SyncService>().ensureEngineStarted());
      return true;
    } on AppError catch (e) {
      if (e is AuthError && e.authCode == AuthErrorCode.cancelled) {
        // 用户主动取消：回到未登录，不视为错误
        state.value = AuthState.unauthorized();
      } else {
        AppLogger.e('OAuth 登录失败', e);
        state.value = const AuthState(status: AuthStatus.error);
      }
      return false;
    } catch (e, st) {
      AppLogger.e('login 异常', e, st);
      state.value = const AuthState(status: AuthStatus.error);
      return false;
    }
  }

  /// 取消当前 OAuth 流程：停止回调服务器，状态回到 unauthorized。
  void cancelLogin() {
    _authService.cancelAuthorize();
    if (state.value.status == AuthStatus.authorizing) {
      state.value = AuthState.unauthorized();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // Token 刷新
  // ═══════════════════════════════════════════════════════════════════

  /// Token 手动刷新（对标 CMP refreshToken）：失败则自动登出。
  ///
  /// 注：自动临期刷新由 MateHttpClient tokenProvider →
  /// [AuthService.ensureValidAccessToken] 完成。
  Future<void> refreshToken() async {
    if (!state.value.isAuthorized) return;

    try {
      AppLogger.i('正在刷新 token...');
      final token = await _authService.refresher.refresh();

      state.value = AuthState.authorized(
        accessToken: token.accessToken,
        refreshToken: token.refreshToken,
        accountName: state.value.accountName,
        expiresAt: token.expiresAt,
      );
      AppLogger.i('Token 刷新成功');
    } catch (e, st) {
      AppLogger.e('refreshToken 异常', e, st);
      await logout();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 登出
  // ═══════════════════════════════════════════════════════════════════

  /// 登出：停止同步引擎 + 清同步状态/缓存 + 清 token.bin + 内存缓存，
  /// 状态回到 unauthorized（对齐 Rust auth_logout 命令面）
  Future<void> logout() async {
    try {
      // 先停引擎并清同步状态（对齐 Rust cleanup_orphan_state 接缝）
      await Get.find<SyncService>().onLogout();
    } catch (e) {
      AppLogger.e('登出时停止同步引擎失败', e);
    }
    try {
      await _authService.logout();
    } catch (e) {
      AppLogger.e('登出清理失败', e);
    }
    state.value = AuthState.unauthorized();
    AppLogger.i('已登出');
  }

  /// 清除错误状态，回到 unauthorized
  void dismissError() {
    if (state.value.status == AuthStatus.error) {
      state.value = AuthState.unauthorized();
    }
  }
}
