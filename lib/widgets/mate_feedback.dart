import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'mate_icon.dart';

// =============================================================================
// MateFeedback —— 反馈与导航组件集合（对标 CMP mate/MateFeedback.kt v2）。
//
// 进度条 / 横幅 / 标签 / 空状态 / 统计芯片 / 区块标题 / 导航项 / 导航分组。
// =============================================================================

/// 线性进度条（v2：h6 圆角条，brand 为渐变填充）。
///
/// h=[height]（默认 6），bg=bgFill；color=brand 时用品牌渐变，其余纯色；
/// value=null 不确定态（30% 宽指示器 1.2s 循环移动；减少动态时静止）。
class MateLinearProgress extends StatelessWidget {
  /// 进度 0..1；null=不确定态。
  final double? value;

  /// 条高（默认 6dp）。
  final double? height;

  /// fill 颜色（默认 brand=品牌渐变）。
  final Color? color;

  const MateLinearProgress({
    super.key,
    this.value,
    this.height,
    this.color,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).feedback.controls;
    final h = height ?? controls.linearProgressHeight;
    final fillColor = color ?? colors.brand;
    final useGradient = fillColor == colors.brand;
    final reducedMotion = MateTheme.reducedMotionOf(context);

    final fillDecoration = BoxDecoration(
      gradient: useGradient
          ? LinearGradient(colors: colors.brandGradient)
          : null,
      color: useGradient ? null : fillColor,
      borderRadius: BorderRadius.circular(h / 2),
    );

    return ClipRRect(
      borderRadius: BorderRadius.circular(h / 2),
      child: Container(
        height: h,
        width: double.infinity,
        color: colors.bgFill,
        child: value != null
            ? FractionallySizedBox(
                alignment: Alignment.centerLeft,
                widthFactor: value!.clamp(0, 1).toDouble(),
                child: DecoratedBox(decoration: fillDecoration),
              )
            : reducedMotion
                // 减少动态：静止 30% 指示条
                ? FractionallySizedBox(
                    alignment: Alignment.centerLeft,
                    widthFactor: 0.3,
                    child: DecoratedBox(decoration: fillDecoration),
                  )
                : _IndeterminateBar(fillDecoration: fillDecoration),
      ),
    );
  }
}

/// 不确定态指示条（30% 宽，0 → 容器宽度 循环移动）。
class _IndeterminateBar extends StatefulWidget {
  final BoxDecoration fillDecoration;

  const _IndeterminateBar({required this.fillDecoration});

  @override
  State<_IndeterminateBar> createState() => _IndeterminateBarState();
}

class _IndeterminateBarState extends State<_IndeterminateBar>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    final duration = MateTheme.metricsOf(context)
        .feedback
        .controls
        .circularProgressRotationDurationMillis;
    _controller = AnimationController(
      vsync: this,
      duration: Duration(milliseconds: duration),
    )..repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (_, _) {
        // 30% 宽指示条从容纳区最左循环移动到最右
        return FractionallySizedBox(
          alignment: Alignment(-1 + _controller.value * 2, 0),
          widthFactor: 0.3,
          child: DecoratedBox(decoration: widget.fillDecoration),
        );
      },
    );
  }
}

/// 环形进度条（对标原 Vue `<MateCircularProgress>`）。
///
/// 轨道 bgFill；填充 color，linecap round，从顶部起笔；
/// value=null 不确定态画约 86° 弧并旋转（减少动态时整圈静态）。
class MateCircularProgress extends StatelessWidget {
  /// 直径（默认 24）。
  final double? size;

  /// 描边宽（默认 2.5）。
  final double? strokeWidth;

  /// 填充色（默认 brand）。
  final Color? color;

  /// 进度 0..1；null=不确定态。
  final double? value;

  const MateCircularProgress({
    super.key,
    this.size,
    this.strokeWidth,
    this.color,
    this.value,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).feedback.controls;
    final resolvedSize = size ?? controls.circularProgressSize;
    final resolvedStroke = strokeWidth ?? controls.circularProgressStrokeWidth;
    final fillColor = color ?? colors.brand;
    final reducedMotion = MateTheme.reducedMotionOf(context);

    final sweep = value != null
        ? value!.clamp(0, 1).toDouble() * 360
        : reducedMotion
            ? 360.0
            : 86.0;

    Widget ring = CustomPaint(
      size: Size.square(resolvedSize),
      painter: _RingPainter(
        trackColor: colors.bgFill,
        fillColor: fillColor,
        strokeWidth: resolvedStroke,
        sweepDegrees: sweep,
      ),
    );

    if (value == null && !reducedMotion) {
      ring = _SpinningRing(
        duration: Duration(
          milliseconds: controls.circularProgressRotationDurationMillis,
        ),
        child: ring,
      );
    }
    return ring;
  }
}

