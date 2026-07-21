import 'package:flutter/material.dart';

import 'mate_semantic_colors.dart';
import 'mate_skin.dart';
import 'mate_typography.dart';
import 'mate_metrics.dart';

export 'mate_semantic_colors.dart';
export 'mate_skin.dart';
export 'mate_typography.dart';
export 'mate_metrics.dart';

// =============================================================================
// Layer 3: 组合入口 —— MateTheme（InheritedWidget）提供皮肤访问，
// MateLinkTheme（StatelessWidget）自动检测平台亮度并注入对应皮肤。
// =============================================================================

/// 应用主题 InheritedWidget。
///
/// 通过静态方法 [colorsOf]、[typographyOf]、[metricsOf]、[skinOf]
/// 从子树中获取当前皮肤的各个部分。
class MateTheme extends InheritedWidget {
  /// 当前皮肤。
  final MateSkin skin;

  const MateTheme({
    super.key,
    required this.skin,
    required super.child,
  });

  /// 从上下文获取当前语义颜色。
  static MateSemanticColors colorsOf(BuildContext context) {
    return _of(context).skin.colors;
  }

  /// 从上下文获取当前排版 token。
  static MateTypography typographyOf(BuildContext context) {
    return _of(context).skin.typography;
  }

  /// 从上下文获取当前度量 token。
  static MateMetrics metricsOf(BuildContext context) {
    return _of(context).skin.metrics;
  }

  /// 从上下文获取当前完整皮肤。
  static MateSkin skinOf(BuildContext context) {
    return _of(context).skin;
  }

  /// 当前环境是否减少动态效果（对齐 CMP LOCAL_REDUCED_MOTION）。
  ///
  /// 跟随系统「减少动态效果」辅助功能设置（MediaQuery.disableAnimations），
  /// 旋转/循环动画组件应据此降级为静态呈现。
  static bool reducedMotionOf(BuildContext context) {
    return MediaQuery.maybeOf(context)?.disableAnimations ?? false;
  }

  /// 从上下文获取 MateTheme 实例。
  static MateTheme _of(BuildContext context) {
    final result = context.dependOnInheritedWidgetOfExactType<MateTheme>();
    assert(result != null, 'No MateTheme found in context. Wrap your app with MateLinkTheme.');
    return result!;
  }

  @override
  bool updateShouldNotify(covariant MateTheme oldWidget) {
    return skin != oldWidget.skin;
  }
}

/// 应用主题入口组件。
///
/// 自动检测平台亮度（[MediaQuery.platformBrightness]），选择对应的
/// [MateSkin.light] 或 [MateSkin.dark]，并注入 [MateTheme]。
/// 同时包裹标准 Flutter [Theme] 以使用品牌色配置 Material 组件。
class MateLinkTheme extends StatelessWidget {
  /// 子组件。
  final Widget child;

  /// 可选的浅色 skin 覆盖。
  final MateSkin? lightSkin;

  /// 可选的深色 skin 覆盖。
  final MateSkin? darkSkin;

  const MateLinkTheme({
    super.key,
    required this.child,
    this.lightSkin,
    this.darkSkin,
  });

  @override
  Widget build(BuildContext context) {
    final brightness = MediaQuery.of(context).platformBrightness;
    final isDark = brightness == Brightness.dark;
    final skin = isDark
        ? (darkSkin ?? MateSkin.dark)
        : (lightSkin ?? MateSkin.light);

    return MateTheme(
      skin: skin,
      child: Theme(
        data: _buildThemeData(skin.colors, isDark),
        child: child,
      ),
    );
  }

  /// 使用品牌色构建标准 Flutter [ThemeData]。
  ThemeData _buildThemeData(MateSemanticColors colors, bool isDark) {
    final colorScheme = ColorScheme(
      brightness: isDark ? Brightness.dark : Brightness.light,
      primary: colors.brand,
      onPrimary: colors.onPrimary,
      secondary: colors.brandHover,
      onSecondary: colors.onPrimary,
      surface: colors.bgContainer,
      error: colors.error,
      onError: colors.onPrimary,
      onSurface: colors.textPrimary,
    );

    return ThemeData(
      colorScheme: colorScheme,
      primaryColor: colors.brand,
      scaffoldBackgroundColor: colors.bgPage,
      cardColor: colors.bgContainer,
      dividerColor: colors.divider,
      hintColor: colors.textPlaceholder,
      disabledColor: colors.textSecondary.withAlpha(128),
      appBarTheme: AppBarTheme(
        backgroundColor: colors.bgContainer,
        foregroundColor: colors.textPrimary,
        elevation: 0,
      ),
      scrollbarTheme: ScrollbarThemeData(
        thumbColor: WidgetStateProperty.all(colors.scrollbarThumb),
      ),
      brightness: isDark ? Brightness.dark : Brightness.light,
    );
  }
}
