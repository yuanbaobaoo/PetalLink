// ignore_for_file: prefer_initializing_formals — 公开命名参数映射私有字段

/// 同步服务门面 —— 引擎生命周期与命令面。
///
/// 严格对齐 Rust 原版 `src/commands.rs`（ensure_engine_started）+
/// `src/commands/sync_control.rs` / `sync_status.rs` / `folder_sync.rs` /
/// `free_up.rs`：
/// - mount_configured（mount_path 非空）且已登录 → MountManager +
///   recoverInterruptedFreeUp + TaskRunner 钩子接线与启动 + NetGuard 探测 +
///   引擎启动；未登录但已配置挂载 → 清库清缓存（cleanup_orphan_state）
/// - 命令面：manualRefresh / retryFailed / getState / itemsByFolder /
///   checkFileLocalStatus / batchFileStatus / checkSafeFreeUp / freeUpSpace /
///   listFreeableInFolder / freeUpBatch / downloadOnDemand / folderRecursive
/// - logout/换挂载目录 → 停止引擎 + 清缓存
library;

import 'dart:async';
import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/free_up.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/mount/xattr_service.dart';
import 'package:petal_link/service/sync/baseline_store.dart';
import 'package:petal_link/service/sync/cloud_tree.dart';
import 'package:petal_link/service/sync/engine.dart';
import 'package:petal_link/service/sync/engine/executor.dart';
import 'package:petal_link/service/sync/folder_sync.dart';
import 'package:petal_link/service/sync/status_aggregator.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// 同步服务（引擎装配 + 命令面门面）。
class SyncService {
  /// 数据库
  final DatabaseService _db;

  /// 配置
  final ConfigService _config;

  /// 文件 API
  final FilesService _filesApi;

  /// 变更 API
  final ChangesService _changesApi;

  /// 上传执行器
  final UploadService _uploadApi;

  /// 下载执行器
  final DownloadService _downloadApi;

  /// 网络守卫
  final NetGuard _netGuard;

  /// 持久化传输执行器
  final TaskRunner _taskRunner;

  /// 登录态判定（生产接 AuthService.isLoggedIn）
  final Future<bool> Function()? _isLoggedIn;

  /// 挂载管理器注册回调（生产接 Get.put）
  final void Function(MountManager mount)? _onMountRegistered;

  /// 挂载管理器注销回调（生产接 Get.delete）
  final void Function()? _onMountUnregistered;

  /// xattr 实现（测试注入；默认原生通道）
  final XattrService? _xattr;

  /// 状态聚合器（进程级）
  final StatusAggregator _aggregator;

  /// 当前引擎
  SyncEngine? _engine;

  /// 当前挂载管理器
  MountManager? _mountManager;

  /// 引擎快照订阅
  StreamSubscription<SyncGlobalState>? _engineStateSub;

  /// 引擎启动串行化
  Future<void>? _starting;

  /// 对外快照流（引擎切换透明）
  final StreamController<SyncGlobalState> _stateCtrl =
      StreamController<SyncGlobalState>.broadcast();

  /// 目录递归同步进度广播（对齐 Rust `folder_sync_progress` 事件）
  final StreamController<FolderSyncProgress> _folderProgressCtrl =
      StreamController<FolderSyncProgress>.broadcast();

  SyncService({
    required DatabaseService db,
    required ConfigService config,
    required FilesService filesApi,
    required ChangesService changesApi,
    required UploadService uploadApi,
    required DownloadService downloadApi,
    required NetGuard netGuard,
    required TaskRunner taskRunner,
    Future<bool> Function()? isLoggedIn,
    void Function(MountManager mount)? onMountRegistered,
    void Function()? onMountUnregistered,
    XattrService? xattr,
    StatusAggregator? aggregator,
  })  : _db = db,
        _config = config,
        _filesApi = filesApi,
        _changesApi = changesApi,
        _uploadApi = uploadApi,
        _downloadApi = downloadApi,
        _netGuard = netGuard,
        _taskRunner = taskRunner,
        _isLoggedIn = isLoggedIn,
        _onMountRegistered = onMountRegistered,
        _onMountUnregistered = onMountUnregistered,
        _xattr = xattr,
        _aggregator = aggregator ?? StatusAggregator.process;

  // ═══════════════════════════════════════════════════════════════════
  // 对外流
  // ═══════════════════════════════════════════════════════════════════

  /// 同步全局状态快照流（revision 单调；对齐 Rust `sync_state` 事件）。
  Stream<SyncGlobalState> get stateStream => _stateCtrl.stream;

