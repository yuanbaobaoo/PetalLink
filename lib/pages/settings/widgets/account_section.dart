import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 账号管理分区（对标 CMP SettingsScreen.kt AccountSection）。
///
/// 头像卡片（56×56 品牌渐变头像 + 用户名）+ 信息面板
/// （账号信息 / 存储配额 / 退出登录）。
class AccountSection extends StatelessWidget {
  /// 页面控制器
  final SettingsController notifier;

  /// 当前状态
  final SettingsState state;

  const AccountSection({
    super.key,
    required this.notifier,
    required this.state,
  });

  /// 文件大小格式化（对齐 design/v2/06-settings.html：GB 保留 1 位小数）
  static String _formatFileSize(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1048576) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    if (bytes < 1073741824) {
      return '${(bytes / 1048576).toStringAsFixed(1)} MB';
    }
    return '${(bytes / 1073741824).toStringAsFixed(1)} GB';
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).settings;
    final typography = MateTheme.typographyOf(context).settings;

    final userInfo = state.userInfo;
    final userLabel = userInfo?.primaryLabel ?? '未获取到';
    final quotaUsed = state.quotaUsed;
    final quotaTotal = state.quotaTotal;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const MateSectionHeader(text: '账号管理', icon: 'info'),

        // 头像卡片（padding 16/24）
        SettingsPanel(
          contentPadding: EdgeInsets.symmetric(
            horizontal: metrics.accountPanelHorizontalPadding,
            vertical: metrics.accountPanelVerticalPadding,
          ),
          children: [
            Row(
              children: [
                Container(
                  width: metrics.accountAvatarSize,
                  height: metrics.accountAvatarSize,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    gradient: LinearGradient(colors: colors.brandGradient),
                  ),
                  alignment: Alignment.center,
                  child: Text(
                    userInfo?.initial ?? '华',
                    style: typography.accountAvatar.copyWith(
                      color: colors.settingsAccountAvatarText,
                    ),
                  ),
                ),
                SizedBox(width: metrics.accountContentSpacing),
                Expanded(
                  child: Text(
                    userLabel,
                    style: typography.accountName.copyWith(
                      color: colors.textPrimary,
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ],
        ),

        SizedBox(height: metrics.accountSectionSpacing),

        // 信息面板
        SettingsPanel(
          children: [
            const SettingsGroupHeader('账号信息', first: true),
            SettingsInfoRow(label: '显示名', value: userInfo?.displayName ?? '—'),
            SettingsInfoRow(label: '手机号', value: userInfo?.mobile ?? '未授权'),
            SettingsInfoRow(label: '邮箱', value: userInfo?.email ?? '未授权'),
            SettingsInfoRow(
              label: 'OpenID',
              value: userInfo?.openId ?? '—',
              mono: true,
            ),

            const SettingsGroupHeader('存储配额'),
            SettingsInfoRow(
              label: '已用空间',
              value: quotaUsed != null ? _formatFileSize(quotaUsed) : '—',
            ),
            SettingsInfoRow(
              label: '总容量',
              value: quotaTotal != null && quotaTotal > 0
                  ? _formatFileSize(quotaTotal)
                  : '—',
            ),
            SettingsInfoRow(
              label: '剩余空间',
              value: quotaTotal != null && quotaTotal > 0 && quotaUsed != null
                  ? _formatFileSize(quotaTotal - quotaUsed)
                  : '—',
            ),

            const SettingsGroupHeader('账号操作'),
            SettingRow(
              label: '退出登录',
              desc: '清除本地 token 并返回登录页。后台进程仍会继续，可从菜单栏彻底退出。',
              showDivider: false,
              control: MateButton(
                label: '退出登录',
                icon: 'x',
                danger: true,
                onClick: _onLogout,
              ),
            ),
          ],
        ),
      ],
    );
  }

  /// 退出登录确认对话框（危险操作二次确认，对齐 CMP confirmDialog）
  void _onLogout() {
    MateDialog.confirm(
      const MateDialogOptions(
        title: '退出登录',
        content: '将清除本地 token 并返回登录页，后台同步会停止。确定退出登录？',
        confirmText: '退出登录',
        danger: true,
        titleIcon: 'x',
      ),
      (confirmed) {
        if (confirmed) notifier.onLogout();
      },
    );
  }
}
