import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/widgets/index.dart';

/// 面包屑导航（对标 CMP Breadcrumb.kt；v2：design/02-main.html .breadcrumb）。
///
/// 高 40px，横向 scroll（超宽不换行），padding 0/20，gap 6；底部 MateHDivider。
/// 分隔符 `›`（placeholder 灰）；普通段 secondary 可点 hover→brand；
/// 当前段 primary + semibold + 不可点。
class FilesBreadcrumb extends StatelessWidget {
  /// 路径栈（最后一个为当前目录）
  final List<Breadcrumb> crumbs;

  /// 点击非末级段跳转（传目标下标）
  final void Function(int index) onNavigate;

  const FilesBreadcrumb({
    super.key,
    required this.crumbs,
    required this.onNavigate,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).navigation;

    return Column(
      children: [
        SizedBox(
          height: metrics.breadcrumbHeight,
          child: Container(
            color: colors.bgContainer,
            padding: EdgeInsets.symmetric(
              horizontal: metrics.breadcrumbHorizontalPadding,
            ),
            child: ListView.builder(
              scrollDirection: Axis.horizontal,
              itemCount: crumbs.length * 2 - 1,
              itemBuilder: (context, i) {
                if (i.isOdd) {
                  // 分隔符 ›（placeholder 灰）
                  return Padding(
                    padding: EdgeInsets.symmetric(
                      horizontal: metrics.breadcrumbItemSpacing / 2,
                    ),
                    child: Center(
                      child: Text(
                        '›',
                        style: typography.breadcrumb.separator.copyWith(
                          color: colors.textPlaceholder,
                        ),
                      ),
                    ),
                  );
                }
                final index = i ~/ 2;
                final crumb = crumbs[index];
                final isCurrent = index == crumbs.length - 1;
                return _BreadcrumbItem(
                  name: crumb.name,
                  isCurrent: isCurrent,
                  onTap: isCurrent ? null : () => onNavigate(index),
                );
              },
            ),
          ),
        ),
        // 底部分隔线（v2 保留 MateHDivider）
        const MateHDivider(),
      ],
    );
  }
}

/// 面包屑单个段（hover→brand 色，对齐 .breadcrumb__item:hover）。
class _BreadcrumbItem extends StatefulWidget {
  final String name;
  final bool isCurrent;
  final VoidCallback? onTap;

  const _BreadcrumbItem({
    required this.name,
    required this.isCurrent,
    this.onTap,
  });

  @override
  State<_BreadcrumbItem> createState() => _BreadcrumbItemState();
}

class _BreadcrumbItemState extends State<_BreadcrumbItem> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);

    final color = widget.isCurrent
        ? colors.textPrimary
        : _hovered
            ? colors.brand
            : colors.textSecondary;

    return MouseRegion(
      cursor: widget.isCurrent
          ? SystemMouseCursors.basic
          : SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onTap,
        child: Center(
          child: Text(
            widget.name,
            style: (widget.isCurrent
                    ? typography.breadcrumb.currentItem
                    : typography.breadcrumb.item)
                .copyWith(color: color),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ),
      ),
    );
  }
}
