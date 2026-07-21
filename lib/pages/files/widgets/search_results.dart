import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/widgets/index.dart';

/// 搜索结果区（对标 CMP MainScreen.kt SearchResults；
/// v2：design/02-main.html 场景 3，header + 56px 结果行 + 32px 色块 tile）。
class FilesSearchResults extends StatelessWidget {
  /// 已提交的搜索关键词
  final String keyword;

  /// 结果列表
  final List<DriveFile> results;

  /// 是否搜索中
  final bool searching;

  /// 点击文件夹结果进入（并退出搜索态）
  final void Function(DriveFile file) onEnterFolder;

  const FilesSearchResults({
    super.key,
    required this.keyword,
    required this.results,
    required this.searching,
    required this.onEnterFolder,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).mainPage;

    return Column(
      children: [
        // header（v2：13.5sp semibold textSecondary，padding 14/12/10）
        Container(
          width: double.infinity,
          padding: EdgeInsets.only(
            left: metrics.searchPanelStartPadding,
            top: metrics.searchPanelTopPadding,
            right: metrics.searchPanelEndPadding,
            bottom: metrics.searchPanelBottomPadding,
          ),
          child: Text(
            searching ? '搜索中…' : '搜索：$keyword',
            style: typography.main.searchHeader.copyWith(
              color: colors.textSecondary,
            ),
          ),
        ),
        if (results.isEmpty && !searching)
          const Expanded(
            child: MateEmpty(
              title: '无匹配结果',
              icon: 'search',
              description: '试试其他关键词',
            ),
          )
        else
          Expanded(
            child: ListView.builder(
              itemCount: results.length,
              itemBuilder: (context, index) {
                final file = results[index];
                return _SearchResultRow(
                  key: ValueKey(
                      file.id.isNotEmpty ? file.id : file.name),
                  file: file,
                  onTap: file.isFolder ? () => onEnterFolder(file) : null,
                );
              },
            ),
          ),
      ],
    );
  }
}

/// 搜索结果行（v2 .file-row：h56，padding 0 12px，gap 12px；
/// 32×32 radius 6 色块 tile：文件夹 folderBg/folder，文件 bgFill/textSecondary）。
class _SearchResultRow extends StatefulWidget {
  final DriveFile file;
  final VoidCallback? onTap;

  const _SearchResultRow({super.key, required this.file, this.onTap});

  @override
  State<_SearchResultRow> createState() => _SearchResultRowState();
}

class _SearchResultRowState extends State<_SearchResultRow> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).mainPage;
    final file = widget.file;

    return MouseRegion(
      cursor:
          widget.onTap != null ? SystemMouseCursors.click : SystemMouseCursors.basic,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onTap,
        child: Container(
          height: metrics.searchResultHeight,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.searchResultHorizontalPadding,
          ),
          color: _hovered ? colors.bgHover : Colors.transparent,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              // 32×32 radius 6 色块 tile
              Container(
                width: metrics.searchResultIconContainerSize,
                height: metrics.searchResultIconContainerSize,
                decoration: BoxDecoration(
                  color: file.isFolder ? colors.folderBg : colors.bgFill,
                  borderRadius:
                      BorderRadius.circular(metrics.searchResultIconRadius),
                ),
                alignment: Alignment.center,
                child: MateIcon(
                  name: file.isFolder ? 'folder' : 'file',
                  size: metrics.searchResultIconSize,
                  tint: file.isFolder ? colors.folder : colors.textSecondary,
                ),
              ),
              SizedBox(width: metrics.searchResultContentSpacing),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      file.name,
                      style: typography.main.searchResultName.copyWith(
                        color: colors.textPrimary,
                      ),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    Text(
                      file.isFolder ? '文件夹' : '${file.size} 字节',
                      style: typography.main.searchResultDescription.copyWith(
                        color: colors.textSecondary,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
