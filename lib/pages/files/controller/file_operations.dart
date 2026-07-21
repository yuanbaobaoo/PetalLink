part of 'file_browser_controller.dart';

/// 文件操作与挂载配置（[FileBrowserController] 的 part 拆分）。
///
/// 对标 CMP ApplicationRoot.kt 的命令接线：openItem / downloadItems /
/// deleteItems / renameItem / moveItem / syncFolder / canFreeUp /
/// previewFreeUpItems / freeUpItems + 缩略图加载。
extension FileBrowserOperations on FileBrowserController {
  // ═══════════════════════════════════════════════════════════════════
  // 挂载配置
  // ═══════════════════════════════════════════════════════════════════

  /// 重新读取挂载配置（页面进入 / 引擎重启后调用）
  Future<void> reloadMountConfig() async {
    try {
      final cfg = await _configService.configLoad();
      mountConfigured.value = cfg.mountConfigured;
      mountDir.value = cfg.mountConfigured ? cfg.expandedMountDir : '';
    } catch (e, st) {
      AppLogger.e('reloadMountConfig 失败', e, st);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 缩略图
  // ═══════════════════════════════════════════════════════════════════

  /// 异步加载文件缩略图（仅 image/video 且有 thumbnailLink；
  /// 已加载或请求中的不重复拉取；失败静默。对齐 CMP loadThumbnail）
  Future<void> loadThumbnail(DriveFile file) async {
    final id = file.id;
    if (id.isEmpty) return;
    final mime = file.mimeType ?? '';
    if (!mime.startsWith('image/') && !mime.startsWith('video/')) return;
    if ((file.thumbnailLink ?? '').isEmpty) return;
    if (thumbnails.containsKey(id)) return;
    if (!_thumbnailRequests.add(id)) return;

    try {
      final result = await _thumbnailService.getThumbnail(id);
      if (result.isOk) {
        thumbnails[id] = (result as Ok<Uint8List>).value;
      }
    } catch (e) {
      AppLogger.d('loadThumbnail 失败: $id: $e');
    } finally {
      _thumbnailRequests.remove(id);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 路径计算
  // ═══════════════════════════════════════════════════════════════════

  /// 基于当前面包屑计算文件相对于挂载根的路径（对标 CMP relativePathFor）
  String relativePathFor(DriveFile file) {
    final segments = [
      ...state.value.breadcrumbs.skip(1).map((b) => b.name),
      file.name,
    ];
    return segments.join('/');
  }

  /// 计算文件在本地挂载目录中的绝对路径（校验不越出挂载根）
  String localPathFor(DriveFile file) {
    return MountPath.safeJoinUnder(mountDir.value, relativePathFor(file));
  }

  // ═══════════════════════════════════════════════════════════════════
  // 文件操作（对标 CMP ApplicationRoot 各命令接线）
  // ═══════════════════════════════════════════════════════════════════

  /// 打开文件项：文件夹进入；文件按需下载到本地后刷新
  /// （下载期间显示进度遮罩，对齐 CMP openItem）。
  Future<void> openItem(DriveFile file) async {
    if (file.isFolder) return enterFolder(file);
    if (!mountConfigured.value) return;
    if (file.id.isEmpty) return;

    downloadProgressText.value = '正在下载 ${file.name}…';
    try {
      await _syncService.downloadOnDemand(
        fileId: file.id,
        destPath: localPathFor(file),
      );
      errorMessage.value = '';
      MateToast.show('已下载 ${file.name}', variant: MateToastVariant.success);
      await refresh();
    } catch (e) {
      AppLogger.e('openItem 失败: ${file.name}', e);
      errorMessage.value = '下载 ${file.name} 失败: $e';
    } finally {
      downloadProgressText.value = '';
    }
  }

  /// 批量下载指定文件项：跳过文件夹（对齐 CMP downloadItems）
  Future<void> downloadItems(List<DriveFile> files) async {
    if (!mountConfigured.value) return;
    final targets = files.where((f) => !f.isFolder).toList();
    if (targets.isEmpty) return;

    var downloaded = 0;
    String? firstError;
    downloadProgressText.value = '正在批量下载 ${targets.length} 项…';
    try {
      for (final file in targets) {
        if (file.id.isEmpty) continue;
        try {
          await _syncService.downloadOnDemand(
            fileId: file.id,
            destPath: localPathFor(file),
          );
          downloaded++;
        } catch (e) {
          firstError ??= '${file.name}: $e';
        }
      }
      if (firstError != null) {
        errorMessage.value = firstError;
        MateToast.show('下载失败：$firstError', variant: MateToastVariant.error);
      } else {
        errorMessage.value = '';
        MateToast.show('已下载 $downloaded 项', variant: MateToastVariant.success);
      }
      await refresh();
    } finally {
      downloadProgressText.value = '';
    }
  }

  /// 批量删除指定文件项：逐条收集失败原因 toast 汇总（对齐 CMP deleteItems）
  Future<void> deleteItems(List<DriveFile> files) async {
    final errors = <String>[];
    for (final file in files) {
      if (file.id.isEmpty) {
        errors.add('${file.name} 缺少 id');
        continue;
      }
      final result = await _filesService.deleteVerified(file.id);
      if (result.isErr) {
        errors.add('${file.name}: ${(result as Err).error.message}');
      }
    }

    if (errors.isEmpty) {
      errorMessage.value = '';
      MateToast.show('已删除 ${files.length} 项', variant: MateToastVariant.success);
    } else {
      errorMessage.value = errors.first;
      MateToast.show(
        '删除失败 ${errors.length} 项：${errors.first}',
        variant: MateToastVariant.error,
      );
    }
    await refresh();
  }

  /// 重命名指定文件项（对齐 CMP renameItem）
  Future<void> renameItem(DriveFile file, String newName) async {
    if (file.id.isEmpty) return;
    final result = await _filesService.rename(file.id, newName.trim());
    if (result.isErr) {
      errorMessage.value = (result as Err).error.message;
      return;
    }
    errorMessage.value = '';
    await refresh();
  }

  /// 将指定文件项移动到新的父目录（对齐 CMP moveItem）
  Future<void> moveItem(DriveFile file, String newParentId) async {
    if (file.id.isEmpty) return;
    // update 内部会 GET 当前唯一 parent 构造成对移动参数（fileId 级幂等）
    final result =
        await _filesService.update(file.id, newParentFolder: newParentId);
    if (result.isErr) {
      errorMessage.value = (result as Err).error.message;
      return;
    }
    errorMessage.value = '';
    await refresh();
  }

  /// 递归检查并同步指定文件夹（双端对齐；对齐 CMP syncFolder）
  Future<void> syncFolder(DriveFile file) async {
    if (file.id.isEmpty) return;
    try {
      await _syncService.folderRecursive(
        folderId: file.id,
        relPath: relativePathFor(file),
      );
      errorMessage.value = '';
      MateToast.show('已开始双端对齐：${file.name}');
    } catch (e) {
      AppLogger.e('syncFolder 失败: ${file.name}', e);
      errorMessage.value = '$e';
      MateToast.show('双端对齐失败：$e', variant: MateToastVariant.error);
    }
  }

  /// 检查指定文件是否可安全释放本地空间（对齐 CMP canFreeUp）
  Future<void> canFreeUp(DriveFile file, void Function(bool) onResult) async {
    if (file.isFolder) {
      onResult(mountConfigured.value);
      return;
    }
    if (file.id.isEmpty) {
      onResult(false);
      return;
    }
    try {
      final result =
          await _syncService.checkSafeFreeUp(relativePathFor(file), file.id);
      onResult(result == 'safe');
    } catch (e) {
      AppLogger.d('canFreeUp 失败: ${file.name}: $e');
      onResult(false);
    }
  }

  /// 展开所选文件和目录，生成去重后的释放空间预览项
  /// （对齐 CMP previewFreeUpItems）。
  Future<void> previewFreeUpItems(
    List<DriveFile> files,
    void Function(List<FreeableItem>) onResult,
  ) async {
    final items = <FreeableItem>[];
    for (final file in files) {
      if (file.isFolder) {
        try {
          final folderItems =
              await _syncService.listFreeableInFolder(relativePathFor(file));
          items.addAll(folderItems);
        } catch (e) {
          AppLogger.d('listFreeableInFolder 失败: ${file.name}: $e');
        }
      } else if (file.id.isNotEmpty) {
        items.add(FreeableItem(
          fileId: file.id,
          relPath: relativePathFor(file),
          name: file.name,
          size: file.size,
        ));
      }
    }
    // 按 fileId 去重
    final seen = <String>{};
    onResult(items.where((it) => seen.add(it.fileId)).toList());
  }

  /// 执行用户已确认的释放空间预览项，完成后刷新（对齐 CMP freeUpItems）
  Future<void> freeUpItems(List<FreeableItem> items) async {
    try {
      final result = await _syncService.freeUpBatch(items);
      errorMessage.value = result.errors.isNotEmpty ? result.errors.first : '';
      MateToast.show(
        '已释放 ${result.freedCount} 项（${_formatBytes(result.freedBytes)}）',
        variant: result.errors.isEmpty
            ? MateToastVariant.success
            : MateToastVariant.warning,
      );
    } catch (e) {
      AppLogger.e('freeUpItems 失败', e);
      errorMessage.value = '$e';
      MateToast.show('释放空间失败：$e', variant: MateToastVariant.error);
    }
    await refresh();
  }

  /// 字节格式化（释放空间反馈文案用，与 widgets/file_format.dart 一致）
  static String _formatBytes(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1048576) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    if (bytes < 1073741824) {
      return '${(bytes / 1048576).toStringAsFixed(1)} MB';
    }
    return '${(bytes / 1073741824).toStringAsFixed(2)} GB';
  }
}
