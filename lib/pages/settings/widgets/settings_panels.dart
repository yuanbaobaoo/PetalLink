import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';

// =============================================================================
// 设置页共享构件（对标 CMP SettingsScreen.kt 的 SettingsPanel / GroupHeader /
// SettingRow / InfoRow）。
// =============================================================================

/// 设置面板（v2 .settings-panel：白卡 bgContainer，radius 10，0.5px 细边，
/// 默认 padding 4/24）。
class SettingsPanel extends StatelessWidget {
  /// 面板内边距（默认水平 24 / 垂直 4；账号卡/日志/关于用其他值覆盖）
  final EdgeInsets? contentPadding;

  /// 直接子项间距（默认 0；日志/关于面板用 14）
  final double? contentSpacing;

  /// 子项列表
  final List<Widget> children;

  const SettingsPanel({
    super.key,
    this.contentPadding,
    this.contentSpacing,
    required this.children,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;

    final padding = contentPadding ??
        EdgeInsets.symmetric(
          horizontal: metrics.panelHorizontalPadding,
          vertical: metrics.panelVerticalPadding,
        );
    final spacing = contentSpacing ?? metrics.panelDefaultContentSpacing;

    return Container(
      width: double.infinity,
      padding: padding,
      decoration: BoxDecoration(
        color: colors.bgContainer,
        borderRadius: BorderRadius.circular(metrics.panelRadius),
        border: Border.all(
          color: colors.border,
          width: metrics.panelBorderWidth,
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (var i = 0; i < children.length; i++) ...[
            if (i > 0 && spacing > 0) SizedBox(height: spacing),
            children[i],
          ],
        ],
      ),
    );
  }
}

/// 分组标题（v2 .group-header：12px semibold secondary，无分割线；
/// 面板内首个上 12，其余上 20）。
class SettingsGroupHeader extends StatelessWidget {
  /// 标题文字
  final String label;

  /// 是否为面板内首个分组
  final bool first;

  const SettingsGroupHeader(this.label, {super.key, this.first = false});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Container(
      width: double.infinity,
      padding: EdgeInsets.only(
        top: first ? metrics.firstGroupTopPadding : metrics.groupTopPadding,
        bottom: metrics.groupBottomPadding,
      ),
      child: Text(
        label,
        style: typography.groupHeader.copyWith(color: colors.textSecondary),
      ),
    );
  }
}

/// 设置行（v2 .setting-row：左侧 label+desc 占满剩余宽度，右侧 control；
/// 非末行底 0.5px 细边）。
class SettingRow extends StatelessWidget {
  /// 设置项标题
  final String label;

  /// 设置项说明
  final String desc;

  /// 右侧控件
  final Widget control;

  /// 是否显示底部分隔线（末行传 false）
  final bool showDivider;

  const SettingRow({
    super.key,
    required this.label,
    required this.desc,
    required this.control,
    this.showDivider = true,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return SizedBox(
      width: double.infinity,
      child: Column(
        children: [
          Padding(
            padding: EdgeInsets.symmetric(
              vertical: metrics.settingRowVerticalPadding,
            ),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        label,
                        style: typography.optionTitle.copyWith(
                          color: colors.textPrimary,
                        ),
                      ),
                      Padding(
                        padding: EdgeInsets.only(
                          top: metrics.settingDescriptionTopPadding,
                        ),
                        child: Text(
                          desc,
                          style: typography.optionDescription.copyWith(
                            color: colors.textSecondary,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
                SizedBox(width: metrics.settingRowContentSpacing),
                control,
              ],
            ),
          ),
          if (showDivider)
            Container(
              width: double.infinity,
              height: metrics.settingRowDividerWidth,
              color: colors.border,
            ),
        ],
      ),
    );
  }
}

/// 信息行（label 96px + value flex，底 0.5px border；对标 CMP InfoRow）。
class SettingsInfoRow extends StatelessWidget {
  /// 标签
  final String label;

  /// 值
  final String value;

  /// 值是否用等宽字体（如 OpenID）
  final bool mono;

  const SettingsInfoRow({
    super.key,
    required this.label,
    required this.value,
    this.mono = false,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Container(
      width: double.infinity,
      padding: EdgeInsets.symmetric(
        vertical: metrics.detailRowVerticalPadding,
      ),
      child: Column(
        children: [
          Row(
            children: [
              SizedBox(
                width: metrics.detailLabelWidth,
                child: Text(
                  label,
                  style: typography.detailLabel.copyWith(
                    color: colors.textSecondary,
                  ),
                ),
              ),
              Expanded(
                child: Text(
                  value,
                  style: typography.detailValue.copyWith(
                    color: colors.textPrimary,
                    fontFamily: mono ? 'Menlo' : null,
                    fontFamilyFallback: mono ? const ['monospace'] : null,
                  ),
                ),
              ),
            ],
          ),
          SizedBox(height: metrics.detailContentSpacing),
          Container(
            width: double.infinity,
            height: metrics.detailDividerWidth,
            color: colors.border,
          ),
        ],
      ),
    );
  }
}