  /// 目录内容变更通知（对齐 Rust `folder_content_changed` 事件）。
  Stream<void> get folderContentChanged =>
      _stateCtrl.stream.where((s) => s.contentChanged).map((_) {});

  /// 上传失败通知流（对齐 Rust `upload_failed` 事件）。
  Stream<UploadFailureNotice> get uploadFailures =>
      _taskRunner.uploadFailures;

  /// 目录递归同步进度流（对齐 Rust `folder_sync_progress` 事件）。
  Stream<FolderSyncProgress> get folderSyncProgress =>
      _folderProgressCtrl.stream;

  /// 引擎是否已启动
  bool get isEngineStarted => _engine != null;

  // ═══════════════════════════════════════════════════════════════════
  // 生命周期（对齐 Rust ensure_engine_started / cleanup_orphan_state）
  // ═══════════════════════════════════════════════════════════════════

  /// 确保引擎已启动（幂等；对齐 Rust `ensure_engine_started`）。
  ///
  /// 条件：mount_path 已配置且已登录；未登录但已配置挂载 →
  /// 清库清缓存并重置挂载配置（cleanup_orphan_state）。
  Future<void> ensureEngineStarted() {
    return _starting ??= _ensureEngineStarted().whenComplete(() {
      _starting = null;
    });
  }

  Future<void> _ensureEngineStarted() async {
    if (_engine != null) return;
    // 装配配置（挂载目录 ~ 展开为绝对路径，对齐 Rust expanded_mount_dir）
    final cfg = await _config.configLoad();
    if (!cfg.mountConfigured) return;
    final mountDir = cfg.expandedMountDir;

    final loggedIn = _isLoggedIn == null ? true : await _isLoggedIn.call();
    if (!loggedIn) {
      AppLogger.w('未登录但挂载目录已配置，清理孤儿同步状态');
      await _cleanupOrphanState();
      return;
    }

    final mount = MountManager(mountDir, xattr: _xattr, db: _db);
    await mount.ensureMountDir();
    _mountManager = mount;
    _onMountRegistered?.call(mount);

    // 启动前收敛上次中断的释放空间操作
    try {
      final recovered =
          await mount.recoverInterruptedFreeUp(await _db.database);
      if (recovered > 0) {
        AppLogger.w('已收敛 $recovered 个中断的释放空间操作');
      }
    } catch (e) {
      AppLogger.w('恢复中断的释放空间操作失败: $e');
    }

    // 基线结算钩子接线
    final baselineStore = SyncBaselineStore(
      db: _db,
      mountProvider: () => _mountManager,
    );
    _taskRunner.setSyncHooks(baselineStore);

    // 引擎配置（对齐 Rust AppConfig：debounce/poll 单位均为秒）
    final engine = SyncEngine(
      filesApi: _filesApi,
      changesApi: _changesApi,
      db: _db,
      statusAggregator: _aggregator,
      baselineStore: baselineStore,
      skipPatterns: cfg.skipPatterns,
      debounce: Duration(seconds: cfg.debounceSec),
      pollInterval: Duration(seconds: cfg.pollIntervalSec),
      onlineCheck: () => _netGuard.isOnline,
      netTransitions: _netGuard.transitions,
      requestNetworkFailureReporter: _netGuard.reportRequestNetworkFailure,
    );
    engine.setMount(mount);
    final executor = SyncExecutor(
      filesApi: _filesApi,
      uploadApi: _uploadApi,
      downloadApi: _downloadApi,
      db: _db,
      mount: mount,
      taskRunner: _taskRunner,
      concurrencyProvider: () async => cfg.concurrency,
      beginActivity: engine.activity.begin,
    );
    engine.setExecutor(executor, _taskRunner);
    _engine = engine;
    _engineStateSub = engine.stateReceiver().listen((snapshot) {
      if (!_stateCtrl.isClosed) _stateCtrl.add(snapshot);
    });

    // NetGuard 探测 + TaskRunner + 引擎启动
    _netGuard.start();
    await _taskRunner.start();
    unawaited(engine.start().catchError((Object e, StackTrace st) {
      AppLogger.e('同步引擎启动失败', e, st);
    }));
    AppLogger.i('同步引擎装配完成：$mountDir');
  }

  /// 停止引擎并清理挂载注册（保留 DB 基线与缓存；换挂载目录用）。
  Future<void> stopEngine() async {
    final engine = _engine;
    _engine = null;
    await _engineStateSub?.cancel();
    _engineStateSub = null;
    _taskRunner.setSyncHooks(null);
    if (engine != null) {
      await engine.shutdown();
    }
    await _taskRunner.stop();
    _onMountUnregistered?.call();
    _mountManager = null;
  }

