import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:get/get.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/entity/auth.dart';
import 'package:petal_link/entity/config_entry.dart';
import 'package:petal_link/entity/drive_file.dart';
import 'package:petal_link/service/auth/auth_service.dart';
import 'package:petal_link/service/config/config_service.dart';
import 'package:petal_link/service/drive/about_service.dart';
import 'package:petal_link/service/platform/platform_service.dart';

/// 设置页 Tab
///
/// 对标 CMP SettingsScreen 的 SettingsTab：6 个 Tab，分「通用/其他」两组。
enum SettingsTab {
  /// 同步目录配置
  syncDir,

  /// 传输设置
  transfer,

  /// 高级设置
  advanced,

  /// 账号管理
  account,

  /// 日志查看
  logs,

  /// 关于
  about,
}

/// 设置页状态
///
/// 字段单位对齐 Rust `AppConfig`：轮询间隔与防抖均为**秒**
/// （页面输入框后缀同为「秒」）。
class SettingsState {
  /// 当前选中的 Tab
  final SettingsTab tab;

  /// 本地挂载/同步目录路径（空串表示未配置）
  final String mountDir;

  /// 并发传输数（1-20）
  final int concurrency;

  /// 云端自动刷新间隔（秒；0 = 关闭，开启时最小 60）
  final int pollInterval;

  /// 变更防抖时长（秒，≥1）
  final int debounce;

  /// OAuth 回调端口（1-65535）
  final int oauthPort;

  /// 跳过文件模式列表（如 .DS_Store、*.tmp）
  final List<String> skipPatterns;

  /// 是否开启开机启动
  final bool launchEnabled;

  /// 当前账号信息（未登录/未拉取到时为 null）
  final UserInfo? userInfo;

  /// 已用空间（字节；未获取到时为 null）
  final int? quotaUsed;

  /// 总容量（字节；未获取到时为 null）
  final int? quotaTotal;

  /// 应用版本号（package_info；获取失败为空串，UI 显示占位）
  final String appVersion;

  /// 校验错误列表（保存前校验不通过的字段错误）
  final List<String> errors;

  /// 是否已保存
  final bool saved;

  const SettingsState({
    this.tab = SettingsTab.syncDir,
    this.mountDir = '',
    this.concurrency = 6,
    this.pollInterval = 60,
    this.debounce = 3,
    this.oauthPort = AppConfig.defaultCallbackPort,
    this.skipPatterns = AppConfig.defaultSkipPatterns,
    this.launchEnabled = false,
    this.userInfo,
    this.quotaUsed,
    this.quotaTotal,
    this.appVersion = '',
    this.errors = const [],
    this.saved = false,
  });

  /// 初始状态
  factory SettingsState.initial() => const SettingsState();

  /// 同步目录是否已配置（对标 CMP mountConfigured）
  bool get mountConfigured => mountDir.trim().isNotEmpty;

  /// 深拷贝并替换指定字段
  SettingsState copyWith({
    SettingsTab? tab,
    String? mountDir,
    int? concurrency,
    int? pollInterval,
    int? debounce,
    int? oauthPort,
    List<String>? skipPatterns,
    bool? launchEnabled,
    UserInfo? userInfo,
    int? quotaUsed,
    int? quotaTotal,
    String? appVersion,
    List<String>? errors,
    bool? saved,
    bool clearErrors = false,
  }) {
    return SettingsState(
      tab: tab ?? this.tab,
      mountDir: mountDir ?? this.mountDir,
      concurrency: concurrency ?? this.concurrency,
      pollInterval: pollInterval ?? this.pollInterval,
      debounce: debounce ?? this.debounce,
      oauthPort: oauthPort ?? this.oauthPort,
      skipPatterns: skipPatterns ?? this.skipPatterns,
      launchEnabled: launchEnabled ?? this.launchEnabled,
      userInfo: userInfo ?? this.userInfo,
      quotaUsed: quotaUsed ?? this.quotaUsed,
      quotaTotal: quotaTotal ?? this.quotaTotal,
      appVersion: appVersion ?? this.appVersion,
      errors: clearErrors ? [] : (errors ?? this.errors),
      saved: saved ?? this.saved,
    );
  }
}

