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
  /// - Cmd+Q / Dock Quit → 隐藏全部窗口 + accessory，拦截退出
  /// - 托盘退出与更新安装走 Dart 侧 exit(0)，不经此回调
  override func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    if isAppleEventSystemQuit() {
      return .terminateNow
    }
    for window in NSApp.windows where window.isVisible {
      window.orderOut(nil)
    }
    NSApp.setActivationPolicy(.accessory)
    return .terminateCancel
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

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }
}
