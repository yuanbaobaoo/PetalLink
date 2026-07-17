package io.github.yuanbaobaoo.petallink.platform

import com.sun.jna.NativeLibrary
import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.channels.FileChannel
import java.nio.channels.FileLock
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.nio.file.attribute.PosixFilePermissions
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

/**
 * 文件锁 + loopback SHOW 通道；第二实例只负责唤醒已有窗口后退出。
 */
class SingleInstanceCoordinator(
    private val dataDir: Path,
    private val onShow: () -> Unit,
) : AutoCloseable {
    private val lockPath = dataDir.resolve("instance.lock")
    private val portPath = dataDir.resolve("instance.port")
    private var channel: FileChannel? = null
    private var lock: FileLock? = null
    private var server: ServerSocket? = null
    private val running = AtomicBoolean(false)
    private val owned = AtomicBoolean(false)

    /**
     * 尝试获取单实例文件锁；成功则启动 loopback 监听线程并返回 true，失败则唤醒已有实例后返回 false。
     */
    fun acquireOrNotify(): Boolean {
        Files.createDirectories(dataDir)
        val opened = FileChannel.open(lockPath, StandardOpenOption.CREATE, StandardOpenOption.WRITE)
        val acquired = try { opened.tryLock() } catch (_: Throwable) { null }
        if (acquired == null) {
            opened.close()
            notifyPrimary()
            return false
        }
        channel = opened
        lock = acquired
        owned.set(true)
        val socket = ServerSocket(0, 16, InetAddress.getLoopbackAddress())
        server = socket
        writeAtomically(portPath, socket.localPort.toString())
        running.set(true)
        thread(name = "petallink-single-instance", isDaemon = true) {
            while (running.get()) {
                val accepted = runCatching { socket.accept() }.getOrNull() ?: break
                accepted.use { client ->
                    val command = BufferedReader(InputStreamReader(client.getInputStream())).readLine()
                    if (command == "SHOW") onShow()
                }
            }
        }
        return true
    }

    /**
     * 作为第二实例读取端口文件并向主实例发送 SHOW 指令，带重试。
     */
    private fun notifyPrimary() {
        repeat(5) {
            val port = runCatching { Files.readString(portPath).trim().toInt() }.getOrNull()
            if (port != null && runCatching {
                Socket(InetAddress.getLoopbackAddress(), port).use { socket ->
                    socket.getOutputStream().bufferedWriter().use { it.write("SHOW\n") }
                }
            }.isSuccess) return
            Thread.sleep(100)
        }
    }

    /**
     * 关闭监听线程与网络资源，并在持锁方释放端口文件与文件锁。
     */
    override fun close() {
        running.set(false)
        runCatching { server?.close() }
        if (owned.compareAndSet(true, false)) {
            runCatching { Files.deleteIfExists(portPath) }
        }
        runCatching { lock?.release() }
        runCatching { channel?.close() }
    }

    /**
     * 先写临时文件再原子移动，避免目标文件被读到半写状态。
     */
    private fun writeAtomically(target: Path, text: String) {
        val tmp = target.resolveSibling("${target.fileName}.tmp")
        Files.writeString(tmp, text, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING)
        try {
            Files.move(tmp, target, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
            Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING)
        }
    }
}

/**
 * macOS LaunchAgent 管理；通过写入/移除 plist 并调用 launchctl 实现开机自启。
 */
