import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 高级设置分区（对标 Tauri SettingsPage.vue advanced 分支）。
///
/// 「通用」（开机自启动 + 显示托盘图标）+「OAUTH」（回调端口 + 回调地址
/// 提示横幅）+「维护」（清空缓存并重启）。
class AdvancedSection extends StatelessWidget {
  /// 页面控制器
  final SettingsController notifier;

  /// 当前状态
  final SettingsState state;

  const AdvancedSection({
    super.key,
    required this.notifier,
    required this.state,
  });

  @override
  Widget build(BuildContext context) {
    final metrics = MateTheme.metricsOf(context).settings;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '高级设置', icon: 'settings'),
        SettingsPanel(
          children: [
            const SettingsGroupHeader('通用', first: true),
            SettingRow(
              label: '开机自启动',
              desc: '开机登录后自动在后台启动（仅菜单栏图标，不显示主窗口）。关闭后需手动打开 App。',
              control: MateSwitch(
                checked: state.launchEnabled,
                onChanged: (v) => notifier.onLaunchAtLoginChange(v),
              ),
            ),
            SettingRow(
              label: '显示托盘图标',
              desc: '在菜单栏显示 PetalLink 图标（后台同步入口）。关闭后 App 仍在后台运行，此时可通过 Cmd+Q 完全退出。',
              control: MateSwitch(
                checked: state.trayVisible,
                onChanged: (v) => notifier.onTrayVisibleChange(v),
              ),
            ),

            const SettingsGroupHeader('OAuth'),
            SettingRow(
              label: 'OAuth 回调端口',
              desc: '本地 HTTP 回调服务器监听端口。修改后需与 AGC 后台 redirect_uri 保持一致。',
              control: MateNumberField(
                value: state.oauthPort,
                onChanged: notifier.setOauthPort,
                min: 1,
                max: 65535,
              ),
            ),
            Padding(
              padding: EdgeInsets.only(
                top: metrics.oauthBannerTopPadding,
                bottom: metrics.oauthBannerBottomPadding,
              ),
              child: const MateInfoBanner(
                message: '回调地址固定为 http://127.0.0.1:<端口>/oauth/callback，修改端口后请同步更新 AGC 后台配置。',
                variant: MateBannerVariant.info,
              ),
            ),

            const SettingsGroupHeader('维护'),
            SettingRow(
              label: '清空缓存并重启',
              desc: '清除登录状态、同步数据库、同步快照与配置文件，然后重启 App。适用于排查同步异常或切换账号时使用。',
              showDivider: false,
              control: MateButton(
                label: '清空',
                icon: 'trash',
                danger: true,
                onClick: _onClearCache,
              ),
            ),
          ],
        ),
      ],
    );
  }

  /// 清空缓存确认对话框（危险操作二次确认，对齐 CMP confirmDialog）
  void _onClearCache() {
    MateDialog.confirm(
      const MateDialogOptions(
        title: '清空缓存并重启',
        content: '将清除登录状态、同步数据库、同步快照与配置文件，并重启 App。云盘文件不受影响，但此操作不可撤销，确定继续？',
        confirmText: '清空并重启',
        danger: true,
        titleIcon: 'trash',
      ),
      (confirmed) {
        if (confirmed) notifier.onClearCache();
      },
    );
  }
}
