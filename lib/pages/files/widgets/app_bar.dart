import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

/// 主区 AppBar（对标 CMP MainScreen.kt 工具行；v2：design/02-main.html .toolbar）。
///
/// 高 64px，padding 0/20，gap 8：
/// 左侧搜索框（flex:1，max-width 420px）+ 清除按钮；
/// 右侧「同步索引」primary 按钮（mountConfigured 时）、「传输队列」soft 按钮、
/// 「Finder」iconText（mountConfigured 时）、设置图标按钮。
class FilesAppBar extends StatelessWidget {
  /// 搜索框控制器
  final TextEditingController searchController;

  /// 当前搜索关键词（控制清除按钮显隐）
  final String searchKeyword;

  /// 挂载目录是否已配置（控制 同步索引/Finder 显隐）
  final bool mountConfigured;

  /// 是否正在索引（同步索引按钮 loading + disabled）
  final bool isIndexing;

  /// 搜索输入变化（清空时已提交态自动退出搜索）
  final ValueChanged<String> onSearchChanged;

  /// 回车提交搜索（仅回车触发远端搜索，对标原 Vue @submit）
  final ValueChanged<String> onSearchSubmit;

  /// 点击清除搜索按钮
  final VoidCallback onSearchClear;

  /// 点击「同步索引」
  final VoidCallback onRefresh;

  /// 点击「传输队列」
  final VoidCallback onToggleTransfer;

  /// 点击「Finder」
  final VoidCallback onOpenFinder;

  /// 点击设置图标
  final VoidCallback onOpenSettings;

  const FilesAppBar({
    super.key,
    required this.searchController,
    required this.searchKeyword,
    required this.mountConfigured,
    required this.isIndexing,
    required this.onSearchChanged,
    required this.onSearchSubmit,
    required this.onSearchClear,
    required this.onRefresh,
    required this.onToggleTransfer,
    required this.onOpenFinder,
    required this.onOpenSettings,
  });

  @override
  Widget build(BuildContext context) {
    final metrics = MateTheme.metricsOf(context).mainPage;

    return SizedBox(
      height: metrics.appBarHeight,
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: metrics.appBarHorizontalPadding,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            // 左侧：搜索框（flex:1，max-width 420px）+ 清除按钮
            Expanded(
              child: Align(
                alignment: Alignment.centerLeft,
                child: ConstrainedBox(
                  constraints:
                      BoxConstraints(maxWidth: metrics.searchMaximumWidth),
                  child: MateSearchField(
                    controller: searchController,
                    placeholder: '搜索文件和文件夹...',
                    onChanged: onSearchChanged,
                    onSubmit: onSearchSubmit,
                  ),
                ),
              ),
            ),
            if (searchKeyword.isNotEmpty)
              MateButton(
                variant: MateButtonVariant.icon,
                icon: 'x',
                onClick: onSearchClear,
              ),

            // 工具组（无分隔线，按钮直接排，gap 8，整体右对齐）
            const Spacer(),
            if (mountConfigured) ...[
              // 「同步索引」：v2 主按钮（PRIMARY 品牌渐变）
              MateButton(
                label: '同步索引',
                variant: MateButtonVariant.primary,
                icon: 'refresh',
                onClick: onRefresh,
                loading: isIndexing,
                disabled: isIndexing,
              ),
              SizedBox(width: metrics.appBarActionSpacing),
            ],
            // 「传输队列」：v2 软色按钮（SOFT 浅蓝底）
            MateButton(
              label: '传输队列',
              variant: MateButtonVariant.soft,
              icon: 'transfer',
              onClick: onToggleTransfer,
            ),
            if (mountConfigured) ...[
              SizedBox(width: metrics.appBarActionSpacing),
              MateButton(
                label: 'Finder',
                variant: MateButtonVariant.iconText,
                icon: 'folder-open',
                onClick: onOpenFinder,
              ),
            ],
            SizedBox(width: metrics.appBarActionSpacing),
            MateButton(
              variant: MateButtonVariant.icon,
              icon: 'settings',
              onClick: onOpenSettings,
            ),
          ],
        ),
      ),
    );
  }
}
