import 'package:flutter/material.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/pages/files/widgets/file_format.dart';
import 'package:petal_link/service/mount/free_up.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// 文件列表对话框集合（对标 CMP FileListScreen.kt 的 4 个对话框 + 删除确认）。
//
// - 删除确认 / 释放空间预览 / 属性：走 MateDialog 命令式宿主（v2 图标徽章标题）
// - 重命名 / 移动：自定义 Dialog（含表单与列表，showDialog 弹出）
// =============================================================================

/// 删除确认对话框（对标 CMP requestDelete）。
///
/// 按本地同步状态区分警告文案（对标原 Vue FileListView 的
/// checkFileLocalStatus 逻辑）：已双端对齐（synced）的项追加双端同删警告。
void confirmDeleteFiles({
  required List<DriveFile> selection,
  required Map<String, String> fileStatuses,
  required VoidCallback onConfirm,
}) {
  final String content;
  if (selection.length == 1) {
    final f = selection.first;
    if (f.isFolder) {
      content = '确定删除文件夹「${f.name}」吗？删除后进入回收站。';
    } else if (fileStatuses[f.id] == 'synced') {
      content = '确定删除「${f.name}」吗？\n\n'
          '⚠️ 此文件已双端对齐到本地，删除后云端和本地文件将同时被移除。'
          '删除后进入回收站，可从回收站恢复。';
    } else {
      content = '确定删除「${f.name}」吗？删除后进入回收站。';
    }
  } else {
    final syncedCount =
        selection.where((f) => fileStatuses[f.id] == 'synced').length;
    final base = '确定删除选中的 ${selection.length} 项吗？删除后进入回收站。';
    content = syncedCount > 0
        ? '$base\n\n⚠️ 其中 $syncedCount 项已双端对齐到本地，'
            '删除后云端和本地文件将同时被移除。删除后进入回收站，可从回收站恢复。'
        : base;
  }

  MateDialog.confirm(
    MateDialogOptions(
      title: '删除文件',
      content: content,
      confirmText: '删除',
      danger: true,
      titleIcon: 'trash',
    ),
    (confirmed) {
      if (confirmed) onConfirm();
    },
  );
}

/// 释放空间预览对话框（对标 CMP requestFreeUp 的预览确认）。
///
/// items 为空 → 「无法释放空间」单按钮；非空 → 逐项列表（前 10 项）+ 确认释放。
void confirmFreeUpItems({
  required List<FreeableItem> items,
  required VoidCallback onConfirm,
}) {
  final totalBytes = items.fold<int>(0, (sum, it) => sum + it.size);
  final String content;
  if (items.isEmpty) {
    content = '所选内容中没有通过远端校验、可安全释放的本地文件。';
  } else {
    final listed = items
        .take(10)
        .map((it) => '${it.name}（${formatFileSize(it.size)}）')
        .join('\n');
    final more = items.length > 10 ? '\n…等 ${items.length} 项' : '';
    content = '将释放 ${items.length} 个本地文件，共 ${formatFileSize(totalBytes)}。'
        '云端内容会保留，本地文件将变为占位符。\n\n$listed$more';
  }

  MateDialog.confirm(
    MateDialogOptions(
      title: items.isEmpty ? '无法释放空间' : '释放空间预览',
      content: content,
      confirmText: items.isEmpty ? '关闭' : '确认释放',
      danger: items.isNotEmpty,
      titleIcon: 'cloud',
    ),
    (confirmed) {
      if (confirmed && items.isNotEmpty) onConfirm();
    },
  );
}

/// 属性对话框（对标 CMP PropsDialog，5 行键值）。
void openFileProps(DriveFile target) {
  final buffer = StringBuffer()
    ..write('文件 ID：${target.id.isEmpty ? "—" : target.id}\n')
    ..write('类型：${target.isFolder ? "文件夹" : (target.mimeType ?? "文件")}\n')
    ..write(
        '大小：${target.isFolder ? "—" : formatFileSize(target.size)}\n')
    ..write('修改时间：${target.editedTime?.toIso8601String() ?? ""}');
  if (target.contentHash != null) {
    buffer.write('\nSHA256：${target.contentHash}');
  }

  MateDialog.open(MateDialogOptions(
    title: target.name,
    content: buffer.toString(),
    confirmText: '关闭',
  ));
}

/// 重命名对话框（对标 CMP RenameDialog；原 Vue MateDialog 重命名）。
///
/// 校验：非空、未变更、不含 `/`、不为 `.`/`..` 时禁用确定；回车确认（对标原 Vue @enter）。
class RenameFileDialog extends StatefulWidget {
  /// 重命名目标
  final DriveFile target;

  /// 确认回调（传去除首尾空白的新名称）
  final void Function(String newName) onConfirm;

  const RenameFileDialog({
    super.key,
    required this.target,
    required this.onConfirm,
  });

  @override
  State<RenameFileDialog> createState() => _RenameFileDialogState();
}

class _RenameFileDialogState extends State<RenameFileDialog> {
  late final TextEditingController _controller;

