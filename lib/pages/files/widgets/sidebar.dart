import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/widgets/index.dart';

/// 侧边栏（对标 CMP Sidebar.kt；v2：design/02-main.html .sidebar）。
///
/// 宽 248px，bg-page，右 0.5px border。纵向三段：
/// 1. Logo 区（60px 高，padding 0/18）
/// 2. section 标签「位置」（12sp semibold textPlaceholder，padding 12/18/6）
/// 3. 目录树（flex:1 scroll，懒加载 + 路径自动展开）
/// 底部：悬浮账号卡（渐变头像 + 用户名 + 配额文本 + 4px 配额进度条），
/// 以及更新卡片（下载进度卡 / 新版本提示卡）。
class FilesSidebar extends StatelessWidget {
  /// 根目录子文件夹列表
  final List<DriveFile> rootChildren;

  /// 各文件夹 ID → 子文件夹列表
  final Map<String, List<DriveFile>> directoryChildren;

  /// 当前选中文件夹 ID（空串表示根目录）
  final String selectedFolderId;

  /// 当前浏览路径上的文件夹 ID 集合（路径上的目录自动展开）
  final Set<String> pathFolderIds;

  /// 用户显示名
  final String? userName;

  /// 配额文本（如 "1.2 GB / 5 GB"）
  final String? quotaText;

  /// 更新是否下载中（侧边栏进度卡）
  final bool updateDownloading;

  /// 更新下载进度 0..1
  final double updateDownloadProgress;

  /// 可用更新版本号（非空显示更新提示卡）
  final String? updateAvailableVersion;

  /// 正在懒加载子目录的文件夹 ID 集合
  final Set<String> treeLoadingIds;

  /// 关闭更新提示
  final VoidCallback onDismissUpdate;

  /// 立即更新（下载并安装）
  final VoidCallback onInstallUpdate;

  /// 查看更新（跳转更新页）
  final VoidCallback onShowUpdate;

  /// 点击目录树节点导航
  final void Function(DriveFile) onNavigate;

  /// 展开未加载过的节点时触发懒加载
  final void Function(DriveFile) onExpandNode;

  const FilesSidebar({
    super.key,
    required this.rootChildren,
    required this.directoryChildren,
    required this.selectedFolderId,
    required this.pathFolderIds,
    required this.onNavigate,
    this.userName,
    this.quotaText,
    this.updateDownloading = false,
    this.updateDownloadProgress = 0.0,
    this.updateAvailableVersion,
    this.treeLoadingIds = const {},
    required this.onDismissUpdate,
    required this.onInstallUpdate,
    required this.onShowUpdate,
    required this.onExpandNode,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).sidebar;

    return Container(
      width: metrics.width,
      height: double.infinity,
      decoration: BoxDecoration(
        color: colors.bgPage,
        // 右 0.5px border（v2 .sidebar border-right）
        border: Border(
          right: BorderSide(color: colors.border, width: 0.5),
        ),
      ),
      child: Column(
        children: [
          // 1. Logo 区（60px，padding 0/18）
          Container(
            height: metrics.logoHeaderHeight,
            padding: EdgeInsets.symmetric(
              horizontal: metrics.logoHeaderHorizontalPadding,
            ),
            alignment: Alignment.centerLeft,
            child: MateAppLogo(size: metrics.logoSize),
          ),

          // 2. section 标签「位置」（12sp semibold textPlaceholder，padding 12/18/6）
          Padding(
            padding: EdgeInsets.only(
              left: metrics.sectionLabelStartPadding,
              top: metrics.sectionLabelTopPadding,
              bottom: metrics.sectionLabelBottomPadding,
            ),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                '位置',
                style: typography.sidebar.sectionLabel.copyWith(
                  color: colors.textPlaceholder,
                ),
              ),
            ),
          ),

          // 3. 目录树（flex:1 scroll）
          Expanded(
            child: SingleChildScrollView(
              padding: EdgeInsets.symmetric(
                horizontal: metrics.treeHorizontalPadding,
                vertical: metrics.treeVerticalPadding,
              ),
              child: _SidebarTreeNode(
                folder: const DriveFile(
                  id: '',
                  name: '全部文件',
                  category: FileCategory.Folder,
                ),
                depth: 0,
                selectedId: selectedFolderId,
                pathFolderIds: pathFolderIds,
                children: rootChildren,
                directoryChildren: directoryChildren,
                onSelect: onNavigate,
                treeLoadingIds: treeLoadingIds,
                onExpandNode: onExpandNode,
              ),
            ),
          ),

          // 4. 悬浮账号卡
          _AccountCard(userName: userName, quotaText: quotaText),

          // 更新下载进度卡（v2 渐变卡片，点击重开更新弹窗）
          if (updateDownloading)
            _UpdateProgressCard(
              progress: updateDownloadProgress,
              onShowUpdate: onShowUpdate,
            ),

          // 更新提示卡（v2 渐变卡片）
          if (updateAvailableVersion != null)
            _UpdateBannerCard(
              version: updateAvailableVersion!,
              onDismiss: onDismissUpdate,
              onInstall: onInstallUpdate,
              onShowUpdate: onShowUpdate,
            ),
        ],
      ),
    );
  }
}

