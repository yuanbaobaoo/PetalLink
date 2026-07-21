import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/widgets/index.dart';

/// 同步目录分区（对标 CMP SettingsScreen.kt SyncDirSection）。
///
/// 已配置：1px success 描边卡片 + 成功徽章 + 路径 chip +「更换目录」；
/// 未配置：MateEmpty 风格徽章引导 +「选择目录」；底部信息横幅。
class SyncDirSection extends StatelessWidget {
  /// 页面控制器
  final SettingsController notifier;

  /// 当前状态
  final SettingsState state;

  const SyncDirSection({
    super.key,
    required this.notifier,
    required this.state,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final configured = state.mountConfigured;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '同步目录', icon: 'folder'),

        // 目录卡片（radius 10；已配置 1px success 描边，未配置 0.5px 细边）
        Container(
          width: double.infinity,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.mountPanelHorizontalPadding,
            vertical: configured
                ? metrics.configuredMountVerticalPadding
                : metrics.emptyMountVerticalPadding,
          ),
          decoration: BoxDecoration(
            color: colors.bgContainer,
            borderRadius: BorderRadius.circular(metrics.mountPanelRadius),
            border: Border.all(
              width: configured
                  ? metrics.configuredMountBorderWidth
                  : metrics.emptyMountBorderWidth,
              color: configured ? colors.success : colors.border,
            ),
          ),
          child: Column(
            children: [
              if (configured)
                _ConfiguredContent(mountDir: state.mountDir, notifier: notifier)
              else
                _EmptyContent(notifier: notifier),
            ],
          ),
        ),

        SizedBox(height: metrics.mountBannerSpacing),
        const MateInfoBanner(
          message: '更换同步目录将清除所有本地缓存与登录状态并重启，云盘文件不受影响。',
          variant: MateBannerVariant.info,
        ),
      ],
    );
  }
}

/// 未配置态内容：72×72 品牌浅底渐变徽章 + 引导文案 + 选择目录按钮。
class _EmptyContent extends StatelessWidget {
  final SettingsController notifier;

  const _EmptyContent({required this.notifier});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Column(
      children: [
        Container(
          width: metrics.emptyMountBadgeSize,
          height: metrics.emptyMountBadgeSize,
          decoration: BoxDecoration(
            gradient: LinearGradient(colors: colors.brandGradientSoft),
            borderRadius:
                BorderRadius.circular(metrics.emptyMountBadgeRadius),
          ),
          alignment: Alignment.center,
          child: MateIcon(
            name: 'folder-open',
            size: metrics.emptyMountIconSize,
            tint: colors.brandHover,
          ),
        ),
        SizedBox(height: metrics.mountPanelContentSpacing),
        Text('尚未配置同步目录', style: typography.emptyMountTitle),
        SizedBox(height: metrics.mountPanelContentSpacing),
        Text(
          '选择一个本地空目录作为云盘镜像，文件将自动双向同步。',
          style: typography.emptyMountDescription.copyWith(
            color: colors.textSecondary,
          ),
          textAlign: TextAlign.center,
        ),
        SizedBox(height: metrics.mountPanelContentSpacing),
        MateButton(
          label: '选择目录',
          icon: 'folder-open',
          onClick: notifier.onSelectDir,
        ),
      ],
    );
  }
}

/// 已配置态内容：成功徽章 + 路径 chip + 更换目录按钮。
class _ConfiguredContent extends StatelessWidget {
  final String mountDir;
  final SettingsController notifier;

  const _ConfiguredContent({required this.mountDir, required this.notifier});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Column(
      children: [
        Container(
          width: metrics.configuredMountBadgeSize,
          height: metrics.configuredMountBadgeSize,
          decoration: BoxDecoration(
            color: colors.successBg,
            borderRadius:
                BorderRadius.circular(metrics.configuredMountBadgeRadius),
          ),
          alignment: Alignment.center,
          child: MateIcon(
            name: 'check',
            size: metrics.configuredMountIconSize,
            tint: colors.success,
          ),
        ),
        SizedBox(height: metrics.mountPanelContentSpacing),
        Text('当前同步目录', style: typography.currentMountTitle),
        SizedBox(height: metrics.mountPanelContentSpacing),
        // 路径 chip（bgFill 底 + radius 12，最多 2 行省略）
        Container(
          padding: EdgeInsets.symmetric(
            horizontal: metrics.mountPathHorizontalPadding,
            vertical: metrics.mountPathVerticalPadding,
          ),
          decoration: BoxDecoration(
            color: colors.bgFill,
            borderRadius: BorderRadius.circular(metrics.mountPathRadius),
          ),
          child: Text(
            mountDir,
            style: typography.currentMountPath.copyWith(
              color: colors.textSecondary,
            ),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
          ),
        ),
        SizedBox(height: metrics.mountPanelContentSpacing),
        MateButton(
          label: '更换目录',
          variant: MateButtonVariant.soft,
          icon: 'folder-open',
          onClick: notifier.onSelectDir,
        ),
      ],
    );
  }
}
