import 'package:flutter/material.dart';

import 'package:petal_link/pages/logs/log_viewer_view.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 日志查看分区（对齐 Tauri SettingsPage 内嵌 LogViewerPage inline 模式）。
///
/// 内嵌完整日志查看器：级别过滤 chips + 导出/清空 + 日志列表（2s 轮询）。
class LogsSection extends StatelessWidget {
  const LogsSection({super.key});

  @override
  Widget build(BuildContext context) {
    return const Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        MateSectionHeader(text: '日志查看', icon: 'list'),
        SettingsPanel(
          contentPadding: EdgeInsets.zero,
          children: [LogViewerView()],
        ),
      ],
    );
  }
}
