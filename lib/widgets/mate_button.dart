import 'package:flutter/material.dart';

import '../app/theme/mate_theme.dart';
import 'mate_icon.dart';

// =============================================================================
// MateButton —— 五变体统一按钮（对标 CMP mate/MateButton.kt v2）。
//
// - primary：品牌渐变底（brandHover→brand），白字，radius 8，h36，品牌色柔影；
//   hover 降透明；pressed→brandActive 纯色；danger→error 纯色
// - soft：brandLighter 浅蓝底，brand 字，radius 8，h36；hover→brand100
// - text：透明底，brand 字，radius 5；hover→brandLighter；danger 字色 error
// - icon：透明，32×32 正圆形；hover→bgFill + textPrimary；danger 字色 error
// - iconText：透明，h36，radius 8；hover→bgFill
//
// hover 仅背景色过渡，无 ripple；loading 时 primary 显 spinner，其余图标 spin + 禁用点击。
// =============================================================================

/// 按钮形态（v2：主按钮 / 软色 / 文字 / 图标 / 图标文字）。
enum MateButtonVariant {
  /// 品牌渐变主按钮。
  primary,

  /// 软色按钮（浅蓝底）。
  soft,

  /// 文字按钮。
  text,

  /// 纯图标圆形按钮。
  icon,

  /// 图标 + 文字按钮。
  iconText,
}

/// 统一按钮（macOS 风格，无水波纹）。
///
/// 示例：
/// ```dart
/// MateButton(
///   label: '确定',
///   variant: MateButtonVariant.primary,
///   onClick: () => print('clicked'),
/// )
/// ```
class MateButton extends StatefulWidget {
  /// 按钮文字（icon 变体可空）。
  final String? label;

  /// 按钮形态。
  final MateButtonVariant variant;

  /// 点击回调。
  final VoidCallback? onClick;

  /// 图标 name（可选；primary/text 用 14px，iconText/soft 16px，icon 18px）。
  final String? icon;

  /// 危险态（红）。
  final bool danger;

  /// 禁用。
  final bool disabled;

  /// 加载中（primary 显 spinner，其余图标 spin + 禁用点击）。
  final bool loading;

  /// 100% 宽（仅 primary/text/soft/iconText 生效）。
  final bool fullWidth;

  /// 角标数字（>0 显示，仅 icon/iconText）。
  final int badge;

  /// 自定义高度（null 用变体默认）。
  final double? height;

  const MateButton({
    super.key,
    this.label,
    this.variant = MateButtonVariant.primary,
    this.onClick,
    this.icon,
    this.danger = false,
    this.disabled = false,
    this.loading = false,
    this.fullWidth = false,
    this.badge = 0,
    this.height,
  });

  @override
  State<MateButton> createState() => _MateButtonState();
}

class _MateButtonState extends State<MateButton> {
  bool _hovered = false;
  bool _pressed = false;

