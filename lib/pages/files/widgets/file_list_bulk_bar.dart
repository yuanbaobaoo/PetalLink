import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

/// 批量操作栏（v2 05-file-ops：深色浮动条 h44 radius10，
/// margin 10/24/0 叠加 file-table 容器 padding 12 → 水平内缩 24）。
///
/// 内容：已选 N 项 + 批量下载 / 释放空间 / 批量删除（mountConfigured 时）+ 关闭。
class FileListBulkBar extends StatelessWidget {
  /// 已选数量
  final int checkedCount;

  /// 挂载目录是否已配置（控制批量删除显隐）
  final bool mountConfigured;

  /// 是否忙碌（索引中禁用 批量下载/批量删除）
  final bool busy;

  /// 批量下载
  final VoidCallback onDownload;

  /// 释放空间
  final VoidCallback onFreeUp;

  /// 批量删除
  final VoidCallback onDelete;

  /// 关闭批量条（清空选择并退出多选）
  final VoidCallback onClose;

  const FileListBulkBar({
    super.key,
    required this.checkedCount,
    required this.mountConfigured,
    required this.busy,
    required this.onDownload,
    required this.onFreeUp,
    required this.onDelete,
    required this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;

    return Container(
      margin: EdgeInsets.only(
        left: controls.bulkBarHorizontalMargin,
        top: controls.bulkBarTopMargin,
        right: controls.bulkBarHorizontalMargin,
      ),
      height: controls.bulkBarHeight,
      padding: EdgeInsets.only(
        left: controls.bulkBarStartPadding,
        right: controls.bulkBarEndPadding,
      ),
      decoration: BoxDecoration(
        color: colors.fileListBulkBackground,
        borderRadius: BorderRadius.circular(controls.bulkBarRadius),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Text(
            '已选 $checkedCount 项',
            style: typography.fileList.selectionSummary.copyWith(
              color: colors.fileListBulkSelectionText,
            ),
          ),
          const Spacer(),
          _BulkBarButton(
            label: '批量下载',
            icon: 'download',
            disabled: busy,
            onClick: onDownload,
          ),
          SizedBox(width: controls.bulkBarContentSpacing),
          _BulkBarButton(
            label: '释放空间',
            icon: 'cloud',
            onClick: onFreeUp,
          ),
          if (mountConfigured) ...[
            SizedBox(width: controls.bulkBarContentSpacing),
            _BulkBarButton(
              label: '批量删除',
              icon: 'trash',
              danger: true,
              disabled: busy,
              onClick: onDelete,
            ),
          ],
          SizedBox(width: controls.bulkBarContentSpacing),
          _BulkBarCloseButton(onClick: onClose),
        ],
      ),
    );
  }
}

/// 批量栏文字按钮（v2 .bulk-bar .btn-ghost：h32 radius8，白字 85% + 图标 70%，
/// hover 白 12% 底；danger 文字/图标转 danger 色、hover 红底；disabled 整体降透明）。
class _BulkBarButton extends StatefulWidget {
  final String label;
  final String icon;
  final bool danger;
  final bool disabled;
  final VoidCallback onClick;

  const _BulkBarButton({
    required this.label,
    required this.icon,
    this.danger = false,
    this.disabled = false,
    required this.onClick,
  });

  @override
  State<_BulkBarButton> createState() => _BulkBarButtonState();
}

class _BulkBarButtonState extends State<_BulkBarButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;

    final contentColor = widget.danger
        ? colors.fileListBulkDangerText
        : colors.fileListBulkActionText;
    final iconColor = widget.danger
        ? colors.fileListBulkDangerIcon
        : colors.fileListBulkActionIcon;
    final bg = widget.disabled
        ? Colors.transparent
        : _hovered
            ? (widget.danger
                ? colors.fileListBulkDangerHoverBackground
                : colors.fileListBulkActionHoverBackground)
            : Colors.transparent;

    return MouseRegion(
      cursor:
          widget.disabled ? SystemMouseCursors.basic : SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.disabled ? null : widget.onClick,
        child: Opacity(
          opacity: widget.disabled ? controls.bulkActionDisabledAlpha : 1,
          child: Container(
            height: controls.bulkActionHeight,
            padding: EdgeInsets.symmetric(
              horizontal: controls.bulkActionHorizontalPadding,
            ),
            decoration: BoxDecoration(
              color: bg,
              borderRadius: BorderRadius.circular(controls.bulkActionRadius),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                MateIcon(
                  name: widget.icon,
                  size: controls.bulkActionIconSize,
                  tint: iconColor,
                ),
                SizedBox(width: controls.bulkActionContentSpacing),
                Text(
                  widget.label,
                  style: typography.fileList.toolbarAction.copyWith(
                    color: contentColor,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 批量栏关闭按钮（v2 .bulk-bar .btn-circle：32×32 正圆，hover 白 12% 底 + 纯白图标）。
class _BulkBarCloseButton extends StatefulWidget {
  final VoidCallback onClick;

  const _BulkBarCloseButton({required this.onClick});

  @override
  State<_BulkBarCloseButton> createState() => _BulkBarCloseButtonState();
}

class _BulkBarCloseButtonState extends State<_BulkBarCloseButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;

    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onClick,
        child: Container(
          width: controls.bulkCloseSize,
          height: controls.bulkCloseSize,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: _hovered
                ? colors.fileListBulkCloseHoverBackground
                : Colors.transparent,
          ),
          alignment: Alignment.center,
          child: MateIcon(
            name: 'x',
            size: controls.bulkCloseIconSize,
            tint: _hovered
                ? colors.fileListBulkCloseHoverIcon
                : colors.fileListBulkCloseIcon,
          ),
        ),
      ),
    );
  }
}
