/// 同步引擎 —— 周期编排基座与组合类。
///
/// 严格对齐 Rust 原版 `src/sync/engine.rs`：
/// - 字段/依赖清单与生命周期标志（started/shutdown/running/folder_syncing）
/// - 触发源 → CycleRequest 位映射与反向优先级推导
/// - 连续 300 次增量强制全量 BFS；可恢复周期错误按指数退避（上限 32s）
///
/// 实现按 Rust `engine/` 子模块切片为 mixin：
/// `cache`（云树缓存）/ `cycle`（周期编排）/ `lifecycle`（启停）/
/// `publication`（状态发布）/ `reconciliation`（对账）/ `results`（结算）/
/// `retry`（重试）；协调原语在 `coordination.dart`，执行器在 `executor.dart`。
library;

import 'dart:async';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/mount/local_watcher.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/sync/baseline_store.dart';
import 'package:petal_link/service/sync/identity/detect_moves.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/service/sync/engine/cache.dart';
import 'package:petal_link/service/sync/engine/coordination.dart';
import 'package:petal_link/service/sync/engine/cycle.dart';
import 'package:petal_link/service/sync/engine/lifecycle.dart';
import 'package:petal_link/service/sync/engine/publication.dart';
import 'package:petal_link/service/sync/engine/reconciliation.dart';
import 'package:petal_link/service/sync/engine/results.dart';
import 'package:petal_link/service/sync/engine/retry.dart';
import 'package:petal_link/service/sync/engine/executor.dart';
import 'package:petal_link/service/sync/path_recovery.dart';
import 'package:petal_link/service/sync/planner.dart';
import 'package:petal_link/service/sync/status_aggregator.dart';
import 'package:petal_link/service/sync/sync_actions.dart';
import 'package:petal_link/service/transfer/task_runner.dart';

/// 同步引擎基座：全部字段与共享辅助（对齐 Rust `SyncEngine` 字段清单）。
///
/// 各编排能力由 mixin 提供；外部只应使用组合类 [SyncEngine]。
abstract class SyncEngineBase {
  /// 文件 API
  final FilesService filesApi;

  /// 变更 API（含 getStartCursor）
  final ChangesService changesApi;

  /// 数据库
  final DatabaseService db;

  /// 状态聚合器（跨引擎共享的发布屏障 + revision 分配器）
  final StatusAggregator statusAggregator;

  /// 跳过模式（用户配置 glob）
  final List<String> skipPatterns;

  /// watcher 防抖窗口
  final Duration debounce;

  /// 云端定时刷新间隔（Duration.zero = 关闭）
  final Duration pollInterval;

  /// 在线判定（生产接 NetGuard.isOnline）
  final bool Function() onlineCheck;

  /// 稳定网络转换流（生产接 NetGuard.transitions，可空）
  final Stream<NetworkTransition>? netTransitions;

  /// 请求级网络失败上报（生产接 NetGuard.reportRequestNetworkFailure）
  final void Function()? requestNetworkFailureReporter;

  /// 当前毫秒时钟（测试注入）
  final int Function() nowMs;

  /// 周期观测点（测试注入；默认空）
  void Function(String point) cycleObserver;

