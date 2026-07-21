import 'dart:async';

import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/sync/sync_controller.dart';
import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/transfer/transfer_controller.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/pages/files/controller/file_browser_controller.dart';
import 'package:petal_link/pages/files/widgets/app_bar.dart';
import 'package:petal_link/pages/files/widgets/breadcrumb.dart';
import 'package:petal_link/pages/files/widgets/file_list_view.dart';
import 'package:petal_link/pages/files/widgets/search_results.dart';
import 'package:petal_link/pages/files/widgets/sidebar.dart';
import 'package:petal_link/pages/files/widgets/sync_setup_banner.dart';
import 'package:petal_link/pages/files/widgets/sync_status_bar.dart';
import 'package:petal_link/pages/files/widgets/transfer_popover.dart';
import 'package:petal_link/widgets/index.dart';

/// 文件浏览主页面（对标 CMP MainScreen.kt 装配层；原 Vue MainPage.vue）。
///
/// 左栏 FilesSidebar(248px) + 右侧主区：
/// FilesAppBar(64px) + 信息区（FilesSyncSetupBanner / FilesSyncStatusBar +
/// 常驻错误横幅）+ FilesBreadcrumb + 文件区（FilesSearchResults /
/// FileListView）+ TransferPopover 浮层 + 加载/下载遮罩。
class FilesPage extends StatefulWidget {
  const FilesPage({super.key});

  @override
  State<FilesPage> createState() => _FilesPageState();
}

class _FilesPageState extends State<FilesPage> {
  /// 页面级控制器（Get.put 注册，dispose 时 Get.delete）
  late final FileBrowserController _browser;

  /// 全局控制器（GlobalBinding 常驻，仅获取不持有生命周期）
  late final SyncController _sync;
  late final TransferController _transfer;
  late final UpdateController _update;
  late final AuthController _auth;

  /// Worker 列表（dispose 统一释放）
  final List<Worker> _workers = [];

  /// 同步权威快照（经 [SyncController.rawSnapshot] 观察；
  /// 不再自行订阅 SyncService.stateStream，避免双重订阅）
  Rx<SyncGlobalState> get _snapshot => _sync.rawSnapshot;

  /// 配置是否已加载（setupPhase 的 loading 判定）
  bool _configLoaded = false;

  /// 搜索关键词：仅回车提交才触发远端搜索；输入过程只更新本地显示
  final TextEditingController _searchController = TextEditingController();
  String _searchKeyword = '';
  String _submittedSearch = '';

  /// TransferPopover 显隐
  bool _showTransferPopover = false;

  @override
  void initState() {
    super.initState();
    _browser = Get.put(FileBrowserController());
    // 全局控制器均由 GlobalBinding permanent 注册（启动必存在），
    // 直接 find——find-or-put 兜底会掩盖注册顺序缺陷并产生孤儿实例
    _sync = Get.find<SyncController>();
    _transfer = Get.find<TransferController>();
    _update = Get.find<UpdateController>();
    _auth = Get.find<AuthController>();

    // 目录内容变更 → 刷新当前目录 + 重读挂载配置（对齐 Vue sidebarRefresh 订阅）
    _workers.add(ever(_sync.sidebarRefresh, (_) {
      _browser.refresh();
      _browser.reloadMountConfig();
    }));

    // 页面进入时加载（对齐 xe-cloud-app-x：Future.microtask 加载）
    Future.microtask(() async {
      await _browser.reloadMountConfig();
      if (mounted) setState(() => _configLoaded = true);
      await _browser.loadFiles();
      await _browser.loadQuota();
      // 晚启动补偿：经控制器拉取一次当前快照
      await _sync.refreshStatus();
    });
  }

  @override
  void dispose() {
    for (final w in _workers) {
      w.dispose();
    }
    _searchController.dispose();
    Get.delete<FileBrowserController>();
    super.dispose();
  }

  /// 派生同步目录配置阶段（对标 CMP deriveSetupPhase）
  SetupPhase _deriveSetupPhase() {
    if (!_configLoaded) return SetupPhase.loading;
    if (!_browser.mountConfigured.value) return SetupPhase.needsSetup;
    final snap = _snapshot.value;
    if (snap.total > 0 || snap.lastSyncTime != null) return SetupPhase.active;
    return SetupPhase.needsFirstSync;
  }

