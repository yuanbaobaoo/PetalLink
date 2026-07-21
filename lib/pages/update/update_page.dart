import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/service/update/update_service.dart';
import 'package:petal_link/widgets/index.dart';

/// 更新对话框（对标 CMP UpdateDialogScreen.kt / design/v2/08-update.html）。
///
/// 自绘 overlay（updateDialogScrim）+ 居中对话框（宽 440，radius 12，柔影），
/// 全局挂载在 App builder 顶层（覆盖所有页面，对齐 CMP Main.kt 顶层
/// `UpdateDialogScreen` 与原 Vue App.vue 顶层 `<UpdateDialog />`）。
///
/// 9 阶段状态机（[UpdatePhase]）：仅 available / downloading /
/// waitingTransfers / ready / failed 且 [UpdateUIState.dialogVisible] 时显示；
/// idle / checking / upToDate / downloaded（瞬态）不渲染。
class UpdatePage extends StatefulWidget {
  const UpdatePage({super.key});

  @override
  State<UpdatePage> createState() => _UpdatePageState();
}

class _UpdatePageState extends State<UpdatePage> {
  /// 全局更新控制器（由 GlobalBinding 注册为 permanent，页面不持有其生命周期）
  late final UpdateController notifier = Get.find<UpdateController>();

  /// 可见阶段集合（对标 CMP visible computed）
  static const _visiblePhases = {
    UpdatePhase.available,
    UpdatePhase.downloading,
    UpdatePhase.waitingTransfers,
    UpdatePhase.ready,
    UpdatePhase.failed,
  };

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);

    return Obx(() {
      final state = notifier.state.value;
      final visible = state.dialogVisible && _visiblePhases.contains(state.phase);
      if (!visible) return const SizedBox.shrink();

      // overlay（fixed inset 0，bg updateDialogScrim，点击遮罩关闭）
      return GestureDetector(
        onTap: notifier.dismiss,
        child: Container(
          color: colors.updateDialogScrim,
          alignment: Alignment.center,
          child: GestureDetector(
            onTap: () {}, // 吞掉点击，避免穿透到遮罩关闭
            child: _UpdateDialog(state: state, notifier: notifier),
          ),
        ),
      );
    });
  }
}

/// 更新对话框主体：header（图标徽章 + 标题）+ 版本号 + body + footer。
class _UpdateDialog extends StatelessWidget {
  final UpdateUIState state;
  final UpdateController notifier;

  const _UpdateDialog({required this.state, required this.notifier});

  static String _titleForPhase(UpdatePhase phase) {
    return switch (phase) {
      UpdatePhase.available => '发现新版本',
      UpdatePhase.downloading => '正在下载更新…',
      UpdatePhase.waitingTransfers => '下载完成',
      UpdatePhase.ready => '更新就绪',
      UpdatePhase.failed => '更新失败',
      _ => '',
    };
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).updateDialog;
    final typography = MateTheme.typographyOf(context).update;
    final phase = state.phase;
    final manifest = state.manifest;

