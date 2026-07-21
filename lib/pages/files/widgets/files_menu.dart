import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

/// 文件页自绘菜单项（v2 .menu__item / .menu__sep）
class FilesMenuEntry {
  /// 是否为分隔线
  final bool divider;

  /// 项文字
  final String label;

  /// 图标 name
  final String icon;

  /// 危险态（红）
  final bool danger;

  /// 是否可用（false 时文字/图标 placeholder 且不响应点击与 hover）
  final bool enabled;

  /// 点击动作（菜单关闭后执行）
  final VoidCallback? action;

  const FilesMenuEntry({
    required this.label,
    required this.icon,
    this.action,
    this.danger = false,
    this.enabled = true,
  }) : divider = false;

  /// 分隔线项
  const FilesMenuEntry.divider()
      : divider = true,
        label = '',
        icon = '',
        danger = false,
        enabled = true,
        action = null;
}

/// 文件页菜单控制器（刷新条目 / 主动关闭）
class FilesMenuController {
  OverlayEntry? _entry;

  /// 条目内容变化后刷新菜单（如异步条件项到达）
  void refresh() => _entry?.markNeedsBuild();

  /// 关闭菜单
  void dismiss() {
    _entry?.remove();
    _entry = null;
  }

  /// 是否打开中
  bool get isOpen => _entry != null;
}

/// 弹出文件页自绘菜单（v2：对标 05-file-ops .menu —— w200 radius10 +
/// 阴影 + 0.5px 描边，padding 6；点击外部关闭）。
///
/// - [cursorPosition]：全局坐标，菜单左上角对齐光标（右键场景），自动视口钳制
/// - [anchorBottomRight]：全局坐标，菜单右上角对齐锚点右下（按钮场景，
///   解决贴右边缘时 MatePopupMenu 不做视口钳制导致的溢出）
/// - [entriesBuilder]：每次构建时取条目（支持异步条件项经
///   [FilesMenuController.refresh] 重建）
FilesMenuController showFilesMenu(
  BuildContext context, {
  Offset? cursorPosition,
  Offset? anchorBottomRight,
  required List<FilesMenuEntry> Function() entriesBuilder,
}) {
  final controller = FilesMenuController();
  final overlay = Overlay.of(context);
  final overlayBox = overlay.context.findRenderObject() as RenderBox;
  final controls = MateTheme.metricsOf(context).fileList.controls;

  // 计算菜单锚点（全局坐标 → overlay 坐标）
  final double anchorX;
  final double anchorY;
  if (cursorPosition != null) {
    final local = overlayBox.globalToLocal(cursorPosition);
    anchorX = local.dx;
    anchorY = local.dy;
  } else if (anchorBottomRight != null) {
    final local = overlayBox.globalToLocal(anchorBottomRight);
    anchorX = local.dx - controls.contextMenuWidth;
    anchorY = local.dy;
  } else {
    anchorX = 0;
    anchorY = 0;
  }

  controller._entry = OverlayEntry(
    builder: (menuContext) => _FilesMenuOverlay(
      controller: controller,
      anchorX: anchorX,
      anchorY: anchorY,
      entriesBuilder: entriesBuilder,
    ),
  );
  overlay.insert(controller._entry!);
  return controller;
}

/// 菜单浮层（屏障 + 钳制后的菜单容器）
class _FilesMenuOverlay extends StatelessWidget {
  final FilesMenuController controller;
  final double anchorX;
  final double anchorY;
  final List<FilesMenuEntry> Function() entriesBuilder;

