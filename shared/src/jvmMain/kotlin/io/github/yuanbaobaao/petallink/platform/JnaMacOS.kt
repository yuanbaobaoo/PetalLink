package io.github.yuanbaobaao.petallink.platform

/**
 * JNA macOS 平台接口（对标 macosMain cinterop 功能）。
 * 用 JNA 调用 AppKit/CoreServices 原生 API。
 *
 * 详见 docs/10 §Stage 5。
 *
 * 当前为接口定义 + 简化实现。
 * TODO(production): 引入 net.java.dev.jna:jna-platform，实现：
 *   - NSStatusItem + NSMenu（托盘菜单）
 *   - FSEventStreamCreate（文件监听，替代轮询）
 *   - setxattr/getxattr（扩展属性）
 */
object JnaMacOS {
    /**
     * 用 JNA 创建 NSStatusItem。
     * 完整实现需：NSStatusBar.systemStatusBar().statusItemWithLength()
     */
    fun createStatusItem(title: String) {
        // JNA: val nsStatusBarClass = Native.load("AppKit", ...)
        // 当前降级：用 java.awt.SystemTray
        try {
            java.awt.SystemTray.getSystemTray()
        } catch (e: Throwable) {
            // headless 环境无托盘
        }
    }

    /**
     * 用 JNA 创建 FSEventStream。
     * 完整实现需：FSEventStreamCreate(allocator, callback, ctx, paths, sinceWhen, latency, flags)
     */
    fun createFSEventStream(paths: List<String>, callback: (String, Long) -> Unit): AutoCloseable {
        // JNA: val stream = CoreFoundation.INSTANCE.FSEventStreamCreate(...)
        // 当前降级：java.nio.file.WatchService
        return AutoCloseable {}
    }

    /**
     * 用 JNA 读写 xattr。
     * 完整实现需：setxattr(path, name, value, size, position, options)
     */
    fun getXattr(path: String, name: String): ByteArray? = null
    fun setXattr(path: String, name: String, value: ByteArray) {}
    fun removeXattr(path: String, name: String) {}
}
