import 'dart:async';

import 'package:get/get.dart';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/service/transfer/task_runner_contracts.dart';
import 'package:petal_link/types/enums.dart';

/// 同步 UI 状态
///
/// 对标 CMP SyncSnapshotUi（ViewModels.kt）与原 Vue sync store（docs/08 §2.2）。
/// 字段对齐 SyncGlobalStatus + RuntimeStatus，涵盖同步全生命周期。
class SyncUIState {
  /// 同步全局状态
  final SyncStatus status;

  /// 总操作数
  final int totalActions;

  /// 已完成操作数
  final int completedActions;

  /// 当前正在执行的操作描述（如 "上传 file.txt"）
  final String? currentAction;

  /// 上次同步完成时间
  final DateTime? lastSyncAt;

  /// 待处理变更数
  final int pendingChanges;

  /// 错误消息
  final String? errorMessage;

  /// 单调版本号（对标 CMP revision，乱序保护）
  ///
  /// 每次 applySnapshot 时检查：revision <= lastRevision → 拒绝。
  final int revision;

  /// 是否正在索引（BFS 树刷新阶段）
  final bool isIndexing;

  /// 索引已扫描文件夹数
  final int indexingScannedFolders;

  /// 索引已发现条目数
  final int indexingDiscoveredItems;

  /// 同步阶段（字符串标识，如 "bfs" / "changes" / "plan" / "execute"）
  final String? syncPhase;

  /// 内容是否已变更（驱动目录树刷新）
  final bool contentChanged;

  const SyncUIState({
    this.status = SyncStatus.idle,
    this.totalActions = 0,
    this.completedActions = 0,
    this.currentAction,
    this.lastSyncAt,
    this.pendingChanges = 0,
    this.errorMessage,
    this.revision = 0,
    this.isIndexing = false,
    this.indexingScannedFolders = 0,
    this.indexingDiscoveredItems = 0,
    this.syncPhase,
    this.contentChanged = false,
  });

  /// 初始状态
  factory SyncUIState.initial() => const SyncUIState();

  /// 从引擎权威快照映射（SyncGlobalState → UI 状态）。
  factory SyncUIState.fromSnapshot(SyncGlobalState s) {
    final SyncStatus status;
    if (s.isIndexing) {
      status = SyncStatus.scanning;
    } else if (s.isRunning || s.uploading + s.downloading > 0) {
      status = SyncStatus.syncing;
    } else {
      status = SyncStatus.idle;
    }
    return SyncUIState(
      status: status,
      totalActions: s.total,
      completedActions: s.completed,
      lastSyncAt: s.lastSyncTime != null
          ? DateTime.fromMillisecondsSinceEpoch(s.lastSyncTime!)
          : null,
      pendingChanges: s.uploading + s.downloading + s.waitingNetwork,
      errorMessage:
          s.failedItems.isNotEmpty ? s.failedItems.first.errorMessage : null,
      revision: s.revision,
      isIndexing: s.isIndexing,
      indexingScannedFolders: s.indexingScannedFolders,
      indexingDiscoveredItems: s.indexingDiscoveredItems,
      syncPhase: s.syncPhase?.wireName,
      contentChanged: s.contentChanged,
    );
  }

  /// 进度 0.0 ~ 1.0（totalActions=0 时返回 0.0）
  double get progress {
    if (totalActions <= 0) return 0.0;
    return (completedActions / totalActions).clamp(0.0, 1.0);
  }

  /// 是否有活跃传输
  bool get isActive => status == SyncStatus.syncing || status == SyncStatus.scanning;

  /// 深拷贝并替换指定字段
  SyncUIState copyWith({
    SyncStatus? status,
    int? totalActions,
    int? completedActions,
    String? currentAction,
    DateTime? lastSyncAt,
    int? pendingChanges,
    String? errorMessage,
    int? revision,
    bool? isIndexing,
    int? indexingScannedFolders,
    int? indexingDiscoveredItems,
    String? syncPhase,
    bool? contentChanged,
    bool clearError = false,
    bool clearCurrentAction = false,
  }) {
    return SyncUIState(
      status: status ?? this.status,
      totalActions: totalActions ?? this.totalActions,
      completedActions: completedActions ?? this.completedActions,
      currentAction:
          clearCurrentAction ? null : (currentAction ?? this.currentAction),
      lastSyncAt: lastSyncAt ?? this.lastSyncAt,
      pendingChanges: pendingChanges ?? this.pendingChanges,
      errorMessage: clearError ? null : (errorMessage ?? this.errorMessage),
      revision: revision ?? this.revision,
      isIndexing: isIndexing ?? this.isIndexing,
      indexingScannedFolders:
          indexingScannedFolders ?? this.indexingScannedFolders,
      indexingDiscoveredItems:
          indexingDiscoveredItems ?? this.indexingDiscoveredItems,
      syncPhase: syncPhase ?? this.syncPhase,
      contentChanged: contentChanged ?? this.contentChanged,
    );
  }
}

