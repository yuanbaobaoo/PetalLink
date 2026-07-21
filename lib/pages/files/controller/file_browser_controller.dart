import 'dart:typed_data';

import 'package:get/get.dart';

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/config_entry.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/thumbnail_service.dart';
import 'package:petal_link/service/mount/free_up.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/widgets/index.dart';

export 'package:petal_link/entity/config_entry.dart' show SortField;

part 'file_operations.dart';

/// 空时间占位（editedTime 为 null 时按最早时间排序）
final DateTime _epoch = DateTime.fromMillisecondsSinceEpoch(0, isUtc: true);

/// 面包屑导航节点
///
/// 对标 CMP BrowserBreadcrumb（ViewModels.kt）
class Breadcrumb {
  /// 文件夹 ID（null 表示根目录）
  final String? id;

  /// 展示名称
  final String name;

  const Breadcrumb({this.id, required this.name});
}

/// 文件浏览器状态
///
/// 对标 CMP FileBrowserState（ViewModels.kt）与 Vue fileBrowser store（docs/08 §2.3）。
class FileBrowserState {
  /// 当前文件夹 ID（null 表示根目录）
  final String? folderId;

  /// 面包屑路径栈
  final List<Breadcrumb> breadcrumbs;

  /// 当前文件列表
  final List<DriveFile> files;

  /// 下一页游标（分页）
  final String? nextCursor;

  /// 搜索关键词（本地过滤）
  final String query;

  /// 排序字段
  final SortField sortField;

  /// 是否升序
  final bool ascending;

  /// 是否加载中
  final bool loading;

  /// 各文件夹的子目录缓存（key 为 folderId，"__root__" 表示根目录）
  final Map<String, List<DriveFile>> directoryChildren;

  /// 正在懒加载子目录的文件夹 ID 集合（目录树节点 loading 指示）
  final Set<String> treeLoadingIds;

  const FileBrowserState({
    this.folderId,
    this.breadcrumbs = const [Breadcrumb(name: '全部文件')],
    this.files = const [],
    this.nextCursor,
    this.query = '',
    this.sortField = SortField.Name,
    this.ascending = true,
    this.loading = false,
    this.directoryChildren = const {},
    this.treeLoadingIds = const {},
  });

  /// 初始状态
  factory FileBrowserState.initial() => const FileBrowserState();

  /// 可见文件列表（经过搜索过滤 + 排序，文件夹优先）
  List<DriveFile> get visibleFiles {
    // 搜索过滤
    final filtered = query.isEmpty
        ? List<DriveFile>.of(files)
        : files.where((f) {
            return f.name.toLowerCase().contains(query.toLowerCase());
          }).toList();

    // 排序：文件夹优先，同类型内按 sortField 排序
    filtered.sort((a, b) {
      // 文件夹优先
      if (a.isFolder && !b.isFolder) return -1;
      if (!a.isFolder && b.isFolder) return 1;

      // 同类型内排序
      int result;
      switch (sortField) {
        case SortField.Name:
          result = a.name.toLowerCase().compareTo(b.name.toLowerCase());
        case SortField.Size:
          result = a.size.compareTo(b.size);
        case SortField.ModifiedTime:
          result = (a.editedTime ?? _epoch).compareTo(b.editedTime ?? _epoch);
      }
      return ascending ? result : -result;
    });

    return filtered;
  }

  /// 深拷贝并替换指定字段
  FileBrowserState copyWith({
    String? folderId,
    List<Breadcrumb>? breadcrumbs,
    List<DriveFile>? files,
    String? nextCursor,
    String? query,
    SortField? sortField,
    bool? ascending,
    bool? loading,
    Map<String, List<DriveFile>>? directoryChildren,
    Set<String>? treeLoadingIds,
    bool clearFolderId = false,
    bool clearNextCursor = false,
    bool clearQuery = false,
  }) {
    return FileBrowserState(
      folderId: clearFolderId ? null : (folderId ?? this.folderId),
      breadcrumbs: breadcrumbs ?? this.breadcrumbs,
      files: files ?? this.files,
      nextCursor: clearNextCursor ? null : (nextCursor ?? this.nextCursor),
      query: clearQuery ? '' : (query ?? this.query),
      sortField: sortField ?? this.sortField,
      ascending: ascending ?? this.ascending,
      loading: loading ?? this.loading,
      directoryChildren: directoryChildren ?? this.directoryChildren,
      treeLoadingIds: treeLoadingIds ?? this.treeLoadingIds,
    );
  }
}

