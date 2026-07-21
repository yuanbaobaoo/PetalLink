import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/pages/files/widgets/file_format.dart';
import 'package:petal_link/pages/files/widgets/files_menu.dart';
import 'package:petal_link/types/enums.dart';
import 'package:petal_link/widgets/index.dart';

/// 传输状态元信息（对标原 Vue stateMeta）
typedef _StateMeta = ({String icon, String label, Color color, bool spin});

/// 传输队列弹窗（v2 视觉，对标 CMP TransferPopoverScreen.kt、
/// design/04-transfer.html，原 Vue TransferPopover.vue）。
///
/// 440×580，radius-xl(12)，shadow-modal，border 0.5px；贴 AppBar 下右侧
/// （top 64 / end 20）；header(60) + stats(stat-pill 卡片行) +
/// body(flex scroll，顶分隔线)；任务行 minHeight 68 padding 10/20：
/// 方向色块(36×36 radius8) + 信息区(dir chip + name + 进度/错误) +
/// 状态区(80) + 重试按钮。
class TransferPopover extends StatefulWidget {
  /// 传输任务列表
  final List<TransferTask> tasks;

  /// 关闭回调
  final VoidCallback onDismiss;

  /// 重试回调（传 taskId 与结果回调，用于防抖与 toast 反馈）
  final void Function(int taskId, void Function(bool ok) onResult) onRetry;

  /// 清除已完成
  final VoidCallback onClearCompleted;

  /// 清除失败历史
  final VoidCallback onClearFailed;

  /// 清除完成+失败
  final VoidCallback onClearFinished;

  const TransferPopover({
    super.key,
    required this.tasks,
    required this.onDismiss,
    required this.onRetry,
    required this.onClearCompleted,
    required this.onClearFailed,
    required this.onClearFinished,
  });

  @override
  State<TransferPopover> createState() => _TransferPopoverState();
}

class _TransferPopoverState extends State<TransferPopover> {
  /// 重试防抖：单任务重试期间禁用重复点击（对标原 Vue retryingId）
  int? _retryingId;

