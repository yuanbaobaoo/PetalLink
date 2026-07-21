import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/config_entry.dart';
import 'package:petal_link/pages/settings/controller/settings_controller.dart';
import 'package:petal_link/pages/settings/widgets/settings_panels.dart';
import 'package:petal_link/widgets/index.dart';

/// 高级设置分区（对标 CMP SettingsScreen.kt ADVANCED 分支）。
///
/// 「通用」（开机自启动）+「OAuth」（回调端口 + 回调地址提示横幅）
/// +「维护」（配置导出/导入 + 清空缓存并重启）。
///
/// 注：CMP SettingsScreen.kt 声明了 onExportConfig/onImportConfig 入参但
/// 未渲染对应按钮；此处按任务要求将其放入「维护」分组（清缓存之前）。
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
              label: '导出配置',
              desc: '将当前配置（不含 token）导出为 JSON 文件，便于备份或迁移。',
              control: MateButton(
                label: '导出',
                variant: MateButtonVariant.soft,
                icon: 'download',
                onClick: () => _onExport(context),
              ),
            ),
            SettingRow(
              label: '导入配置',
              desc: '从 JSON 文件导入配置，经确认后覆盖当前配置并立即应用。',
              control: MateButton(
                label: '导入',
                variant: MateButtonVariant.soft,
                icon: 'folder-open',
                onClick: () => _onImport(context),
              ),
            ),
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

  /// 导出配置：成功 toast（对齐 CMP exportConfig 的 syncStatus 提示）
  Future<void> _onExport(BuildContext context) async {
    final ok = await notifier.onExportConfig();
    if (ok) {
      MateToast.show('配置已导出', variant: MateToastVariant.success);
    }
  }

  /// 导入配置：选取解析 → 确认对话框 → 应用（对齐 CMP importConfig 流程）
  Future<void> _onImport(BuildContext context) async {
    final config = await notifier.pickImportConfig();
    if (config == null) return; // 用户取消或解析失败（错误已入 state.errors）
    if (!context.mounted) return;
    _confirmImport(config);
  }

  /// 导入确认对话框（导入将覆盖当前配置并立即应用）
  void _confirmImport(AppConfig config) {
    MateDialog.confirm(
      const MateDialogOptions(
        title: '导入配置',
        content: '导入将覆盖当前配置并立即应用（含同步目录设置），是否继续？',
        confirmText: '导入并应用',
        danger: true,
        titleIcon: 'info',
      ),
      (confirmed) async {
        if (!confirmed) return;
        final ok = await notifier.applyImportedConfig(config);
        if (ok) {
          MateToast.show('配置已导入', variant: MateToastVariant.success);
        }
      },
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