  String get _currentName => widget.target.name;

  bool get _valid {
    final v = _controller.text.trim();
    return v.isNotEmpty &&
        v != _currentName &&
        !v.contains('/') &&
        v != '.' &&
        v != '..';
  }

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.target.name);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _confirm() {
    if (!_valid) return;
    Navigator.of(context).pop();
    widget.onConfirm(_controller.text.trim());
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).fileList;

    return Dialog(
      backgroundColor: Colors.transparent,
      elevation: 0,
      child: Container(
        width: metrics.renameDialogWidth,
        padding: EdgeInsets.all(metrics.renameDialogPadding),
        decoration: BoxDecoration(
          color: colors.bgContainer,
          borderRadius: BorderRadius.circular(metrics.renameDialogRadius),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '重命名',
              style: typography.fileList.renameDialogTitle.copyWith(
                color: colors.textPrimary,
              ),
            ),
            SizedBox(height: metrics.renameDialogContentSpacing),
            MateTextField(
              controller: _controller,
              error: _controller.text.isNotEmpty && !_valid,
              autofocus: true,
              onChanged: (_) => setState(() {}),
              onSubmit: (_) => _confirm(),
            ),
            SizedBox(height: metrics.renameDialogContentSpacing),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                MateButton(
                  label: '取消',
                  variant: MateButtonVariant.text,
                  onClick: () => Navigator.of(context).pop(),
                ),
                SizedBox(width: metrics.renameDialogActionSpacing),
                MateButton(
                  label: '确定',
                  variant: MateButtonVariant.primary,
                  disabled: !_valid,
                  onClick: _confirm,
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// 移动到对话框（对标 CMP MoveDialog：从已加载目录树选择目标文件夹）。
class MoveFileDialog extends StatefulWidget {
  /// 移动目标
  final DriveFile target;

  /// 可选目标文件夹（已排除自身）
  final List<DriveFile> folders;

  /// 确认回调（传目标父目录 ID）
  final void Function(String parentId) onConfirm;

  const MoveFileDialog({
    super.key,
    required this.target,
    required this.folders,
    required this.onConfirm,
  });

  @override
  State<MoveFileDialog> createState() => _MoveFileDialogState();
}

class _MoveFileDialogState extends State<MoveFileDialog> {
  String? _selectedParentId;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).fileList;

    return Dialog(
      backgroundColor: Colors.transparent,
      elevation: 0,
      child: Container(
        width: metrics.moveDialogWidth,
        padding: EdgeInsets.all(metrics.moveDialogPadding),
        decoration: BoxDecoration(
          color: colors.bgContainer,
          borderRadius: BorderRadius.circular(metrics.moveDialogRadius),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '移动“${widget.target.name}”',
              style: typography.fileList.moveDialogTitle.copyWith(
                color: colors.textPrimary,
              ),
            ),
            SizedBox(height: metrics.moveDialogContentSpacing),
            if (widget.folders.isEmpty)
              Text(
                '当前已加载的目录树中没有可选目标，请先在侧边栏展开目标目录。',
                style: typography.fileList.moveDialogDescription.copyWith(
                  color: colors.textSecondary,
                ),
              )
            else
              SizedBox(
                height: metrics.moveDialogFolderListHeight,
                child: ListView.builder(
                  itemCount: widget.folders.length,
                  itemBuilder: (context, index) {
                    final folder = widget.folders[index];
                    final selected = folder.id == _selectedParentId;
                    return GestureDetector(
                      onTap: () =>
                          setState(() => _selectedParentId = folder.id),
                      child: Container(
                        width: double.infinity,
                        padding: EdgeInsets.all(metrics.moveDialogFolderPadding),
                        decoration: BoxDecoration(
                          color: selected
                              ? colors.brandLighter
                              : Colors.transparent,
                          borderRadius: BorderRadius.circular(
                            metrics.moveDialogFolderRadius,
                          ),
                        ),
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.center,
                          children: [
                            MateIcon(
                              name: 'folder',
                              size: metrics.moveDialogFolderIconSize,
                              tint: colors.folder,
                            ),
                            SizedBox(
                              width: metrics.moveDialogFolderContentSpacing,
                            ),
                            Expanded(
                              child: Text(
                                folder.name,
                                style:
                                    typography.fileList.moveDialogFolder.copyWith(
                                  color: colors.textPrimary,
                                ),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                          ],
                        ),
                      ),
                    );
                  },
                ),
              ),
            SizedBox(height: metrics.moveDialogContentSpacing),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                MateButton(
                  label: '取消',
                  variant: MateButtonVariant.text,
                  onClick: () => Navigator.of(context).pop(),
                ),
                SizedBox(width: metrics.moveDialogActionSpacing),
                MateButton(
                  label: '移动',
                  variant: MateButtonVariant.primary,
                  disabled: _selectedParentId == null,
                  onClick: () {
                    final id = _selectedParentId;
                    if (id == null) return;
                    Navigator.of(context).pop();
                    widget.onConfirm(id);
                  },
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
