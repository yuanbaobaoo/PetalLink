import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:window_manager/window_manager.dart';

import 'app/app.dart';
import 'app/bindings/global_binding.dart';
// 更新聚焦检查临时停用期间无需导入 UpdateController（恢复时还原）
// import 'app/update/update_controller.dart';
import 'core/logger/logger.dart';
import 'service/config/config_service.dart';
import 'service/platform/platform_service.dart';
import 'service/platform/tray_service.dart';

/// PetalLink 程序入口
///
/// 启动语义（对齐 Rust src/lib.rs setup）：
/// - `--hidden` 启动（LaunchAgent 自启）：保持窗口隐藏，仅托盘驻留
/// - 关窗（含 Cmd+W）→ 隐藏到 accessory 模式，不退出
/// - 窗口聚焦 → 恢复 regular 模式 + 触发更新聚焦检查（10 分钟节流）
Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // 初始化日志
  await AppLogger.instance.init();

  // 初始化窗口管理器（macOS 桌面）
  await windowManager.ensureInitialized();

  // 检测 --hidden 启动参数（LaunchAgent 自启静默模式）
  final hidden = await _isHiddenLaunch();

  // 配置 macOS 窗口属性（尺寸/最小尺寸/标题对齐 Tauri tauri.conf.json）
  await windowManager.setMinimumSize(const Size(700, 480));
  await windowManager.setSize(const Size(1280, 800));
  await windowManager.setTitle('PetalLink - 华为云盘客户端开源版');
  await windowManager.center();
  await windowManager.center();

  // 关窗拦截：隐藏到托盘而非退出（对齐 Rust CloseRequested 拦截）
  await windowManager.setPreventClose(true);
  windowManager.addListener(_AppLifecycleListener());

  // --hidden 保持隐藏；手动启动才显窗（对齐 Rust setup 显窗分支）
  if (!hidden) {
    await windowManager.show();
  }

  // 初始化全局依赖
  await GlobalBinding().dependencies();

  // 注册原生反向回调（退出期 flush：对齐 Rust flush_with_timeout）
  Get.find<PlatformService>().registerNativeCallbacks();

  // 托盘初始化（失败不阻断主流程）；随后按配置恢复托盘图标可见性
  // （对齐 Tauri「显示托盘图标」开关的启动恢复）
  unawaited(Get.find<TrayService>().init().then((_) async {
    try {
      final config = await Get.find<ConfigService>().configLoad();
      await Get.find<TrayService>().setVisible(config.trayVisible);
    } catch (e) {
      AppLogger.w('恢复托盘可见性失败: $e');
    }
  }));

  // 运行应用
  runApp(const MateLinkApp());
}

/// 读取原生启动参数，检测 `--hidden`（对齐 Rust is_launched_manually）
Future<bool> _isHiddenLaunch() async {
  try {
    const channel = MethodChannel('com.petallink/platform');
    final args = await channel.invokeMethod<List<Object?>>('getLaunchArgs');
    return (args ?? const []).whereType<String>().contains('--hidden');
  } catch (_) {
    return false;
  }
}

/// 应用窗口生命周期监听（关窗拦截 + 聚焦联动）
class _AppLifecycleListener extends WindowListener {
  /// 关窗（含 Cmd+W）：隐藏窗口 + accessory 模式，保持后台驻留
  /// （对齐 Rust：prevent_close + hide + set_accessory）
  @override
  Future<void> onWindowClose() async {
    await windowManager.hide();
    await Get.find<PlatformService>().setAccessoryMode();
  }

  /// 窗口聚焦：accessory → regular（对齐 ensure_regular_if_was_accessory）
  ///
  /// 【2026-07-21 临时停用】更新聚焦检查（10 分钟节流，对齐 Vue
  /// onFocusChanged）：随自动检查整体停用，恢复时取消下行注释。
  @override
  Future<void> onWindowFocus() async {
    await Get.find<PlatformService>().ensureRegularIfWasAccessory();
    // unawaited(Get.find<UpdateController>().checkOnFocus());
  }
}