    return Container(
      width: metrics.dialogWidth,
      decoration: BoxDecoration(
        color: colors.bgContainer,
        borderRadius: BorderRadius.circular(metrics.dialogRadius),
        boxShadow: [
          BoxShadow(
            // 柔影色派生自遮罩 token（与 MatePopupMenu 同一惯例）
            color: colors.overlayDialogScrim.withAlpha(40),
            blurRadius: metrics.dialogShadowElevation,
            offset: Offset(0, metrics.dialogShadowElevation / 2),
          ),
        ],
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // header：40×40 brandLighter 图标徽章 + 标题（参照 MateDialogHost）
          Padding(
            padding: EdgeInsets.only(
              left: metrics.headerHorizontalPadding,
              right: metrics.headerHorizontalPadding,
              top: metrics.headerTopPadding,
              bottom: metrics.headerBottomPadding,
            ),
            child: Row(
              children: [
                Container(
                  width: metrics.headerBadgeSize,
                  height: metrics.headerBadgeSize,
                  decoration: BoxDecoration(
                    color: colors.brandLighter,
                    borderRadius:
                        BorderRadius.circular(metrics.headerBadgeRadius),
                  ),
                  alignment: Alignment.center,
                  child: MateIcon(
                    name: 'download',
                    size: metrics.headerIconSize,
                    tint: colors.brand,
                  ),
                ),
                SizedBox(width: metrics.headerContentSpacing),
                Flexible(
                  child: Text(
                    _titleForPhase(phase),
                    style: typography.dialogTitle.copyWith(
                      color: colors.textPrimary,
                    ),
                  ),
                ),
              ],
            ),
          ),

          // 版本号（available 态显示）
          if (phase == UpdatePhase.available && manifest != null)
            Padding(
              padding: EdgeInsets.symmetric(
                horizontal: metrics.versionHorizontalPadding,
              ),
              child: Text(
                'v${manifest.version}',
                style: typography.version.copyWith(color: colors.brand),
              ),
            ),

          // body
          Padding(
            padding: EdgeInsets.only(
              left: metrics.bodyHorizontalPadding,
              right: metrics.bodyHorizontalPadding,
              top: metrics.bodyTopPadding,
              bottom: metrics.bodyBottomPadding,
            ),
            child: SizedBox(
              width: double.infinity,
              child: _UpdateBody(state: state),
            ),
          ),

          // footer（按钮组按阶段切换）
          Padding(
            padding: EdgeInsets.only(
              left: metrics.footerHorizontalPadding,
              right: metrics.footerHorizontalPadding,
              top: metrics.footerTopPadding,
              bottom: metrics.footerBottomPadding,
            ),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                _UpdateFooter(state: state, notifier: notifier),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// body 内容（按阶段切换）。
class _UpdateBody extends StatelessWidget {
  final UpdateUIState state;

  const _UpdateBody({required this.state});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).update;

    return switch (state.phase) {
      UpdatePhase.available => _AvailableBody(manifest: state.manifest),
      UpdatePhase.downloading =>
        _DownloadingBody(progress: state.downloadProgress),
      UpdatePhase.waitingTransfers || UpdatePhase.ready => _WaitingBody(
          phase: state.phase,
          hasActiveTransfers: state.hasActiveTransfers,
        ),
      UpdatePhase.failed => Text(
          state.errorMessage ?? '更新失败，请稍后重试。',
          style: typography.failureMessage.copyWith(color: colors.error),
        ),
      _ => const SizedBox.shrink(),
    };
  }
}

/// available 态正文：更新日志块 / 无日志提示（v2：notes 块 radius 8 + bgPage）。
class _AvailableBody extends StatelessWidget {
  final UpdateManifest? manifest;

  const _AvailableBody({required this.manifest});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).updateDialog;
    final typography = MateTheme.typographyOf(context).update;

    final notes = manifest?.notes;
    if (notes != null && notes.trim().isNotEmpty) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            '更新内容',
            style: typography.releaseNotesLabel.copyWith(
              color: colors.textSecondary,
            ),
          ),
          SizedBox(height: metrics.releaseNotesLabelSpacing),
          // notes 文本块：bg-page，radius 8，padding 12，max-height 180，scroll
          Container(
            width: double.infinity,
            constraints: BoxConstraints(
              maxHeight: metrics.releaseNotesMaximumHeight,
            ),
            padding: EdgeInsets.all(metrics.releaseNotesPadding),
            decoration: BoxDecoration(
              color: colors.bgPage,
              borderRadius:
                  BorderRadius.circular(metrics.releaseNotesRadius),
            ),
            child: SingleChildScrollView(
              child: Text(
                notes,
                style: typography.releaseNotesBody.copyWith(
                  color: colors.textSecondary,
                ),
              ),
            ),
          ),
        ],
      );
    }

    return Text(
      '暂无更新日志。是否下载并安装此更新？',
      style: typography.noReleaseNotesMessage.copyWith(
        color: colors.textSecondary,
      ),
    );
  }
}

/// downloading 态正文：进度条（品牌渐变 fill）+ 百分比。
class _DownloadingBody extends StatelessWidget {
  final double progress;