/// 同步控制器 — 全局同步状态管理
///
/// 对标 CMP SyncViewModel（ViewModels.kt）与 Vue sync store（docs/08 §2.2）。
///
/// 核心机制：
/// - **revision 单调递增**：订阅 [SyncService.stateStream] 引擎权威快照，
///   若 incoming.revision <= lastAppliedRevision → 拒绝（乱序保护）。
/// - **sidebarRefresh**：contentChanged + isNewRevision 时递增，
///   供目录树订阅刷新。
/// - **upload_failed 透传**：[lastUploadFailure] 携带最近一次上传失败通知。
class SyncController extends GetxController {
  /// 同步 UI 状态（响应式）
  final Rx<SyncUIState> state = SyncUIState.initial().obs;

  /// 最后应用的 revision（乱序保护）
  int _lastAppliedRevision = -1;

  /// 目录树刷新计数器（contentChanged 且新 revision 时 +1）
  final RxInt sidebarRefresh = 0.obs;

  /// 最近一次上传失败通知（对齐 Rust `upload_failed` 事件透传）
  final Rx<UploadFailureNotice?> lastUploadFailure =
      Rx<UploadFailureNotice?>(null);

  /// 引擎权威快照（原始 SyncGlobalState，含 uploading/failed 分桶计数；
  /// 页面观察此字段而非重复订阅 SyncService.stateStream）
  final Rx<SyncGlobalState> rawSnapshot = const SyncGlobalState().obs;

  /// 引擎快照订阅
  StreamSubscription<SyncGlobalState>? _snapshotSub;

  /// 上传失败订阅
  StreamSubscription<UploadFailureNotice>? _uploadFailureSub;

  @override
  void onInit() {
    super.onInit();

    // 订阅引擎权威快照流（revision 防乱序由 applySnapshot 兜底）
    final syncService = Get.find<SyncService>();
    _snapshotSub = syncService.stateStream.listen((snapshot) {
      rawSnapshot.value = snapshot;
      applySnapshot(SyncUIState.fromSnapshot(snapshot));
    });
    _uploadFailureSub = syncService.uploadFailures.listen((notice) {
      lastUploadFailure.value = notice;
      AppLogger.w('上传失败通知: ${notice.name}: ${notice.error}');
    });

    // 监听状态变化，用于日志与副作用
    ever<SyncUIState>(state, (s) {
      AppLogger.d(
        '同步状态变更: status=${s.status.name} phase=${s.syncPhase ?? "-"} '
        'progress=${s.progress.toStringAsFixed(2)} rev=${s.revision}',
      );
    });

    // 主动拉取一次当前状态（晚启动补偿）
    Future.microtask(refreshStatus);
  }

  @override
  void onClose() {
    _snapshotSub?.cancel();
    _uploadFailureSub?.cancel();
    super.onClose();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 状态应用（revision 乱序保护）
  // ═══════════════════════════════════════════════════════════════════

  /// 应用完整同步快照（乱序保护 + sidebarRefresh 触发）
  ///
  /// 对标 CMP SyncViewModel.applySnapshot()：
  /// - revision <= lastAppliedRevision → 拒绝
  /// - contentChanged + isNewRevision → sidebarRefresh++
  ///
  /// @return 是否被接受（false 表示过期快照被拒绝）
  bool applySnapshot(SyncUIState snapshot) {
    if (snapshot.revision <= _lastAppliedRevision) {
      AppLogger.d(
        'applySnapshot 拒绝：revision ${snapshot.revision} <= $_lastAppliedRevision',
      );
      return false;
    }

    final isNewRevision = snapshot.revision > _lastAppliedRevision;
    _lastAppliedRevision = snapshot.revision;

    // 更新状态
    state.value = snapshot;

    // contentChanged + 新 revision → 目录树刷新
    if (snapshot.contentChanged && isNewRevision) {
      sidebarRefresh.value++;
      AppLogger.d('sidebarRefresh → ${sidebarRefresh.value}');
    }

    return true;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 同步控制
  // ═══════════════════════════════════════════════════════════════════

  /// 手动刷新同步（对齐 Rust `sync_manual_refresh`）：
  /// 触发云端全量刷新 + 一个同步周期；进度由快照流推送。
  Future<void> startSync() async {
    try {
      await Get.find<SyncService>().manualRefresh();
      AppLogger.i('手动刷新已触发');
    } catch (e, st) {
      AppLogger.e('手动刷新失败', e, st);
      final rev = ++_lastAppliedRevision;
      state.value = state.value.copyWith(
        status: SyncStatus.error,
        errorMessage: '同步失败: $e',
        revision: rev,
        isIndexing: false,
      );
    }
  }


  /// 重试全部失败任务（对齐 Rust `sync_retry_failed`）。
  Future<void> retryFailed() async {
    try {
      await Get.find<SyncService>().retryFailed();
    } catch (e, st) {
      AppLogger.e('重试失败任务失败', e, st);
    }
  }

  /// 刷新同步状态（主动拉取当前状态，对齐 CMP sync.init() 的
  /// 主动 getSyncState()）。
  Future<void> refreshStatus() async {
    try {
      final snapshot = await Get.find<SyncService>().getState();
      rawSnapshot.value = snapshot;
      applySnapshot(SyncUIState.fromSnapshot(snapshot));
    } catch (e) {
      AppLogger.d('refreshStatus 失败（引擎可能未启动）: $e');
    }
  }
}