  bool get _effectiveDisabled => widget.disabled || widget.loading;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).button;
    final metrics = MateTheme.metricsOf(context).button;

    // ---- 形状：icon 为正圆，text 为 radius-sm，其余 radius-md ----
    final BoxShape shape;
    final BorderRadius borderRadius;
    switch (widget.variant) {
      case MateButtonVariant.icon:
        shape = BoxShape.circle;
        borderRadius = BorderRadius.zero;
      case MateButtonVariant.text:
        shape = BoxShape.rectangle;
        borderRadius = BorderRadius.circular(metrics.textRadius);
      case MateButtonVariant.soft:
        shape = BoxShape.rectangle;
        borderRadius = BorderRadius.circular(metrics.softRadius);
      case MateButtonVariant.iconText:
        shape = BoxShape.rectangle;
        borderRadius = BorderRadius.circular(metrics.iconTextRadius);
      case MateButtonVariant.primary:
        shape = BoxShape.rectangle;
        borderRadius = BorderRadius.circular(metrics.primaryRadius);
    }

    // ---- 背景与文字色按变体 × 状态组合计算（无 ripple，纯背景过渡） ----
    // primary 返回的 bgColor 仅作 danger/disabled 兜底；正常态用渐变刷。
    final Color bgColor;
    final Color contentColor;
    switch (widget.variant) {
      case MateButtonVariant.primary:
        if (_effectiveDisabled) {
          bgColor = colors.border;
          contentColor = colors.buttonDisabledPrimaryText;
        } else if (widget.danger) {
          bgColor = _pressed
              ? colors.error.withAlpha((metrics.dangerPressedAlpha * 255).round())
              : colors.error;
          contentColor = colors.buttonDangerText;
        } else {
          bgColor = colors.brand;
          contentColor = colors.buttonPrimaryText;
        }
      case MateButtonVariant.soft:
        if (_effectiveDisabled) {
          bgColor = colors.bgFill;
          contentColor = colors.textPlaceholder;
        } else {
          bgColor = _hovered ? colors.brand100 : colors.brandLighter;
          contentColor = colors.brand;
        }
      case MateButtonVariant.text:
        if (_effectiveDisabled) {
          bgColor = Colors.transparent;
          contentColor = colors.textPlaceholder;
        } else if (widget.danger) {
          bgColor = _hovered ? colors.brandLighter : Colors.transparent;
          contentColor = colors.error;
        } else {
          bgColor = _hovered ? colors.brandLighter : Colors.transparent;
          contentColor = colors.brand;
        }
      case MateButtonVariant.icon:
        if (_effectiveDisabled) {
          bgColor = Colors.transparent;
          contentColor = colors.textPlaceholder;
        } else if (_hovered) {
          bgColor = colors.bgFill;
          contentColor = colors.textPrimary;
        } else {
          bgColor = Colors.transparent;
          contentColor = widget.danger ? colors.error : colors.textSecondary;
        }
      case MateButtonVariant.iconText:
        if (_effectiveDisabled) {
          bgColor = Colors.transparent;
          contentColor = colors.textPlaceholder;
        } else if (_hovered) {
          bgColor = colors.bgFill;
          contentColor = colors.textPrimary;
        } else {
          bgColor = Colors.transparent;
          contentColor = colors.textPrimary;
        }
    }

    // ---- 高度 / 图标尺寸 / 水平内边距 ----
    final resolvedHeight = widget.height ??
        switch (widget.variant) {
          MateButtonVariant.icon => metrics.iconButtonSize,
          MateButtonVariant.text => metrics.textHeight,
          MateButtonVariant.soft => metrics.softHeight,
          MateButtonVariant.iconText => metrics.iconTextHeight,
          MateButtonVariant.primary => metrics.primaryHeight,
        };
    final iconSize = switch (widget.variant) {
      MateButtonVariant.icon => metrics.iconVariantIconSize,
      MateButtonVariant.iconText => metrics.iconTextVariantIconSize,
      MateButtonVariant.soft => metrics.softVariantIconSize,
      MateButtonVariant.primary => widget.icon != null
          ? metrics.primaryVariantIconSize
          : metrics.primaryWithoutIconSize,
      MateButtonVariant.text => widget.icon != null
          ? metrics.textVariantIconSize
          : metrics.textWithoutIconSize,
    };
    final horizontalPadding = switch (widget.variant) {
      MateButtonVariant.icon => metrics.iconHorizontalPadding,
      MateButtonVariant.iconText => metrics.iconTextHorizontalPadding,
      MateButtonVariant.text => metrics.textHorizontalPadding,
      MateButtonVariant.soft => metrics.softHorizontalPadding,
      MateButtonVariant.primary => metrics.primaryHorizontalPadding,
    };
    final labelStyle = switch (widget.variant) {
      MateButtonVariant.primary => typography.primaryLabel,
      MateButtonVariant.soft => typography.softLabel,
      MateButtonVariant.text => typography.textLabel,
      MateButtonVariant.iconText => typography.iconTextLabel,
      MateButtonVariant.icon => typography.iconTextLabel,
    };

    // primary 正常态（非 danger 非禁用）用品牌渐变 + 品牌色柔影，pressed 转 brandActive 纯色。
    final useGradient = widget.variant == MateButtonVariant.primary &&
        !widget.danger &&
        !_effectiveDisabled;
    final pressedSolid = useGradient && _pressed;

    final decoration = BoxDecoration(
      shape: shape,
      borderRadius: shape == BoxShape.circle ? null : borderRadius,
      color: useGradient && !pressedSolid ? null : bgColor,
      gradient: useGradient && !pressedSolid
          ? LinearGradient(
              colors: colors.brandGradient,
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
            )
          : null,
      boxShadow: useGradient
          ? [
              BoxShadow(
                color: colors.brand
                    .withAlpha((metrics.primaryShadowAlpha * 255).round()),
                blurRadius: metrics.primaryShadowElevation,
                offset: Offset(0, metrics.primaryShadowElevation / 2),
              ),
            ]
          : null,
    );

    // ---- 内容行 ----
    final children = <Widget>[];
    if (widget.loading && widget.variant == MateButtonVariant.primary) {
      children.add(_MateButtonSpinner(
        size: metrics.loadingSpinnerSize,
        color: contentColor,
        trackAlpha: metrics.spinnerTrackAlpha,
        duration: Duration(milliseconds: metrics.spinnerRotationDurationMillis),
        reducedMotion: MateTheme.reducedMotionOf(context),
      ));
      if (widget.label != null) {
        children.add(SizedBox(width: metrics.loadingLabelSpacing));
      }
    } else if (widget.icon != null) {
      final spin = widget.loading && widget.variant != MateButtonVariant.primary;
      children.add(MateIcon(
        name: widget.icon!,
        size: iconSize,
        tint: contentColor,
        spin: spin,
      ));
      if (widget.label != null) {
        children.add(SizedBox(width: metrics.iconLabelSpacing));
      }
    }
    if (widget.label != null) {
      children.add(Text(
        widget.label!,
        style: labelStyle.copyWith(color: contentColor),
      ));
    }
    if (widget.badge > 0 && widget.variant == MateButtonVariant.iconText) {
      children.add(Padding(
        padding: EdgeInsets.only(left: metrics.badgeStartPadding),
        child: _MateBadge(count: widget.badge),
      ));
    }

    Widget button = AnimatedContainer(
      duration: const Duration(milliseconds: 150),
      curve: Curves.easeOut,
      height: resolvedHeight,
      width: widget.variant == MateButtonVariant.icon
          ? metrics.iconButtonSize
          : (widget.fullWidth ? double.infinity : null),
      padding: EdgeInsets.symmetric(horizontal: horizontalPadding),
      decoration: decoration,
      child: Opacity(
        opacity: _effectiveDisabled ? metrics.disabledAlpha : 1,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: children,
        ),
      ),
    );

    // icon 变体的 badge 置于 32×32 命中区外侧（避免圆形容器内溢出）
    if (widget.badge > 0 && widget.variant == MateButtonVariant.icon) {
      button = Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          button,
          Padding(
            padding: EdgeInsets.only(left: metrics.badgeStartPadding),
            child: _MateBadge(count: widget.badge),
          ),
        ],
      );
    }

    return MouseRegion(
      cursor: _effectiveDisabled
          ? SystemMouseCursors.basic
          : SystemMouseCursors.click,
      onEnter: (_) => _setHover(true),
      onExit: (_) => _setHover(false),
      child: GestureDetector(
        onTapDown: _effectiveDisabled ? null : (_) => _setPressed(true),
        onTapUp: _effectiveDisabled ? null : (_) => _setPressed(false),
        onTapCancel: _effectiveDisabled ? null : () => _setPressed(false),
        onTap: _effectiveDisabled ? null : widget.onClick,
        behavior: HitTestBehavior.opaque,
        child: button,
      ),
    );
  }

  void _setHover(bool v) {
    if (_hovered != v) setState(() => _hovered = v);
  }

  void _setPressed(bool v) {
    if (_pressed != v) setState(() => _pressed = v);
  }
}