/// 悬浮账号卡（v2：margin 10，bg-container radius 10 + 0.5px border，padding 12，gap 10；
/// 32×32 圆形品牌渐变头像 + 用户名 14sp semibold + 配额 12.5sp secondary + 4px 配额进度条）。
class _AccountCard extends StatelessWidget {
  final String? userName;
  final String? quotaText;

  const _AccountCard({this.userName, this.quotaText});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).sidebar;
    final quotaRatio =
        quotaText != null ? _parseQuotaRatio(quotaText!) : null;

    return Padding(
      padding: EdgeInsets.all(metrics.accountOuterPadding),
      child: Container(
        width: double.infinity,
        padding: EdgeInsets.all(metrics.accountInnerPadding),
        decoration: BoxDecoration(
          color: colors.bgContainer,
          borderRadius: BorderRadius.circular(metrics.accountRadius),
          border: Border.all(
            color: colors.border,
            width: metrics.accountBorderWidth,
          ),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            // 32×32 圆形品牌渐变头像（白色 initial 占位字）
            Container(
              width: metrics.accountAvatarSize,
              height: metrics.accountAvatarSize,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                gradient: LinearGradient(colors: colors.brandGradient),
              ),
              alignment: Alignment.center,
              child: Text(
                (userName != null && userName!.isNotEmpty)
                    ? userName!.characters.first
                    : '华',
                style: typography.sidebar.accountAvatar.copyWith(
                  color: colors.sidebarAccountAvatarText,
                ),
              ),
            ),
            SizedBox(width: metrics.accountContentSpacing),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    userName ?? '加载账号中…',
                    style: typography.sidebar.accountName.copyWith(
                      color: colors.textPrimary,
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                  if (quotaText != null) ...[
                    SizedBox(height: metrics.accountQuotaTopPadding),
                    Text(
                      quotaText!,
                      style: typography.sidebar.quotaDescription.copyWith(
                        color: colors.textSecondary,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    // 配额进度条（4px，品牌渐变；比例解析失败不显示）
                    if (quotaRatio != null) ...[
                      SizedBox(height: metrics.accountQuotaProgressSpacing),
                      MateLinearProgress(
                        value: quotaRatio,
                        height: metrics.accountQuotaProgressHeight,
                      ),
                    ],
                  ],
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 更新下载进度卡（v2：margin 0/10/10，品牌渐变底 radius 10 padding 12，
/// 白字 + 白色进度条；点击重开更新弹窗）。
class _UpdateProgressCard extends StatelessWidget {
  final double progress;
  final VoidCallback onShowUpdate;

  const _UpdateProgressCard({required this.progress, required this.onShowUpdate});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).sidebar;

    return Padding(
      padding: EdgeInsets.only(
        left: metrics.updateCardHorizontalMargin,
        right: metrics.updateCardHorizontalMargin,
        bottom: metrics.updateCardBottomMargin,
      ),
      child: GestureDetector(
        onTap: onShowUpdate,
        child: Container(
          width: double.infinity,
          padding: EdgeInsets.all(metrics.updateCardPadding),
          decoration: BoxDecoration(
            gradient: LinearGradient(colors: colors.brandGradient),
            borderRadius: BorderRadius.circular(metrics.updateCardRadius),
          ),
          child: Column(
            children: [
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  Text(
                    '正在下载更新',
                    style: typography.sidebar.downloadUpdateLabel.copyWith(
                      color: colors.sidebarUpdateText,
                    ),
                  ),
                  Text(
                    '${(progress * 100).toInt()}%',
                    style: typography.sidebar.downloadUpdateProgress.copyWith(
                      color: colors.sidebarUpdateText,
                    ),
                  ),
                ],
              ),
              SizedBox(height: metrics.downloadProgressSpacing),
              MateLinearProgress(
                value: progress,
                color: colors.sidebarUpdateProgress,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 更新提示卡（v2：margin 0/10/10，品牌渐变底 radius 10 padding 12，
/// 白字标题 + 圆形半透明 × + 「日志」+ 白底「立即更新」按钮）。
class _UpdateBannerCard extends StatelessWidget {
  final String version;
  final VoidCallback onDismiss;
  final VoidCallback onInstall;
  final VoidCallback onShowUpdate;

  const _UpdateBannerCard({
    required this.version,
    required this.onDismiss,
    required this.onInstall,
    required this.onShowUpdate,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).sidebar;

    return Padding(
      padding: EdgeInsets.only(
        left: metrics.updateCardHorizontalMargin,
        right: metrics.updateCardHorizontalMargin,
        bottom: metrics.updateCardBottomMargin,
      ),
      child: Container(
        width: double.infinity,
        padding: EdgeInsets.all(metrics.updateCardPadding),
        decoration: BoxDecoration(
          gradient: LinearGradient(colors: colors.brandGradient),
          borderRadius: BorderRadius.circular(metrics.updateCardRadius),
        ),
        child: Column(
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                Text(
                  '新版本 $version',
                  style: typography.sidebar.availableUpdateLabel.copyWith(
                    color: colors.sidebarUpdateText,
                  ),
                ),
                // × 关闭按钮（20×20 圆形半透明白）
                GestureDetector(
                  onTap: onDismiss,
                  child: MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: Container(
                      width: metrics.dismissButtonSize,
                      height: metrics.dismissButtonSize,
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: colors.sidebarDismissBackground,
                      ),
                      alignment: Alignment.center,
                      child: Text(
                        '×',
                        style: typography.sidebar.dismissUpdateAction.copyWith(
                          color: colors.sidebarDismissText,
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
            SizedBox(height: metrics.availableActionSpacing),
            Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                // 「日志」按钮（查看更新日志，跳转更新页）
                GestureDetector(
                  onTap: onShowUpdate,
                  child: MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: Container(
                      height: metrics.installButtonHeight,
                      padding: EdgeInsets.symmetric(
                        horizontal: metrics.updateCardPadding,
                      ),
                      alignment: Alignment.center,
                      child: Text(
                        '日志',
                        style: typography.sidebar.installUpdateAction.copyWith(
                          color: colors.sidebarUpdateText,
                        ),
                      ),
                    ),
                  ),
                ),
                // 「立即更新」按钮（白底 h28 radius 5，brand 字）
                Expanded(
                  child: GestureDetector(
                    onTap: onInstall,
                    child: MouseRegion(
                      cursor: SystemMouseCursors.click,
                      child: Container(
                        height: metrics.installButtonHeight,
                        decoration: BoxDecoration(
                          color: colors.sidebarInstallBackground,
                          borderRadius:
                              BorderRadius.circular(metrics.installButtonRadius),
                        ),
                        alignment: Alignment.center,
                        child: Text(
                          '立即更新',
                          style: typography.sidebar.installUpdateAction.copyWith(
                            color: colors.brand,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// 递归目录树节点（v2：design/02-main.html .tree-node）。
///
/// 行高 32px，缩进 depth*14+8，gap 8，radius 6；
/// chevron(16px 宽，arrow 图标展开 rotate 90°)；文件夹图标 16px folder 色；名称 14px；
/// 三态：默认 secondary / hover bg-hover / 选中 brandLighter 底 + brand 字 + medium。
/// 当前浏览路径上的节点自动展开；离开路径时收起，避免树与右侧文件列表脱节。
class _SidebarTreeNode extends StatefulWidget {
  final DriveFile folder;
  final int depth;
  final String selectedId;
  final Set<String> pathFolderIds;
  final List<DriveFile> children;
  final Map<String, List<DriveFile>> directoryChildren;
  final void Function(DriveFile) onSelect;
  final Set<String> treeLoadingIds;
  final void Function(DriveFile) onExpandNode;

  const _SidebarTreeNode({
    super.key,
    required this.folder,
    required this.depth,
    required this.selectedId,
    required this.pathFolderIds,
    required this.children,
    required this.directoryChildren,
    required this.onSelect,
    required this.treeLoadingIds,
    required this.onExpandNode,
  });

  @override
  State<_SidebarTreeNode> createState() => _SidebarTreeNodeState();
}

class _SidebarTreeNodeState extends State<_SidebarTreeNode> {
  late bool _expanded;
  bool _hovered = false;

  @override
  void initState() {
    super.initState();
    _expanded = widget.depth == 0;
  }

  @override
  void didUpdateWidget(covariant _SidebarTreeNode oldWidget) {
    super.didUpdateWidget(oldWidget);
    // 对齐 CMP LaunchedEffect(pathFolderIds)：路径上的节点自动展开，离开路径收起
    if (widget.pathFolderIds != oldWidget.pathFolderIds) {
      setState(() {
        _expanded =
            widget.depth == 0 || widget.pathFolderIds.contains(widget.folder.id);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).sidebar;
    final isSelected = widget.folder.id == widget.selectedId;
    final isLoading = widget.folder.id.isNotEmpty &&
        widget.treeLoadingIds.contains(widget.folder.id);

    return Column(
      children: [
        MouseRegion(
          onEnter: (_) => setState(() => _hovered = true),
          onExit: (_) => setState(() => _hovered = false),
          child: GestureDetector(
            onTap: () {
              setState(() => _expanded = true);
              widget.onSelect(widget.folder);
            },
            child: Container(
              height: metrics.treeNodeHeight,
              padding: EdgeInsets.only(
                left: metrics.treeNodeStartPadding +
                    metrics.treeDepthIndent * widget.depth,
                right: metrics.treeNodeEndPadding,
              ),
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(metrics.treeNodeRadius),
                color: isSelected
                    ? colors.brandLighter
                    : _hovered
                        ? colors.bgHover
                        : Colors.transparent,
              ),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  // chevron（16px 命中区，arrow 图标展开 rotate 90°）；
                  // 展开未加载过的节点时触发懒加载
                  GestureDetector(
                    onTap: () {
                      setState(() => _expanded = !_expanded);
                      if (_expanded &&
                          widget.folder.id.isNotEmpty &&
                          widget.directoryChildren[widget.folder.id] == null) {
                        widget.onExpandNode(widget.folder);
                      }
                    },
                    child: SizedBox(
                      width: metrics.treeExpanderSize,
                      height: metrics.treeExpanderSize,
                      child: Center(
                        child: isLoading
                            ? MateCircularProgress(
                                size: metrics.treeArrowIconSize)
                            : Transform.rotate(
                                angle: _expanded ? 3.141592653589793 / 2 : 0,
                                child: MateIcon(
                                  name: 'arrow',
                                  size: metrics.treeArrowIconSize,
                                  tint: colors.textSecondary,
                                ),
                              ),
                      ),
                    ),
                  ),
                  SizedBox(width: metrics.treeNodeContentSpacing),
                  // 文件夹图标（16px folder 色）
                  MateIcon(
                    name: 'folder',
                    size: metrics.treeFolderIconSize,
                    tint: colors.folder,
                  ),
                  SizedBox(width: metrics.treeNodeContentSpacing),
                  // 名称（14px，选中 brand+medium，默认 secondary）
                  Expanded(
                    child: Text(
                      widget.folder.name,
                      style: (isSelected
                              ? typography.sidebar.selectedTreeNodeLabel
                              : typography.sidebar.treeNodeLabel)
                          .copyWith(
                        color: isSelected ? colors.brand : colors.textSecondary,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
        // 递归子节点
        if (_expanded)
          for (final child in widget.children)
            if (child.id.isNotEmpty)
              _SidebarTreeNode(
                key: ValueKey(child.id),
                folder: child,
                depth: widget.depth + 1,
                selectedId: widget.selectedId,
                pathFolderIds: widget.pathFolderIds,
                children: widget.directoryChildren[child.id] ?? const [],
                directoryChildren: widget.directoryChildren,
                onSelect: widget.onSelect,
                treeLoadingIds: widget.treeLoadingIds,
                onExpandNode: widget.onExpandNode,
              ),
      ],
    );
  }
}

/// 从配额文本（"36.5 GB / 200 GB"）解析已用比例，
/// 仅用于账号卡配额进度条的显示；解析失败返回 null（不显示进度条）。
/// 对齐 CMP Sidebar.kt parseQuotaRatio。
double? _parseQuotaRatio(String quotaText) {
  final parts = quotaText.split(' / ');
  if (parts.length != 2) return null;
  final used = _parseSizeBytes(parts[0]);
  final total = _parseSizeBytes(parts[1]);
  if (used == null || total == null || total <= 0) return null;
  return (used / total).clamp(0.0, 1.0);
}

/// 解析 "X.X GB/MB/KB" 或 "N B" 为字节数；格式不符返回 null。
int? _parseSizeBytes(String text) {
  final tokens = text.trim().split(' ');
  if (tokens.length != 2) return null;
  final value = double.tryParse(tokens[0]);
  if (value == null) return null;
  final multiplier = switch (tokens[1]) {
    'B' => 1,
    'KB' => 1024,
    'MB' => 1024 * 1024,
    'GB' => 1024 * 1024 * 1024,
    _ => 0,
  };
  if (multiplier == 0) return null;
  return (value * multiplier).toInt();
}
