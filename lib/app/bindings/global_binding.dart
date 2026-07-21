import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:get/get.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:window_manager/window_manager.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/sync/sync_controller.dart';
import 'package:petal_link/app/transfer/transfer_controller.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/http/mate_http_client.dart';
import 'package:petal_link/core/net/net_guard.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/about_service.dart';
import 'package:petal_link/service/drive/changes_service.dart';
import 'package:petal_link/service/drive/download_service.dart';
import 'package:petal_link/service/drive/files_service.dart';
import 'package:petal_link/service/drive/thumbnail_service.dart';
import 'package:petal_link/service/drive/upload_service.dart';
import 'package:petal_link/service/mount/file_hasher.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/service/mount/stability.dart';
import 'package:petal_link/service/platform/platform_service.dart';
import 'package:petal_link/service/platform/tray_service.dart';
import 'package:petal_link/service/sync/cloud_tree.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/service/transfer/drive_task_operations.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:petal_link/service/update/update_service.dart';
import 'package:petal_link/types/enums.dart';

/// 全局依赖绑定
class GlobalBinding {
  /// 初始化所有全局依赖（在 runApp 之前调用）
  Future<void> dependencies() async {
    // ============================================
    // 核心基础设施
    // ============================================

    /// 数据库服务（单例）
    Get.put(DatabaseService.instance, permanent: true);

    /// 认证服务（token.bin 加密存储 + OAuth 编排 + 401 刷新源）
    ///
    /// 必须先于 MateHttpClient（tokenProvider/refreshTokenProvider 依赖）
    /// 与 AuthController（onInit 恢复登录态依赖）注册。
    final authService = AuthService();
    Get.put(authService, permanent: true);

    /// HTTP 客户端（Bearer 注入 + 401 自动刷新）
    Get.put(
      MateHttpClient(
        baseUrl: '',
        tokenProvider: () async {
          try {
            // 临期自动刷新（对齐 Rust ensure_valid_access_token）
            return await authService.ensureValidAccessToken();
          } catch (_) {
            // 未登录 / 刷新失败 → 空 token（不注入 Bearer，等 401 流程）
            return '';
          }
        },
        refreshTokenProvider: () async {
          try {
            final token = await authService.refresher.refresh();
            return token.accessToken;
          } catch (_) {
            return null;
          }
        },
        onAuthExpired: () {
          Get.find<AuthController>().logout();
        },
      ),
      permanent: true,
    );

    // ============================================
    // 控制器 — 全局状态管理
    // ============================================

    /// 认证控制器
    Get.put(AuthController(), permanent: true);

    /// 同步控制器
    Get.put(SyncController(), permanent: true);

    /// 传输控制器
    Get.put(TransferController(), permanent: true);

    /// 更新控制器
    Get.put(UpdateController(), permanent: true);

    // ============================================
    // 服务层 — API 与业务逻辑
    // ============================================

    /// 安全存储（用于配置服务的敏感数据）
    final secureStorage = const FlutterSecureStorage();

    /// 配置服务（键值存储 + Rust config.rs 命令面；
    /// 挂载目录变更/引擎启动编排经回调延迟解析，避免注册顺序依赖）
    Get.put(
      ConfigService(
        DatabaseService.instance,
        secureStorage,
        onEngineConfigChanged: () =>
            Get.find<SyncService>().onMountConfigChanged(),
        onMountDirChanged: (oldAbs, newAbs) async {
          // 对齐 Rust config_save 分支1：停运行时 → 清两侧缓存 → 重启 app
          await Get.find<SyncService>().stopEngine();
          if (oldAbs != null) await CachePaths.clearForMount(oldAbs);
          if (newAbs != null) await CachePaths.clearForMount(newAbs);
          await Get.find<PlatformService>().relaunchApp();
        },
      ),
      permanent: true,
    );

    /// 文件服务
    Get.put(
      FilesService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 配额服务
    Get.put(
      AboutService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 缩略图服务
    Get.put(
      ThumbnailService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 变更服务
    Get.put(
      ChangesService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 上传服务
    Get.put(
      UploadService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 下载服务
    Get.put(
      DownloadService(Get.find<MateHttpClient>()),
      permanent: true,
    );

    /// 传输服务
    Get.put(
      TransferService(DatabaseService.instance),
      permanent: true,
    );

    /// 网络守卫（TCP 探测迟滞 + 稳定状态转换流；探测循环由同步引擎任务接线）
    Get.put(NetGuard.instance, permanent: true);

    /// 上传前稳定性检查（mtime 滞后 + 大小稳定 + lsof 三重）
    Get.put(StabilityChecker(), permanent: true);

    /// 本地文件 SHA-256（远端核验内容比对）
    Get.put(FileHasher(), permanent: true);

    /// TaskRunner 操作执行适配层（上传/下载/files API 分发 + 远端核验）
    Get.put(
      DriveTaskOperations(
        uploadService: Get.find<UploadService>(),
        downloadService: Get.find<DownloadService>(),
        filesService: Get.find<FilesService>(),
        stability: Get.find<StabilityChecker>(),
        fileHasher: Get.find<FileHasher>(),
        isOnline: () => Get.find<NetGuard>().isOnline,
        onUploadFailed: (notice) =>
            Get.find<TaskRunner>().publishUploadFailure(notice),
      ),
      permanent: true,
    );

    /// 持久化传输任务执行器（9 态机 + 并发调度 + 退避重试 + 崩溃恢复）
    ///
    /// 仅注册装配；start/stop 生命周期由后续同步引擎任务统一接线。
    Get.put(
      TaskRunner(
        transferService: Get.find<TransferService>(),
        operations: Get.find<DriveTaskOperations>(),
        isOnline: () => Get.find<NetGuard>().isOnline,
        netTransitions: Get.find<NetGuard>().transitions,
        onRequestNetworkFailure:
            Get.find<NetGuard>().reportRequestNetworkFailure,
        concurrencyProvider: () async {
          final cfg = await Get.find<ConfigService>().configLoad();
          return cfg.concurrency;
        },
        mountRootProvider: () => Get.isRegistered<MountManager>()
            ? Get.find<MountManager>().mountDir
            : null,
        isPlaceholder: (path) => Get.isRegistered<MountManager>()
            ? Get.find<MountManager>().isPlaceholderFile(path)
            : Future.value(false),
      ),
      permanent: true,
    );

    /// 平台服务（Finder/开机启动/statfs/日志命令面/清缓存重启；
    /// 全量清理编排经回调延迟解析，避免注册顺序依赖）
    Get.put(
      PlatformService(
        onTeardownRuntime: () => Get.find<SyncService>().stopEngine(),
        onClearAuth: () => Get.find<AuthService>().logout(),
        onClearDatabase: () async {
          final db = DatabaseService.instance;
          await db.deleteAllData();
          await db.deleteDatabaseFile();
        },
        onClearSecureConfig: () => secureStorage.deleteAll(),
      ),
      permanent: true,
    );

    /// 更新服务（GitHub Releases manifest + DMG 安装；
    /// 当前版本取 package_info）
    final packageInfo = await PackageInfo.fromPlatform();
    Get.put(
      UpdateService(currentVersion: packageInfo.version),
      permanent: true,
    );

    /// 同步服务（引擎装配 + 命令面门面）
    ///
    /// 引擎生命周期（ensureEngineStarted）由 AuthController 登录/恢复成功
    /// 与 SettingsController 保存挂载配置后触发；MountManager 随引擎启停
    /// 注册/注销（TaskRunner 的 mountRootProvider/isPlaceholder 依赖）。
    Get.put(
      SyncService(
        db: DatabaseService.instance,
        config: Get.find<ConfigService>(),
        filesApi: Get.find<FilesService>(),
        changesApi: Get.find<ChangesService>(),
        uploadApi: Get.find<UploadService>(),
        downloadApi: Get.find<DownloadService>(),
        netGuard: Get.find<NetGuard>(),
        taskRunner: Get.find<TaskRunner>(),
        isLoggedIn: () => authService.isLoggedIn(),
        onMountRegistered: (mount) {
          if (Get.isRegistered<MountManager>()) {
            Get.delete<MountManager>(force: true);
          }
          Get.put<MountManager>(mount, permanent: true);
        },
        onMountUnregistered: () {
          if (Get.isRegistered<MountManager>()) {
            Get.delete<MountManager>(force: true);
          }
        },
      ),
      permanent: true,
    );

    /// 托盘服务（菜单栏图标 + 活动传输菜单；对齐 Rust tray.rs）
    ///
    /// 菜单数据源：transfer_queue 中 Pending/Running 任务按创建时间升序
    /// （对齐 Rust build_menu 的 SQL 语义）；刷新触发源为 TaskRunner
    /// 快照流（transfer_update）与引擎状态流（sync_state）。
    final trayService = TrayService(
      activeTransfersProvider: () async {
        final result = await Get.find<TransferService>().getAllTasks();
        if (result.isErr) return const <TransferTask>[];
        final tasks = (result as Ok<List<TransferTask>>).value
            .where((t) =>
                t.state == TransferState.Pending ||
                t.state == TransferState.Running)
            .toList()
          ..sort((a, b) => a.createdAt.compareTo(b.createdAt));
        return tasks;
      },
      onShowWindow: () async {
        // 对齐 Rust：show + set_focus + macOS set_regular
        await windowManager.show();
        await windowManager.focus();
        await Get.find<PlatformService>().setRegularMode();
      },
      onQuit: () async {
        // 托盘退出 = 真正退出（对齐 Rust mark_real_quit + app.exit）
        try {
          await Get.find<SyncService>()
              .stopEngine()
              .timeout(const Duration(seconds: 2));
        } catch (_) {
          // 尽力而为，不阻塞退出
        }
        exit(0);
      },
    );
    trayService.bindRefreshTriggers(
      transferUpdates: Get.find<TaskRunner>().snapshots,
      syncStates: Get.find<SyncService>().stateStream,
    );
    Get.put(trayService, permanent: true);
  }
}
