import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/about_section.dart';
import 'package:petal_link/pages/settings/widgets/account_section.dart';
import 'package:petal_link/pages/settings/widgets/advanced_section.dart';
import 'package:petal_link/pages/settings/widgets/logs_section.dart';
import 'package:petal_link/pages/settings/widgets/settings_nav.dart';
import 'package:petal_link/pages/settings/widgets/sync_dir_section.dart';
import 'package:petal_link/pages/settings/widgets/transfer_section.dart';
import 'package:petal_link/widgets/index.dart';

/// 设置页（对标 CMP SettingsScreen.kt / design/v2/06-settings.html）。
///
/// 双栏布局：左导航 240px（「通用/其他」分组）+ 右设置区（bgPage，scroll）。
/// Footer（64px）：保存/重置 + 错误/已保存状态，仅同步目录/传输设置/
/// 高级设置三个 Tab 显示。
class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  /// 页面控制器（xe-cloud 惯例：页面持有，dispose 时释放）
  final SettingsController notifier = Get.put(SettingsController());

  /// 保存成功 toast 副作用（ever Worker：saved 由 false→true 时提示）
  late final Worker _savedWorker;

  /// 上一次的 saved 值（避免载入/重置路径误触发 toast）
  bool _wasSaved = false;

  @override
  void initState() {
    super.initState();
    _wasSaved = notifier.state.value.saved;
    _savedWorker = ever(notifier.state, (SettingsState s) {
      if (s.saved && !_wasSaved) {
        MateToast.show('配置已保存', variant: MateToastVariant.success);
      }
      _wasSaved = s.saved;
    });
  }

  @override
  void dispose() {
    _savedWorker.dispose();
    Get.delete<SettingsController>();
    super.dispose();
  }

  /// 仅「通用」分组三个 Tab 显示保存底栏（对标 CMP showFooter）
  bool get _showFooter => switch (notifier.state.value.tab) {
        SettingsTab.syncDir || SettingsTab.transfer || SettingsTab.advanced =>
          true,
        _ => false,
      };

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Scaffold(
      body: Column(
        children: [
          // AppBar（56px，bgContainer，返回箭头 rotate 180°）
          Container(
            height: metrics.headerHeight,
            color: colors.bgContainer,
            padding: EdgeInsets.symmetric(
              horizontal: metrics.headerHorizontalPadding,
            ),
            child: Row(
              children: [
                Transform.rotate(
                  angle: 3.141592653589793, // 180°：arrow 图标朝左
                  child: MateButton(
                    variant: MateButtonVariant.icon,
                    icon: 'arrow',
                    onClick: () => Get.back(),
                  ),
                ),
                SizedBox(width: metrics.headerContentSpacing),
                Text(
                  '设置',
                  style: typography.pageTitle.copyWith(
                    color: colors.textPrimary,
                  ),
                ),
              ],
            ),
          ),

          // Body：左导航 + 0.5px 细边 + 右设置区
          Expanded(
            child: Obx(() {
              final state = notifier.state.value;
              return Row(
                children: [
                  SettingsNav(
                    current: state.tab,
                    onSelect: notifier.switchTab,
                  ),
                  Container(
                    width: metrics.navigationBorderWidth,
                    color: colors.border,
                  ),
                  Expanded(
                    child: Column(
                      children: [
                        Expanded(child: _buildContent(state)),
                        if (_showFooter) _buildFooter(state),
                      ],
                    ),
                  ),
                ],
              );
            }),
          ),
        ],
      ),
    );
  }

  /// 右设置区（bgPage，scroll，padding 28/32；内容包白色 settings-panel）
  Widget _buildContent(SettingsState state) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;

    return Container(
      color: colors.bgPage,
      child: SingleChildScrollView(
        padding: EdgeInsets.symmetric(
          horizontal: metrics.bodyHorizontalPadding,
          vertical: metrics.bodyVerticalPadding,
        ),
        child: switch (state.tab) {
          SettingsTab.syncDir =>
            SyncDirSection(notifier: notifier, state: state),
          SettingsTab.transfer =>
            TransferSection(notifier: notifier, state: state),
          SettingsTab.advanced =>
            AdvancedSection(notifier: notifier, state: state),
          SettingsTab.account =>
            AccountSection(notifier: notifier, state: state),
          SettingsTab.logs => const LogsSection(),
          SettingsTab.about =>
            AboutSection(notifier: notifier, state: state),
        },
      ),
    );
  }

  /// 保存底栏（64px，顶 0.5px 细边：保存/重置 + 错误/已保存状态）
  Widget _buildFooter(SettingsState state) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    return Column(
      children: [
        Container(
          width: double.infinity,
          height: metrics.footerBorderWidth,
          color: colors.border,
        ),
        Container(
          height: metrics.footerHeight,
          color: colors.bgContainer,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.footerHorizontalPadding,
          ),
          child: Row(
            children: [
              MateButton(
                label: state.saved ? '已保存' : '保存设置',
                icon: 'check',
                onClick: notifier.onSave,
                disabled: state.saved,
              ),
              SizedBox(width: metrics.footerActionSpacing),
              MateButton(
                label: '重置默认',
                variant: MateButtonVariant.iconText,
                onClick: notifier.onReset,
              ),
              const Spacer(),
              if (state.errors.isNotEmpty)
                Flexible(
                  child: Text(
                    '⚠️ ${state.errors.first}',
                    style: typography.validationError.copyWith(
                      color: colors.error,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                )
              else if (state.saved)
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Container(
                      width: metrics.savedIndicatorSize,
                      height: metrics.savedIndicatorSize,
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: colors.success,
                      ),
                    ),
                    SizedBox(width: metrics.savedIndicatorSpacing),
                    Text(
                      '配置已保存',
                      style: typography.saveSuccess.copyWith(
                        color: colors.success,
                      ),
                    ),
                  ],
                ),
            ],
          ),
        ),
      ],
    );
  }
}