  /// 登出清理（对齐 Rust logout + cleanup_orphan_state）：
  /// 停止引擎 → 清 sync_items / transfer_queue → 清全部缓存文件 →
  /// 重置挂载配置。
  Future<void> onLogout() async {
    await stopEngine();
    _netGuard.shutdown();
    await _cleanupOrphanState();
    AppLogger.i('登出：同步状态与缓存已清理');
  }

  /// 清理孤儿同步状态（清表 + 清缓存 + 重置挂载配置）。
  Future<void> _cleanupOrphanState() async {
    final db = await _db.database;
    await db.delete('sync_items');
    await db.delete('transfer_queue');
    await CachePaths.clearAll();
    await _config.remove('mount_path');
  }

  /// 挂载配置变更（settings 保存后调用）：
  /// 目录变化 → 停止旧引擎（DB 基线保留）并按新配置重启。
  Future<void> onMountConfigChanged() async {
    final cfg = await _config.configLoad();
    final mountDir = cfg.mountConfigured ? cfg.expandedMountDir : '';
    final current = _mountManager?.mountDir ?? '';
    if (_engine == null) {
      await ensureEngineStarted();
      return;
    }
    if (mountDir == current) return;
    await stopEngine();
    await ensureEngineStarted();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 控制命令（对齐 sync_control.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 手动刷新（云端全量 + 一个同步周期）。
  Future<void> manualRefresh() => _requireEngine().triggerManualSync();

  /// 全局重试失败任务。
  Future<void> retryFailed() => _requireEngine().retryFailed();

  /// 单任务重试。
  Future<void> retryTransfer(int taskId) =>
      _requireEngine().retryTransfer(taskId);

  /// 当前同步全局状态（引擎未启动时返回 DB 兜底盘，不广播）。
  Future<SyncGlobalState> getState() async {
    final engine = _engine;
    if (engine != null) {
      return engine.recomputeAndBroadcastState();
    }
    return _aggregator.lockPublication(() async {
      return _aggregator.snapshot(await _db.database, RuntimeStatus());
    });
  }

  /// 当前缓存的最近一次快照（引擎未启动时为 null）。
  SyncGlobalState? get currentState => _engine?.currentState();

  /// 清除传输历史并广播。
  Future<SyncGlobalState> clearTransferHistory({
    required bool includeCompleted,
    required bool includeFailed,
  }) {
    return _requireEngine().clearTransferHistoryAndBroadcast(
      includeCompleted: includeCompleted,
      includeFailed: includeFailed,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 状态查询命令（对齐 sync_status.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 按目录查询同步项（LIKE 前缀无边界，对齐 Rust `sync_items_by_folder`）。
  Future<List<SyncItem>> itemsByFolder(String folderLocalPath) async {
    final db = await _db.database;
    final rows = await db.query('sync_items',
        where: 'local_path LIKE ?', whereArgs: ['$folderLocalPath%']);
    return rows.map(SyncItem.fromRow).toList();
  }

  /// 查询文件本地同步状态（folder/synced/placeholder/not_synced）。
  Future<String> checkFileLocalStatus(String fileId) async {
    final mount = _mountManager;
    if (mount == null) return 'not_synced';
    return mount.checkFileLocalStatus(fileId);
  }

  /// 批量查询文件同步状态（挂载不可用时回退 DB 判定）。
  Future<Map<String, String>> batchFileStatus(List<String> fileIds) async {
    final mount = _mountManager;
    if (mount != null) {
      return mount.batchFileLocalStatus(fileIds);
    }
    final db = await _db.database;
    final out = <String, String>{};
    for (final fileId in fileIds) {
      final record = await MountManager.findByFileId(db, fileId);
      if (record == null) {
        out[fileId] = 'not_synced';
      } else if (record.isFolder) {
        out[fileId] = 'folder';
      } else {
        out[fileId] =
            record.status == SyncItemStatus.Synced ? 'synced' : 'not_synced';
      }
    }
    return out;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 释放空间命令（对齐 free_up.rs；FreeUpRemoteGate 引擎接线）
  // ═══════════════════════════════════════════════════════════════════

  FreeUpService _freeUpService() {
    final mount = _mountManager;
    if (mount == null) {
      throw AppError.config('同步引擎未启动（尚未配置同步目录）');
    }
    return FreeUpService(
      mount: mount,
      db: _db,
      gate: _engine != null ? _EngineFreeUpGate(_engine!, _filesApi) : null,
    );
  }

  /// 检查文件是否可安全释放本地空间（safe/not_in_cloud/not_synced）。
  Future<String> checkSafeFreeUp(String relPath, String fileId) async {
    final engine = _engine;
    if (engine == null) return 'not_synced';
    final result = await engine.canSafelyFreeUp(relPath, fileId);
    return switch (result) {
      FreeUpCheckResult.safe => 'safe',
      FreeUpCheckResult.notInCloud => 'not_in_cloud',
      FreeUpCheckResult.notSynced => 'not_synced',
    };
  }

  /// 释放单个文件本地空间（替换为占位符），返回释放的字节数。
  Future<int> freeUpSpace({
    required String fileId,
    required String relPath,
    required int size,
    String? localPath,
  }) {
    return _freeUpService().freeUpSpace(
      fileId: fileId,
      relPath: relPath,
      size: size,
      localPath: localPath,
    );
  }

  /// 枚举目录下可释放空间的候选项。
  Future<List<FreeableItem>> listFreeableInFolder(String folderRelPath) {
    final mount = _mountManager;
    if (mount == null) {
      throw AppError.config('同步引擎未启动（尚未配置同步目录）');
    }
    return FreeUpService(mount: mount, db: _db)
        .listFreeableInFolder(folderRelPath);
  }

  /// 批量释放空间（单项失败不中断）。
  Future<FreeUpBatchResult> freeUpBatch(List<FreeableItem> items) {
    return _freeUpService().freeUpBatch(items);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 按需下载（对齐 sync_download_on_demand）
  // ═══════════════════════════════════════════════════════════════════

  /// 按需下载文件到指定路径（占位符/缺失文件的显式下载）。
  Future<bool> downloadOnDemand({
    required String fileId,
    required String destPath,
  }) async {
    final engine = _requireEngine();
    final guard = engine.beginExternalActivity();
    try {
      if (engine.currentState().isIndexing) {
        throw AppError.generic('正在读取云端索引，请稍后再试');
      }
      final mount = _requireMount();
      final frontendRel =
          MountPath.relativePathFromMount(mount.mountDir, destPath);
      final record =
          await MountManager.findByFileId(await _db.database, fileId);
      final String destRel;
      if (record != null) {
        MountPath.validateRelativePath(record.localPath);
        if (record.localPath != frontendRel) {
          throw AppError.config(
              '下载路径不一致：rel_path=${record.localPath}, '
              'dest_path=$frontendRel');
        }
        destRel = record.localPath;
      } else {
        destRel = frontendRel;
      }
      final dest = MountPath.safeJoinUnder(mount.mountDir, destRel);

      // 云端元数据（必须有可信 editedTime 与 size）
      final cachedTrusted =
          record != null && record.size >= 0 && record.cloudEditedTime != null;
      DriveFile? remote;
      final getResult = await _filesApi.get(fileId);
      if (getResult.isOk) {
        remote = (getResult as Ok<DriveFile>).value;
      } else if (!cachedTrusted) {
        throw (getResult as Err).error;
      }
      final editedMs = remote?.editedTime?.millisecondsSinceEpoch ??
          record?.cloudEditedTime;
      if (editedMs == null) {
        throw AppError.generic('按需下载缺少可信云端 editedTime，拒绝创建任务');
      }
      final size = remote != null ? remote.size : (record?.size ?? -1);
      if (size < 0) {
        throw AppError.generic('按需下载缺少可信云端文件大小，拒绝创建任务');
      }

      // 目标快照
      LocalDestinationSnapshot? snapshot;
      final type = await FileSystemEntity.type(dest, followLinks: false);
      if (type == FileSystemEntityType.link ||
          (type != FileSystemEntityType.notFound &&
              type != FileSystemEntityType.file)) {
        throw AppError.generic('按需下载目标不是安全的普通文件');
      }
      if (type == FileSystemEntityType.file) {
        final stat = await FileStat.stat(dest);
        final placeholder = await mount.isPlaceholderFile(dest);
        if (!(stat.size == 0 && placeholder)) {
          snapshot = LocalDestinationSnapshot(
            mtimeMs: stat.modified.millisecondsSinceEpoch,
            size: stat.size,
          );
        }
      }
      final isUpdate = snapshot != null;
      final task = TransferTask(
        direction: isUpdate
            ? TransferDirection.DownloadUpdate
            : TransferDirection.Download,
        fileId: fileId,
        localPath: dest,
        name: p.basename(dest),
        totalSize: size,
        createdAt: DateTime.now().millisecondsSinceEpoch,
        relativePath: destRel,
        parentFileId: remote?.parentId ?? record?.parentFolderId,
        operation: isUpdate
            ? TransferOperation.DownloadUpdate
            : TransferOperation.Download,
        sourceMtime: snapshot?.mtimeMs,
        sourceSize: snapshot?.size,
        expectedCloudEditedTime: editedMs,
      );
      final result = await _taskRunner.enqueueAndRun(task);
      final outcome = result.unwrap().outcome;
      if (outcome.disposition == TaskDisposition.completed) return true;
      throw AppError.generic('下载已进入恢复队列：${outcome.disposition.name}');
    } finally {
      guard.close();
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 目录递归同步（对齐 folder_sync.rs：后台 BFS 子树双端对齐）
  // ═══════════════════════════════════════════════════════════════════

  /// 目录递归同步：立即返回，后台 BFS 云端子树与本地对齐
  /// （云端有本地无 → 下载；本地有云端无 → 上传；共有文件不做内容比较）。
  Future<int> folderRecursive({
    required String folderId,
    required String relPath,
  }) async {
    final engine = _requireEngine();
    final guard = engine.beginExternalActivity();
    if (engine.currentState().isIndexing) {
      guard.close();
      throw AppError.generic('正在读取云端索引，请稍后再试');
    }
    if (!engine.tryBeginFolderSync()) {
      guard.close();
      throw AppError.generic('已有同步周期或目录同步正在运行，本次请求未开始');
    }
    final runner = FolderSyncRunner(
      filesApi: _filesApi,
      taskRunner: _taskRunner,
      mountProvider: _requireMount,
      emitProgress: (progress) {
        if (!_folderProgressCtrl.isClosed) {
          _folderProgressCtrl.add(progress);
        }
      },
    );
    unawaited(() async {
      try {
        await runner.run(engine, folderId: folderId, relPath: relPath);
      } catch (e, st) {
        AppLogger.e('目录递归同步失败: $relPath', e, st);
      } finally {
        engine.endFolderSync();
        guard.close();
        try {
          await engine.updateRuntimeAndBroadcast((r) {
            r.contentChanged = true;
            r.lastSyncTime = engine.nowMs();
          });
        } catch (_) {
          // 尽力发布
        }
      }
    }());
    return 0;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 辅助
  // ═══════════════════════════════════════════════════════════════════

  SyncEngine _requireEngine() {
    final engine = _engine;
    if (engine == null) {
      throw AppError.generic('同步引擎未启动（尚未配置同步目录）');
    }
    return engine;
  }

  MountManager _requireMount() {
    final mount = _mountManager;
    if (mount == null) {
      throw AppError.config('同步引擎未启动（尚未配置同步目录）');
    }
    return mount;
  }

  /// 释放资源（应用退出时调用）。
  Future<void> dispose() async {
    await stopEngine();
    await _stateCtrl.close();
    await _folderProgressCtrl.close();
  }
}

/// FreeUpRemoteGate 的引擎接线实现（对齐 Rust free_up.rs 的引擎/API 依赖）。
class _EngineFreeUpGate implements FreeUpRemoteGate {
  final SyncEngine _engine;
  final FilesService _filesApi;

  _EngineFreeUpGate(this._engine, this._filesApi);

  @override
  bool get cloudTreeTrusted => _engine.cloudTreeIsTrusted();

  @override
  String? cloudFileIdAt(String relPath) =>
      _engine.cloudIndex.tree[relPath]?.id;

  @override
  Future<FreeUpRemoteSnapshot> fetchRemote(String fileId) async {
    final result = await _filesApi.get(fileId);
    final file = result.unwrap();
    return FreeUpRemoteSnapshot(
      id: file.id,
      size: file.size,
      editedTimeMs: file.editedTime?.millisecondsSinceEpoch,
    );
  }

  @override
  Future<bool> verifyDeleted(String fileId) async {
    final result = await _filesApi.verifyDeleted(fileId);
    return result.unwrap();
  }

  @override
  FreeUpPathLease beginExclusivePathActivity(String relPath) {
    return _ActivityLease(_engine.beginExclusivePathActivity(relPath));
  }
}

/// 活动守卫 → 释放租约适配。
class _ActivityLease implements FreeUpPathLease {
  final dynamic _guard;

  _ActivityLease(this._guard);

  @override
  void close() {
    _guard.close();
  }
}