/// 环形进度绘制（轨道整圈 + 从顶部起笔的圆头填充弧）。
class _RingPainter extends CustomPainter {
  final Color trackColor;
  final Color fillColor;
  final double strokeWidth;
  final double sweepDegrees;

  _RingPainter({
    required this.trackColor,
    required this.fillColor,
    required this.strokeWidth,
    required this.sweepDegrees,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final rect = (Offset.zero & size).deflate(strokeWidth / 2);
    final trackPaint = Paint()
      ..color = trackColor
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth;
    final fillPaint = Paint()
      ..color = fillColor
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round;
    canvas.drawArc(rect, 0, 2 * 3.141592653589793, false, trackPaint);
    canvas.drawArc(
      rect,
      -3.141592653589793 / 2, // 从顶部起笔
      sweepDegrees * 3.141592653589793 / 180,
      false,
      fillPaint,
    );
  }

  @override
  bool shouldRepaint(covariant _RingPainter oldDelegate) =>
      oldDelegate.trackColor != trackColor ||
      oldDelegate.fillColor != fillColor ||
      oldDelegate.strokeWidth != strokeWidth ||
      oldDelegate.sweepDegrees != sweepDegrees;
}

/// 不确定态旋转包装。
class _SpinningRing extends StatefulWidget {
  final Duration duration;
  final Widget child;

  const _SpinningRing({required this.duration, required this.child});

  @override
  State<_SpinningRing> createState() => _SpinningRingState();
}

class _SpinningRingState extends State<_SpinningRing>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(vsync: this, duration: widget.duration)
      ..repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (_, child) => Transform.rotate(
        angle: _controller.value * 2 * 3.141592653589793,
        child: child,
      ),
      child: widget.child,
    );
  }
}

/// 横幅变体（对标 CMP MateBannerVariant）。
enum MateBannerVariant {
  /// 信息（brandLighter 底 / brand 图标）。
  info,

  /// 成功（successBg 底 / success 图标）。
  success,

  /// 警告（warningBg 底 / warning 图标）。
  warning,

  /// 错误（errorBg 底 / error 图标）。
  error,
}

/// 信息横幅（v2：radius 10 无描边，图标与文字、右侧 action 统一垂直居中）。
class MateInfoBanner extends StatelessWidget {
  /// 消息正文。
  final String message;

  /// 变体。
  final MateBannerVariant variant;

  /// 标题（可选）。
  final String? title;

  /// 是否显示关闭按钮。
  final bool closable;

  /// 关闭回调。
  final VoidCallback? onClose;

  /// 右侧操作区内容（margin-left auto）。
  final Widget? action;

