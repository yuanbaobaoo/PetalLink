package io.github.yuanbaobaoo.petallink.platform

import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi
import io.github.yuanbaobaoo.petallink.sync.TransferState
import java.awt.AWTException
import java.awt.Image
import java.awt.MenuItem
import java.awt.PopupMenu
import java.awt.SystemTray
import java.awt.TrayIcon
import java.awt.event.ActionEvent
import java.awt.event.ActionListener
import javax.imageio.ImageIO

/**
 * 原生系统托盘（对标原 Tauri `src/platform/tray.rs` NSStatusItem）。
 *
 * 用 AWT [TrayIcon] 替代 Compose `Tray`，获得两项原版能力：
 * 1. [TrayIcon.setImageAutoSize]`(true)`：图标自适应状态栏尺寸，避免大图缩放模糊；
 * 2. macOS 上 AWT TrayIcon 左键点击即弹出 [PopupMenu]（对标原版 `show_menu_on_left_click(true)`）。
 *
 * 菜单结构（对齐原版 buildStatusMenu）：
 * 版本标识 → 分隔 → 显示主窗口 → [分隔 → 进行中传输项…] → 分隔 → 退出 PetalLink。
 *
 * @param onShow 「显示主窗口」回调
 * @param onQuit 「退出」回调
 */
class DesktopTray(
    private val onShow: () -> Unit,
    private val onQuit: () -> Unit,
) {
    private var trayIcon: TrayIcon? = null

    /**
     * 进行中的传输任务（菜单动态段）。
     *
     * 对标原版 `load_active_transfers`：仅 state IN (PENDING, RUNNING)。
     * 设值时按 5 秒节流重建菜单（`MENU_REBUILD_INTERVAL_MS`），避免进度高频变化导致已展开菜单闪烁。
     */
    var activeTransfers: List<TransferTaskUi> = emptyList()
        set(value) {
            field = value
            scheduleRebuild()
        }

    /**
     * 当前 tooltip。
     */
    var tooltip: String = "PetalLink"
        set(value) {
            field = value
            trayIcon?.toolTip = value
        }

    /**
     * 创建并添加托盘图标到系统托盘。失败（不支持/无桌面）返回 false。
     */
    fun install(): Boolean {
        if (!SystemTray.isSupported()) return false
        val iconImage = loadTrayImage() ?: return false
        val icon = TrayIcon(iconImage, tooltip, buildMenu()).apply {
            isImageAutoSize = true // 图标自适应状态栏尺寸，保持清晰（对标原版 NSStatusItem 渲染）
            addMouseListener(object : java.awt.event.MouseAdapter() {
                // macOS AWT TrayIcon 左键点击会自动弹 PopupMenu；
                // 这里额外处理：双击时触发 onShow（对标原版 on_tray_click 可选行为）。
                override fun mouseClicked(e: java.awt.event.MouseEvent) {
                    if (e.clickCount >= 2 && e.button == java.awt.event.MouseEvent.BUTTON1) onShow()
                }
            })
        }
        return try {
            SystemTray.getSystemTray().add(icon)
            trayIcon = icon
            true
        } catch (e: AWTException) {
            false
        }
    }

    /**
     * 重建菜单（传输任务变化时调用）。
     */
    fun rebuildMenu() {
        trayIcon?.setPopupMenu(buildMenu())
    }

    /**
     * 菜单重建最小间隔（毫秒，对标原版 MENU_REBUILD_INTERVAL_MS = 5000）。
     */
    private var lastMenuRebuildMs: Long = 0L

    /**
     * 节流重建菜单（对标原版 refresh_menu 节流逻辑）。
     *
     * 有进行中传输时按 5 秒节流，避免进度高频回调导致已展开菜单闪烁消失；
     * 无传输时立即重建（清场，不残留已完成项）。
     */
    private fun scheduleRebuild() {
        val now = System.currentTimeMillis()
        if (activeTransfers.isNotEmpty()) {
            val last = lastMenuRebuildMs
            if (last != 0L && now - last < MENU_REBUILD_INTERVAL_MS) return // 节流窗口内跳过
        }
        lastMenuRebuildMs = now
        rebuildMenu()
    }

    /**
     * 菜单重建最小间隔（毫秒）。传输进度高频变化，过频重建会让已展开菜单闪烁消失。
     */
    private companion object {
        const val MENU_REBUILD_INTERVAL_MS: Long = 5000L
    }

    /**
     * 从系统托盘移除图标。
     */
    fun remove() {
        val icon = trayIcon ?: return
        runCatching { SystemTray.getSystemTray().remove(icon) }
        trayIcon = null
    }

    /**
     * 加载 menubar-icon.png；失败回退系统默认图标。
     */
    private fun loadTrayImage(): Image? = runCatching {
        val loader = Thread.currentThread().contextClassLoader ?: ClassLoader.getSystemClassLoader()
        val stream = loader.getResourceAsStream("assets/menubar-icon.png") ?: return null
        ImageIO.read(stream)
    }.getOrNull()

    /**
     * 构建托盘菜单（对齐原版 build_menu）。
     */
    private fun buildMenu(): PopupMenu = PopupMenu().apply {
        // 版本标识（disabled 纯展示，对标原版「PetalLink - 华为云盘 Mac 客户端开源版」）
        add(MenuItem("PetalLink - 华为云盘 Mac 客户端开源版").apply { isEnabled = false })
        addSeparator()
        // 显示主窗口（对标原版 show_window）
        add(MenuItem("显示主窗口").apply { addActionListener(ActionListener { onShow() }) })
        // 注：原版不提供「立即刷新」项（tray.rs 注释：按需求直接不提供，菜单与
        // Flutter 默认态 canSync=false 一致），故此处也不加。
        // 进行中传输段（每个任务两行：文件名 + 正在X…N%，disabled）
        if (activeTransfers.isNotEmpty()) {
            addSeparator()
            activeTransfers.forEach { task ->
                add(MenuItem(task.fileName.take(20)).apply { isEnabled = false })
                add(MenuItem(formatTransferStatus(task)).apply { isEnabled = false })
            }
        }
        addSeparator()
        // 退出 PetalLink（对标原版 quit）
        add(MenuItem("退出 PetalLink").apply { addActionListener(ActionListener { onQuit() }) })
    }

    /**
     * 传输状态文案（对标原版 format_transfer_status）。
     *
     * - upload → 「正在上传…N%」
     * - download → 「正在下载…N%」
     * - download_update → 「正在更新…N%」
     * - delete → 「正在删除…N%」
     * - pending → 「等待中」
     *
     * total 缺失（0）时百分比按 0 显示。
     */
    private fun formatTransferStatus(task: TransferTaskUi): String {
        if (task.state == TransferState.Pending) return "等待中"
        val label = when (task.direction) {
            "upload" -> "正在上传"
            "download" -> "正在下载"
            "delete" -> "正在删除"
            else -> "正在传输"
        }
        val pct = if (task.bytesTotal > 0) {
            minOf(100, (task.progress * 100).toInt())
        } else 0
        return "$label…$pct%"
    }
}
