import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

/// 同步目录配置阶段（对标 CMP SetupPhase，ViewModels.kt）
enum SetupPhase {
  /// 配置加载中
  loading,

  /// 尚未配置同步目录
  needsSetup,

  /// 已配置但从未成功同步
  needsFirstSync,

  /// 正常同步中
  active,
}

/// 首次同步引导条（对标 CMP SyncSetupBanner.kt；原 Vue SyncSetupBanner.vue）。
///
/// 三态互斥（error 优先 > needsSetup > needsFirstSync）：
/// - error：MateInfoBanner error + 重试按钮
/// - needsSetup：MateInfoBanner info + 选择目录按钮
/// - needsFirstSync：MateInfoBanner warning + 同步索引按钮
///
/// 视觉来源 v2（design/03-sync-states.html 引导条）：容器 padding 8/20，
/// 底部分隔线保留；banner 本体为 MateInfoBanner v2 新样式
/// （radius 10 无描边、墨色正文 + 着色图标），action 为 MateButton text。
class FilesSyncSetupBanner extends StatelessWidget {
  /// 同步目录配置阶段
  final SetupPhase setupPhase;

  /// 当前挂载目录（needsFirstSync 文案展示）
  final String mountDir;

  /// 错误消息（非空时优先 error 态）
  final String? errorMessage;

  /// 选择目录回调
  final VoidCallback onSelectDir;

  /// 触发首次同步回调
  final VoidCallback onFirstSync;

  /// 重试回调
  final VoidCallback onRetry;

  const FilesSyncSetupBanner({
    super.key,
    required this.setupPhase,
    required this.mountDir,
    required this.onSelectDir,
    required this.onFirstSync,
    required this.onRetry,
    this.errorMessage,
  });

  @override
  Widget build(BuildContext context) {
    if (errorMessage != null && errorMessage!.isNotEmpty) {
      return _BannerWrapper(
        child: MateInfoBanner(
          message: errorMessage!,
          variant: MateBannerVariant.error,
          action: MateButton(
            label: '重试',
            variant: MateButtonVariant.text,
            icon: 'refresh',
            onClick: onRetry,
          ),
        ),
      );
    }
    switch (setupPhase) {
      case SetupPhase.needsSetup:
        return _BannerWrapper(
          child: MateInfoBanner(
            message: '尚未配置同步目录，选择一个空目录开始同步',
            variant: MateBannerVariant.info,
            action: MateButton(
              label: '选择目录',
              variant: MateButtonVariant.text,
              icon: 'folder-open',
              onClick: onSelectDir,
            ),
          ),
        );
      case SetupPhase.needsFirstSync:
        return _BannerWrapper(
          child: MateInfoBanner(
            message:
                '同步目录已就绪：${mountDir.isEmpty ? "未配置" : mountDir}，点击「同步索引」拉取云端索引',
            variant: MateBannerVariant.warning,
            action: MateButton(
              label: '同步索引',
              variant: MateButtonVariant.text,
              icon: 'sync',
              onClick: onFirstSync,
            ),
          ),
        );
      // active / loading 不显示引导条
      case SetupPhase.active:
      case SetupPhase.loading:
        return const SizedBox.shrink();
    }
  }
}

/// 引导条容器：padding 8/20（v2），底部分隔线保留。
class _BannerWrapper extends StatelessWidget {
  final Widget child;

  const _BannerWrapper({required this.child});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).syncSetup;

    return Column(
      children: [
        Container(
          width: double.infinity,
          color: colors.bgContainer,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.horizontalPadding,
            vertical: metrics.verticalPadding,
          ),
          child: child,
        ),
        // 底分隔线（v2 保留 banner 区与下方内容的分隔）
        const MateHDivider(),
      ],
    );
  }
}