/// 文件浏览器控制器 — 文件浏览页状态管理
///
/// 对标 CMP FileBrowserViewModel（ViewModels.kt）+ ApplicationRoot 文件操作
/// 接线（ApplicationRoot.kt）与 Vue fileBrowser store（docs/08 §2.3）。
///
/// 核心机制：
/// - **requestId 乱序保护**（对标 CMP requestSequence）：每次 beginLoad 递增，
///   applyPage 时过期请求或不同目录的请求被拒绝。
/// - **目录树缓存 + 懒加载**：directoryChildren 缓存各文件夹的直接子目录，
///   [loadTreeChildren] 懒加载未加载过的节点（treeLoadingIds 做加载指示）。
/// - **文件操作**（[file_operations.dart]，part 拆分）：删除/重命名/移动/
///   双端对齐/按需下载/释放空间，完成后刷新并反馈 toast/errorMessage。
class FileBrowserController extends GetxController {
  /// 构造（服务可注入伪造实现；未注入时延迟解析 Get 单例）
  FileBrowserController({
    FilesService? filesService,
    ThumbnailService? thumbnailService,
    SyncService? syncService,
    ConfigService? configService,
  })  : _filesServiceOverride = filesService,
        _thumbnailServiceOverride = thumbnailService,
        _syncServiceOverride = syncService,
        _configServiceOverride = configService;

  final FilesService? _filesServiceOverride;
  final ThumbnailService? _thumbnailServiceOverride;
  final SyncService? _syncServiceOverride;
  final ConfigService? _configServiceOverride;

  FilesService get _filesService =>
      _filesServiceOverride ?? Get.find<FilesService>();
  ThumbnailService get _thumbnailService =>
      _thumbnailServiceOverride ?? Get.find<ThumbnailService>();
  SyncService get _syncService =>
      _syncServiceOverride ?? Get.find<SyncService>();
  ConfigService get _configService =>
      _configServiceOverride ?? Get.find<ConfigService>();

  /// 文件浏览器状态（响应式）
  final Rx<FileBrowserState> state = FileBrowserState.initial().obs;

  /// 当前目录文件的本地同步状态（fileId `->` folder/synced/placeholder/not_synced）
  final RxMap<String, String> fileStatuses = <String, String>{}.obs;

  /// 已加载的缩略图（fileId `->` 二进制内容）
  final RxMap<String, Uint8List> thumbnails = <String, Uint8List>{}.obs;

  /// 挂载目录是否已配置（驱动 AppBar 按钮/右键菜单/批量条条件渲染）
  final RxBool mountConfigured = false.obs;

  /// 挂载目录绝对路径（~ 已展开；未配置为空串）
  final RxString mountDir = ''.obs;

  /// 常驻错误横幅文案（空串 = 无；对齐 CMP MainScreen errorMessage）
  final RxString errorMessage = ''.obs;

  /// 按需下载进度文案（非空时文件区显示下载遮罩；对齐 CMP downloadProgressText）
  final RxString downloadProgressText = ''.obs;

  /// 请求序列（monotonic increasing，乱序保护）
  int _requestSequence = 0;

  /// 最后接受的请求序列号
  int _acceptedRequest = -1;

  /// 各文件夹子目录缓存（folderId `->` `List<DriveFile>`）
  final Map<String?, List<DriveFile>> _childrenByFolder = {};

  /// 请求中的缩略图 fileId 集合（防重复拉取）
  final Set<String> _thumbnailRequests = {};

  /// 根目录 key
  static const String rootKey = '__root__';

  /// 目录树懒加载分页上限（对齐 CMP loadTreeChildren pages < 20）
  static const int _treeLoadMaxPages = 20;

  // ═══════════════════════════════════════════════════════════════════
  // 文件加载（requestId 乱序保护）
  // ═══════════════════════════════════════════════════════════════════

