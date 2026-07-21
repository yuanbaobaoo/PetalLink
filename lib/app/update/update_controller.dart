import 'dart:async';

import 'package:get/get.dart';

import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/update/update_service.dart';

/// 更新阶段状态机
///
/// 对齐 Rust 版前端 updater store（app/stores/updater.ts）：
///   idle → checking → available | upToDate | error(→failed)
///   available → downloading → downloaded → waitingTransfers → ready
///   waitingTransfers 超时 → downloaded（可重试等待）
///   任意阶段 → failed → idle（用户 dismiss）
enum UpdatePhase {
  /// 空闲
  idle,

  /// 检查更新中
  checking,

  /// 有新版本可用
  available,

  /// 已是最新（手动检查结果展示）
  upToDate,

  /// 下载更新包中（带进度）
  downloading,

  /// 下载与校验完成（等待开始安装 / 等待传输重试点）
  downloaded,

  /// 等待活跃传输完成
  waitingTransfers,

  /// 准备就绪（可重启安装）
  ready,

  /// 失败
  failed,
}

/// 更新 UI 状态
class UpdateUIState {
  /// 当前更新阶段
  final UpdatePhase phase;

  /// 更新清单（有新版本时非空）
  final UpdateManifest? manifest;

  /// 下载进度（0.0 ~ 1.0）
  final double downloadProgress;

  /// 错误消息
  final String? errorMessage;

  /// 是否有活跃传输（waitingTransfers 阶段展示用）
  final bool hasActiveTransfers;

  /// 更新对话框是否可见（静默检查发现更新不弹窗，对齐 Vue dialogOpen）
  final bool dialogVisible;

  const UpdateUIState({
    this.phase = UpdatePhase.idle,
    this.manifest,
    this.downloadProgress = 0.0,
    this.errorMessage,
    this.hasActiveTransfers = false,
    this.dialogVisible = false,
  });

  /// 初始状态
  factory UpdateUIState.initial() => const UpdateUIState();

  /// 深拷贝并替换指定字段
  UpdateUIState copyWith({
    UpdatePhase? phase,
    UpdateManifest? manifest,
    double? downloadProgress,
    String? errorMessage,
    bool? hasActiveTransfers,
    bool? dialogVisible,
    bool clearError = false,
    bool clearManifest = false,
  }) {
    return UpdateUIState(
      phase: phase ?? this.phase,
      manifest: clearManifest ? null : (manifest ?? this.manifest),
      downloadProgress: downloadProgress ?? this.downloadProgress,
      errorMessage: clearError ? null : (errorMessage ?? this.errorMessage),
      hasActiveTransfers: hasActiveTransfers ?? this.hasActiveTransfers,
      dialogVisible: dialogVisible ?? this.dialogVisible,
    );
  }
}

/// 更新控制器 — 全局应用更新检查与管理
///
/// 对齐 Rust 版前端 updater store 与 CMP UpdaterViewModel：
/// - 检查时机：启动 3s 后、每小时、窗口聚焦（10 分钟节流）
/// - 下载进度封顶 99%，完成置 100%（对齐 Vue 进度语义）
/// - waitingTransfers：每 2s 轮询 transfer_has_active，最多等 5 分钟；
///   无活跃传输 → ready；超时 → downloaded（可重试）
/// - relaunch：安装已校验 DMG 并退出（后台脚本完成替换）
class UpdateController extends GetxController {
  /// 启动后首次静默检查延迟（对齐 Vue App.vue 3s）
  static const Duration startupCheckDelay = Duration(seconds: 3);

  /// 定期检查间隔（对齐 Vue CHECK_INTERVAL_MS）
  static const Duration periodicCheckInterval = Duration(hours: 1);

  /// 聚焦检查节流（对齐 Vue FOCUS_THROTTLE_MS）
  static const Duration focusThrottle = Duration(minutes: 10);

  /// 等待传输轮询间隔（对齐 Vue 2s）
  static const Duration defaultWaitPollInterval = Duration(seconds: 2);

