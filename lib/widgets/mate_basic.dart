import 'package:flutter/material.dart';

import '../app/theme/mate_theme.dart';
import 'mate_icon.dart';

// =============================================================================
// MateBasic —— 基础品牌组件（对标 CMP mate/MateBasic.kt v2）。
//
// Logo（assets/logo.png，失败回退品牌渐变 + 云朵图标）、脚手架、悬停探测器、
// 竖分隔线、底部分隔线容器。
// =============================================================================

/// 应用 Logo（v2：直接使用真实 logo.png，品牌蓝 squircle）。
///
/// container 模式显示 64×64 大图（登录页用），不再包白色容器。
/// text 非空时在右侧显示「PetalLink」文字（semibold）。
///
/// 示例：
/// ```dart
/// MateAppLogo(size: 26, text: 'PetalLink')
/// MateAppLogo(container: true) // 登录页 64×64
/// ```
class MateAppLogo extends StatelessWidget {
  /// 图标尺寸（dp），默认 26。
  final double? size;

  /// 附加文字，空串则隐藏。
  final String text;

  /// 是否 64×64 大图模式（登录页用）。
  final bool container;

  const MateAppLogo({
    super.key,
    this.size,
    this.text = 'PetalLink',
    this.container = false,
  });

  @override
  Widget build(BuildContext context) {
    final basic = MateTheme.metricsOf(context).basic;
    if (container) {
      // container 模式：64×64 真实 logo（自带品牌蓝 squircle 底）
      return _LogoImage(size: basic.largeLogoSize);
    }
    final resolvedSize = size ?? basic.compactLogoSize;
    return Row(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        _LogoImage(size: resolvedSize),
        if (text.isNotEmpty) ...[
          SizedBox(width: basic.compactLogoTextSpacing),
          Text(
            text,
            style: MateTheme.typographyOf(context)
                .brand
                .compactLogoLabel
                .copyWith(color: MateTheme.colorsOf(context).appLogoCompactText),
          ),
        ],
      ],
    );
  }
}

/// Logo + 文字组合（对标原 Vue `<MateLogoWithText height>`）。
///
/// 内部复用 [_LogoImage]，PNG 加载失败回退纯文字 + 渐变方块。
class MateLogoWithText extends StatelessWidget {
  /// 整体高度（dp），默认 32；图标 = height。
  final double? height;

  const MateLogoWithText({super.key, this.height});

  @override
  Widget build(BuildContext context) {
    final basic = MateTheme.metricsOf(context).basic;
    final resolvedHeight = height ?? basic.fullLogoHeight;
    return Row(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        _LogoImage(size: resolvedHeight),
        SizedBox(width: basic.fullLogoTextSpacing),
        Text(
          'PetalLink',
          style: MateTheme.typographyOf(context)
              .brand
              .fullLogoLabel
              .copyWith(color: MateTheme.colorsOf(context).appLogoFullText),
        ),
      ],
    );
  }
}

/// 从 assets/logo.png 加载真实 Logo（对标 CMP resources/assets/logo.png）。
///
/// PNG 缺失/解码失败时回退品牌渐变圆角方块 + 云朵图标。
class _LogoImage extends StatelessWidget {
  final double size;

  const _LogoImage({required this.size});

  @override
  Widget build(BuildContext context) {
    final radius = BorderRadius.circular(size * 0.225);
    return ClipRRect(
      borderRadius: radius,
      child: Image.asset(
        'assets/logo.png',
        width: size,
        height: size,
        fit: BoxFit.contain,
        errorBuilder: (context, _, _) {
          final colors = MateTheme.colorsOf(context);
          // 回退：品牌渐变圆角方块 + 云朵图标（仅在 logo.png 加载失败时出现）
          return Container(
            width: size,
            height: size,
            decoration: BoxDecoration(
              gradient: LinearGradient(colors: colors.brandGradient),
              borderRadius: radius,
            ),
            alignment: Alignment.center,
            child: MateIcon(
              name: 'cloud',
              size: size * 0.6,
              tint: colors.appLogoCompactIcon,
            ),
          );
        },
      ),
    );
  }
}

/// 页面脚手架（对标原 Vue `<MateScaffold flush>`）。
///
/// 全屏 column 容器；默认 bg=bgPage，flush 时 bg=bgContainer。
class MateScaffold extends StatelessWidget {
  /// 是否平铺（bg 转 bgContainer）。
  final bool flush;

  /// 页面内容。
  final Widget child;

  const MateScaffold({super.key, this.flush = false, required this.child});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    return Container(
      width: double.infinity,
      height: double.infinity,
      color: flush ? colors.bgContainer : colors.bgPage,
      child: child,
    );
  }
}

/// 悬停探测器（对标原 Vue `<MateHover>` scoped slot）。
///
/// 把 hovered 状态暴露给 builder。
class MateHover extends StatefulWidget {
  /// 接收 hovered 的内容构造器。
  final Widget Function(bool hovered) builder;

  const MateHover({super.key, required this.builder});

  @override
  State<MateHover> createState() => _MateHoverState();
}

class _MateHoverState extends State<MateHover> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: widget.builder(_hovered),
    );
  }
}

/// 竖分隔线（对标原 Vue `<MateVerticalSeparator height>`）。w 1px，bg border。
class MateVerticalSeparator extends StatelessWidget {
  /// 线高（默认 20dp）。
  final double? height;

  const MateVerticalSeparator({super.key, this.height});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final basic = MateTheme.metricsOf(context).basic;
    return Container(
      width: basic.verticalSeparatorWidth,
      height: height ?? basic.verticalSeparatorHeight,
      color: colors.border,
    );
  }
}

/// 底部分隔线容器（对标原 Vue `<MateBottomDivider>`）。border-bottom 0.5px。
class MateBottomDivider extends StatelessWidget {
  /// 被包裹的内容。
  final Widget child;

  const MateBottomDivider({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final basic = MateTheme.metricsOf(context).basic;
    return Container(
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(
            color: colors.border,
            width: basic.bottomBorderThickness,
          ),
        ),
      ),
      child: child,
    );
  }
}