  /// 开始一次加载：递增请求序列，进入 loading 态
  ///
  /// 对标 CMP FileBrowserViewModel.beginLoad()
  /// @return 本次请求 ID
  int _beginLoad() {
    _requestSequence++;
    state.value = state.value.copyWith(loading: true);
    return _requestSequence;
  }

  /// 加载当前文件夹的文件列表
  ///
  /// 从 FilesService.list 获取，自动应用请求 ID 乱序保护。
  Future<void> loadFiles() async {
    final requestId = _beginLoad();
    final folderId = state.value.folderId;

    AppLogger.d('loadFiles: folderId=$folderId requestId=$requestId');

    try {
      final result = await _filesService.list(parentId: folderId);

      if (result.isErr) {
        AppLogger.e('loadFiles 失败: ${(result as Err).error}');
        state.value = state.value.copyWith(loading: false);
        return;
      }

      final page = (result as Ok<FileListResult>).value;
      _applyPage(
        requestId: requestId,
        folderId: folderId,
        files: page.files,
        nextCursor: page.nextCursor,
        append: false,
      );
    } catch (e, st) {
      AppLogger.e('loadFiles 异常', e, st);
      state.value = state.value.copyWith(loading: false);
    }
  }

  /// 加载更多文件：基于分页游标追加下一页结果（对标 CMP loadMore）
  Future<void> loadMore() async {
    final current = state.value;
    final cursor = current.nextCursor;
    if (cursor == null) return;
    final requestId = _beginLoad();
    final folderId = current.folderId;

    try {
      final result =
          await _filesService.list(parentId: folderId, cursor: cursor);
      if (result.isErr) {
        AppLogger.e('loadMore 失败: ${(result as Err).error}');
        state.value = state.value.copyWith(loading: false);
        return;
      }
      final page = (result as Ok<FileListResult>).value;
      _applyPage(
        requestId: requestId,
        folderId: folderId,
        files: page.files,
        nextCursor: page.nextCursor,
        append: true,
      );
    } catch (e, st) {
      AppLogger.e('loadMore 异常', e, st);
      state.value = state.value.copyWith(loading: false);
    }
  }

  /// 应用一页文件结果（乱序保护）
  ///
  /// 对标 CMP FileBrowserViewModel.applyPage()：
  /// - requestId < acceptedRequest → 拒绝
  /// - folderId 不匹配当前 → 拒绝
  /// - append=false → 替换；append=true → 追加（去重）
  ///
  /// @return 是否被接受
  bool _applyPage({
    required int requestId,
    required String? folderId,
    required List<DriveFile> files,
    String? nextCursor,
    bool append = false,
  }) {
    // 乱序保护：过期请求或不同目录
    if (requestId < _acceptedRequest || folderId != state.value.folderId) {
      AppLogger.d(
          '_applyPage 拒绝: requestId=$requestId accepted=$_acceptedRequest');
      return false;
    }

    _acceptedRequest = requestId;

    // 合并文件列表
    final merged = append ? _mergeUnique(state.value.files, files) : files;

    // 缓存子目录
    final key = folderId ?? rootKey;
    _childrenByFolder[key] = merged.where((f) => f.isFolder).toList();

    state.value = state.value.copyWith(
      files: merged,
      nextCursor: nextCursor,
      loading: false,
      directoryChildren: {
        ...state.value.directoryChildren,
        key: _childrenByFolder[key]!,
      },
    );

    AppLogger.d(
        '_applyPage: ${merged.length} 个文件 (目录:${_childrenByFolder[key]!.length})');

    // 刷新文件本地同步状态（对齐 CMP refreshInternal 的 syncBatchFileStatus）
    _refreshFileStatuses(merged);
    return true;
  }

  /// 合并列表并去重（按 id），使用 `Set` 跟踪已见过的 id 避免重复
  List<DriveFile> _mergeUnique(
      List<DriveFile> existing, List<DriveFile> incoming) {
    final seen = <String>{};
    for (final item in existing) {
      if (item.id.isNotEmpty) seen.add(item.id);
    }
    final result = List<DriveFile>.from(existing);
    for (final item in incoming) {
      if (!seen.contains(item.id)) {
        result.add(item);
        if (item.id.isNotEmpty) seen.add(item.id);
      }
    }
    return result;
  }