  /// 等待传输超时（对齐 Vue 5 分钟）
  static const Duration defaultWaitTimeout = Duration(minutes: 5);

  final UpdateService _updateService;
  final Future<bool> Function() _hasActiveTransfers;
  final Duration _waitPollInterval;
  final Duration _waitTimeout;
  final Duration _focusThrottle;

  /// 更新 UI 状态（响应式）
  final Rx<UpdateUIState> state = UpdateUIState.initial().obs;

  /// 已下载并校验的更新包路径（downloaded/ready 后非空）
  String? _stagedDmgPath;

  /// waitForTransfers 轮询定时器
  Timer? _waitTimer;

  /// 启动首次检查与定期检查定时器
  Timer? _startupTimer;
  Timer? _periodicTimer;

  /// 上次检查时间（聚焦节流用；每次检查都更新，对齐 Vue lastCheckTime）
  DateTime? _lastCheckTime;

  UpdateController({
    UpdateService? updateService,
    Future<bool> Function()? hasActiveTransfersProvider,
    Duration? waitPollInterval,
    Duration? waitTimeout,
    Duration focusThrottleDuration = focusThrottle,
  })  : _updateService = updateService ?? Get.find<UpdateService>(),
        _hasActiveTransfers =
            hasActiveTransfersProvider ?? _defaultHasActiveTransfers,
        _waitPollInterval = waitPollInterval ?? defaultWaitPollInterval,
        _waitTimeout = waitTimeout ?? defaultWaitTimeout,
        _focusThrottle = focusThrottleDuration;

  /// 默认活跃传输查询（委托 TaskRunner，对齐 transfer_has_active）
  static Future<bool> _defaultHasActiveTransfers() async {
    try {
      final result = await Get.find<TaskRunner>().hasActive();
      return result is Ok<bool> && result.value;
    } catch (_) {
      return false;
    }
  }

  // 【2026-07-21 临时停用】自动检查更新：原 onReady 中的
  // 启动 3s 静默检查 + 每小时定期检查（对齐 Vue App.vue）已停用。
  // 更新策略待重新调整后恢复，恢复时还原以下 onReady 实现：
  //
  // @override
  // void onReady() {
  //   super.onReady();
  //   _startupTimer = Timer(startupCheckDelay, () => unawaited(silentCheck()));
  //   _periodicTimer =
  //       Timer.periodic(periodicCheckInterval, (_) => unawaited(periodicCheck()));
  // }