/// primary 加载态的小 spinner（16×16，0.8s 线性旋转；减少动态时降级为静态半透明圆）。
class _MateButtonSpinner extends StatefulWidget {
  final double size;
  final Color color;
  final double trackAlpha;
  final Duration duration;
  final bool reducedMotion;

  const _MateButtonSpinner({
    required this.size,
    required this.color,
    required this.trackAlpha,
    required this.duration,
    required this.reducedMotion,
  });

  @override
  State<_MateButtonSpinner> createState() => _MateButtonSpinnerState();
}

class _MateButtonSpinnerState extends State<_MateButtonSpinner>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(vsync: this, duration: widget.duration);
    if (!widget.reducedMotion) _controller.repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final spinner = CustomPaint(
      size: Size.square(widget.size),
      painter: _ArcSpinnerPainter(
        color: widget.color,
        trackAlpha: widget.trackAlpha,
      ),
    );
    if (widget.reducedMotion) return spinner;
    return AnimatedBuilder(
      animation: _controller,
      builder: (_, child) => Transform.rotate(
        angle: _controller.value * 2 * 3.141592653589793,
        child: child,
      ),
      child: spinner,
    );
  }
}

/// 圆弧 spinner 绘制（淡色轨道整圈 + 高亮 90° 弧）。
class _ArcSpinnerPainter extends CustomPainter {
  final Color color;
  final double trackAlpha;

  _ArcSpinnerPainter({required this.color, required this.trackAlpha});

  @override
  void paint(Canvas canvas, Size size) {
    const strokeWidth = 2.0;
    final rect = Offset.zero & size;
    final trackPaint = Paint()
      ..color = color.withAlpha((trackAlpha * 255).round())
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth;
    final arcPaint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round;
    canvas.drawCircle(size.center(Offset.zero), size.width / 2 - strokeWidth / 2, trackPaint);
    canvas.drawArc(
      rect.deflate(strokeWidth / 2),
      -3.141592653589793 / 2,
      3.141592653589793 / 2,
      false,
      arcPaint,
    );
  }

  @override
  bool shouldRepaint(covariant _ArcSpinnerPainter oldDelegate) =>
      oldDelegate.color != color || oldDelegate.trackAlpha != trackAlpha;
}

/// 角标（badge）。
///
/// h16，padding 0/5，pill 圆角，bg brand，白字 10 semibold（>99 显示 99+）。
class _MateBadge extends StatelessWidget {
  final int count;

  const _MateBadge({required this.count});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).button;
    final display = count > 99 ? '99+' : count.toString();
    return Container(
      height: metrics.badgeHeight,
      padding: EdgeInsets.symmetric(horizontal: metrics.badgeHorizontalPadding),
      decoration: BoxDecoration(
        color: colors.brand,
        borderRadius: BorderRadius.circular(metrics.badgeHeight / 2),
      ),
      alignment: Alignment.center,
      child: Text(
        display,
        style: MateTheme.typographyOf(context)
            .button
            .badgeLabel
            .copyWith(color: colors.buttonBadgeText),
      ),
    );
  }
}
