import 'dart:async';

import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'mate_button.dart';
import 'mate_icon.dart';

// =============================================================================
// MateOverlay —— 浮层组件集合（对标 CMP mate/MateOverlay.kt v2）。
//
// - MatePopupMenu：trigger + Popup 下拉菜单（168 宽、radius 10）
// - MateDialog：全局命令式对话框（open/confirm/close + MateDialogHost 挂载）
// - MateToast：全局命令式 Toast（show + MateToastHost 挂载，单条语义）
// =============================================================================

/// 弹出菜单项（对标 CMP MatePopupItem）。
class MatePopupItem {
  /// 项值（选中回调传出）。
  final String value;

  /// 显示文字（默认等于 value）。
  final String label;

  /// 图标 name（可选）。
  final String? icon;

  /// 危险态（红）。
  final bool danger;

  /// 是否为分隔线。
  final bool divider;

  const MatePopupItem({
    required this.value,
    this.label = '',
    this.icon,
    this.danger = false,
    this.divider = false,
  });

  /// 分隔线项。
  const MatePopupItem.divider()
      : value = '',
        label = '',
        icon = null,
        danger = false,
        divider = true;
}

/// 弹出菜单（v2：radius 10 浮层 + radius 8 菜单项）。
///
/// trigger + Popup + menu；menu 宽 [menuWidth]（默认 168），radius 10；
/// 点击外部或选择后关闭；item h36，radius 8，hover bgFill；danger color error；
/// divider 0.5px。
///
/// 示例：
/// ```dart
/// MatePopupMenu(
///   items: [MatePopupItem(value: 'rename', label: '重命名', icon: 'edit')],
///   onSelect: (v) => print(v),
///   trigger: MateButton(variant: MateButtonVariant.icon, icon: 'list', onClick: null),
/// )
/// ```
class MatePopupMenu extends StatefulWidget {
  /// 菜单项列表。
  final List<MatePopupItem> items;

  /// 菜单宽度（默认 168）。
  final double? menuWidth;

  /// 选中回调（传 item.value）。
  final ValueChanged<String>? onSelect;

  /// 关闭回调（点击外部或选择后）。
  final VoidCallback? onDismiss;

  /// 是否禁用触发。
  final bool disabled;

  /// 触发器内容。
  final Widget trigger;

  const MatePopupMenu({
    super.key,
    required this.items,
    required this.trigger,
    this.menuWidth,
    this.onSelect,
    this.onDismiss,
    this.disabled = false,
  });

  @override
  State<MatePopupMenu> createState() => _MatePopupMenuState();
}

class _MatePopupMenuState extends State<MatePopupMenu> {
  OverlayEntry? _entry;

  /// 触发器全局几何（弹出时捕获，用于视口钳制定位）
  Offset _triggerOffset = Offset.zero;
  Size _triggerSize = Size.zero;

  bool get _expanded => _entry != null;

  @override
  void dispose() {
    _removeEntry(notify: false);
    super.dispose();
  }

  void _toggle() {
    if (widget.disabled) return;
    if (_expanded) {
      _removeEntry();
    } else {
      _showEntry();
    }
  }

  void _showEntry() {
    final overlay = Overlay.of(context);
    // 捕获触发器全局位置与尺寸，供弹出层做视口钳制
    final box = context.findRenderObject() as RenderBox?;
    if (box != null && box.attached) {
      _triggerOffset = box.localToGlobal(Offset.zero);
      _triggerSize = box.size;
    }
    _entry = OverlayEntry(builder: (ctx) => _buildPopup(ctx));
    overlay.insert(_entry!);
  }

  void _removeEntry({bool notify = true}) {
    _entry?.remove();
    _entry = null;
    if (notify) widget.onDismiss?.call();
  }

  void _select(String value) {
    _entry?.remove();
    _entry = null;
    widget.onSelect?.call(value);
    widget.onDismiss?.call();
  }

