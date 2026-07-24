import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  /// 是否以 `--hidden` 启动（LaunchAgent 自启静默模式）
  private var launchedHidden: Bool {
    ProcessInfo.processInfo.arguments.contains("--hidden")
  }

  override func applicationDidFinishLaunching(_ notification: Notification) {
    // --hidden 自启：尽早切 accessory，避免 Dock 图标闪现
    // （对齐 Rust init_activation_policy）
    if launchedHidden {
      NSApp.setActivationPolicy(.accessory)
    }
    super.applicationDidFinishLaunching(notification)
  }

  /// 关窗不退出（关窗拦截在 Dart 侧 window_manager 实现：隐藏到托盘）
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  /// 退出拦截（对齐 Rust terminate_override）：
  /// - 系统关机/登出（AppleEvent 'aevt'/'quit'）→ 放行
  /// - 托盘不可见 → 放行真退出（防僵尸进程，对齐 Rust
  ///   `!is_tray_icon_visible()` 分支：用户失去托盘退出入口）
  /// - Cmd+Q / Dock Quit 且托盘可见 → 隐藏全部窗口 + accessory，拦截退出
  /// - 托盘退出与更新安装走 Dart 侧 exit(0)，不经此回调
  override func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    if isAppleEventSystemQuit() {
      return .terminateNow
    }
    // 托盘不可见时必须放行真退出，否则进程无法退出（僵尸）
    if platformChannel() != nil {
      resolveTerminateByTrayVisibility(sender)
      return .terminateLater
    } else {
      return hideToTray()
    }
  }

  /// 隐藏到托盘（accessory 模式后台保活）
  private func hideToTray() -> NSApplication.TerminateReply {
    for window in NSApp.windows where window.isVisible {
      window.orderOut(nil)
    }
    NSApp.setActivationPolicy(.accessory)
    return .terminateCancel
  }

  /// 查询 Dart 侧托盘可见性并决定退出策略（对齐 Rust 退出联动）
  private func resolveTerminateByTrayVisibility(_ sender: NSApplication) {
    guard let channel = platformChannel() else {
      sender.reply(toApplicationShouldTerminate: true)
      return
    }
    channel.invokeMethod("isTrayVisible", arguments: nil) { result in
      let visible = (result as? Bool) ?? true
      if visible {
        // 托盘可见：隐藏到后台
        _ = self.hideToTray()
        sender.reply(toApplicationShouldTerminate: false)
      } else {
        // 托盘不可见：放行真退出（防僵尸）
        sender.reply(toApplicationShouldTerminate: true)
      }
    }
  }

  /// 平台 MethodChannel（com.petallink/platform）
  private func platformChannel() -> FlutterMethodChannel? {
    guard let window = mainFlutterWindow,
          let controller = window.contentViewController as? FlutterViewController else {
      return nil
    }
    return FlutterMethodChannel(
      name: "com.petallink/platform",
      binaryMessenger: controller.engine.binaryMessenger)
  }

  /// Dock 图标点击重新激活：显示主窗口并恢复 regular
  /// （单实例语义复用 LaunchServices 默认行为：重复启动即再激活）
  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    if !flag {
      for window in NSApp.windows {
        window.makeKeyAndOrderFront(nil)
      }
    }
    NSApp.setActivationPolicy(.regular)
    NSApp.activate(ignoringOtherApps: true)
    return false
  }

  /// 判定当前退出请求是否来自系统关机/登出
  /// （对齐 Rust is_apple_event_system_quit：
  /// currentAppleEvent 为 'aevt'/'quit' 时为系统退出；
  /// 用户 Cmd+Q / Dock Quit 时 currentAppleEvent 为 nil）
  private func isAppleEventSystemQuit() -> Bool {
    guard let event = NSAppleEventManager.shared().currentAppleEvent else {
      return false
    }
    // 'aevt' = 0x61657674, 'quit' = 0x71756974
    return event.eventClass == 0x6165_7674 && event.eventID == 0x7175_6974
  }

  /// 真退出前 flush（对齐 Rust `flush_with_timeout`）：
  /// 系统关机/登出/真退出时触发，经 MethodChannel 让 Dart 侧停止引擎、
  /// 释放 FSEvents watcher，并在索引未完成时清理候选缓存残留。
  ///
  /// 注意：系统在本回调返回后会很快终止进程，Dart 异步 flush 受时限约束，
  /// 失败仅记日志，绝不阻塞退出（对齐 Rust 3.2s 超时兜底语义）。
  override func applicationWillTerminate(_ notification: Notification) {
    guard let channel = platformChannel() else {
      super.applicationWillTerminate(notification)
      return
    }
    // 同步等待 Dart flush 完成（系统允许数秒）；
    // 即便超时被杀，候选缓存清理已是尽力而为语义。
    let semaphore = DispatchSemaphore(value: 0)
    channel.invokeMethod("shutdownFlush", arguments: nil) { _ in
      semaphore.signal()
    }
    _ = semaphore.wait(timeout: .now() + 3.2)
    super.applicationWillTerminate(notification)
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }
}
