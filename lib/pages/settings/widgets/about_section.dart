import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:url_launcher/url_launcher.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 关于分区（对标 CMP SettingsScreen.kt AboutSection）。
///
/// LogoWithText + 版本/检查更新 + 可用更新操作（安装/查看更新日志）
/// + 下载进度条（点击重开更新弹窗）+ 简介 + GitHub/GitCode 外链。
class AboutSection extends StatelessWidget {
  /// 页面控制器
  final SettingsController notifier;

  /// 当前状态
  final SettingsState state;

  const AboutSection({
    super.key,
    required this.notifier,
    required this.state,
  });

  /// 更新状态文案（对标 CMP ApplicationRoot 的 updateStatus 计算）
  static String _updateStatus(UpdateUIState update) {
    return switch (update.phase) {
      UpdatePhase.checking => '正在检查更新…',
      UpdatePhase.upToDate => '已是最新版本',
      UpdatePhase.available => '发现新版本 ${update.manifest?.version ?? ''}',
      UpdatePhase.waitingTransfers => '等待传输完成…',
      UpdatePhase.failed => '检查更新失败',
      _ => '',
    };
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;
    final updater = Get.find<UpdateController>();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '关于', icon: 'cloud'),
        SettingsPanel(
          contentPadding: EdgeInsets.all(metrics.aboutPanelPadding),
          contentSpacing: metrics.aboutPanelContentSpacing,
          children: [
            MateLogoWithText(height: metrics.aboutLogoHeight),

            // 版本 + 检查更新 + 状态文案
            Obx(() {
              final update = updater.state.value;
              final checking = update.phase == UpdatePhase.checking;
              final status = _updateStatus(update);
              return Row(
                children: [
                  Text(
                    '版本 ${state.appVersion.isEmpty ? '...' : state.appVersion}',
                    style: typography.version.copyWith(
                      color: colors.textSecondary,
                    ),
                  ),
                  SizedBox(width: metrics.versionContentSpacing),
                  MateButton(
                    label: checking ? '检查中…' : '检查更新',
                    variant: MateButtonVariant.text,
                    icon: 'refresh',
                    onClick: notifier.onCheckUpdate,
                    disabled: checking,
                  ),
                  if (status.isNotEmpty) ...[
                    SizedBox(width: metrics.versionContentSpacing),
                    Flexible(
                      child: Text(
                        status,
                        style: typography.updateStatus.copyWith(
                          color: colors.textSecondary,
                        ),
                      ),
                    ),
                  ],
                ],
              );
            }),

            // 可用更新：安装 + 查看更新日志
            Obx(() {
              final update = updater.state.value;
              final manifest = update.manifest;
              if (manifest == null) return const SizedBox.shrink();
              return Row(
                children: [
                  MateButton(
                    label: '安装 ${manifest.version}',
                    icon: 'download',
                    onClick: notifier.onInstallUpdate,
                  ),
                  SizedBox(width: metrics.versionContentSpacing),
                  MateButton(
                    label: '查看更新日志',
                    variant: MateButtonVariant.text,
                    icon: 'info',
                    onClick: notifier.onShowUpdate,
                  ),
                ],
              );
            }),

            // 下载中：可点击进度条（点击重开更新弹窗）
            Obx(() {
              final update = updater.state.value;
              if (update.phase != UpdatePhase.downloading) {
                return const SizedBox.shrink();
              }
              return GestureDetector(
                onTap: notifier.onShowUpdate,
                child: ClipRRect(
                  borderRadius:
                      BorderRadius.circular(metrics.mountPanelRadius),
                  child: MateLinearProgress(
                    value: update.downloadProgress,
                  ),
                ),
              );
            }),

            Text(
              '一款开源免费的华为云盘客户端',
              style: typography.aboutDescription.copyWith(
                color: colors.textSecondary,
              ),
            ),

            // GitHub / GitCode 外链
            Row(
              children: [
                const _LinkItem(
                  label: 'GitHub',
                  icon: 'github',
                  url: 'https://github.com/yuanbaobaoo/PetalLink',
                ),
                SizedBox(width: metrics.externalLinksSpacing),
                const _LinkItem(
                  label: 'GitCode',
                  icon: 'gitcode',
                  url: 'https://gitcode.com/yuanbaobaoo/PetalLink',
                ),
              ],
            ),
          ],
        ),
      ],
    );
  }
}

/// 外链项（brand 色，点击打开浏览器；对标 CMP LinkItem）。
class _LinkItem extends StatelessWidget {
  final String label;
  final String icon;
  final String url;

  const _LinkItem({required this.label, required this.icon, required this.url});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        onTap: () => _open(url),
        child: Padding(
          padding: EdgeInsets.symmetric(
            vertical: metrics.externalLinkVerticalPadding,
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              MateIcon(
                name: icon,
                size: metrics.externalLinkIconSize,
                tint: colors.brand,
              ),
              SizedBox(width: metrics.externalLinkContentSpacing),
              Text(
                label,
                style: typography.externalLink.copyWith(color: colors.brand),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// 打开外部浏览器（失败静默，对齐 CMP runCatching browse）
  static Future<void> _open(String url) async {
    try {
      await launchUrl(Uri.parse(url), mode: LaunchMode.externalApplication);
    } catch (_) {
      // 无法打开浏览器时静默（对齐 CMP runCatching）
    }
  }
}