  /// 手动刷新：触发同步引擎手动刷新 + 重载当前目录
  /// （对齐 CMP refresh：file list reload + syncManualRefresh）
  void _manualRefresh() {
    _sync.startSync();
    _browser.refresh();
  }

  /// 清空搜索态并恢复当前目录
  void _clearSearch() {
    _searchController.clear();
    setState(() {
      _searchKeyword = '';
      _submittedSearch = '';
    });
    _browser.clearSearch();
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);

    return Scaffold(
      body: Row(
        children: [
          // ---- 左栏 Sidebar（248px） ----
          Obx(() {
            final browser = _browser.state.value;
            final auth = _auth.state.value;
            final update = _update.state.value;
            return FilesSidebar(
              rootChildren:
                  browser.directoryChildren[FileBrowserController.rootKey] ??
                      const [],
              directoryChildren: browser.directoryChildren,
              selectedFolderId: browser.folderId ?? '',
              pathFolderIds: browser.breadcrumbs
                  .map((b) => b.id ?? '')
                  .toSet(),
              userName: auth.accountName,
              quotaText: _browser.quotaText.value,
              updateDownloading: update.phase == UpdatePhase.downloading,
              updateDownloadProgress: update.downloadProgress,
              updateAvailableVersion: update.phase == UpdatePhase.available
                  ? update.manifest?.version
                  : null,
              treeLoadingIds: browser.treeLoadingIds,
              onDismissUpdate: _update.dismiss,
              onInstallUpdate: _update.downloadAndInstall,
              onShowUpdate: () => Get.toNamed('/update'),
              onNavigate: _browser.enterFolderFromTree,
              onExpandNode: _browser.loadTreeChildren,
            );
          }),

          // ---- 右侧主区 ----
          Expanded(
            child: Container(
              color: colors.bgContainer,
              child: Column(
                children: [
                  _buildAppBar(),
                  const MateHDivider(),
                  _buildInfoArea(),
                  Obx(() => FilesBreadcrumb(
                        crumbs: _browser.state.value.breadcrumbs,
                        onNavigate: _browser.navigateToBreadcrumb,
                      )),
                  Expanded(child: _buildFileArea()),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  /// AppBar（64px；搜索 + 同步索引 + 传输队列 + Finder + 设置）
  Widget _buildAppBar() {
    return Obx(() {
      final mountConfigured = _browser.mountConfigured.value;
      final isIndexing = _snapshot.value.isIndexing;
      return FilesAppBar(
        searchController: _searchController,
        searchKeyword: _searchKeyword,
        mountConfigured: mountConfigured,
        isIndexing: isIndexing,
        onSearchChanged: (v) {
          setState(() => _searchKeyword = v);
          // 搜索态下清空关键词时退出搜索，恢复展示全部目录
          if (v.trim().isEmpty && _submittedSearch.isNotEmpty) {
            _clearSearch();
          }
        },
        onSearchSubmit: (query) {
          // 仅回车提交才触发搜索（对标原 Vue @submit）
          final keyword = query.trim();
          if (keyword.isNotEmpty) {
            setState(() {
              _searchKeyword = keyword;
              _submittedSearch = keyword;
            });
            _browser.searchFiles(keyword);
          }
        },
        onSearchClear: _clearSearch,
        onRefresh: _manualRefresh,
        onToggleTransfer: () =>
            setState(() => _showTransferPopover = !_showTransferPopover),
        onOpenFinder: _browser.openInFinder,
        onOpenSettings: () => Get.toNamed('/settings'),
      );
    });
  }

  /// 信息区（引导条三态 / 状态条 + ACTIVE 态常驻错误横幅）
  Widget _buildInfoArea() {
    return Obx(() {
      final phase = _deriveSetupPhase();
      final mountConfigured = _browser.mountConfigured.value;
      final errorMessage = _browser.errorMessage.value;

      if (!mountConfigured || phase == SetupPhase.needsFirstSync) {
        return FilesSyncSetupBanner(
          setupPhase: phase,
          mountDir: _browser.mountDir.value,
          errorMessage: errorMessage.isEmpty ? null : errorMessage,
          onSelectDir: () => Get.toNamed('/settings'),
          onFirstSync: _manualRefresh,
          onRetry: _manualRefresh,
        );
      }
      if (mountConfigured) {
        return Column(
          children: [
            FilesSyncStatusBar(
              sync: _snapshot.value,
              transfers: _transfer.state.value.tasks,
            ),
            // ACTIVE 态错误横幅（对标原 Vue MainPage.vue info-area 常驻错误展示）
            if (errorMessage.isNotEmpty) ...[
              Container(
                width: double.infinity,
                color: MateTheme.colorsOf(context).bgContainer,
                padding: EdgeInsets.symmetric(
                  horizontal:
                      MateTheme.metricsOf(context).syncSetup.horizontalPadding,
                  vertical:
                      MateTheme.metricsOf(context).syncSetup.verticalPadding,
                ),
                child: MateInfoBanner(
                  message: errorMessage,
                  variant: MateBannerVariant.error,
                ),
              ),
              const MateHDivider(),
            ],
          ],
        );
      }
      return const SizedBox.shrink();
    });
  }

  /// 文件区（搜索结果 / 文件列表 + 加载遮罩 + TransferPopover 浮层）
  Widget _buildFileArea() {
    return Obx(() {
      final browser = _browser.state.value;
      final downloadText = _browser.downloadProgressText.value;
      final colors = MateTheme.colorsOf(context);
      final metrics = MateTheme.metricsOf(context).mainPage;

      return Stack(
        children: [
          // 搜索结果区 / 文件列表
          Positioned.fill(
            child: _submittedSearch.isNotEmpty
                ? FilesSearchResults(
                    keyword: _submittedSearch,
                    results: browser.visibleFiles,
                    searching: browser.loading,
                    onEnterFolder: (file) {
                      _browser.enterFolder(file);
                      _clearSearch();
                    },
                  )
                : FileListView(
                    browser: browser,
                    fileStatuses: _browser.fileStatuses,
                    thumbnails: _browser.thumbnails,
                    mountConfigured: _browser.mountConfigured.value,
                    isIndexing: _snapshot.value.isIndexing,
                    onSort: _browser.sort,
                    onEnterFolder: _browser.enterFolder,
                    onOpenItem: _browser.openItem,
                    onThumbnailNeeded: _browser.loadThumbnail,
                    onDelete: _browser.deleteItems,
                    onPreviewFreeUp: _browser.previewFreeUpItems,
                    onFreeUp: _browser.freeUpItems,
                    onDownload: _browser.downloadItems,
                    onSyncFolder: _browser.syncFolder,
                    onRename: _browser.renameItem,
                    onMove: _browser.moveItem,
                    onCanFreeUp: _browser.canFreeUp,
                  ),
          ),

          // 加载遮罩（browser.loading；下载中显示进度文案）
          if (browser.loading || downloadText.isNotEmpty)
            Positioned.fill(
              child: Container(
                color: colors.mainLoadingScrim,
                alignment: Alignment.center,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    MateCircularProgress(size: metrics.loadingSize),
                    if (downloadText.isNotEmpty) ...[
                      const SizedBox(height: 12),
                      Text(
                        downloadText,
                        style: MateTheme.typographyOf(context)
                            .statusBar
                            .currentStatus
                            .copyWith(color: colors.textPrimary),
                      ),
                    ],
                  ],
                ),
              ),
            ),

          // TransferPopover 浮层（贴 AppBar 下右侧，点击外部关闭）
          if (_showTransferPopover) ...[
            Positioned.fill(
              child: GestureDetector(
                onTap: () => setState(() => _showTransferPopover = false),
                behavior: HitTestBehavior.opaque,
              ),
            ),
            Positioned.fill(
              child: TransferPopover(
                tasks: _transfer.state.value.tasks,
                onDismiss: () => setState(() => _showTransferPopover = false),
                onRetry: (taskId, onResult) async {
                  final result = await _transfer.retry(taskId);
                  onResult(result.isOk);
                },
                onClearCompleted: _transfer.clearCompleted,
                onClearFailed: _transfer.clearFailed,
                onClearFinished: _transfer.clearFinished,
              ),
            ),
          ],
        ],
      );
    });
  }
}