  Widget _buildPopup(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final menuMetrics = MateTheme.metricsOf(context).menu;
    final overlayMetrics = MateTheme.metricsOf(context).overlay;
    final width = widget.menuWidth ?? menuMetrics.defaultWidth;

    // 视口钳制：估算菜单高度（分隔线按普通项估算，足够钳制用），
    // 水平方向防右缘溢出，垂直方向下方不够则翻到触发器上方。
    final screenSize = MediaQuery.sizeOf(context);
    const edgeMargin = 8.0;
    const gap = 4.0;
    final estimatedHeight = widget.items.length * menuMetrics.itemHeight +
        overlayMetrics.menuPadding * 2;
    final maxLeft = screenSize.width - width - edgeMargin;
    final left = _triggerOffset.dx.clamp(edgeMargin, maxLeft < edgeMargin ? edgeMargin : maxLeft);
    final belowTop = _triggerOffset.dy + _triggerSize.height + gap;
    final overflowBelow = belowTop + estimatedHeight > screenSize.height - edgeMargin;
    final aboveTop = _triggerOffset.dy - gap - estimatedHeight;
    final top = overflowBelow && aboveTop >= edgeMargin
        ? aboveTop
        : belowTop.clamp(edgeMargin, screenSize.height - edgeMargin);

    return Stack(
      children: [
        // 透明全屏屏障：点击外部关闭
        Positioned.fill(
          child: GestureDetector(
            onTap: _removeEntry,
            behavior: HitTestBehavior.opaque,
            child: const SizedBox.expand(),
          ),
        ),
        Positioned(
          left: left.toDouble(),
          top: top.toDouble(),
          child: Container(
            width: width,
            decoration: BoxDecoration(
              color: colors.bgContainer,
              borderRadius:
                  BorderRadius.circular(menuMetrics.containerRadius),
              border: Border.all(
                color: colors.border,
                width: overlayMetrics.menuBorderWidth,
              ),
              boxShadow: [
                BoxShadow(
                  // 派生自遮罩 token 的柔影（dropdown 阴影）
                  color: colors.overlayDialogScrim.withAlpha(20),
                  blurRadius: 16,
                  offset: const Offset(0, 4),
                ),
              ],
            ),
            padding: EdgeInsets.all(overlayMetrics.menuPadding),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final item in widget.items)
                  item.divider
                      ? Container(
                          height: overlayMetrics.menuDividerHeight,
                          margin: EdgeInsets.symmetric(
                            horizontal:
                                overlayMetrics.menuDividerHorizontalPadding,
                            vertical:
                                overlayMetrics.menuDividerVerticalPadding,
                          ),
                          color: colors.border,
                        )
                      : _MatePopupMenuItem(
                          item: item,
                          onSelect: () => _select(item.value),
                        ),
              ],
            ),
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: _toggle,
      behavior: HitTestBehavior.opaque,
      // 触发器内常嵌 MateButton 等自带手势的组件，
      // IgnorePointer 保证点击统一由本组件处理（对齐 CMP trigger 语义）
      child: IgnorePointer(child: widget.trigger),
    );
  }
}

/// 弹出菜单项行（h36，radius 8，hover bgFill）。
class _MatePopupMenuItem extends StatefulWidget {
  final MatePopupItem item;
  final VoidCallback onSelect;

  const _MatePopupMenuItem({required this.item, required this.onSelect});

  @override
  State<_MatePopupMenuItem> createState() => _MatePopupMenuItemState();
}

