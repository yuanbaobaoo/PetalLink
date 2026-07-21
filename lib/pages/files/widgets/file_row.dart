import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/widgets/file_format.dart';
import 'package:petal_link/pages/files/widgets/files_menu.dart';
import 'package:petal_link/widgets/index.dart';

/// v2：文档类扩展名 → file-text tile（对标原型 .docx/.md/.pdf）
const Set<String> _docTileExts = {
  'doc', 'docx', 'txt', 'md', 'pdf', 'rtf', 'odt', 'pages',
};

/// v2：表格/图表类扩展名 → chart tile（对标原型 .xlsx）
const Set<String> _sheetTileExts = {
  'xls', 'xlsx', 'csv', 'ods', 'numbers', 'et',
};

/// 文件类型图标（对标 CMP fileTypeIcon / 原 Vue driveApi.fileTypeIcon）。
///
/// v2 扩展返回 tile 类型（folder / file-text / image / video / chart / file），
/// 名称列色块依此取色；image/video 判断逻辑与原有一致，doc/sheet 为原
/// "file" 桶的细分。
String fileTypeIcon(DriveFile file) {
  if (file.isFolder) return 'folder';
  final mime = file.mimeType ?? '';
  final ext =
      file.name.contains('.') ? file.name.split('.').last.toLowerCase() : '';
  if (file.category == FileCategory.image || mime.startsWith('image/')) {
    return 'image';
  }
  if (file.category == FileCategory.video || mime.startsWith('video/')) {
    return 'video';
  }
  // 表格/图表类（扩展名）→ sheet tile
  if (_sheetTileExts.contains(ext)) return 'chart';
  // 文档类（text/*、pdf、常见文档扩展名）→ doc tile
  if (file.category == FileCategory.document ||
      mime.startsWith('text/') ||
      mime == 'application/pdf' ||
      _docTileExts.contains(ext)) {
    return 'file-text';
  }
  return 'file';
}

/// 文件类型色块 tile（v2 .ftile：32×32 radius 6，柔和底色 + 彩色图标 18dp）。
/// 图片有缩略图时直接显示缩略图（同尺寸 clip radius 6），解码失败回退色块图标。
class FileTypeTile extends StatelessWidget {
  /// 文件
  final DriveFile file;

  /// 缩略图二进制（可空）
  final Uint8List? thumbnail;

  const FileTypeTile({super.key, required this.file, required this.thumbnail});

  @override
  Widget build(BuildContext context) {
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final type = fileTypeIcon(file);

    if (thumbnail != null && !file.isFolder) {
      return ClipRRect(
        borderRadius: BorderRadius.circular(controls.thumbnailRadius),
        child: Image.memory(
          thumbnail!,
          width: controls.thumbnailSize,
          height: controls.thumbnailSize,
          fit: BoxFit.cover,
          // 解码失败回退到色块图标
          errorBuilder: (_, _, _) => _colorTile(context, type),
        ),
      );
    }
    return _colorTile(context, type);
  }

  Widget _colorTile(BuildContext context, String type) {
    final colors = MateTheme.colorsOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final (bg, tint) = switch (type) {
      'folder' => (colors.folderBg, colors.folder),
      'file-text' => (colors.documentBg, colors.document),
      'image' => (colors.imageBg, colors.image),
      'video' => (colors.videoBg, colors.video),
      'chart' => (colors.sheetBg, colors.sheet),
      _ => (colors.bgFill, colors.textSecondary),
    };
    return Container(
      width: controls.thumbnailSize,
      height: controls.thumbnailSize,
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(controls.thumbnailRadius),
      ),
      alignment: Alignment.center,
      child: MateIcon(
        name: type,
        size: controls.fileTypeIconSize,
        tint: tint,
      ),
    );
  }
}

/// 文件行（v2：56px，radius 8，hover bgHover，selected brandLighter，
/// 双击触发，右键菜单条件渲染）。
class FileListRow extends StatefulWidget {
  /// 文件
  final DriveFile file;

  /// 是否勾选（多选模式）
  final bool checked;

  /// 是否单击选中
  final bool selected;

  /// 是否显示勾选框
  final bool showCheckbox;

  /// 本地同步状态（folder/synced/placeholder/not_synced）
  final String status;

  /// 缩略图二进制
  final Uint8List? thumbnail;

  /// 挂载目录是否已配置（右键菜单条件渲染）
  final bool mountConfigured;

  /// 是否正在索引（部分菜单项禁用）
  final bool isIndexing;

  /// 大小列宽
  final double sizeWidth;

  /// 时间列宽
  final double timeWidth;

  /// 勾选变化
  final void Function(bool checked) onCheckedChange;

  /// 单击选中
  final VoidCallback onClick;

  /// 双击（目录进入 / 文件按需下载）
  final VoidCallback onDoubleClick;