class LaunchAgentManager(
    private val bundleId: String,
    private val command: Path,
    private val launchAgentsDir: Path = Path.of(System.getProperty("user.home"), "Library", "LaunchAgents"),
) {
    val plistPath: Path get() = launchAgentsDir.resolve("$bundleId.plist")

    /**
     * 自启是否启用，以 plist 文件存在为准。
     */
    fun isEnabled(): Boolean = Files.isRegularFile(plistPath)

    /**
     * 启用或禁用开机自启；启用时写入 plist 并 bootstrap 到 launchd，禁用时 bootout 后删除 plist。
     */
    fun setEnabled(enabled: Boolean) {
        if (!enabled) {
            launchctl("bootout", "gui/${currentUid()}/$bundleId")
            Files.deleteIfExists(plistPath)
            return
        }
        Files.createDirectories(launchAgentsDir)
        val escapedCommand = xml(command.toAbsolutePath().normalize().toString())
        val content = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>${xml(bundleId)}</string>
  <key>ProgramArguments</key><array><string>$escapedCommand</string><string>--hidden</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict></plist>
"""
        val tmp = plistPath.resolveSibling("${plistPath.fileName}.tmp")
        Files.writeString(tmp, content, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING)
        runCatching { Files.setPosixFilePermissions(tmp, PosixFilePermissions.fromString("rw-------")) }
        try {
            Files.move(tmp, plistPath, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
            Files.move(tmp, plistPath, StandardCopyOption.REPLACE_EXISTING)
        }
        launchctl("bootout", "gui/${currentUid()}/$bundleId")
        launchctl("bootstrap", "gui/${currentUid()}", plistPath.toString())
    }

    /**
     * 获取当前用户的 uid（通过 `id -u`），失败时回退为 501。
     */
    private fun currentUid(): String = runCatching {
        ProcessBuilder("id", "-u").start().let { process ->
            process.inputStream.bufferedReader().readText().trim().also { process.waitFor() }
        }.takeIf { it.isNotBlank() }
    }.getOrNull() ?: "501"

    /**
     * 仅在 macOS 且 LaunchAgents 目录为系统默认位置时才调用 launchctl，避免在测试环境误触。
     */
    private fun launchctl(vararg args: String) {
        if (!System.getProperty("os.name").contains("mac", ignoreCase = true)) return
        val productionDir = Path.of(System.getProperty("user.home"), "Library", "LaunchAgents")
        if (launchAgentsDir.toAbsolutePath().normalize() != productionDir.toAbsolutePath().normalize()) return
        runCatching { ProcessBuilder(listOf("launchctl") + args).start().waitFor() }
    }

    /**
     * 对字符串做 XML 实体转义，用于安全嵌入 plist 模板。
     */
    private fun xml(value: String) = value
        .replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        .replace("\"", "&quot;").replace("'", "&apos;")
}

/**
 * AppKit activationPolicy 的 best-effort JNA 桥；失败时保留 Compose 默认行为。
 */
object MacActivationPolicy {
    private val available = System.getProperty("os.name").contains("mac", ignoreCase = true)

    /**
     * 将应用切换为 accessory 策略（无 Dock 图标、不抢占焦点）。
     */
    fun accessory() = setPolicy(1L, activate = false)
    /**
     * 将应用切换为 regular 策略并激活到前台。
     */
    fun regularAndActivate() = setPolicy(0L, activate = true)

    /**
     * 判定当前 Apple Event 是否为系统的退出事件（kAEQuitApplication），用于区分关闭窗口与退出应用。
     */
    fun isSystemQuitAppleEvent(): Boolean {
        if (!available) return false
        return runCatching {
            val objc = NativeLibrary.getInstance("objc")
            val getClass = objc.getFunction("objc_getClass")
            val selector = objc.getFunction("sel_registerName")
            val send = objc.getFunction("objc_msgSend")
            val managerClass = getClass.invokePointer(arrayOf("NSAppleEventManager"))
            val shared = selector.invokePointer(arrayOf("sharedAppleEventManager"))
            val manager = send.invokePointer(arrayOf(managerClass, shared))
            val current = selector.invokePointer(arrayOf("currentAppleEvent"))
            val event = send.invokePointer(arrayOf(manager, current)) ?: return@runCatching false
            val eventClass = send.invokeLong(arrayOf(event, selector.invokePointer(arrayOf("eventClass"))))
            val eventId = send.invokeLong(arrayOf(event, selector.invokePointer(arrayOf("eventID"))))
            eventClass.toInt() == 0x61657674 && eventId.toInt() == 0x71756974
        }.getOrDefault(false)
    }

    /**
     * 通过 Objective-C runtime 设置 NSApp 的 activationPolicy，失败时静默忽略。
     */
    private fun setPolicy(policy: Long, activate: Boolean) {
        if (!available) return
        runCatching {
            val objc = NativeLibrary.getInstance("objc")
            val getClass = objc.getFunction("objc_getClass")
            val selector = objc.getFunction("sel_registerName")
            val send = objc.getFunction("objc_msgSend")
            val appClass = getClass.invokePointer(arrayOf("NSApplication"))
            val sharedSel = selector.invokePointer(arrayOf("sharedApplication"))
            val app = send.invokePointer(arrayOf(appClass, sharedSel))
            val policySel = selector.invokePointer(arrayOf("setActivationPolicy:"))
            send.invoke(Void.TYPE, arrayOf(app, policySel, policy))
            if (activate) {
                val activateSel = selector.invokePointer(arrayOf("activateIgnoringOtherApps:"))
                send.invoke(Void.TYPE, arrayOf(app, activateSel, true))
            }
        }
    }
}