  const _DownloadingBody({required this.progress});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).updateDialog;
    final typography = MateTheme.typographyOf(context).update;
    final pct = (progress * 100).toInt();

    return Row(
      children: [
        // 进度条轨道（h8，bg-page，radius 4）
        Expanded(
          child: ClipRRect(
            borderRadius: BorderRadius.circular(metrics.progressTrackRadius),
            child: Container(
              height: metrics.progressTrackHeight,
              color: colors.bgPage,
              // fill（brandGradient 品牌渐变，width = progress）
              child: FractionallySizedBox(
                alignment: Alignment.centerLeft,
                widthFactor: progress.clamp(0.0, 1.0),
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    gradient: LinearGradient(colors: colors.brandGradient),
                    borderRadius:
                        BorderRadius.circular(metrics.progressFillRadius),
                  ),
                ),
              ),
            ),
          ),
        ),
        SizedBox(width: metrics.progressContentSpacing),
        Text(
          '$pct%',
          style: typography.progress.copyWith(color: colors.textPrimary),
        ),
      ],
    );
  }
}

/// waitingTransfers / ready 态正文：spinner + 提示。
class _WaitingBody extends StatelessWidget {
  final UpdatePhase phase;
  final bool hasActiveTransfers;

  const _WaitingBody({required this.phase, required this.hasActiveTransfers});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).updateDialog;
    final typography = MateTheme.typographyOf(context).update;

    final text = switch (phase) {
      UpdatePhase.ready => '更新已准备就绪，重启即可生效。',
      UpdatePhase.waitingTransfers when hasActiveTransfers =>
        '下载完成。等待所有文档上传/下载完成后自动重启…',
      _ => '准备安装…',
    };

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: EdgeInsets.only(top: metrics.spinnerTopPadding),
          child: MateCircularProgress(
            size: metrics.spinnerRingSize,
            strokeWidth: metrics.spinnerRingStrokeWidth,
          ),
        ),
        SizedBox(width: metrics.waitingContentSpacing),
        Expanded(
          child: Text(
            text,
            style: typography.waitingMessage.copyWith(
              color: colors.textSecondary,
            ),
          ),
        ),
      ],
    );
  }
}

/// footer 按钮组合（v2：次要按钮 iconText 幽灵灰，主按钮 primary 渐变+柔影）。
class _UpdateFooter extends StatelessWidget {
  final UpdateUIState state;
  final UpdateController notifier;

  const _UpdateFooter({required this.state, required this.notifier});

  @override
  Widget build(BuildContext context) {
    final metrics = MateTheme.metricsOf(context).updateDialog;
    final gap = SizedBox(width: metrics.footerActionSpacing);

    return switch (state.phase) {
      UpdatePhase.available => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            MateButton(
              label: '稍后提醒',
              variant: MateButtonVariant.iconText,
              onClick: notifier.dismiss,
            ),
            gap,
            MateButton(
              label: '立即更新',
              icon: 'download',
              onClick: notifier.downloadAndInstall,
            ),
          ],
        ),
      UpdatePhase.downloading => MateButton(
          label: '后台下载',
          variant: MateButtonVariant.iconText,
          onClick: notifier.dismiss,
        ),
      UpdatePhase.ready => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            MateButton(
              label: '稍后',
              variant: MateButtonVariant.iconText,
              onClick: notifier.dismiss,
            ),
            gap,
            MateButton(
              label: '立即重启',
              icon: 'check',
              onClick: notifier.relaunch,
            ),
          ],
        ),
      UpdatePhase.waitingTransfers => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            MateButton(
              label: '后台等待',
              variant: MateButtonVariant.iconText,
              onClick: notifier.dismiss,
            ),
            gap,
            MateButton(
              label: state.hasActiveTransfers ? '等待传输完成…' : '立即重启',
              icon: 'check',
              onClick: notifier.relaunch,
              disabled: state.hasActiveTransfers,
            ),
          ],
        ),
      UpdatePhase.failed => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            MateButton(
              label: '关闭',
              variant: MateButtonVariant.iconText,
              onClick: notifier.dismiss,
            ),
            gap,
            MateButton(
              label: '重试',
              icon: 'refresh',
              onClick: notifier.downloadAndInstall,
            ),
          ],
        ),
      _ => const SizedBox.shrink(),
    };
  }
}