  void _requestRetry(int id) {
    if (_retryingId != null) return;
    setState(() => _retryingId = id);
    widget.onRetry(id, (ok) {
      if (!mounted) return;
      setState(() => _retryingId = null);
      if (ok) {
        MateToast.show('已重新提交传输任务', variant: MateToastVariant.success);
      } else {
        MateToast.show('重试失败，请稍后再试', variant: MateToastVariant.error);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).transferPopover;
    final tasks = widget.tasks;

    final processing = tasks.where((t) {
      return t.state == TransferState.running ||
          t.state == TransferState.verifyingRemote ||
          t.state == TransferState.pending;
    }).length;
    final waiting = tasks.where((t) {
      return t.state == TransferState.waitingForNetwork ||
          t.state == TransferState.backingOff ||
          t.state == TransferState.restartRequired;
    }).length;
    final completed =
        tasks.where((t) => t.state == TransferState.completed).length;
    final failed = tasks.where((t) => t.state == TransferState.failed).length;

    return Align(
      alignment: Alignment.topRight,
      child: Container(
        width: metrics.panelWidth,
        height: metrics.panelHeight,
        margin: EdgeInsets.only(
          top: metrics.panelTopOffset,
          right: metrics.panelEndOffset,
        ),
        decoration: BoxDecoration(
          color: colors.bgContainer,
          borderRadius: BorderRadius.circular(metrics.panelRadius),
          border: Border.all(
            color: colors.border,
            width: metrics.panelBorderWidth,
          ),
          boxShadow: [
            BoxShadow(
              color: colors.overlayDialogScrim.withAlpha((0.18 * 255).round()),
              blurRadius: metrics.panelShadowElevation,
              offset: const Offset(0, 6),
            ),
          ],
        ),
        child: Column(
          children: [
            // header 60（v2：transfer 图标 18 brand + 标题 18 semibold + ICON 关闭）
            SizedBox(
              height: metrics.headerHeight,
              child: Padding(
                padding: EdgeInsets.only(
                  left: metrics.headerStartPadding,
                  right: metrics.headerEndPadding,
                ),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    MateIcon(
                      name: 'transfer',
                      size: metrics.headerIconSize,
                      tint: colors.brand,
                    ),
                    SizedBox(width: metrics.headerContentSpacing),
                    Expanded(
                      child: Text(
                        '传输队列',
                        style: typography.transfer.panelTitle.copyWith(
                          color: colors.textPrimary,
                        ),
                      ),
                    ),
                    MateButton(
                      variant: MateButtonVariant.icon,
                      icon: 'x',
                      onClick: widget.onDismiss,
                    ),
                  ],
                ),
              ),
            ),
            // stats（v2：stat-pill 卡片行，padding 0/20/14，gap 8；右侧清空菜单）
            Padding(
              padding: EdgeInsets.only(
                left: metrics.summaryHorizontalPadding,
                right: metrics.summaryHorizontalPadding,
                bottom: metrics.summaryBottomPadding,
              ),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  _StatPill(num: processing, label: '处理中'),
                  SizedBox(width: metrics.summaryItemSpacing),
                  _StatPill(num: waiting, label: '等待中'),
                  SizedBox(width: metrics.summaryItemSpacing),
                  _StatPill(num: completed, label: '已完成'),
                  if (failed > 0) ...[
                    SizedBox(width: metrics.summaryItemSpacing),
                    _StatPill(num: failed, label: '历史失败', error: true),
                  ],
                  SizedBox(width: metrics.summaryItemSpacing),
                  // 清空菜单（自绘菜单右下锚定：贴窗口右边缘不溢出，
                  // 对齐 CMP MatePopupMenu 三项与图标）
                  Builder(
                    builder: (buttonContext) => MateButton(
                      variant: MateButtonVariant.icon,
                      icon: 'transfer',
                      onClick: () {
                        final box = buttonContext.findRenderObject()
                            as RenderBox;
                        final anchor = box.localToGlobal(
                            Offset(box.size.width, box.size.height));
                        showFilesMenu(
                          buttonContext,
                          anchorBottomRight: anchor,
                          entriesBuilder: () => [
                            FilesMenuEntry(
                              label: '清除已完成',
                              icon: 'check',
                              action: widget.onClearCompleted,
                            ),
                            FilesMenuEntry(
                              label: '清除失败历史',
                              icon: 'x',
                              danger: true,
                              action: widget.onClearFailed,
                            ),
                            FilesMenuEntry(
                              label: '清除完成+失败历史',
                              icon: 'transfer',
                              action: widget.onClearFinished,
                            ),
                          ],
                        );
                      },
                    ),
                  ),
                ],
              ),
            ),
            // body（v2：列表区顶部分隔线）
            const MateHDivider(),
            Expanded(
              child: tasks.isEmpty
                  ? const MateEmpty(title: '暂无传输任务', icon: 'cloud')
                  : ListView.builder(
                      itemCount: tasks.length,
                      itemBuilder: (context, index) {
                        final task = tasks[index];
                        return _TransferTaskRow(
                          key: ValueKey(task.id),
                          task: task,
                          retrying: _retryingId == task.id,
                          onRetry: _requestRetry,
                        );
                      },
                    ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 9 态 stateMeta 映射（对标原 Vue TransferPopover stateMeta，逻辑与文案不变）。
///
/// v2：中性灰由硬编码改为语义色（[neutral] 传 textSecondary，浅色下与原值一致）。
_StateMeta _stateMeta(TransferState state, MateSemanticColors colors) {
  return switch (state) {
    TransferState.pending =>
      (icon: 'clock', label: '等待调度', color: colors.textSecondary, spin: false),
    TransferState.running =>
      (icon: 'sync', label: '传输中', color: colors.brand, spin: true),
    TransferState.waitingForNetwork =>
      (icon: 'clock', label: '等待网络', color: colors.warning, spin: false),
    TransferState.backingOff =>
      (icon: 'clock', label: '等待重试', color: colors.warning, spin: false),
    TransferState.verifyingRemote =>
      (icon: 'sync', label: '核验远端', color: colors.brand, spin: true),
    TransferState.restartRequired =>
      (icon: 'refresh', label: '等待重新规划', color: colors.warning, spin: false),
    TransferState.completed =>
      (icon: 'check', label: '已完成', color: colors.success, spin: false),
    TransferState.failed =>
      (icon: 'x', label: '失败', color: colors.error, spin: false),
    TransferState.canceled =>
      (icon: 'x', label: '已取消', color: colors.textSecondary, spin: false),
  };
}

/// 方向图标（对标原 Vue dirIcon）
String _dirIcon(TransferDirection direction) {
  return switch (direction) {
    TransferDirection.download => 'download',
    TransferDirection.downloadUpdate => 'refresh',
    TransferDirection.delete => 'trash',
    _ => 'transfer',
  };
}

/// 方向标签（对标原 Vue DIR_LABEL）
String _dirLabel(TransferDirection direction) {
  return switch (direction) {
    TransferDirection.upload => '上传',
    TransferDirection.download => '下载',
    TransferDirection.downloadUpdate => '下载',
    TransferDirection.delete => '删除',
  };
}

/// 进度条颜色（对标原 Vue progressColor；等待类传 textPlaceholder 语义色）
Color _progressColor(TransferState state, MateSemanticColors colors) {
  return switch (state) {
    TransferState.completed => colors.success,
    TransferState.failed => colors.error,
    TransferState.pending ||
    TransferState.waitingForNetwork ||
    TransferState.backingOff ||
    TransferState.restartRequired =>
      colors.textPlaceholder,
    _ => colors.brand,
  };
}

/// 是否可重试（对标原 Vue canRetryTransferTask：
/// Failed/RestartRequired + upload/download 方向，delete 不可）
bool _canRetry(TransferTask task) {
  final stateOk = task.state == TransferState.failed ||
      task.state == TransferState.restartRequired;
  final dirOk = task.direction == TransferDirection.upload ||
      task.direction.isDownload;
  return stateOk && dirOk;
}

/// stat-pill 统计卡片（v2：bgFill radius-md(8)，padding 8/10，上数字下标签）。
///
/// 数字 17 bold（tabular-nums），标签 12 textSecondary；
/// error 变体（历史失败）：errorBg 底 + error 数字。
class _StatPill extends StatelessWidget {
  final int num;
  final String label;
  final bool error;

  const _StatPill({required this.num, required this.label, this.error = false});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).transferPopover;

    return Expanded(
      child: Container(
        padding: EdgeInsets.symmetric(
          horizontal: metrics.summaryHorizontalContentPadding,
          vertical: metrics.summaryVerticalContentPadding,
        ),
        decoration: BoxDecoration(
          color: error ? colors.errorBg : colors.bgFill,
          borderRadius: BorderRadius.circular(metrics.summaryRadius),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '$num',
              style: typography.transfer.summaryValue.copyWith(
                color: error ? colors.error : colors.textPrimary,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
            SizedBox(height: metrics.summaryTextSpacing),
            Text(
              label,
              style: typography.transfer.summaryLabel.copyWith(
                color: colors.textSecondary,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 单个传输任务行（v2：minHeight 68，padding 10/20，含底分隔线）。
class _TransferTaskRow extends StatelessWidget {
  final TransferTask task;
  final bool retrying;
  final void Function(int taskId) onRetry;

  const _TransferTaskRow({
    super.key,
    required this.task,
    required this.retrying,
    required this.onRetry,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).transferPopover;
    final meta = _stateMeta(task.state, colors);

    // v2 方向色块配色：上传 brandLighter/brand；下载 infoBg/info；删除 bgFill/textSecondary
    final (dirBg, dirFg) = switch (task.direction) {
      TransferDirection.download ||
      TransferDirection.downloadUpdate =>
        (colors.infoBg, colors.info),
      TransferDirection.delete => (colors.bgFill, colors.textSecondary),
      _ => (colors.brandLighter, colors.brand),
    };

    final showError = (task.state == TransferState.failed ||
            task.state == TransferState.restartRequired) &&
        task.errorMessage != null;
    final showProgress =
        task.direction != TransferDirection.delete && task.totalSize > 0;

    return Column(
      children: [
        Container(
          constraints: BoxConstraints(minHeight: metrics.taskMinimumHeight),
          padding: EdgeInsets.symmetric(
            horizontal: metrics.taskHorizontalPadding,
            vertical: metrics.taskVerticalPadding,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              // 方向色块（36×36 radius 8）
              Container(
                width: metrics.directionBadgeSize,
                height: metrics.directionBadgeSize,
                decoration: BoxDecoration(
                  color: dirBg,
                  borderRadius:
                      BorderRadius.circular(metrics.directionBadgeRadius),
                ),
                alignment: Alignment.center,
                child: MateIcon(
                  name: _dirIcon(task.direction),
                  size: metrics.directionIconSize,
                  tint: dirFg,
                ),
              ),
              SizedBox(width: metrics.taskContentSpacing),
              // 信息区（flex:1，v2 gap 5）
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    // 名称行（v2：dir 文字 chip MateTag small + 文件名 14.5 medium）
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.center,
                      children: [
                        MateTag(
                          label: _dirLabel(task.direction),
                          theme: task.direction == TransferDirection.upload
                              ? MateTagTheme.primary
                              : MateTagTheme.normal,
                          size: MateTagSize.small,
                        ),
                        SizedBox(width: metrics.taskNameSpacing),
                        Expanded(
                          child: Text(
                            task.name,
                            style: typography.transfer.taskName.copyWith(
                              color: colors.textPrimary,
                            ),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                      ],
                    ),
                    SizedBox(height: metrics.taskInfoSpacing),
                    // 第二行：错误 or 进度条（带字节） or 删除操作
                    if (showError)
                      Text(
                        task.errorMessage!,
                        style: typography.transfer.taskDescription.copyWith(
                          color: colors.error,
                        ),
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                      )
                    else if (showProgress)
                      // 进度条 + 百分比 + 已传/总字节（对标原 Vue tp-item__pct）
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.center,
                        children: [
                          Expanded(
                            child: MateLinearProgress(
                              value: task.progress,
                              color: _progressColor(task.state, colors),
                            ),
                          ),
                          SizedBox(width: metrics.taskProgressSpacing),
                          Text(
                            '${(task.progress * 100).toInt()}% · '
                            '${formatFileSize(task.transferred)}/${formatFileSize(task.totalSize)}',
                            style: typography.transfer.taskProgress.copyWith(
                              color: colors.textSecondary,
                              fontFeatures: const [
                                FontFeature.tabularFigures(),
                              ],
                            ),
                          ),
                        ],
                      )
                    else if (task.direction == TransferDirection.delete)
                      Text(
                        '删除操作',
                        style: typography.transfer.deleteOperation.copyWith(
                          color: colors.textSecondary,
                        ),
                      ),
                  ],
                ),
              ),
              SizedBox(width: metrics.taskContentSpacing),
              // 状态区（80px 右对齐，stateMeta 九态映射，13sp medium）
              SizedBox(
                width: metrics.taskStateWidth,
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    MateIcon(
                      name: meta.icon,
                      size: metrics.taskStateIconSize,
                      tint: meta.color,
                      spin: meta.spin,
                    ),
                    SizedBox(width: metrics.taskStateSpacing),
                    Flexible(
                      child: Text(
                        meta.label,
                        style: typography.transfer.taskState.copyWith(
                          color: meta.color,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ],
                ),
              ),
              // 重试按钮（条件；重试中显示进度指示并防抖）
              if (_canRetry(task)) ...[
                SizedBox(width: metrics.taskStateSpacing),
                if (retrying)
                  MateCircularProgress(size: metrics.taskStateIconSize)
                else
                  MateButton(
                    variant: MateButtonVariant.icon,
                    icon: 'refresh',
                    onClick: () => onRetry(task.id),
                  ),
              ],
            ],
          ),
        ),
        // item 底分隔线（对标 .transfer-item border-bottom）
        const MateHDivider(),
      ],
    );
  }
}
