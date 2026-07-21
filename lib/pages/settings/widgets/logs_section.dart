import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 日志查看分区（对标 CMP SettingsScreen.kt LOGS 分支）。
///
/// 保留策略说明 +「打开日志查看器」（跳转独立日志页 /logs）。
class LogsSection extends StatelessWidget {
  const LogsSection({super.key});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '日志查看', icon: 'list'),
        SettingsPanel(
          contentPadding: EdgeInsets.all(metrics.logPanelPadding),
          contentSpacing: metrics.logPanelContentSpacing,
          children: [
            Text(
              '运行日志使用共享 1000 条 ring buffer，并保留 30 天滚动文件。',
              style: typography.logRetentionDescription.copyWith(
                color: colors.textPrimary,
              ),
            ),
            MateButton(
              label: '打开日志查看器',
              onClick: () => Get.toNamed('/logs'),
            ),
          ],
        ),
      ],
    );
  }
}
