import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';

// =============================================================================
// MateDivider —— 分隔线（对标 CMP mate/MateDivider.kt）。
// =============================================================================

/// 水平 0.5px 分隔线（对标原 Vue `border-bottom/top: 0.5px solid var(--border)`）。
///
/// 颜色取当前主题 border，可用 [color] 覆盖。
class MateHDivider extends StatelessWidget {
  /// 线宽（默认 0.5dp）。
  final double? thickness;

  /// 自定义颜色（null 则使用主题 border）。
  final Color? color;

  const MateHDivider({super.key, this.thickness, this.color});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final divider = MateTheme.metricsOf(context).divider;
    return Container(
      width: double.infinity,
      height: thickness ?? divider.horizontalThickness,
      color: color ?? colors.border,
    );
  }
}

/// 垂直分隔线（对标原 Vue 1px 竖线，如 app-bar__sep、tp-stats__sep）。
class MateVDivider extends StatelessWidget {
  /// 线高（默认 24dp）。
  final double? height;

  /// 自定义颜色（null 则使用主题 border）。
  final Color? color;

  const MateVDivider({super.key, this.height, this.color});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final divider = MateTheme.metricsOf(context).divider;
    return Container(
      width: divider.verticalWidth,
      height: height ?? divider.verticalHeight,
      color: color ?? colors.border,
    );
  }
}
