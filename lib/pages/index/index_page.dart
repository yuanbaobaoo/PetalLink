import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_controller.dart';

/// 首页（仅做路由重定向，无 UI）
class IndexPage extends StatelessWidget {
  const IndexPage({super.key});

  @override
  Widget build(BuildContext context) {
    final authController = Get.find<AuthController>();

    // 根据登录状态重定向
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (authController.state.value.isAuthorized) {
        Get.offAllNamed('/files');
      } else {
        Get.offAllNamed('/login');
      }
    });

    return const Scaffold(
      body: Center(child: CircularProgressIndicator()),
    );
  }
}
