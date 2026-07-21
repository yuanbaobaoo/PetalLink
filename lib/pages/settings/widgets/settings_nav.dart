import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/widgets/index.dart';

/// 设置页左侧导航（对标 CMP SettingsScreen.kt 的 settings-nav）。
///
/// 宽 240，「通用」（同步目录/传输设置/高级设置）+「其他」
/// （账号管理/日志查看/关于）两组 [MateNavItem]，全部子项间距 6。
class SettingsNav extends StatelessWidget {
  /// 当前选中 Tab
  final SettingsTab current;

  /// 切换回调
  final ValueChanged<SettingsTab> onSelect;

  const SettingsNav({
    super.key,
    required this.current,
    required this.onSelect,
  });

  /// 「通用」分组
  static const _generalTabs = [
    SettingsTab.syncDir,
    SettingsTab.transfer,
    SettingsTab.advanced,
  ];

  /// 「其他」分组
  static const _otherTabs = [
    SettingsTab.account,
    SettingsTab.logs,
    SettingsTab.about,
  ];

  /// Tab → 显示名与图标（对标 CMP SettingsTab(label, icon)）
  static (String, String) _meta(SettingsTab tab) {
    return switch (tab) {
      SettingsTab.syncDir => ('同步目录', 'folder'),
      SettingsTab.transfer => ('传输设置', 'transfer'),
      SettingsTab.advanced => ('高级设置', 'settings'),
      SettingsTab.account => ('账号管理', 'info'),
      SettingsTab.logs => ('日志查看', 'list'),
      SettingsTab.about => ('关于', 'cloud'),
    };
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;

    // 子项序列（分组标签 + 导航项），对标 CMP spacedBy(6) 全项等距
    final entries = <Widget>[
      const MateNavGroupLabel(label: '通用'),
      for (final tab in _generalTabs) _navItem(tab),
      const MateNavGroupLabel(label: '其他'),
      for (final tab in _otherTabs) _navItem(tab),
    ];

    return Container(
      width: metrics.navigationWidth,
      color: colors.bgPage,
      padding: EdgeInsets.symmetric(
        horizontal: metrics.navigationHorizontalPadding,
        vertical: metrics.navigationVerticalPadding,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (var i = 0; i < entries.length; i++) ...[
            if (i > 0) SizedBox(height: metrics.navigationItemSpacing),
            entries[i],
          ],
        ],
      ),
    );
  }

  Widget _navItem(SettingsTab tab) {
    final (label, icon) = _meta(tab);
    return MateNavItem(
      label: label,
      icon: icon,
      active: current == tab,
      onClick: () => onSelect(tab),
    );
  }
}
