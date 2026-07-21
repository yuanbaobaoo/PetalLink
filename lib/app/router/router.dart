import 'package:get/get.dart';

import 'package:petal_link/app/router/router_middleware.dart';
import 'package:petal_link/pages/index/index_page.dart';
import 'package:petal_link/pages/login/login_page.dart';
import 'package:petal_link/pages/files/files_page.dart';
import 'package:petal_link/pages/settings/settings_page.dart';
import 'package:petal_link/pages/logs/logs_page.dart';
import 'package:petal_link/pages/update/update_page.dart';

/// 路由配置
class MateRoutes {
  /// 初始路由（进入后根据登录状态自动跳转）
  static const String initial = '/';

  /// 路由列表
  static final List<GetPage> pages = [
    GetPage(
      name: '/',
      page: () => const IndexPage(),
      middlewares: [AuthMiddleware(authorized: false)],
    ),
    GetPage(
      name: '/login',
      page: () => const LoginPage(),
      middlewares: [AuthMiddleware(authorized: false)],
    ),
    GetPage(
      name: '/files',
      page: () => const FilesPage(),
      middlewares: [AuthMiddleware(authorized: true)],
      transition: Transition.fadeIn,
    ),
    GetPage(
      name: '/settings',
      page: () => const SettingsPage(),
      middlewares: [AuthMiddleware(authorized: true)],
      transition: Transition.rightToLeft,
    ),
    GetPage(
      name: '/logs',
      page: () => const LogsPage(),
      middlewares: [AuthMiddleware(authorized: true)],
      transition: Transition.rightToLeft,
    ),
    GetPage(
      name: '/update',
      page: () => const UpdatePage(),
      middlewares: [AuthMiddleware(authorized: true)],
      transition: Transition.rightToLeft,
    ),
  ];
}