  /// 批量查询当前目录文件的本地同步状态（失败静默，保留旧状态）
  Future<void> _refreshFileStatuses(List<DriveFile> files) async {
    final ids = files.map((f) => f.id).where((id) => id.isNotEmpty).toList();
    if (ids.isEmpty) {
      fileStatuses.clear();
      return;
    }
    try {
      final statuses = await _syncService.batchFileStatus(ids);
      fileStatuses.assignAll(statuses);
    } catch (e) {
      AppLogger.d('批量查询文件状态失败（引擎可能未启动）: $e');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 导航
  // ═══════════════════════════════════════════════════════════════════

  /// 进入指定文件夹
  ///
  /// 对标 CMP FileBrowserViewModel.enter()：
  /// - 仅文件夹可进入
  /// - 追加面包屑节点
  /// - 清空文件列表 + 进入 loading
  void enterFolder(DriveFile folder) {
    if (!folder.isFolder) {
      AppLogger.d('enterFolder: ${folder.name} 不是文件夹，忽略');
      return;
    }
    if (folder.id.isEmpty) {
      // 根节点（"全部文件"）回到首级
      navigateToBreadcrumb(0);
      return;
    }

    _acceptedRequest = ++_requestSequence;
    state.value = state.value.copyWith(
      folderId: folder.id,
      breadcrumbs: [
        ...state.value.breadcrumbs,
        Breadcrumb(id: folder.id, name: folder.name),
      ],
      files: [],
      nextCursor: null,
      query: '',
      loading: true,
    );

    loadFiles();
  }

  /// 目录树节点导航：用节点自带完整路径替换面包屑
  ///
  /// 对标 CMP ApplicationRoot.enterFolderFromTree（原 Vue SidebarTreeNode
  /// 的 pathStack 替换）。
  void enterFolderFromTree(DriveFile folder) {
    if (!folder.isFolder) return;
    if (folder.id.isEmpty) {
      navigateToBreadcrumb(0);
      return;
    }
    final path = treePathTo(folder.id);
    _acceptedRequest = ++_requestSequence;
    state.value = state.value.copyWith(
      folderId: folder.id,
      breadcrumbs: path,
      files: [],
      nextCursor: null,
      query: '',
      loading: true,
    );
    loadFiles();
  }

  /// 计算目录树节点从根到自身的完整路径（对标 CMP treePathTo）
  List<Breadcrumb> treePathTo(String folderId) {
    const root = [Breadcrumb(name: '全部文件')];
    final names = <String, String>{};
    final parentOf = <String, String?>{};
    for (final entry in state.value.directoryChildren.entries) {
      for (final child in entry.value) {
        if (child.id.isEmpty) continue;
        names[child.id] = child.name;
        parentOf[child.id] = entry.key == rootKey ? null : entry.key;
      }
    }
    final segments = <Breadcrumb>[];
    String? current = folderId;
    var guard = 0;
    while (current != null && guard++ < 100) {
      final name = names[current];
      if (name == null) break;
      segments.add(Breadcrumb(id: current, name: name));
      current = parentOf[current];
    }
    return [...root, ...segments.reversed];
  }

  /// 跳转到面包屑路径中的第 index 级
  ///
  /// 对标 CMP FileBrowserViewModel.navigateTo()：
  /// - 截断路径至该节点
  /// - 清空文件列表 + 进入 loading
  void navigateToBreadcrumb(int index) {
    if (index < 0 || index >= state.value.breadcrumbs.length) return;

    _acceptedRequest = ++_requestSequence;
    final path = state.value.breadcrumbs.take(index + 1).toList();
    final target = path.last;

    // 根目录 target.id 为 null，需显式清空 folderId
    state.value = state.value.copyWith(
      folderId: target.id,
      clearFolderId: target.id == null,
      breadcrumbs: path,
      files: [],
      nextCursor: null,
      query: '',
      loading: true,
    );

    loadFiles();
  }

  /// 返回上一级
  void goUp() {
    if (state.value.breadcrumbs.length > 1) {
      navigateToBreadcrumb(state.value.breadcrumbs.length - 2);
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 目录树懒加载
  // ═══════════════════════════════════════════════════════════════════

  /// 懒加载指定文件夹的全部子目录写入目录树缓存（不改变当前浏览位置）。
  ///
  /// 对标 CMP ApplicationRoot.loadTreeChildren：失败静默，重新展开可重试。
  Future<void> loadTreeChildren(DriveFile folder) async {
    final id = folder.id;
    if (id.isEmpty) return;
    if (state.value.treeLoadingIds.contains(id)) return;

    state.value = state.value.copyWith(
      treeLoadingIds: {...state.value.treeLoadingIds, id},
    );

    try {
      final all = <DriveFile>[];
      String? cursor;
      var pages = 0;
      while (pages < _treeLoadMaxPages) {
        final result = await _filesService.list(
          parentId: id,
          cursor: cursor,
          pageSize: 100,
        );
        if (result.isErr) {
          throw (result as Err).error;
        }
        final page = (result as Ok<FileListResult>).value;
        all.addAll(page.files);
        pages++;
        if (!page.hasNext) break;
        cursor = page.nextCursor;
      }

      final folders = all.where((f) => f.isFolder).toList();
      _childrenByFolder[id] = folders;
      state.value = state.value.copyWith(
        directoryChildren: {...state.value.directoryChildren, id: folders},
        treeLoadingIds: {...state.value.treeLoadingIds}..remove(id),
      );
    } catch (e, st) {
      AppLogger.e('loadTreeChildren 失败: $id', e, st);
      state.value = state.value.copyWith(
        treeLoadingIds: {...state.value.treeLoadingIds}..remove(id),
      );
    }
  }

  /// 获取指定文件夹下的缓存子目录列表（供目录树懒加载）
  ///
  /// 对标 CMP FileBrowserViewModel.treeChildren()
  List<DriveFile> treeChildren(String? folderId) {
    return _childrenByFolder[folderId] ?? [];
  }

  // ═══════════════════════════════════════════════════════════════════
  // 搜索与排序
  // ═══════════════════════════════════════════════════════════════════

  /// 搜索文件（远程搜索）
  ///
  /// 对标 CMP ApplicationRoot.search()：
  /// - 空关键词 → 恢复当前文件夹列表
  /// - 非空 → beginLoad + FilesService.search + applyPage（乱序保护）
  Future<void> searchFiles(String query) async {
    if (query.trim().isEmpty) {
      await clearSearch();
      return;
    }

    final keyword = query.trim();
    final requestId = _beginLoad();
    final folderId = state.value.folderId;
    state.value = state.value.copyWith(query: keyword);

    try {
      final result = await _filesService.search(keyword);

      if (result.isErr) {
        AppLogger.e('searchFiles 失败: ${(result as Err).error}');
        state.value = state.value.copyWith(loading: false);
        return;
      }

      final page = (result as Ok<FileListResult>).value;
      _applyPage(
        requestId: requestId,
        folderId: folderId,
        files: page.files,
        nextCursor: page.nextCursor,
        append: false,
      );
    } catch (e, st) {
      AppLogger.e('searchFiles 异常', e, st);
      state.value = state.value.copyWith(loading: false);
    }
  }

  /// 清空搜索，恢复当前文件夹
  Future<void> clearSearch() async {
    state.value = state.value.copyWith(clearQuery: true);
    await loadFiles();
  }

  /// 切换排序字段
  ///
  /// 对标 CMP FileBrowserViewModel.sort()：
  /// - 同字段再次点击 → 反转升降序
  /// - 不同字段 → 升序（默认升序）
  void sort(SortField field) {
    final current = state.value;
    state.value = current.copyWith(
      sortField: field,
      ascending: field == current.sortField ? !current.ascending : true,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 刷新
  // ═══════════════════════════════════════════════════════════════════

  /// 刷新当前文件夹
  ///
  /// 对标 CMP fileBrowser.refresh()
  @override
  Future<void> refresh() async {
    AppLogger.d('refresh: ${state.value.folderId}');
    await loadFiles();
  }
}