  /// 行可见时请求缩略图
  final VoidCallback onThumbnailNeeded;

  /// 执行双端对齐
  final VoidCallback onSync;

  /// 重命名
  final VoidCallback onRename;

  /// 移动到…
  final VoidCallback onMove;

  /// 属性
  final VoidCallback onShowProps;

  /// 删除
  final VoidCallback onDelete;

  /// 释放空间
  final VoidCallback onFreeUp;

  /// 查询是否可释放空间（异步回传）
  final void Function(DriveFile file, void Function(bool canFree) onResult)
      onCanFreeUp;

  const FileListRow({
    super.key,
    required this.file,
    required this.checked,
    required this.selected,
    required this.showCheckbox,
    required this.status,
    required this.thumbnail,
    required this.mountConfigured,
    required this.isIndexing,
    required this.sizeWidth,
    required this.timeWidth,
    required this.onCheckedChange,
    required this.onClick,
    required this.onDoubleClick,
    required this.onThumbnailNeeded,
    required this.onSync,
    required this.onRename,
    required this.onMove,
    required this.onShowProps,
    required this.onDelete,
    required this.onFreeUp,
    required this.onCanFreeUp,
  });

  @override
  State<FileListRow> createState() => _FileListRowState();
}

class _FileListRowState extends State<FileListRow> {
  bool _hovered = false;

  /// 右键菜单控制器（可刷新条目/主动关闭）
  FilesMenuController? _menuController;

  /// 上一次单击时间（手动双击检测；GestureDetector 的 onDoubleTap 会让
  /// 内层 checkbox 等手势被拒，故自行判定）
  DateTime? _lastTapAt;

  /// 菜单打开时异步查询的可释放状态（null=查询中/未查）
  bool? _menuCanFree;

  /// 未配置同步目录时，文件不能按需下载；目录仍可用于浏览云端层级。
  bool get _rowClickEnabled => widget.file.isFolder || widget.mountConfigured;

  /// 单击选中；300ms 内第二次点击判定为双击（目录进入/文件打开），
  /// 语义对齐 CMP combinedClickable（onClick 每次触发 + onDoubleClick 双击触发）。
  void _handleTap() {
    final now = DateTime.now();
    final last = _lastTapAt;
    _lastTapAt = now;
    widget.onClick();
    if (last != null && now.difference(last) < kDoubleTapTimeout) {
      _lastTapAt = null;
      widget.onDoubleClick();
    }
  }

  @override
  void initState() {
    super.initState();
    widget.onThumbnailNeeded();
  }