  const MateInfoBanner({
    super.key,
    required this.message,
    this.variant = MateBannerVariant.info,
    this.title,
    this.closable = false,
    this.onClose,
    this.action,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final metrics = MateTheme.metricsOf(context).feedback;

    final (bg, iconColor) = switch (variant) {
      MateBannerVariant.info => (colors.brandLighter, colors.brand),
      MateBannerVariant.success => (colors.successBg, colors.success),
      MateBannerVariant.warning => (colors.warningBg, colors.warning),
      MateBannerVariant.error => (colors.errorBg, colors.error),
    };
    final iconName = switch (variant) {
      MateBannerVariant.info => 'info',
      MateBannerVariant.success => 'check',
      MateBannerVariant.warning => 'alert',
      MateBannerVariant.error => 'x',
    };

    return Container(
      width: double.infinity,
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(metrics.bannerRadius),
      ),
      padding: EdgeInsets.symmetric(
        horizontal: metrics.controls.bannerHorizontalPadding,
        vertical: metrics.controls.bannerVerticalPadding,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          MateIcon(
            name: iconName,
            size: metrics.controls.bannerIconSize,
            tint: iconColor,
          ),
          SizedBox(width: metrics.controls.bannerContentSpacing),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (title != null)
                  Text(
                    title!,
                    style: typography.bannerTitle.copyWith(
                      color: colors.textPrimary,
                    ),
                  ),
                Text(
                  message,
                  style: typography.bannerMessage.copyWith(
                    color: colors.textPrimary,
                  ),
                ),
              ],
            ),
          ),
          if (action != null) ...[
            SizedBox(width: metrics.controls.bannerContentSpacing),
            action!,
          ],
          if (closable) ...[
            SizedBox(width: metrics.controls.bannerContentSpacing),
            GestureDetector(
              onTap: onClose,
              child: MouseRegion(
                cursor: SystemMouseCursors.click,
                child: MateIcon(
                  name: 'x',
                  size: metrics.controls.bannerCloseIconSize,
                  tint: iconColor.withAlpha(
                    (metrics.controls.tagIconAlpha * 255).round(),
                  ),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// 标签主题（对标 CMP MateTagTheme；Dart 无 default 枚举值，用 normal 代指）。
enum MateTagTheme {
  /// 默认（bgFill 底 / textSecondary 字）。
  normal,

  /// 品牌（brandLighter 底 / brand 字）。
  primary,

  /// 成功（successBg 底 / success 字）。
  success,

  /// 警告（warningBg 底 / warning 字）。
  warning,

  /// 错误（errorBg 底 / error 字）。
  error,
}

/// 标签尺寸。
enum MateTagSize {
  /// 小标签。
  small,

  /// 中标签。
  medium,
}

/// 标签 chip（v2：radius 5 纯底色无描边）。
class MateTag extends StatelessWidget {
  /// 标签文字。
  final String label;

  /// 颜色主题。
  final MateTagTheme theme;

  /// 尺寸。
  final MateTagSize size;

  /// 图标 name（可选）。
  final String? icon;

  /// 点击回调（可选）。
  final VoidCallback? onClick;

  const MateTag({
    super.key,
    required this.label,
    this.theme = MateTagTheme.normal,
    this.size = MateTagSize.medium,
    this.icon,
    this.onClick,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final metrics = MateTheme.metricsOf(context).feedback;
    final controls = metrics.controls;

    final (bg, fg) = switch (theme) {
      MateTagTheme.normal => (colors.bgFill, colors.textSecondary),
      MateTagTheme.primary => (colors.brandLighter, colors.brand),
      MateTagTheme.success => (colors.successBg, colors.success),
      MateTagTheme.warning => (colors.warningBg, colors.warning),
      MateTagTheme.error => (colors.errorBg, colors.error),
    };
    final isSmall = size == MateTagSize.small;
    final labelStyle =
        isSmall ? typography.smallTagLabel : typography.mediumTagLabel;

    final tag = Container(
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(
          isSmall ? metrics.smallTagRadius : metrics.mediumTagRadius,
        ),
      ),
      padding: EdgeInsets.symmetric(
        horizontal: isSmall
            ? controls.smallTagHorizontalPadding
            : controls.mediumTagHorizontalPadding,
        vertical: isSmall
            ? controls.smallTagVerticalPadding
            : controls.mediumTagVerticalPadding,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (icon != null) ...[
            MateIcon(
              name: icon!,
              size: isSmall
                  ? controls.smallTagIconSize
                  : controls.mediumTagIconSize,
              tint: fg,
            ),
            SizedBox(width: controls.tagContentSpacing),
          ],
          Text(label, style: labelStyle.copyWith(color: fg)),
        ],
      ),
    );

    if (onClick == null) return tag;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(onTap: onClick, child: tag),
    );
  }
}

/// 空状态占位（v2：品牌浅底渐变徽章 + 大号图标）。
class MateEmpty extends StatelessWidget {
  /// 标题文字。
  final String title;

  /// 图标 name（默认 "list"）。
  final String icon;

  /// 可选的说明文字。
  final String? description;

  /// 可选的操作区组件。
  final Widget? action;

  const MateEmpty({
    super.key,
    required this.title,
    this.icon = 'list',
    this.description,
    this.action,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final metrics = MateTheme.metricsOf(context).feedback;
    final controls = metrics.controls;

    return Container(
      width: double.infinity,
      padding: EdgeInsets.all(controls.emptyStatePadding),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: metrics.emptyBadgeSize,
            height: metrics.emptyBadgeSize,
            decoration: BoxDecoration(
              gradient: LinearGradient(colors: colors.brandGradientSoft),
              borderRadius: BorderRadius.circular(metrics.emptyBadgeRadius),
            ),
            alignment: Alignment.center,
            child: MateIcon(
              name: icon,
              size: controls.emptyStateIconSize,
              tint: colors.brandHover,
            ),
          ),
          SizedBox(height: controls.emptyStateTitleSpacing),
          Text(
            title,
            style: typography.emptyStateTitle.copyWith(
              color: colors.textPrimary,
            ),
          ),
          if (description != null) ...[
            SizedBox(height: controls.emptyStateDescriptionSpacing),
            Text(
              description!,
              textAlign: TextAlign.center,
              style: typography.emptyStateDescription.copyWith(
                color: colors.textSecondary,
              ),
            ),
          ],
          if (action != null) ...[
            SizedBox(height: controls.emptyStateActionSpacing),
            action!,
          ],
        ],
      ),
    );
  }
}

/// 统计芯片（对标原 Vue `<MateStatChip icon count label>`）。
///
/// 模板：{icon} {count} {label}。
class MateStatChip extends StatelessWidget {
  /// 图标 name。
  final String icon;

  /// 数量。
  final int count;

  /// 标签文字。
  final String label;