  const _FilesMenuOverlay({
    required this.controller,
    required this.anchorX,
    required this.anchorY,
    required this.entriesBuilder,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final entries = entriesBuilder();

    // 视口钳制（对齐 CMP Popup 自动钳制）
    final overlaySize =
        (Overlay.of(context).context.findRenderObject() as RenderBox).size;
    final menuHeight = entries.fold<double>(
          0,
          (sum, e) =>
              sum +
              (e.divider
                  ? controls.contextDividerHeight +
                      controls.contextDividerVerticalPadding * 2
                  : controls.contextActionHeight),
        ) +
        controls.contextMenuPadding * 2;
    final left = anchorX.clamp(
      8.0,
      (overlaySize.width - controls.contextMenuWidth - 8)
          .clamp(8.0, double.infinity),
    );
    final top = anchorY.clamp(
      8.0,
      (overlaySize.height - menuHeight - 8).clamp(8.0, double.infinity),
    );

    return Stack(
      children: [
        // 透明屏障：点击外部关闭
        Positioned.fill(
          child: GestureDetector(
            onTap: controller.dismiss,
            onSecondaryTap: controller.dismiss,
            behavior: HitTestBehavior.opaque,
          ),
        ),
        Positioned(
          left: left,
          top: top,
          child: Container(
            width: controls.contextMenuWidth,
            padding: EdgeInsets.all(controls.contextMenuPadding),
            decoration: BoxDecoration(
              color: colors.bgContainer,
              borderRadius: BorderRadius.circular(controls.contextMenuRadius),
              border: Border.all(
                color: colors.border,
                width: controls.contextMenuBorderWidth,
              ),
              boxShadow: [
                BoxShadow(
                  color:
                      colors.overlayDialogScrim.withAlpha((0.18 * 255).round()),
                  blurRadius: controls.contextMenuShadowElevation,
                  offset: const Offset(0, 6),
                ),
              ],
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final entry in entries)
                  entry.divider
                      ? const _FilesMenuDivider()
                      : _FilesMenuItem(
                          entry: entry,
                          onClick: () {
                            controller.dismiss();
                            entry.action?.call();
                          },
                        ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// 菜单项（v2 .menu__item：h36 radius8，hover bgFill，padding 0 12，gap 10；
/// icon 16（默认 textSecondary，danger error），文字 15sp；
/// enabled=false 时文字/图标 textPlaceholder 且不响应点击与 hover）。
class _FilesMenuItem extends StatefulWidget {
  final FilesMenuEntry entry;
  final VoidCallback onClick;

  const _FilesMenuItem({required this.entry, required this.onClick});

  @override
  State<_FilesMenuItem> createState() => _FilesMenuItemState();
}

class _FilesMenuItemState extends State<_FilesMenuItem> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final entry = widget.entry;

    final contentColor = !entry.enabled
        ? colors.textPlaceholder
        : entry.danger
            ? colors.error
            : colors.textPrimary;
    final iconColor = !entry.enabled
        ? colors.textPlaceholder
        : entry.danger
            ? colors.error
            : colors.textSecondary;

    return MouseRegion(
      cursor: entry.enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: entry.enabled ? widget.onClick : null,
        behavior: HitTestBehavior.opaque,
        child: Container(
          height: controls.contextActionHeight,
          padding: EdgeInsets.symmetric(
            horizontal: controls.contextActionHorizontalPadding,
          ),
          decoration: BoxDecoration(
            color:
                _hovered && entry.enabled ? colors.bgFill : Colors.transparent,
            borderRadius: BorderRadius.circular(controls.contextActionRadius),
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              MateIcon(
                name: entry.icon,
                size: controls.contextActionIconSize,
                tint: iconColor,
              ),
              SizedBox(width: controls.contextActionContentSpacing),
              Expanded(
                child: Text(
                  entry.label,
                  style: typography.fileList.secondaryAction.copyWith(
                    color: contentColor,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 菜单分隔线（v2 .menu__sep：0.5px，margin 8/4，bg border）。
class _FilesMenuDivider extends StatelessWidget {
  const _FilesMenuDivider();

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    return Container(
      height: controls.contextDividerHeight,
      margin: EdgeInsets.symmetric(
        horizontal: controls.contextDividerHorizontalPadding,
        vertical: controls.contextDividerVerticalPadding,
      ),
      color: colors.border,
    );
  }
}
