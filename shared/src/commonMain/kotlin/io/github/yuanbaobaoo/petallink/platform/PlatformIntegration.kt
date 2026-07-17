package io.github.yuanbaobaoo.petallink.platform

/**
 * 托盘菜单接口（对标 src/platform/tray.rs）。
 *
 * 详见 docs/10 阶段 5 item 25。
 * NSStatusItem + 菜单（rebuild 节流 5000ms，transfer_signature 防闪烁）。
 */
interface TrayManager {
    /** 显示托盘图标与菜单 */
    fun show()

    /** 隐藏托盘 */
    fun hide()

    /** 更新托盘标题/状态（同步状态摘要） */
    fun updateStatus(text: String)

    /** 托盘重建节流间隔（ms） */
    companion object {
        const val REBUILD_THROTTLE_MS = 5000L
    }
}

/**
 * 应用激活策略（对标 src/platform/activation.rs）。
 *
 * 详见 docs/10 阶段 5 item 26、难点 6。
 * regular=0（Dock 显示）/ accessory=1（后台）。
 * swizzle NSApplication terminate: 拦截 Dock/Cmd+Q。
 */
interface ActivationManager {
    /** 切换到 regular（显示窗口/Dock） */
    fun activateRegular()

    /** 切换到 accessory（后台托盘） */
    fun deactivateToAccessory()

    /** 检测 --hidden 启动参数 */
    fun isHiddenLaunch(): Boolean

    companion object {
        const val POLICY_REGULAR = 0
        const val POLICY_ACCESSORY = 1
        // Apple Event 常量（拦截 terminate 用）
        const val K_CORE_EVENT_CLASS = 0x61657674  // 'aevt'
        const val K_AE_QUIT_APPLICATION = 0x71756974  // 'quit'
    }
}

/**
 * 开机自启动（对标 src/platform/launch_at_login.rs）。
 *
 * 详见 docs/10 阶段 5 item 27。
 * LaunchAgent plist + --hidden 参数，launchctl bootstrap/bootout。
 */
interface LaunchAtLoginManager {
    /** 启用开机自启 */
    fun enable(): Boolean

    /** 禁用开机自启 */
    fun disable(): Boolean

    /** 查询当前是否启用 */
    fun isEnabled(): Boolean
}

/**
 * 应用关闭流程（对标 src/platform/shutdown.rs）。
 *
 * 详见 docs/10 阶段 5 item 28。
 * flush + sentinel，3.2s 超时兜底。
 */
interface ShutdownManager {
    /**
     * 优雅关闭：等待同步周期完成 + 已提交任务 + 标记缓存不完整。
     * @return true 正常关闭；false 超时强制退出
     */
    suspend fun gracefulShutdown(): Boolean

    companion object {
        /** 关闭超时：3.2 秒 */
        const val SHUTDOWN_TIMEOUT_MS = 3200L
    }
}

/**
 * 单实例守护（对标原项目 single_instance 插件）。
 *
 * 详见 docs/10 阶段 5 item 29。
 * 文件锁 / 端口占用，第二个实例 exit(0) + 聚焦已有实例。
 * 必须最先注册（防双 FSEvents watcher 互相同步循环）。
 */
interface SingleInstanceGuard {
    /**
     * 尝试获取单实例锁。
     * @return true 表示本实例是首个（继续运行）；false 表示已有实例（应退出）
     */
    fun acquire(): Boolean

    /** 释放锁（正常退出时） */
    fun release()

    /** 聚焦已有实例窗口 */
    fun focusExisting()
}
