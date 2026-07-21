import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_controller.dart';

/// 鉴权中间件
class AuthMiddleware extends GetMiddleware {
  /// 当前路由是否需要登录
  final bool authorized;

  AuthMiddleware({this.authorized = true});

  @override
  RouteSettings? redirect(String? route) {
    final authController = Get.find<AuthController>();
    final authState = authController.state.value;

    // 需要登录但当前未登录 → 跳转登录页
    if (authorized && !authState.isAuthorized) {
      return const RouteSettings(name: '/login');
    }

    // 已登录但访问登录页 → 跳转文件页
    if (authState.isAuthorized && route == '/login') {
      return const RouteSettings(name: '/files');
    }

    return null;
  }
}
