import 'dart:typed_data';

import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/pages/files/widgets/file_list_bulk_bar.dart';
import 'package:petal_link/pages/files/widgets/file_list_dialogs.dart';
import 'package:petal_link/pages/files/widgets/file_row.dart';
import 'package:petal_link/service/mount/free_up.dart';
import 'package:petal_link/widgets/index.dart';

/// 文件列表（对标 CMP FileListScreen.kt；原 Vue FileListView.vue；
/// 视觉按 v2 原型 design/02-main.html + 05-file-ops.html）。
///
/// 6 列 + 拖拽列宽 + hover/selected 态 + 右键菜单条件渲染 + 双击行 +
/// 批量操作栏（v2 深色浮动条）+ 对话框（重命名/移动/属性/删除/释放空间预览）。
class FileListView extends StatefulWidget {
  /// 文件浏览器状态（visibleFiles/sortField/ascending/folderId/loading/directoryChildren）
  final FileBrowserState browser;

  /// 文件本地同步状态（fileId → folder/synced/placeholder/not_synced）
  final Map<String, String> fileStatuses;

  /// 已加载缩略图（fileId → 二进制）
  final Map<String, Uint8List> thumbnails;

  /// 挂载目录是否已配置（右键菜单/批量条条件渲染）
  final bool mountConfigured;

  /// 是否正在索引（批量与部分菜单项禁用）
  final bool isIndexing;

  /// 排序字段切换
  final void Function(SortField field) onSort;

  /// 双击进入文件夹
  final void Function(DriveFile file) onEnterFolder;

  /// 双击打开文件（按需下载）
  final void Function(DriveFile file) onOpenItem;

  /// 行可见时请求缩略图
  final void Function(DriveFile file) onThumbnailNeeded;

  /// 删除确认后执行
  final void Function(List<DriveFile> files) onDelete;

  /// 释放空间预览（异步回传可释放项）
  final void Function(
    List<DriveFile> files,
    void Function(List<FreeableItem> items) onResult,
  ) onPreviewFreeUp;

  /// 确认释放空间
  final void Function(List<FreeableItem> items) onFreeUp;

  /// 批量下载
  final void Function(List<DriveFile> files) onDownload;

  /// 执行双端对齐
  final void Function(DriveFile file) onSyncFolder;

  /// 重命名确认
  final void Function(DriveFile file, String newName) onRename;

  /// 移动确认（传目标父目录 ID）
  final void Function(DriveFile file, String parentId) onMove;

  /// 查询是否可释放空间（异步回传）
  final void Function(DriveFile file, void Function(bool canFree) onResult)
      onCanFreeUp;

  const FileListView({
    super.key,
    required this.browser,
    required this.fileStatuses,
    required this.thumbnails,
    required this.mountConfigured,
    required this.isIndexing,
    required this.onSort,
    required this.onEnterFolder,
    required this.onOpenItem,
    required this.onThumbnailNeeded,
    required this.onDelete,
    required this.onPreviewFreeUp,
    required this.onFreeUp,
    required this.onDownload,
    required this.onSyncFolder,
    required this.onRename,
    required this.onMove,
    required this.onCanFreeUp,
  });

  @override
  State<FileListView> createState() => _FileListViewState();
}

class _FileListViewState extends State<FileListView> {
  /// 勾选集合（切换目录时清空，对齐 CMP remember(browser.folderId)）
  Set<String> _checked = {};
  bool _showCheckboxes = false;
  String? _selectedId;

  /// v2 列宽（size 110 / time 160 起，拖拽可调）；didChangeDependencies 初始化
  late double _sizeWidth;
  late double _timeWidth;

  /// 释放空间防重复（对标原 Vue freeUpConfirmLoading）
  bool _freeUpBusy = false;

