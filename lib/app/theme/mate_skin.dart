import 'mate_semantic_colors.dart';
import 'mate_typography.dart';
import 'mate_metrics.dart';

// =============================================================================
// Layer 2: 皮肤包 —— 将一套完整的语义颜色、排版、度量绑定为不可变皮肤。
// MateSkin.light 和 MateSkin.dark 是预置的明暗皮肤实例（对标 CMP PetalSkin）。
// =============================================================================

/// PetalLink 皮肤。
///
/// 将语义颜色、排版 token、度量 token 组合为一个不可变的皮肤包。
/// 使用者通过 [MateTheme.skinOf] 获取当前皮肤。
class MateSkin {
  /// 皮肤名称。
  final String name;

  /// 语义颜色。
  final MateSemanticColors colors;

  /// 排版 token。
  final MateTypography typography;

  /// 度量 token。
  final MateMetrics metrics;

  const MateSkin({
    required this.name,
    required this.colors,
    required this.typography,
    required this.metrics,
  });

  /// 浅色皮肤。
  static final MateSkin light = MateSkin(
    name: 'light',
    colors: MateSemanticColors.light,
    typography: MateTypography.standard(),
    metrics: MateMetrics.standard(),
  );

  /// 深色皮肤。
  static final MateSkin dark = MateSkin(
    name: 'dark',
    colors: MateSemanticColors.dark,
    typography: MateTypography.standard(),
    metrics: MateMetrics.standard(),
  );
}