  SyncEngineBase({
    required this.filesApi,
    required this.changesApi,
    required this.db,
    required this.statusAggregator,
    required this.baselineStore,
    this.skipPatterns = const [],
    this.debounce = const Duration(seconds: 3),
    this.pollInterval = const Duration(seconds: 60),
    bool Function()? onlineCheck,
    this.netTransitions,
    this.requestNetworkFailureReporter,
    int Function()? nowMs,
    void Function(String point)? cycleObserver,
    InodeIdentityStore? identity,
  })  : onlineCheck = onlineCheck ?? (() => true),
        nowMs = nowMs ?? (() => DateTime.now().millisecondsSinceEpoch),
        cycleObserver = cycleObserver ?? ((_) {}) {
    this.identity = identity ?? SqfliteInodeIdentityStore(db);
    // 基线结算存储共享同一身份映射实例（测试注入 Memory 时保持一致）
    baselineStore.identity = this.identity;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 后期绑定（对齐 Rust set_mount / set_executor）
  // ═══════════════════════════════════════════════════════════════════

  /// 挂载管理器（setMount 注入）
  MountManager? mount;

  /// 挂载根目录（绝对路径）
  String mountDir = '';

  /// 动作执行器（setExecutor 注入）
  SyncExecutor? executor;

  /// 持久化传输执行器（setExecutor 时从 executor 取）
  TaskRunner? taskRunner;

  /// sync_items 基线结算存储（TaskRunner 钩子实现）
  final SyncBaselineStore baselineStore;

  /// 注入挂载管理器（同时记录挂载根）。
  void setMount(MountManager m) {
    mount = m;
    mountDir = m.mountDir;
  }

  /// 注入执行器（共享 TaskRunner 与活动门 + 身份映射实例）。
  void setExecutor(SyncExecutor exec, TaskRunner runner) {
    exec.identity = identity;
    executor = exec;
    taskRunner = runner;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 云树 live 状态（engine/cache.dart 管理）
  // ═══════════════════════════════════════════════════════════════════

  /// 云树 live 状态索引（engine/cache.dart 实现）
  late final CloudTreeIndex cloudIndex = CloudTreeIndex();

  /// inode 身份映射（docs/design/10；阶段 1 仅写不读，
  /// 阶段 2 起用于移动检测）。测试可注入 [MemoryInodeIdentityStore]。
  late InodeIdentityStore identity;

  /// 最近一轮扫描检测到的本地移动（inode 配对；cycle 在 planner 后消费）
  List<DetectedMove> lastScanMoves = [];

  // ═══════════════════════════════════════════════════════════════════
  // 运行状态与发布
  // ═══════════════════════════════════════════════════════════════════

  /// 当前权威快照
  SyncGlobalState state = const SyncGlobalState();

  /// 当前运行时字段（无 DB 来源的部分）
  final RuntimeStatus runtime = RuntimeStatus();

  /// 快照广播（对齐 Rust state_tx；Dart 广播流无容量上限，
  /// 慢消费者由 revision 防乱序兜底，语义等价）
  final StreamController<SyncGlobalState> stateCtrl =
      StreamController<SyncGlobalState>.broadcast();

  /// 反振荡：路径 → 删除时刻 ms（TTL 5 分钟）
  final Map<String, int> recentlyDeletedPaths = {};

  /// recently_deleted 保留时长（对齐 Rust 300_000ms）
  static const int recentlyDeletedTtlMs = 300000;

  // ═══════════════════════════════════════════════════════════════════
  // 生命周期标志与协调原语
  // ═══════════════════════════════════════════════════════════════════

  /// 周期协调器（sticky 位合并 + 唯一 owner drain）
  final CycleCoordinator cycle = CycleCoordinator();

  /// 活动门（共享给 TaskRunner / free-up / folder sync）
  final ActivityTracker activity = ActivityTracker();

  /// 周期进行中
  bool syncing = false;

  /// 目录递归同步进行中（独立于 syncing；周期 drain 会检查它）
  bool folderSyncing = false;

  /// 引擎运行中（已发布 is_running）
  bool running = false;

  /// 启动完成（此后 runSyncCycle 才接受非 startup 触发）
  bool started = false;

  /// 停止标志（发布后拒绝一切新周期/发布）
  bool shutdownFlag = false;

  /// 关闭广播（watch 语义：监听者收到 true 后退出）
  final StreamController<bool> shutdownCtrl =
      StreamController<bool>.broadcast();

  /// 本地监视器（保活 FSEvents）
  LocalWatcher? watcher;

  /// 云端定时刷新定时器
  Timer? cloudRefreshTimer;

  /// backoff 调度定时器
  Timer? backoffTimer;

  /// 网络转换订阅
  StreamSubscription<NetworkTransition>? netSub;

  /// watcher 事件订阅
  StreamSubscription<List<String>>? watcherSub;

  /// backoff 调度变更通知（Completer 一次性信号）
  Completer<void> backoffChanged = Completer<void>()..complete();

  /// backoff 调度版本（变化时唤醒调度器）
  int scheduleRevision = 0;

  /// 后台 drain 任务唯一性标志
  bool backgroundScheduled = false;

  // ═══════════════════════════════════════════════════════════════════
  // 常量（对齐 Rust engine.rs）
  // ═══════════════════════════════════════════════════════════════════

  /// 连续增量次数阈值：达到后强制全量 BFS（≈ 5 小时一次纠偏）
  static const int incrementalForcedFullThreshold = 300;

  /// 可恢复周期错误的最大退避秒数
  static const int recoverableCycleRetryMaxSecs = 32;

  /// backoff 调度兜底复查间隔（对齐 Rust BLOCKED_DEADLINE_RECHECK_INTERVAL）
  static const Duration blockedDeadlineRecheckInterval = Duration(seconds: 1);

  /// 有上限指数退避（对齐 Rust `recoverable_cycle_retry_delay`）：
  /// exponent = min(failures-1, 5)，delay = min(2^exponent, 32) 秒。
  static Duration recoverableCycleRetryDelay(int consecutiveFailures) {
    final exponent = (consecutiveFailures - 1).clamp(0, 5);
    final secs = (1 << exponent).clamp(1, recoverableCycleRetryMaxSecs);
    return Duration(seconds: secs);
  }

  /// 可恢复周期错误判定（对齐 Rust `is_recoverable_cycle_error`）：
  /// 仅 DriveApi 且（transport 错误或 429 或 5xx）。
  static bool isRecoverableCycleError(Object error) {
    if (error is! DriveApiError) return false;
    final status = error.statusCode;
    return error.transportKind != null ||
        status == 429 ||
        (status != null && status >= 500);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 共享辅助
  // ═══════════════════════════════════════════════════════════════════

  /// 是否在线
  bool isOnline() => onlineCheck();

  /// shutdown 即抛错（副作用前必查，对齐 Rust `ensure_cycle_active`）。
  void ensureCycleActive() {
    if (shutdownFlag) {
      throw AppError.generic('同步引擎已停止');
    }
  }

  /// 登记外部活动（命令面持有，阻止 shutdown 竞态）。
  ActivityGuard beginExternalActivity() => activity.begin();

  /// 登记排他路径活动（先校验相对路径合法性）。
  ActivityGuard beginExclusivePathActivity(String relativePath) {
    return activity.beginExclusive(relativePath);
  }

  /// 取 TaskRunner（未初始化报错）。
  TaskRunner requireTaskRunner() {
    final runner = taskRunner;
    if (runner == null) {
      throw AppError.generic('TaskRunner 未初始化');
    }
    return runner;
  }

  /// 引擎是否正在运行
  bool get isRunning => running;

  /// 尝试开始目录递归同步（folderSyncing || syncing || shutdown 时拒绝）。
  bool tryBeginFolderSync() {
    if (folderSyncing || syncing || shutdownFlag) return false;
    folderSyncing = true;
    return true;
  }

  /// 结束目录递归同步；释放后若有 pending 周期请求则补跑一轮。
  void endFolderSync() {
    folderSyncing = false;
    if (!cycle.isIdle()) {
      requestCycleBackground('local-watcher');
    }
  }

  /// backoff 调度已变化（唤醒调度器重新计算到期时间）。
  void notifyBackoffScheduleChanged() {
    scheduleRevision++;
    if (!backoffChanged.isCompleted) backoffChanged.complete();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 由各 mixin 实现的编排方法（在此声明以支持 mixin 间交叉调用）
  // ═══════════════════════════════════════════════════════════════════

  // ---- EnginePublication（engine/publication.dart）----

  /// 唯一发布通道
  Future<SyncGlobalState> updateRuntimeAndBroadcast(
    void Function(RuntimeStatus runtime) update,
  );

  /// 重算并广播完整快照
  Future<SyncGlobalState> recomputeAndBroadcastState();

  /// 出错后恢复空闲运行时
  Future<void> restoreIdleRuntimeAfterError();

  /// 周期收尾发布
  Future<void> updateAndPushState({required bool contentChanged});

  /// 尽力重算并广播当前状态
  Future<void> pushLiveTransferState();

  /// 清除传输历史并广播
  Future<SyncGlobalState> clearTransferHistoryAndBroadcast({
    required bool includeCompleted,
    required bool includeFailed,
  });

  /// 当前快照（engine/publication.dart）。
  SyncGlobalState currentState();

  /// 快照广播流（engine/publication.dart）。
  Stream<SyncGlobalState> stateReceiver();

  // ---- EngineCache（engine/cache.dart）----

  /// 云树是否可信
  bool cloudTreeIsTrusted();

  /// 启动期加载或全量重建云树
  Future<bool> loadOrRefreshCloudTree();

  /// 全量构建并提交可信 checkpoint
  Future<void> buildAndCommitFullCheckpoint();

  /// 周期内云端全量刷新
  Future<void> refreshCloudFullForCycle();

  /// 周期内云端增量刷新
  Future<void> refreshCloudIncrementalForCycle();

  /// 增量优先、连续 300 次强制全量
  Future<void> tryIncrementalOrFullRefresh();

  /// 提交任务恢复确认的远端文件
  Future<void> commitRecoveredCloudFiles(List<RecoveredCloudFile> recovered);

  /// 提交恢复结果（失败时请求全量重建）
  Future<void> commitRecoveryCheckpoint(List<RecoveredCloudFile> recovered);

  /// 清理 DELETED 墓碑（仅可信）
  Future<void> purgeDeletedTombstonesIfTrusted(
    List<BlockedPathChange> blocked,
  );

  // ---- EngineCycle（engine/cycle.dart）----

  /// 合并请求并安排后台 drain（engine/cycle.dart）。
  void requestCycleBackground(String triggeredBy);

  /// 触发源 → 请求位映射
  CycleRequest cycleRequestForTrigger(String triggeredBy);

  /// 提交请求并等待本序列结算
  Future<void> runSyncCycle(String triggeredBy);

  /// 手动全量刷新入口
  Future<void> triggerManualSync();

  /// 后台 drain 调度
  void scheduleBackgroundDrain();

  /// 唯一 owner 合并排空
  Future<void> drainCycleRequestsFor({int? awaited});

  /// 单周期阶段序列
  Future<void> runCoordinatedCycle(CycleRequest request);

  /// 扫描 → 对账 → 规划 → 过滤 → 执行 → 结算
  Future<void> runSyncCycleInner(
    String triggeredBy,
    List<BlockedPathChange> blockedPathChanges,
  );

  // ---- EngineLifecycle（engine/lifecycle.dart）----

  /// 启动引擎
  Future<void> start();

  /// 停止引擎
  Future<void> shutdown();

  /// 同步停止语义
  Future<void> shutdownSync();

  // ---- EngineRetry（engine/retry.dart）----

  /// 全局重试失败任务
  Future<void> retryFailed();

  /// 单任务重试
  Future<void> retryTransfer(int taskId);

  // ---- EngineReconciliation（engine/reconciliation.dart）----

  /// 扫描本地挂载目录
  Future<Map<String, LocalFileEntry>> scanLocal();

  /// 加载 DB 基线快照
  Future<Map<String, DbSnapshotEntry>> loadDbSnapshot();

  /// 用可信云树 + 本地身份补 DB 基线
  Future<void> reconcileDbRecords(
    Map<String, LocalFileEntry> local,
    Map<String, DbSnapshotEntry> dbSnapshot,
    List<BlockedPathChange> blocked,
  );

  /// FAILED 记录复核与残余清理
  Future<FailedRecordReconciliation> reconcileFailedAndPurgeStaleRecords(
    Map<String, LocalFileEntry> local,
    Map<String, DriveFile> cloud,
    List<BlockedPathChange> blocked,
  );

  /// xattr fileId 识别改名
  /// free-up 安全判定
  Future<FreeUpCheckResult> canSafelyFreeUp(String relPath, String fileId);

  // ---- EngineResults（engine/results.dart）----

  /// 按顺序执行动作
  Future<List<ActionResult>> executeActionsOrdered(
    SyncExecutor exec,
    List<SyncAction> actions,
  );

  /// 应用执行结果
  Future<void> applyResults(
    List<SyncAction> actions,
    List<ActionResult> results,
  );
}

/// 同步引擎组合类（对齐 Rust `SyncEngine` 的完整 impl）。
class SyncEngine extends SyncEngineBase
    with
        EngineCache,
        EngineCycle,
        EngineLifecycle,
        EnginePublication,
        EngineRetry,
        EngineReconciliation,
        EngineResults {
  /// 创建同步引擎（生产由 SyncService 装配）。
  SyncEngine({
    required super.filesApi,
    required super.changesApi,
    required super.db,
    required super.statusAggregator,
    required super.baselineStore,
    super.skipPatterns,
    super.debounce,
    super.pollInterval,
    super.onlineCheck,
    super.netTransitions,
    super.requestNetworkFailureReporter,
    super.nowMs,
    super.cycleObserver,
  });
}