  const MateStatChip({
    super.key,
    required this.icon,
    required this.count,
    required this.label,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final controls = MateTheme.metricsOf(context).feedback.controls;

    return Container(
      decoration: BoxDecoration(
        color: colors.bgFill,
        borderRadius: BorderRadius.circular(controls.statChipRadius),
      ),
      padding: EdgeInsets.symmetric(
        horizontal: controls.statChipHorizontalPadding,
        vertical: controls.statChipVerticalPadding,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          MateIcon(
            name: icon,
            size: controls.statChipIconSize,
            tint: colors.textSecondary,
          ),
          SizedBox(width: controls.statChipContentSpacing),
          Text(
            count.toString(),
            style: typography.statChipCount.copyWith(
              color: colors.textSecondary,
            ),
          ),
          SizedBox(width: controls.statChipContentSpacing),
          Text(
            label,
            style: typography.statChipLabel.copyWith(
              color: colors.textSecondary,
            ),
          ),
        ],
      ),
    );
  }
}

/// 分区标题（v2：18px semibold，无分割线）。
class MateSectionHeader extends StatelessWidget {
  /// 标题文字。
  final String text;

  /// 图标 name（可选，brand 色）。
  final String? icon;

  /// 可选的右侧尾部组件。
  final Widget? trailing;

  const MateSectionHeader({
    super.key,
    required this.text,
    this.icon,
    this.trailing,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final controls = MateTheme.metricsOf(context).feedback.controls;

    return Padding(
      padding: EdgeInsets.only(bottom: controls.sectionHeaderBottomPadding),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (icon != null) ...[
            MateIcon(
              name: icon!,
              size: controls.sectionHeaderIconSize,
              tint: colors.brand,
            ),
            SizedBox(width: controls.sectionHeaderContentSpacing),
          ],
          Text(
            text,
            style: typography.sectionHeader.copyWith(
              color: colors.textPrimary,
            ),
          ),
          if (trailing != null)
            Expanded(
              child: Align(alignment: Alignment.centerRight, child: trailing!),
            ),
        ],
      ),
    );
  }
}

/// 侧栏导航项（v2 放松版：46px 行高 + radius 8 + 18px 图标）。
///
/// hover bgFill；active bg=brandLighter color=brand medium。
class MateNavItem extends StatefulWidget {
  /// 标签文字。
  final String label;

  /// 点击回调。
  final VoidCallback? onClick;

  /// 图标 name（可选）。
  final String? icon;

  /// 是否激活。
  final bool active;

  /// 缩进层级。
  final int indent;

  /// 行高（默认 46）。
  final double? height;

  const MateNavItem({
    super.key,
    required this.label,
    this.onClick,
    this.icon,
    this.active = false,
    this.indent = 0,
    this.height,
  });

  @override
  State<MateNavItem> createState() => _MateNavItemState();
}

class _MateNavItemState extends State<MateNavItem> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final controls = MateTheme.metricsOf(context).feedback.controls;

    final textColor = widget.active ? colors.brand : colors.textPrimary;
    final iconColor = widget.active ? colors.brand : colors.textSecondary;
    final bgColor = widget.active
        ? colors.brandLighter
        : _hovered
            ? colors.bgFill
            : Colors.transparent;

    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onClick,
        behavior: HitTestBehavior.opaque,
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 150),
          height: widget.height ?? controls.navigationItemHeight,
          decoration: BoxDecoration(
            color: bgColor,
            borderRadius:
                BorderRadius.circular(controls.navigationItemRadius),
          ),
          padding: EdgeInsets.only(
            left: controls.navigationItemHorizontalPadding +
                controls.navigationItemIndentPerLevel * widget.indent,
            right: controls.navigationItemHorizontalPadding,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              if (widget.icon != null) ...[
                MateIcon(
                  name: widget.icon!,
                  size: controls.navigationItemIconSize,
                  tint: iconColor,
                ),
                SizedBox(width: controls.navigationItemContentSpacing),
              ],
              Expanded(
                child: Text(
                  widget.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: (widget.active
                          ? typography.activeNavigationItem
                          : typography.navigationItem)
                      .copyWith(color: textColor),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 导航分组标签（v2 设置页：12px semibold placeholder，上 20 下 6）。
class MateNavGroupLabel extends StatelessWidget {
  /// 分组标签文字。
  final String label;

  const MateNavGroupLabel({super.key, required this.label});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context).feedback;
    final controls = MateTheme.metricsOf(context).feedback.controls;

    return Padding(
      padding: EdgeInsets.only(
        left: controls.navigationGroupStartPadding,
        top: controls.navigationGroupTopPadding,
        bottom: controls.navigationGroupBottomPadding,
      ),
      child: Text(
        label,
        style: typography.navigationGroupLabel.copyWith(
          color: colors.textPlaceholder,
        ),
      ),
    );
  }
}