  List<DriveFile> get _files => widget.browser.visibleFiles;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final controls = MateTheme.metricsOf(context).fileList.controls;
    _sizeWidth = controls.sizeColumnInitialWidth;
    _timeWidth = controls.timeColumnInitialWidth;
  }

  @override
  void didUpdateWidget(covariant FileListView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.browser.folderId != oldWidget.browser.folderId) {
      _checked = {};
    }
  }

  /// 删除请求：确认对话框 → 执行 + 后置清理
  void _requestDelete(List<DriveFile> selection, VoidCallback afterDelete) {
    confirmDeleteFiles(
      selection: selection,
      fileStatuses: widget.fileStatuses,
      onConfirm: () {
        widget.onDelete(selection);
        afterDelete();
      },
    );
  }

  /// 释放空间请求：预览 → 确认对话框（防重复）
  void _requestFreeUp(List<DriveFile> selection) {
    if (_freeUpBusy) return;
    _freeUpBusy = true;
    widget.onPreviewFreeUp(selection, (items) {
      _freeUpBusy = false;
      confirmFreeUpItems(
        items: items,
        onConfirm: () => widget.onFreeUp(items),
      );
    });
  }

  /// 打开重命名对话框
  void _openRename(DriveFile file) {
    showDialog<void>(
      context: context,
      builder: (_) => RenameFileDialog(
        target: file,
        onConfirm: (newName) => widget.onRename(file, newName),
      ),
    );
  }

  /// 打开移动对话框（候选为已加载目录树中的全部文件夹，排除自身）
  void _openMove(DriveFile file) {
    final known = <String, DriveFile>{};
    for (final children in widget.browser.directoryChildren.values) {
      for (final f in children) {
        if (f.isFolder && f.id.isNotEmpty && f.id != file.id) {
          known[f.id] = f;
        }
      }
    }
    showDialog<void>(
      context: context,
      builder: (_) => MoveFileDialog(
        target: file,
        folders: known.values.toList(),
        onConfirm: (parentId) => widget.onMove(file, parentId),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final files = _files;

    return Column(
      children: [
        // 批量操作栏（选中>0 时；v2 05-file-ops：深色浮动条 h44 radius10）
        if (_checked.isNotEmpty)
          FileListBulkBar(
            checkedCount: _checked.length,
            mountConfigured: widget.mountConfigured,
            busy: widget.isIndexing,
            onDownload: () {
              final s = files.where((f) => _checked.contains(f.id)).toList();
              widget.onDownload(s);
            },
            onFreeUp: () {
              final s = files.where((f) => _checked.contains(f.id)).toList();
              _requestFreeUp(s);
            },
            onDelete: () {
              final s = files.where((f) => _checked.contains(f.id)).toList();
              _requestDelete(s, () => setState(() => _checked = {}));
            },
            onClose: () => setState(() {
              _checked = {};
              _showCheckboxes = false;
            }),
          ),

        // 空状态
        if (files.isEmpty && !widget.browser.loading)
          const Expanded(
            child: MateEmpty(
              title: '此文件夹为空',
              icon: 'folder-open',
              description: '上传或拖入文件即可同步到云端',
            ),
          ),

        if (files.isNotEmpty)
          // v2：file-table 容器 padding 0 12
          Expanded(
            child: Padding(
              padding: EdgeInsets.symmetric(
                horizontal: controls.tableHorizontalPadding,
              ),
              child: Column(
                children: [
                  _buildHeader(controls, colors, typography, files),
                  const MateHDivider(),
                  Expanded(
                    child: ListView.builder(
                      itemCount: files.length,
                      itemBuilder: (context, index) {
                        final file = files[index];
                        return FileListRow(
                          key: ValueKey(file.id.isNotEmpty
                              ? file.id
                              : '${file.name}-${file.editedTime}'),
                          file: file,
                          checked: _checked.contains(file.id),
                          selected: file.id == _selectedId,
                          showCheckbox: _showCheckboxes,
                          status: widget.fileStatuses[file.id] ??
                              (file.isFolder ? 'folder' : 'not_synced'),
                          thumbnail: widget.thumbnails[file.id],
                          mountConfigured: widget.mountConfigured,
                          isIndexing: widget.isIndexing,
                          sizeWidth: _sizeWidth,
                          timeWidth: _timeWidth,
                          onCheckedChange: (c) {
                            if (file.id.isEmpty) return;
                            setState(() {
                              _checked = c
                                  ? {..._checked, file.id}
                                  : ({..._checked}..remove(file.id));
                            });
                          },
                          onClick: () =>
                              setState(() => _selectedId = file.id),
                          onDoubleClick: () {
                            if (file.isFolder) {
                              widget.onEnterFolder(file);
                            } else {
                              widget.onOpenItem(file);
                            }
                          },
                          onThumbnailNeeded: () =>
                              widget.onThumbnailNeeded(file),
                          onSync: () => widget.onSyncFolder(file),
                          onRename: () => _openRename(file),
                          onMove: () => _openMove(file),
                          onShowProps: () => openFileProps(file),
                          onDelete: () => _requestDelete([file], () {}),
                          onFreeUp: () => _requestFreeUp([file]),
                          onCanFreeUp: widget.onCanFreeUp,
                        );
                      },
                    ),
                  ),
                  // 底部信息（v2 file-footer：h36，13sp textPlaceholder）
                  SizedBox(
                    height: controls.footerHeight,
                    child: Center(
                      child: Text(
                        '${files.length} 项 · 已全部加载',
                        style: typography.fileList.loadedSummary.copyWith(
                          color: colors.textPlaceholder,
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
      ],
    );
  }

  /// 表头（v2：38px，12.5sp semibold textSecondary，底部分隔线）
  Widget _buildHeader(
    FileListControlMetrics controls,
    MateSemanticColors colors,
    MateTypography typography,
    List<DriveFile> files,
  ) {
    final checkedCount = _checked.length;
    final headerCheck = checkedCount == 0
        ? false
        : checkedCount == files.length
            ? true
            : null;

    return SizedBox(
      height: controls.headerHeight,
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: controls.headerHorizontalPadding,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            SizedBox(
              width: controls.checkboxColumnWidth,
              child: Align(
                alignment: Alignment.centerLeft,
                child: _showCheckboxes
                    ? MateCheckbox(
                        checked: headerCheck,
                        onChanged: (_) {
                          setState(() {
                            _checked = checkedCount == files.length
                                ? {}
                                : files
                                    .map((f) => f.id)
                                    .where((id) => id.isNotEmpty)
                                    .toSet();
                          });
                        },
                      )
                    : MateButton(
                        variant: MateButtonVariant.icon,
                        icon: 'check',
                        onClick: () => setState(() => _showCheckboxes = true),
                      ),
              ),
            ),
            Expanded(
              child: FileListHeaderCell(
                title: '名称',
                active: widget.browser.sortField == SortField.Name,
                ascending: widget.browser.ascending,
                onClick: () => widget.onSort(SortField.Name),
              ),
            ),
            SizedBox(
              width: _sizeWidth,
              child: FileListHeaderCell(
                title: '大小',
                active: widget.browser.sortField == SortField.Size,
                ascending: widget.browser.ascending,
                resizable: true,
                onResize: (delta) {
                  setState(() {
                    _sizeWidth = (_sizeWidth + delta).clamp(
                      controls.resizableColumnMinimumWidth,
                      controls.resizableColumnMaximumWidth,
                    );
                  });
                },
                onClick: () => widget.onSort(SortField.Size),
              ),
            ),
            SizedBox(
              width: _timeWidth,
              child: FileListHeaderCell(
                title: '修改时间',
                active: widget.browser.sortField == SortField.ModifiedTime,
                ascending: widget.browser.ascending,
                resizable: true,
                onResize: (delta) {
                  setState(() {
                    _timeWidth = (_timeWidth + delta).clamp(
                      controls.resizableColumnMinimumWidth,
                      controls.resizableColumnMaximumWidth,
                    );
                  });
                },
                onClick: () => widget.onSort(SortField.ModifiedTime),
              ),
            ),
            // v2 列宽：状态 72 / 操作 44
            SizedBox(
              width: controls.statusColumnWidth,
              child: Center(
                child: Text(
                  '状态',
                  style: typography.fileList.statusColumnHeader.copyWith(
                    color: colors.textSecondary,
                  ),
                ),
              ),
            ),
            SizedBox(
              width: controls.actionColumnWidth,
              child: Center(
                child: Text(
                  '操作',
                  style: typography.fileList.actionColumnHeader.copyWith(
                    color: colors.textSecondary,
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 表头单元格（排序指示 + 可选 resize-handle）。
class FileListHeaderCell extends StatelessWidget {
  /// 列标题
  final String title;

  /// 当前排序字段是否命中本列
  final bool active;

  /// 是否升序（active 时显示方向箭头）
  final bool ascending;

  /// 是否显示列宽拖动手柄
  final bool resizable;

  /// 列宽拖动回调（传水平增量）
  final void Function(double delta)? onResize;

  /// 点击切换排序
  final VoidCallback onClick;

  const FileListHeaderCell({
    super.key,
    required this.title,
    required this.active,
    required this.ascending,
    this.resizable = false,
    this.onResize,
    required this.onClick,
  });

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;

    return Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            onTap: onClick,
            behavior: HitTestBehavior.opaque,
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                // v2：表头 12.5sp semibold textSecondary
                Text(
                  title,
                  style: typography.fileList.genericColumnHeader.copyWith(
                    color: colors.textSecondary,
                  ),
                ),
                if (active) ...[
                  SizedBox(width: controls.headerSortSpacing),
                  Transform.rotate(
                    angle: ascending ? 0 : 3.141592653589793 / 2,
                    child: MateIcon(
                      name: 'arrow',
                      size: controls.headerSortIconSize,
                      tint: colors.textSecondary,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
        if (resizable && onResize != null)
          Positioned(
            right: 0,
            top: 0,
            bottom: 0,
            width: controls.resizeHandleWidth,
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeColumn,
              child: GestureDetector(
                onHorizontalDragUpdate: (details) =>
                    onResize!(details.delta.dx),
                behavior: HitTestBehavior.opaque,
              ),
            ),
          ),
      ],
    );
  }
}
