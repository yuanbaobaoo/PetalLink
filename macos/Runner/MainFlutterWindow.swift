import Cocoa
import FlutterMacOS
import IOKit

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)

    // PetalLink 平台通道：
    // - getPlatformUUID：本机 IOPlatformUUID（token.bin 密钥派生用，
    //   对齐 Rust 原版 src/auth/token_store.rs 的 ioreg 读取语义）
    // - getXattr/setXattr/removeXattr/listXattrs：占位符状态与 Finder 标签的
    //   xattr 读写（对齐 Rust 原版 src/mount/manager.rs，直接调 getxattr(2) 族）
    let platformChannel = FlutterMethodChannel(
      name: "com.petallink/platform",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    platformChannel.setMethodCallHandler { (call, result) in
      switch call.method {
      case "getPlatformUUID":
        if let uuid = Self.readPlatformUUID() {
          result(uuid)
        } else {
          result(FlutterError(
            code: "UUID_UNAVAILABLE",
            message: "无法读取 IOPlatformUUID",
            details: nil))
        }
      case "getXattr":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String,
          let name = args["name"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "getXattr 需要 path/name 参数", details: nil))
          return
        }
        Self.getXattr(path: path, name: name, result: result)
      case "setXattr":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String,
          let name = args["name"] as? String,
          let value = args["value"] as? FlutterStandardTypedData
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "setXattr 需要 path/name/value 参数", details: nil))
          return
        }
        Self.setXattr(path: path, name: name, value: value.data, result: result)
      case "removeXattr":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String,
          let name = args["name"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "removeXattr 需要 path/name 参数", details: nil))
          return
        }
        Self.removeXattr(path: path, name: name, result: result)
      case "listXattrs":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "listXattrs 需要 path 参数", details: nil))
          return
        }
        Self.listXattrs(path: path, result: result)
      case "getFreeSpace":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "getFreeSpace 需要 path 参数", details: nil))
          return
        }
        Self.getFreeSpace(path: path, result: result)
      case "getInodeInfo":
        guard let args = call.arguments as? [String: Any],
          let path = args["path"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "getInodeInfo 需要 path 参数", details: nil))
          return
        }
        Self.getInodeInfo(path: path, result: result)
      case "setActivationPolicy":
        guard let args = call.arguments as? [String: Any],
          let policy = args["policy"] as? String
        else {
          result(FlutterError(
            code: "BAD_ARGS", message: "setActivationPolicy 需要 policy 参数", details: nil))
          return
        }
        Self.setActivationPolicy(policy: policy)
        result(nil)
      case "getLaunchArgs":
        result(ProcessInfo.processInfo.arguments)
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    super.awakeFromNib()
  }

  /// 读取 IOPlatformUUID（IOKit，无需 root，沙盒内可用）。
  private static func readPlatformUUID() -> String? {
    let service: io_service_t
    if #available(macOS 12.0, *) {
      service = IOServiceGetMatchingService(
        kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"))
    } else {
      service = IOServiceGetMatchingService(
        kIOMasterPortDefault, IOServiceMatching("IOPlatformExpertDevice"))
    }
    guard service != 0 else { return nil }
    defer { IOObjectRelease(service) }
    let property = IORegistryEntryCreateCFProperty(
      service, kIOPlatformUUIDKey as CFString, kCFAllocatorDefault, 0)
    return property?.takeRetainedValue() as? String
  }

  // ============================================================
  // xattr（对齐 Rust xattr crate：默认跟随符号链接，options=0）
  // ============================================================

  /// 读取 xattr；属性不存在（ENOATTR）返回 nil，Dart 侧收到 null。
  private static func getXattr(path: String, name: String, result: FlutterResult) {
    let size = path.withCString { pathPtr in
      name.withCString { namePtr in
        getxattr(pathPtr, namePtr, nil, 0, 0, 0)
      }
    }
    if size < 0 {
      let err = errno
      if err == ENOATTR {
        result(nil)
      } else {
        result(Self.xattrError(code: "XATTR_GET_FAILED", name: name, err: err))
      }
      return
    }
    var buffer = [UInt8](repeating: 0, count: size)
    let read = buffer.withUnsafeMutableBytes { raw in
      path.withCString { pathPtr in
        name.withCString { namePtr in
          getxattr(pathPtr, namePtr, raw.baseAddress, size, 0, 0)
        }
      }
    }
    if read < 0 {
      let err = errno
      result(Self.xattrError(code: "XATTR_GET_FAILED", name: name, err: err))
      return
    }
    result(FlutterStandardTypedData(bytes: Data(buffer)))
  }

  /// 写入 xattr。
  private static func setXattr(
    path: String, name: String, value: Data, result: FlutterResult
  ) {
    let status = value.withUnsafeBytes { raw in
      path.withCString { pathPtr in
        name.withCString { namePtr in
          setxattr(pathPtr, namePtr, raw.baseAddress, value.count, 0, 0)
        }
      }
    }
    if status != 0 {
      let err = errno
      result(Self.xattrError(code: "XATTR_SET_FAILED", name: name, err: err))
      return
    }
    result(nil)
  }

  /// 移除 xattr（幂等：不存在视为成功，对齐 Rust `let _ = xattr::remove(...)`）。
  private static func removeXattr(path: String, name: String, result: FlutterResult) {
    let status = path.withCString { pathPtr in
      name.withCString { namePtr in
        removexattr(pathPtr, namePtr, 0)
      }
    }
    if status != 0 {
      let err = errno
      if err != ENOATTR {
        result(Self.xattrError(code: "XATTR_REMOVE_FAILED", name: name, err: err))
        return
      }
    }
    result(nil)
  }

  /// 列出全部 xattr 名（listxattr 返回 NUL 分隔的名字串）。
  private static func listXattrs(path: String, result: FlutterResult) {
    let size = path.withCString { listxattr($0, nil, 0, 0) }
    if size < 0 {
      let err = errno
      result(Self.xattrError(code: "XATTR_LIST_FAILED", name: "*", err: err))
      return
    }
    if size == 0 {
      result([String]())
      return
    }
    var buffer = [UInt8](repeating: 0, count: size)
    let read = buffer.withUnsafeMutableBytes { raw in
      path.withCString { listxattr($0, raw.baseAddress, size, 0) }
    }
    if read < 0 {
      let err = errno
      result(Self.xattrError(code: "XATTR_LIST_FAILED", name: "*", err: err))
      return
    }
    let data = Data(buffer[0..<read])
    let names = data.split(separator: 0).compactMap { String(bytes: $0, encoding: .utf8) }
    result(names)
  }

  /// 构造统一的 xattr 错误（携带 errno 便于排查）。
  private static func xattrError(code: String, name: String, err: Int32) -> FlutterError {
    return FlutterError(
      code: code,
      message: "\(name)：errno=\(err)（\(String(cString: strerror(err)))）",
      details: nil)
  }

  // ============================================================
  // statfs / lstat（Rust 版无对应命令，Flutter 侧新增平台原语）
  // ============================================================

  /// 卷可用空间（statfs：f_bavail * f_bsize，字节）。
  private static func getFreeSpace(path: String, result: FlutterResult) {
    var st = statfs()
    let status = path.withCString { statfs($0, &st) }
    if status != 0 {
      let err = errno
      result(FlutterError(
        code: "STATFS_FAILED",
        message: "statfs 失败：errno=\(err)（\(String(cString: strerror(err)))）",
        details: nil))
      return
    }
    result(Int64(st.f_bavail) * Int64(st.f_bsize))
  }

  /// 文件 inode 与元数据（lstat，不跟随符号链接）。
  private static func getInodeInfo(path: String, result: FlutterResult) {
    var st = stat()
    let status = path.withCString { lstat($0, &st) }
    if status != 0 {
      let err = errno
      result(FlutterError(
        code: "LSTAT_FAILED",
        message: "lstat 失败：errno=\(err)（\(String(cString: strerror(err)))）",
        details: nil))
      return
    }
    result([
      "ino": Int64(st.st_ino),
      "dev": Int64(st.st_dev),
      "mode": Int64(st.st_mode),
      "nlink": Int64(st.st_nlink),
      "size": Int64(st.st_size),
      "mtimeMs": Int64(st.st_mtimespec.tv_sec) * 1000
        + Int64(st.st_mtimespec.tv_nsec) / 1_000_000,
    ])
  }

  // ============================================================
  // 激活策略（对齐 Rust src/platform/activation.rs）
  // ============================================================

  /// regular=0 / accessory=1；regular 附带激活应用（对齐 set_regular）。
  private static func setActivationPolicy(policy: String) {
    switch policy {
    case "accessory":
      NSApp.setActivationPolicy(.accessory)
    default:
      NSApp.setActivationPolicy(.regular)
      NSApp.activate(ignoringOtherApps: true)
    }
  }
}