/// 设置页控制器 — 设置页 UI 状态与配置持久化
///
/// 职责：
/// - 管理设置页 UI 状态（[SettingsState]）
/// - 经 [ConfigService] 命令面加载/保存配置（对齐 Rust config_load/save）
/// - 挂载目录变更由 configSave 编排清运行时+缓存+重启
/// - 开机启动、清缓存、检查更新、退出登录委托各服务/控制器
/// - 配置导出/导入（对齐 Rust config_export_json / config_import_json
///   与 CMP ApplicationRoot.exportConfig/importConfig）
class SettingsController extends GetxController {
  final ConfigService _configService = Get.find<ConfigService>();

  /// 设置页状态（响应式）
  final Rx<SettingsState> state = SettingsState.initial().obs;

  /// 原始值（用于检测是否有修改 / 重置）
  SettingsState? _original;

  @override
  void onInit() {
    super.onInit();
    loadSettings();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 加载
  // ═══════════════════════════════════════════════════════════════════

  /// 加载配置 + 开机启动状态 + 账号信息 + 存储配额 + 应用版本
  Future<void> loadSettings() async {
    try {
      final config = await _configService.configLoad();
      final launchEnabled =
          Get.find<PlatformService>().launchAtLoginIsEnabled();

      final loaded = SettingsState(
        tab: state.value.tab, // 保留当前 Tab 选择
        mountDir: config.mountDir,
        concurrency: config.concurrency,
        pollInterval: config.pollIntervalSec,
        debounce: config.debounceSec,
        oauthPort: config.oauthCallbackPort,
        skipPatterns: config.skipPatterns,
        launchEnabled: launchEnabled,
        userInfo: _loadUserInfo(),
        quotaUsed: null,
        quotaTotal: null,
        appVersion: await _loadAppVersion(),
        // 对齐 CMP：载入后 saved=false（「保存设置」可点击，保存成功才转已保存）
        saved: false,
      );

      state.value = loaded;
      _original = loaded;
      AppLogger.i('设置已加载');
    } catch (e, st) {
      AppLogger.e('loadSettings 异常', e, st);
    }
    unawaited(_loadQuota());
  }

  /// 当前账号信息（AuthService 内存缓存；未登录/未拉取到时为 null）
  UserInfo? _loadUserInfo() {
    try {
      return Get.find<AuthService>().currentUserInfo;
    } catch (_) {
      return null;
    }
  }

  /// 应用版本号（package_info；测试/获取失败时为空串）
  Future<String> _loadAppVersion() async {
    try {
      return (await PackageInfo.fromPlatform()).version;
    } catch (_) {
      return '';
    }
  }

  /// 存储配额（AboutService；失败静默，配额区显示占位）
  Future<void> _loadQuota() async {
    try {
      final result = await Get.find<AboutService>().get();
      if (result is Ok<DriveAbout>) {
        state.value = state.value.copyWith(
          quotaUsed: result.value.usedSpace,
          quotaTotal: result.value.userCapacity,
        );
      }
    } catch (e) {
      AppLogger.d('加载存储配额失败: $e');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // Tab 切换
  // ═══════════════════════════════════════════════════════════════════

  /// 切换设置 Tab
  void switchTab(SettingsTab tab) {
    state.value = state.value.copyWith(
      tab: tab,
      clearErrors: true,
    );
  }

  // ═══════════════════════════════════════════════════════════════════
  // 字段更新（UI 双向绑定）
  // ═══════════════════════════════════════════════════════════════════

  /// 更新挂载目录路径
  void setMountDir(String dir) {
    state.value = state.value.copyWith(mountDir: dir, saved: false);
  }

  /// 更新并发数
  void setConcurrency(int value) {
    state.value = state.value.copyWith(concurrency: value, saved: false);
  }

  /// 更新轮询间隔（秒）
  void setPollInterval(int value) {
    state.value = state.value.copyWith(pollInterval: value, saved: false);
  }

  /// 更新防抖间隔（秒）
  void setDebounce(int value) {
    state.value = state.value.copyWith(debounce: value, saved: false);
  }

  /// 更新 OAuth 回调端口
  void setOauthPort(int value) {
    state.value = state.value.copyWith(oauthPort: value, saved: false);
  }

  /// 更新跳过文件模式
  void setSkipPatterns(List<String> patterns) {
    state.value = state.value.copyWith(skipPatterns: patterns, saved: false);
  }

  /// 更新开机启动开关
  void setLaunchEnabled(bool enabled) {
    state.value = state.value.copyWith(launchEnabled: enabled, saved: false);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 保存与重置
  // ═══════════════════════════════════════════════════════════════════

  /// 保存配置（委托 [ConfigService.configSave]），返回是否成功。
  ///
  /// 校验规则对齐 Rust `AppConfig.validate`；挂载目录变更/取消配置由
  /// configSave 编排停运行时 + 清缓存 + 重启 app，首次配置启动引擎。
  Future<bool> onSave() async {
    final s = state.value;
    final config = AppConfig(
      mountDir: s.mountDir.trim(),
      mountConfigured: s.mountDir.trim().isNotEmpty,
      concurrency: s.concurrency,
      pollIntervalSec: s.pollInterval,
      debounceSec: s.debounce,
      oauthCallbackPort: s.oauthPort,
      skipPatterns: s.skipPatterns,
    );

    // 先走实体校验（与 Rust 一致的错误文案）
    final invalid = config.validate();
    if (invalid != null) {
      state.value = state.value.copyWith(errors: [invalid.message]);
      AppLogger.d('onSave: 校验未通过: ${invalid.message}');
      return false;
    }

    try {
      // configSave 内部含目录可写探测与分支编排
      await _configService.configSave(config);

      final saved = s.copyWith(saved: true, clearErrors: true);
      state.value = saved;
      _original = saved;
      AppLogger.i('设置已保存');
      return true;
    } on ConfigError catch (e) {
      state.value = state.value.copyWith(errors: [e.message]);
    } catch (e, st) {
      AppLogger.e('onSave 异常', e, st);
      state.value = state.value.copyWith(errors: ['保存失败: $e']);
    }
    return false;
  }

  /// 重置为原始值（放弃修改；对齐 CMP「重置默认」：恢复字段并回到未保存态）
  void onReset() {
    final original = _original;
    if (original != null) {
      state.value = original.copyWith(saved: false, clearErrors: true);
      AppLogger.d('设置已重置');
    }
  }

  // ═══════════════════════════════════════════════════════════════════
  // 配置导出 / 导入（对齐 CMP ApplicationRoot.exportConfig/importConfig）
  // ═══════════════════════════════════════════════════════════════════

  /// 导出配置：file_picker 保存对话框 + [ConfigService.configExportJson]
  /// 写盘（默认文件名 PetalLink-config.json，不含 token）。
  ///
  /// 返回是否成功导出（用户取消返回 false 且不记错误）。
  Future<bool> onExportConfig() async {
    try {
      final target = await FilePicker.platform.saveFile(
        dialogTitle: '导出配置',
        fileName: 'PetalLink-config.json',
      );
      if (target == null) return false; // 用户取消

      final json = await _configService.configExportJson();
      await File(target).writeAsString(json);
      AppLogger.i('配置已导出: $target');
      return true;
    } catch (e, st) {
      AppLogger.e('onExportConfig 异常', e, st);
      state.value = state.value.copyWith(errors: ['导出配置失败: $e']);
      return false;
    }
  }

  /// 选取并解析导入配置文件（仅解析校验，不应用）。
  ///
  /// 对齐 CMP importConfig 的前半段：弹文件选择 → 读 JSON →
  /// [ConfigService.configImportJson] 解析 + 校验。返回合法的 [AppConfig]；
  /// 用户取消或解析失败返回 null（失败时错误写入 [SettingsState.errors]）。
  Future<AppConfig?> pickImportConfig() async {
    try {
      final picked = await FilePicker.platform.pickFiles(
        dialogTitle: '导入配置',
        withData: true,
      );
      final file = picked?.files.firstOrNull;
      if (file == null) return null; // 用户取消

      final bytes = file.bytes;
      final content = bytes != null
          ? utf8.decode(bytes)
          : await File(file.path!).readAsString();
      return _configService.configImportJson(content);
    } on ConfigError catch (e) {
      state.value = state.value.copyWith(errors: [e.message]);
    } catch (e, st) {
      AppLogger.e('pickImportConfig 异常', e, st);
      state.value = state.value.copyWith(errors: ['导入配置失败: $e']);
    }
    return null;
  }

  /// 应用导入的配置（经确认对话框后调用）：映射进状态并立即保存。
  ///
  /// 对齐 CMP importConfig 的确认分支：saveConfig(parsed) 立即生效
  /// （挂载目录变更由 configSave 编排清缓存并重启）。
  Future<bool> applyImportedConfig(AppConfig config) async {
    state.value = state.value.copyWith(
      mountDir: config.mountDir,
      concurrency: config.concurrency,
      pollInterval: config.pollIntervalSec,
      debounce: config.debounceSec,
      oauthPort: config.oauthCallbackPort,
      skipPatterns: config.skipPatterns,
      saved: false,
      clearErrors: true,
    );
    return onSave();
  }

  // ═══════════════════════════════════════════════════════════════════
  // 操作
  // ═══════════════════════════════════════════════════════════════════

  /// 选择挂载目录（file_picker 目录选择器）
  Future<void> onSelectDir() async {
    try {
      final result = await FilePicker.platform.getDirectoryPath(
        dialogTitle: '选择同步目录',
      );
      if (result != null) {
        setMountDir(result);
        AppLogger.i('已选择同步目录: $result');
      }
    } catch (e, st) {
      AppLogger.e('onSelectDir 异常', e, st);
    }
  }

  /// 切换开机启动（委托 PlatformService；失败回滚开关）
  Future<void> onLaunchAtLoginChange(bool enabled) async {
    setLaunchEnabled(enabled);
    final ok = await Get.find<PlatformService>().setLaunchAtLoginEnabled(enabled);
    if (!ok) {
      // 平台层失败（已记日志）→ 回滚开关
      setLaunchEnabled(!enabled);
      AppLogger.w('开机启动设置失败，已回滚: $enabled');
    }
  }

  /// 清空缓存并重启（对齐 Rust `app_clear_cache`：
  /// 清登录态、同步数据库、同步快照与配置，然后重启 App）
  Future<void> onClearCache() async {
    AppLogger.i('onClearCache: 全量清理并重启');
    final result = await Get.find<PlatformService>().appClearCache();
    if (result.isErr) {
      final err = (result as Err).error;
      AppLogger.e('清缓存失败: ${err.message}');
      state.value =
          state.value.copyWith(errors: ['清缓存失败: ${err.message}']);
    }
    // 成功路径：进程已退出
  }

  /// 手动检查更新（委托 UpdateController；有更新自动弹窗）
  Future<void> onCheckUpdate() async {
    AppLogger.d('onCheckUpdate（委托 UpdateController）');
    await Get.find<UpdateController>().manualCheck();
  }

  /// 安装更新（委托 UpdateController 下载并安装）
  Future<void> onInstallUpdate() async {
    AppLogger.d('onInstallUpdate（委托 UpdateController）');
    await Get.find<UpdateController>().downloadAndInstall();
  }

  /// 重开更新弹窗（「查看更新日志」；对齐 CMP showUpdateDialog）。
  ///
  /// 委托 [UpdateController.showUpdate]：有可展示内容直接重开，
  /// 否则触发一次手动检查。
  void onShowUpdate() {
    Get.find<UpdateController>().showUpdate();
  }

  /// 退出登录（委托 AuthController；含同步引擎停止与状态清理）
  Future<void> onLogout() async {
    AppLogger.i('onLogout（委托 AuthController）');
    await Get.find<AuthController>().logout();
  }
}