  @override
  void onClose() {
    _waitTimer?.cancel();
    _startupTimer?.cancel();
    _periodicTimer?.cancel();
    super.onClose();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 检查更新
  // ═══════════════════════════════════════════════════════════════════

  /// 静默检查更新（不弹窗，失败不提示；对齐 Vue silentCheck）
  Future<void> silentCheck() async {
    AppLogger.d('silentCheck: 开始静默检查');
    await _doCheck(showDialog: false);
  }

  /// 手动检查更新（有更新自动弹窗；对齐 Vue manualCheck）
  ///
  /// @return 是否有新版本
  Future<bool> manualCheck() async {
    AppLogger.i('manualCheck: 用户主动检查更新');
    return _doCheck(showDialog: true);
  }

  /// 定期检查（每小时一次；对齐 Vue periodicCheck）
  Future<void> periodicCheck() async {
    AppLogger.d('periodicCheck');
    await silentCheck();
  }

  /// 聚焦时检查（10 分钟节流；对齐 Vue checkOnFocus）
  Future<void> checkOnFocus() async {
    if (_lastCheckTime != null) {
      final elapsed = DateTime.now().difference(_lastCheckTime!);
      if (elapsed < _focusThrottle) {
        AppLogger.d('checkOnFocus 节流：距上次检查 ${elapsed.inMinutes} 分钟');
        return;
      }
    }
    await silentCheck();
  }

  /// 打开更新对话框（对齐 CMP showUpdateDialog）。
  ///
  /// 有可展示内容（已拿到 manifest 且处于可展示阶段）时直接重开弹窗；
  /// 否则触发一次手动检查，有更新会自动弹窗。
  Future<void> showUpdate() async {
    const visiblePhases = {
      UpdatePhase.available,
      UpdatePhase.downloading,
      UpdatePhase.waitingTransfers,
      UpdatePhase.ready,
      UpdatePhase.failed,
    };
    if (state.value.manifest != null &&
        visiblePhases.contains(state.value.phase)) {
      state.value = state.value.copyWith(dialogVisible: true);
      return;
    }
    await manualCheck();
  }

  /// 内部检查逻辑
  Future<bool> _doCheck({required bool showDialog}) async {
    // 防止重复检查
    if (state.value.phase == UpdatePhase.checking) {
      AppLogger.d('_doCheck 防重复：已在检查中');
      return false;
    }

    state.value = state.value.copyWith(
      phase: UpdatePhase.checking,
      clearError: true,
    );

    final result = await _updateService.check();
    _lastCheckTime = DateTime.now();

    if (result.isErr) {
      final err = (result as Err).error;
      // 静默检查失败不显示错误（对齐 Vue silentCheck 静默失败）
      if (showDialog) {
        state.value = state.value.copyWith(
          phase: UpdatePhase.failed,
          errorMessage: '检查更新失败: ${err.message}',
          dialogVisible: true,
        );
      } else {
        state.value = state.value.copyWith(phase: UpdatePhase.idle);
      }
      return false;
    }

    final manifest = (result as Ok<UpdateManifest?>).value;
    if (manifest == null) {
      // 已是最新版本
      state.value = state.value.copyWith(
        phase: showDialog ? UpdatePhase.upToDate : UpdatePhase.idle,
        dialogVisible: showDialog,
      );
      AppLogger.d('已是最新版本');
      return false;
    }

    // 发现新版本（静默检查不弹窗，对齐 Vue）
    _stagedDmgPath = null;
    state.value = state.value.copyWith(
      phase: UpdatePhase.available,
      manifest: manifest,
      dialogVisible: showDialog,
    );
    AppLogger.i('发现新版本: ${manifest.version}');
    return true;
  }

  // ═══════════════════════════════════════════════════════════════════
  // 下载与安装
  // ═══════════════════════════════════════════════════════════════════

  /// 下载更新包（含 SHA-256 校验），完成后自动等待传输并进入就绪。
  ///
  /// 流程：available → downloading → downloaded → waitingTransfers → ready
  Future<void> downloadAndInstall() async {
    final manifest = state.value.manifest;
    if (manifest == null) {
      AppLogger.e('downloadAndInstall: 无更新清单');
      return;
    }
    if (manifest.url.isEmpty) {
      state.value = state.value.copyWith(
        phase: UpdatePhase.failed,
        errorMessage: '下载地址为空',
        dialogVisible: true,
      );
      return;
    }

    // 下载（进度封顶 99%，完成置 100%；对齐 Vue 进度语义）
    state.value = state.value.copyWith(
      phase: UpdatePhase.downloading,
      downloadProgress: 0.0,
      clearError: true,
      dialogVisible: true,
    );

    final result = await _updateService.downloadAndStage(
      manifest,
      onProgress: (received, total) {
        if (total != null && total > 0) {
          onDownloadProgress((received / total).clamp(0.0, 0.99));
        }
      },
    );

    if (result.isErr) {
      final err = (result as Err).error;
      state.value = state.value.copyWith(
        phase: UpdatePhase.failed,
        errorMessage: err.message,
      );
      return;
    }

    _stagedDmgPath = (result as Ok<String>).value;
    state.value = state.value.copyWith(
      phase: UpdatePhase.downloaded,
      downloadProgress: 1.0,
    );
    AppLogger.i('更新包下载校验完成');

    // 下载完成自动等待传输（对齐 Vue handleStartUpdate）
    await waitForTransfers();
  }

  /// 下载进度回调（仅 downloading 阶段生效）
  void onDownloadProgress(double progress) {
    if (state.value.phase == UpdatePhase.downloading) {
      state.value = state.value.copyWith(
        downloadProgress: progress.clamp(0.0, 1.0),
      );
    }
  }

  /// 安装并重启（ready/downloaded 阶段用户点击「立即重启」）
  ///
  /// 成功后进程退出，由后台脚本完成 .app 替换并重新打开。
  Future<void> relaunch() async {
    final dmg = _stagedDmgPath;
    if (dmg == null) {
      state.value = state.value.copyWith(
        phase: UpdatePhase.failed,
        errorMessage: '更新包不存在，请重新下载',
        dialogVisible: true,
      );
      return;
    }
    AppLogger.i('开始安装更新: $dmg');
    final result = await _updateService.installAndRelaunch(dmg);
    if (result.isErr) {
      final err = (result as Err).error;
      state.value = state.value.copyWith(
        phase: UpdatePhase.failed,
        errorMessage: err.message,
      );
    }
    // 成功路径：进程已退出，不会返回
  }

  // ═══════════════════════════════════════════════════════════════════
  // 等待传输完成
  // ═══════════════════════════════════════════════════════════════════

  /// 等待活跃传输完成（每 2s 轮询，最多 5 分钟；对齐 Vue waitForTransfers）
  ///
  /// - 无活跃传输 → ready + 弹窗，返回 true
  /// - 超时 → 回退 downloaded + 弹窗（可重试），返回 false
  Future<bool> waitForTransfers() async {
    _waitTimer?.cancel();
    state.value = state.value.copyWith(phase: UpdatePhase.waitingTransfers);

    final startedAt = DateTime.now();
    final completer = Completer<bool>();

    Future<void> poll() async {
      final hasActive = await _queryHasActiveTransfers();
      state.value = state.value.copyWith(hasActiveTransfers: hasActive);
      if (!hasActive) {
        _waitTimer?.cancel();
        _waitTimer = null;
        state.value = state.value.copyWith(
          phase: UpdatePhase.ready,
          dialogVisible: true,
        );
        AppLogger.i('waitForTransfers: 无活跃传输，进入就绪');
        if (!completer.isCompleted) completer.complete(true);
        return;
      }
      if (DateTime.now().difference(startedAt) >= _waitTimeout) {
        _waitTimer?.cancel();
        _waitTimer = null;
        state.value = state.value.copyWith(
          phase: UpdatePhase.downloaded,
          dialogVisible: true,
        );
        AppLogger.i('waitForTransfers: 等待超时（${_waitTimeout.inMinutes} 分钟）');
        if (!completer.isCompleted) completer.complete(false);
      }
    }

    await poll();
    if (!completer.isCompleted) {
      _waitTimer = Timer.periodic(_waitPollInterval, (_) => unawaited(poll()));
    }
    return completer.future;
  }

  /// 查询活跃传输；失败保守视为仍有活跃（对齐 CMP 保守阻塞语义）
  Future<bool> _queryHasActiveTransfers() async {
    try {
      return await _hasActiveTransfers();
    } catch (e) {
      AppLogger.w('查询活跃传输失败，保守等待: $e');
      return true;
    }
  }

  /// 更新 hasActiveTransfers 状态（外部传输快照同步入口）
  void updateActiveTransfers(bool hasActive) {
    state.value = state.value.copyWith(hasActiveTransfers: hasActive);
  }

  // ═══════════════════════════════════════════════════════════════════
  // UI 操作
  // ═══════════════════════════════════════════════════════════════════

  /// 关闭更新提示（回到 idle；对齐 Vue dismiss）
  void dismiss() {
    _waitTimer?.cancel();
    _waitTimer = null;
    state.value = state.value.copyWith(
      phase: UpdatePhase.idle,
      clearManifest: true,
      clearError: true,
      dialogVisible: false,
    );
  }
}