  @override
  void didUpdateWidget(covariant FileListRow oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.file.id != widget.file.id ||
        oldWidget.file.thumbnailLink != widget.file.thumbnailLink) {
      widget.onThumbnailNeeded();
    }
  }

  @override
  void dispose() {
    _menuController?.dismiss();
    super.dispose();
  }

  /// 打开右键菜单（对标原 Vue @contextmenu；cursorPosition 为空时锚定行尾底部）
  void _openMenu({Offset? cursorPosition}) {
    _menuController?.dismiss();
    _menuCanFree = null;
    // 菜单打开即查询可释放状态，结果到达后刷新菜单（对标 CMP canFree）
    widget.onCanFreeUp(widget.file, (canFree) {
      _menuCanFree = canFree;
      _menuController?.refresh();
    });

    Offset? anchor;
    if (cursorPosition == null) {
      // 锚定行尾底部（对齐 CMP Alignment.BottomEnd）
      final rowBox = context.findRenderObject() as RenderBox;
      anchor = rowBox
          .localToGlobal(Offset(rowBox.size.width, rowBox.size.height));
    }
    _menuController = showFilesMenu(
      context,
      cursorPosition: cursorPosition,
      anchorBottomRight: anchor,
      entriesBuilder: _menuEntries,
    );
  }

  /// 菜单条目清单（按条件渲染，对标原 Vue ctx-menu）
  List<FilesMenuEntry> _menuEntries() {
    return [
      if (widget.mountConfigured) ...[
        FilesMenuEntry(
          label: '执行双端对齐',
          icon: 'sync',
          enabled: !widget.isIndexing,
          action: widget.onSync,
        ),
        const FilesMenuEntry.divider(),
      ],
      if (_menuCanFree == true) ...[
        FilesMenuEntry(label: '释放空间', icon: 'cloud', action: widget.onFreeUp),
        const FilesMenuEntry.divider(),
      ],
      if (widget.mountConfigured) ...[
        FilesMenuEntry(
          label: '重命名',
          icon: 'edit',
          enabled: !widget.isIndexing,
          action: widget.onRename,
        ),
        FilesMenuEntry(
          label: '移动到…',
          icon: 'folder-open',
          enabled: !widget.isIndexing,
          action: widget.onMove,
        ),
      ],
      FilesMenuEntry(label: '属性', icon: 'info', action: widget.onShowProps),
      if (widget.mountConfigured) ...[
        const FilesMenuEntry.divider(),
        FilesMenuEntry(
          label: '删除',
          icon: 'trash',
          danger: true,
          enabled: !widget.isIndexing,
          action: widget.onDelete,
        ),
      ],
    ];
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final controls = MateTheme.metricsOf(context).fileList.controls;
    final file = widget.file;

    final bg = widget.selected
        ? colors.brandLighter
        : _hovered
            ? colors.bgHover
            : Colors.transparent;

    return Column(
      children: [
        MouseRegion(
          onEnter:
              _rowClickEnabled ? (_) => setState(() => _hovered = true) : null,
          onExit:
              _rowClickEnabled ? (_) => setState(() => _hovered = false) : null,
          child: Listener(
            // 右键：在光标处弹菜单（对标原 Vue @contextmenu）
            onPointerDown: (event) {
              if (event.kind == PointerDeviceKind.mouse &&
                  event.buttons == kSecondaryMouseButton) {
                _openMenu(cursorPosition: event.position);
              }
            },
            child: GestureDetector(
              onTap: _rowClickEnabled ? _handleTap : null,
              onLongPress: _rowClickEnabled ? () => _openMenu() : null,
              behavior: HitTestBehavior.opaque,
              child: Container(
                height: controls.rowHeight,
                padding: EdgeInsets.symmetric(
                  horizontal: controls.rowHorizontalPadding,
                ),
                decoration: BoxDecoration(
                  color: bg,
                  borderRadius: BorderRadius.circular(controls.rowRadius),
                ),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    // checkbox 列
                    SizedBox(
                      width: controls.checkboxColumnWidth,
                      child: widget.showCheckbox
                          ? Align(
                              alignment: Alignment.centerLeft,
                              child: MateCheckbox(
                                checked: widget.checked,
                                // 三态循环 false→null 时回退为取反，保证可勾选
                                onChanged: (c) => widget
                                    .onCheckedChange(c ?? !widget.checked),
                              ),
                            )
                          : null,
                    ),
                    // name 列（v2：图标 32×32 色块 tile，间距 12）
                    Expanded(
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.center,
                        children: [
                          FileTypeTile(
                            file: file,
                            thumbnail: widget.thumbnail,
                          ),
                          SizedBox(width: controls.rowNameContentSpacing),
                          Expanded(
                            child: Text(
                              file.name,
                              style: typography.fileList.rowFileName.copyWith(
                                color: colors.textPrimary,
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        ],
                      ),
                    ),
                    // size 列
                    SizedBox(
                      width: widget.sizeWidth,
                      child: Text(
                        file.isFolder ? '—' : formatFileSize(file.size),
                        style: typography.fileList.rowFileSize.copyWith(
                          color: colors.textSecondary,
                        ),
                      ),
                    ),
                    // time 列
                    SizedBox(
                      width: widget.timeWidth,
                      child: Text(
                        file.editedTime == null
                            ? ''
                            : DateFormat('yyyy-MM-dd HH:mm')
                                .format(file.editedTime!),
                        style: typography.fileList.rowModifiedTime.copyWith(
                          color: colors.textSecondary,
                        ),
                      ),
                    ),
                    // status 列（v2 列宽 72；hover 显示文案 tooltip）
                    SizedBox(
                      width: controls.statusColumnWidth,
                      child: Center(child: _buildStatusIcon(colors, controls)),
                    ),
                    // actions 列（v2 列宽 44；操作按钮 → 右键菜单）
                    SizedBox(
                      width: controls.actionColumnWidth,
                      child: Center(
                        child: MateButton(
                          variant: MateButtonVariant.icon,
                          icon: 'list',
                          onClick: () => _openMenu(),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
        // 行底分隔线 0.5px
        const MateHDivider(),
      ],
    );
  }

  /// 状态列图标（hover 显示文案 tooltip，对标原 Vue :title）
  Widget _buildStatusIcon(
    MateSemanticColors colors,
    FileListControlMetrics controls,
  ) {
    final typography = MateTheme.typographyOf(context);
    final (icon, color, tip) = switch (widget.status) {
      'synced' => ('local', colors.success, '已同步到本地'),
      'placeholder' => ('cloud', colors.textSecondary, '本地占位'),
      'folder' => ('folder', colors.brand, '文件夹'),
      _ => ('cloud', colors.textPlaceholder, '仅云端（未同步到本地）'),
    };
    return Tooltip(
      message: tip,
      decoration: BoxDecoration(
        color: colors.bgContainer,
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: colors.border, width: 0.5),
      ),
      textStyle: typography.fileList.rowFileSize.copyWith(
        color: colors.textSecondary,
      ),
      child: MateIcon(
        name: icon,
        size: controls.rowStatusIconSize,
        tint: color,
      ),
    );
  }
}

