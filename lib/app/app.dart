import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/router/router.dart';
import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/update/update_page.dart';
import 'package:petal_link/widgets/index.dart';

/// PetalLink 应用入口 Widget
///
/// [MateLinkTheme] 自动检测系统亮度并注入 MateTheme + Flutter ThemeData。
/// builder 层挂载全局更新对话框 [UpdatePage] 与 [MateDialogHost]、
/// [MateToastHost]（全局命令式浮层宿主，对标 CMP 在根部挂载的
/// UpdateDialog/Dialog/Toast Host）。
class MateLinkApp extends StatelessWidget {
  const MateLinkApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MateLinkTheme(
      child: GetMaterialApp(
        debugShowCheckedModeBanner: false,
        title: 'PetalLink',
        themeMode: ThemeMode.system,
        getPages: MateRoutes.pages,
        initialRoute: MateRoutes.initial,
        builder: (context, child) => Stack(
          children: [
            ?child,
            // 全局更新对话框（覆盖所有页面，对齐 CMP Main.kt 顶层 UpdateDialogScreen）
            const UpdatePage(),
            const MateDialogHost(),
            const MateToastHost(),
          ],
        ),
      ),
    );
  }
}
