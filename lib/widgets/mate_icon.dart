import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../app/theme/mate_theme.dart';

// =============================================================================
// MateIcon —— 矢量图标组件（对标 CMP components/MateIcon.kt）。
//
// 从 assets/icons/<name>.svg 加载 SVG，tint 用 SrcIn 混合（等价 SVG currentColor）。
// spin 为 1s 线性循环旋转，尊重系统「减少动态效果」设置。
// =============================================================================

/// 图标 name 注册表（对标 CMP MateIcons.NAMES 的 32 个 symbol）。
///
/// 与 assets/icons/ 下的 .svg 文件一一对应。
abstract final class MateIcons {
  /// 已注册的全部图标 name。
  static const List<String> names = [
    'cloud', 'local', 'folder', 'folder-open', 'file', 'file-text', 'image',
    'chart', 'search', 'refresh', 'transfer', 'settings', 'check', 'sync',
    'alert', 'clock', 'copy', 'info', 'lock', 'list', 'arrow', 'pause', 'play',
    'x', 'video', 'edit', 'archive', 'download', 'share', 'trash', 'github',
    'gitcode',
  ];

  /// SVG viewBox 内在尺寸（所有图标统一 24×24）。
  static const double viewBoxSize = 24;
}

/// 矢量图标。
///
/// 示例：
/// ```dart
/// MateIcon(name: 'cloud', size: 24)
/// MateIcon(name: 'sync', spin: true)
/// ```
class MateIcon extends StatelessWidget {
  /// 图标 name（不带扩展名），如 "cloud"、"folder-open"。
  final String name;

  /// 图标显示尺寸（dp），默认取主题 icon.defaultSize（16）。
  final double? size;

  /// 着色（null 取主题 defaultIconTint；遵循原 currentColor 语义）。
  final Color? tint;

  /// 是否旋转（1s 线性循环），用于同步中图标；减少动态时降级为静态。
  final bool spin;

  const MateIcon({
    super.key,
    required this.name,
    this.size,
    this.tint,
    this.spin = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context);
    final resolvedSize = size ?? metrics.icon.defaultSize;
    final resolvedTint = tint ?? colors.defaultIconTint;
    final effectiveSpin = spin && !MateTheme.reducedMotionOf(context);

    Widget icon = SvgPicture.asset(
      'assets/icons/$name.svg',
      width: resolvedSize,
      height: resolvedSize,
      // SrcIn：把图标非透明像素染成 tint（等价 currentColor）
      colorFilter: ColorFilter.mode(resolvedTint, BlendMode.srcIn),
      // 图标缺失/解析失败：渲染占位空盒，不抛异常
      placeholderBuilder: (_) => SizedBox(
        width: resolvedSize,
        height: resolvedSize,
      ),
    );

    if (effectiveSpin) {
      icon = _MateIconSpin(
        duration: Duration(milliseconds: metrics.icon.spinDurationMillis),
        child: icon,
      );
    }
    return icon;
  }
}

/// 旋转包装（1s 线性循环，Transform.rotate GPU 合成变换）。
class _MateIconSpin extends StatefulWidget {
  final Duration duration;
  final Widget child;

  const _MateIconSpin({required this.duration, required this.child});

  @override
  State<_MateIconSpin> createState() => _MateIconSpinState();
}

class _MateIconSpinState extends State<_MateIconSpin>
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