class _MatePopupMenuItemState extends State<_MatePopupMenuItem> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).menu;
    final menuMetrics = MateTheme.metricsOf(context).menu;
    final overlayMetrics = MateTheme.metricsOf(context).overlay;
    final item = widget.item;

    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onSelect,
        behavior: HitTestBehavior.opaque,
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 120),
          height: menuMetrics.itemHeight,
          decoration: BoxDecoration(
            color: _hovered ? colors.bgFill : Colors.transparent,
            borderRadius: BorderRadius.circular(menuMetrics.itemRadius),
          ),
          padding: EdgeInsets.symmetric(
            horizontal: overlayMetrics.menuItemHorizontalPadding,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              if (item.icon != null) ...[
                MateIcon(
                  name: item.icon!,
                  size: overlayMetrics.menuItemIconSize,
                  tint: item.danger ? colors.error : colors.textSecondary,
                ),
                SizedBox(width: overlayMetrics.menuItemContentSpacing),
              ],
              Text(
                item.label.isEmpty ? item.value : item.label,
                style: typography.itemLabel.copyWith(
                  color: item.danger ? colors.error : colors.textPrimary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ============================================================
// Dialog（v2：radius 12 + 图标徽章标题，全局命令式宿主）
// ============================================================

/// 对话框配置（对标 CMP MateDialogOptions）。
class MateDialogOptions {
  /// 标题。
  final String title;

  /// 标题图标 name（可选，显示在 40×40 徽章内）。
  final String? titleIcon;

  /// 危险态（徽章/确认按钮转 error 配色）。
  final bool danger;

  /// 正文内容。
  final String content;

  /// 点击遮罩是否关闭。
  final bool closeOnOverlay;

  /// 弹窗宽度。
  final double width;

  /// 取消按钮文字。
  final String cancelText;

  /// 确认按钮文字。
  final String confirmText;

  const MateDialogOptions({
    this.title = '',
    this.titleIcon,
    this.danger = false,
    this.content = '',
    this.closeOnOverlay = true,
    this.width = 460,
    this.cancelText = '取消',
    this.confirmText = '确定',
  });
}

/// 全局对话框状态条目：配置 + 确认回调（null 表示非确认型）。
typedef _DialogEntry = (MateDialogOptions, void Function(bool)?);

/// 全局命令式对话框（对标 CMP openDialog/confirmDialog/closeDialog）。
///
/// 需在应用根部挂载一次 [MateDialogHost]（如 GetMaterialApp.builder）。
///
/// 示例：
/// ```dart
/// MateDialog.confirm(
///   const MateDialogOptions(title: '确认删除', content: '此操作不可撤销', danger: true),
///   (ok) { if (ok) doDelete(); },
/// );
/// ```
abstract final class MateDialog {
  static final ValueNotifier<_DialogEntry?> _state =
      ValueNotifier<_DialogEntry?>(null);

  /// 显示对话框（非确认型，仅确认按钮）。
  static void open(MateDialogOptions options) {
    _state.value = (options, null);
  }

  /// 确认对话框；[onResult] 收到 true=确认 / false=取消。
  static void confirm(
    MateDialogOptions options,
    void Function(bool) onResult,
  ) {
    _state.value = (options, onResult);
  }

  /// 关闭对话框（确认型会触发结果回调）。
  static void close([bool value = false]) {
    final entry = _state.value;
    _state.value = null;
    entry?.$2?.call(value);
  }
}

/// 对话框宿主（v2）。
///
/// 绑定 [MateDialog] 全局状态；confirm 时 footer 为「取消(iconText) + 确认(primary)」两按钮。
/// overlay 全屏 rgba(0,0,0,0.36)；dialog radius 12。
class MateDialogHost extends StatelessWidget {
  const MateDialogHost({super.key});

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<_DialogEntry?>(
      valueListenable: MateDialog._state,
      builder: (context, entry, _) {
        if (entry == null) return const SizedBox.shrink();
        final (options, resolver) = entry;
        final colors = MateTheme.colorsOf(context);
        final typography = MateTheme.typographyOf(context).dialog;
        final dialogMetrics = MateTheme.metricsOf(context).dialog;
        final overlayMetrics = MateTheme.metricsOf(context).overlay;

        return GestureDetector(
          onTap: options.closeOnOverlay ? () => MateDialog.close(false) : null,
          child: Container(
            color: colors.overlayDialogScrim,
            alignment: Alignment.center,
            child: GestureDetector(
              onTap: () {}, // 吞掉点击，避免穿透到遮罩关闭
              child: Container(
                width: options.width,
                decoration: BoxDecoration(
                  color: colors.bgContainer,
                  borderRadius:
                      BorderRadius.circular(dialogMetrics.containerRadius),
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    // header：图标徽章 + 标题
                    Padding(
                      padding: EdgeInsets.only(
                        left: overlayMetrics.dialogHeaderHorizontalPadding,
                        right: overlayMetrics.dialogHeaderHorizontalPadding,
                        top: overlayMetrics.dialogHeaderTopPadding,
                        bottom: overlayMetrics.dialogHeaderBottomPadding,
                      ),
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.center,
                        children: [
                          if (options.titleIcon != null) ...[
                            Container(
                              width: dialogMetrics.iconBadgeSize,
                              height: dialogMetrics.iconBadgeSize,
                              decoration: BoxDecoration(
                                color: options.danger
                                    ? colors.errorBg
                                    : colors.brandLighter,
                                borderRadius: BorderRadius.circular(
                                  dialogMetrics.iconBadgeRadius,
                                ),
                              ),
                              alignment: Alignment.center,
                              child: MateIcon(
                                name: options.titleIcon!,
                                size: overlayMetrics.dialogTitleIconSize,
                                tint: options.danger
                                    ? colors.error
                                    : colors.brand,
                              ),
                            ),
                            SizedBox(
                              width:
                                  overlayMetrics.dialogHeaderContentSpacing,
                            ),
                          ],
                          Flexible(
                            child: Text(
                              options.title,
                              style: typography.title.copyWith(
                                color: colors.textPrimary,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                    // body
                    Padding(
                      padding: EdgeInsets.only(
                        left: overlayMetrics.dialogBodyHorizontalPadding,
                        right: overlayMetrics.dialogBodyHorizontalPadding,
                        top: overlayMetrics.dialogBodyTopPadding,
                        bottom: overlayMetrics.dialogBodyBottomPadding,
                      ),
                      child: SizedBox(
                        width: double.infinity,
                        child: Text(
                          options.content,
                          style: typography.body.copyWith(
                            color: colors.textSecondary,
                          ),
                        ),
                      ),
                    ),
                    // footer
                    Padding(
                      padding: EdgeInsets.only(
                        left: overlayMetrics.dialogFooterHorizontalPadding,
                        right: overlayMetrics.dialogFooterHorizontalPadding,
                        bottom: overlayMetrics.dialogFooterBottomPadding,
                      ),
                      child: Row(
                        mainAxisAlignment: MainAxisAlignment.end,
                        crossAxisAlignment: CrossAxisAlignment.center,
                        children: [
                          if (resolver != null) ...[
                            MateButton(
                              label: options.cancelText,
                              variant: MateButtonVariant.iconText,
                              onClick: () => MateDialog.close(false),
                            ),
                            SizedBox(
                              width: overlayMetrics.dialogActionSpacing,
                            ),
                            MateButton(
                              label: options.confirmText,
                              variant: MateButtonVariant.primary,
                              danger: options.danger,
                              onClick: () => MateDialog.close(true),
                            ),
                          ] else
                            MateButton(
                              label: options.confirmText,
                              onClick: () => MateDialog.close(false),
                            ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

// ============================================================
// Toast（v2：深色浮条 + 状态图标，全局命令式宿主）
// ============================================================

/// Toast 变体（对标 CMP MateToastVariant；Dart 无 default 枚举值，用 normal 代指）。
enum MateToastVariant {
  /// 默认（info 图标，白色）。
  normal,

  /// 成功（check 图标，绿）。
  success,

  /// 警告（alert 图标，橙）。
  warning,

  /// 错误（alert 图标，粉红）。
  error,
}

/// Toast 通知条目：消息文本及其展示样式变体。
typedef _ToastEntry = (String, MateToastVariant);

/// 全局命令式 Toast（对标 CMP showToast；单条语义：新 toast 清空旧的）。
///
/// 需在应用根部挂载一次 [MateToastHost]。
///
/// 示例：
/// ```dart
/// MateToast.show('保存成功', variant: MateToastVariant.success);
/// ```
abstract final class MateToast {
  static final ValueNotifier<_ToastEntry?> _state =
      ValueNotifier<_ToastEntry?>(null);

  /// 默认显示时长（对齐 CMP 2000ms）。
  static const Duration defaultDuration = Duration(milliseconds: 2000);

  /// 显示 Toast（单条语义：新 toast 清空旧的）。
  static void show(
    String message, {
    MateToastVariant variant = MateToastVariant.normal,
  }) {
    _state.value = (message, variant);
  }

  /// 立即关闭当前 Toast。
  static void dismiss() {
    _state.value = null;
  }
}

/// Toast 宿主（v2：深色浮条）。
///
/// 底部居中，padding 10/18，radius 10，bg rgba(28,28,30,0.92)；
/// 图标按 variant 着色；2 秒后自动清除（单条语义）。
class MateToastHost extends StatefulWidget {
  const MateToastHost({super.key});

  @override
  State<MateToastHost> createState() => _MateToastHostState();
}

class _MateToastHostState extends State<MateToastHost> {
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    MateToast._state.addListener(_onEntryChanged);
  }

  @override
  void dispose() {
    MateToast._state.removeListener(_onEntryChanged);
    _timer?.cancel();
    super.dispose();
  }

  void _onEntryChanged() {
    _timer?.cancel();
    if (MateToast._state.value != null) {
      _timer = Timer(MateToast.defaultDuration, MateToast.dismiss);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder<_ToastEntry?>(
      valueListenable: MateToast._state,
      builder: (context, entry, _) {
        if (entry == null) return const SizedBox.shrink();
        final (message, variant) = entry;
        final colors = MateTheme.colorsOf(context);
        final typography = MateTheme.typographyOf(context).dialog;
        final dialogMetrics = MateTheme.metricsOf(context).dialog;
        final overlayMetrics = MateTheme.metricsOf(context).overlay;

        final (iconName, iconColor) = switch (variant) {
          MateToastVariant.normal => ('info', colors.toastDefaultIcon),
          MateToastVariant.success => ('check', colors.toastSuccessIcon),
          MateToastVariant.warning => ('alert', colors.warning),
          MateToastVariant.error => ('alert', colors.toastErrorIcon),
        };

        return IgnorePointer(
          child: Padding(
            padding: EdgeInsets.all(overlayMetrics.toastOuterPadding),
            child: Align(
              alignment: Alignment.bottomCenter,
              child: Container(
                constraints: const BoxConstraints(maxWidth: 480),
                decoration: BoxDecoration(
                  color: colors.toastBackground,
                  borderRadius:
                      BorderRadius.circular(dialogMetrics.toastRadius),
                ),
                padding: EdgeInsets.symmetric(
                  horizontal: overlayMetrics.toastHorizontalPadding,
                  vertical: overlayMetrics.toastVerticalPadding,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    MateIcon(
                      name: iconName,
                      size: overlayMetrics.toastIconSize,
                      tint: iconColor,
                    ),
                    SizedBox(width: overlayMetrics.toastContentSpacing),
                    Flexible(
                      child: Text(
                        message,
                        style: typography.toastMessage.copyWith(
                          color: colors.toastText,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}
