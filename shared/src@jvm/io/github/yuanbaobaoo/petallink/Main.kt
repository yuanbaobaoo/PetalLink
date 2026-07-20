package io.github.yuanbaobaoo.petallink

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.CircularProgressIndicator
import androidx.compose.material.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import io.github.yuanbaobaoo.petallink.app.AppPage
import io.github.yuanbaobaoo.petallink.app.ApplicationRoot
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.core.BuildInfo
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.github.yuanbaobaoo.petallink.core.logging.LoggerRuntime
import io.github.yuanbaobaoo.petallink.platform.MacActivationPolicy
import io.github.yuanbaobaoo.petallink.platform.SingleInstanceCoordinator
import io.github.yuanbaobaoo.petallink.ui.pages.main.*
import io.github.yuanbaobaoo.petallink.ui.theme.PetalLinkTheme
import io.github.yuanbaobaoo.petallink.ui.viewmodel.UpdaterPhase
import kotlinx.coroutines.launch
import java.awt.Desktop
import java.awt.event.WindowAdapter
import java.awt.event.WindowEvent
import java.util.concurrent.atomic.AtomicReference
import javax.swing.SwingUtilities

/**
 * JVM 应用入口：协调单实例锁、窗口/托盘生命周期，并根据应用状态分发到各页面。
 */
fun main(args: Array<String>) {
    val paths = AppPaths.fromEnvironment()
    // 尽早初始化日志后端（对标原 Tauri 在 run() 开头 init_logger），保证入口日志落入文件。
    LoggerRuntime.configure(paths.logsDir)
    val logger = Logger()
    logger.info("app") { "PetalLink 启动中：bundleId=${AppPaths.currentBundleId()}, version=${BuildInfo.VERSION}" }
    if ("--hidden" in args) {
        logger.info("app") { "--hidden 模式：主窗口保持隐藏，仅保留菜单栏图标" }
    } else {
        logger.info("app") { "手动启动：显示主窗口" }
    }
    val showWindow = AtomicReference<() -> Unit>({})
    val instance = SingleInstanceCoordinator(paths.dataDir) {
        SwingUtilities.invokeLater { showWindow.get().invoke() }
    }
    if (!instance.acquireOrNotify()) return

    try {
        application(exitProcessOnExit = false) {
            val root = remember { ApplicationRoot(paths) }
            val state by root.viewModel.state.collectAsState()
            var visible by remember { mutableStateOf("--hidden" !in args) }
            var exiting by remember { mutableStateOf(false) }
            val windowState = rememberWindowState()
            val scope = rememberCoroutineScope()

            /**
             * 显示主窗口并将其切回常规应用模式（含 macOS ActivationPolicy 激活）。
             */
            fun show() {
                visible = true
                MacActivationPolicy.regularAndActivate()
            }

            /**
             * 隐藏主窗口并切换为 accessory（仅托盘驻留）模式。
             */
            fun hide() {
                visible = false
                MacActivationPolicy.accessory()
            }

            /**
             * 退出应用：清理根组件与单实例锁后调用 exitApplication；重入时直接返回。
             */
            fun quit() {
                if (exiting) return
                exiting = true
                scope.launch {
                    root.close()
                    instance.close()
                    exitApplication()
                }
            }

            DisposableEffect(Unit) {
                showWindow.set(::show)
                val desktop = runCatching { Desktop.getDesktop() }.getOrNull()
                if (desktop?.isSupported(Desktop.Action.APP_QUIT_HANDLER) == true) {
                    desktop.setQuitHandler { _, response ->
                        // 保持原短路语义：仅在未处于退出流程时才探测系统 Apple Event。
                        val systemQuit = !exiting && MacActivationPolicy.isSystemQuitAppleEvent()
                        if (systemQuit) {
                            logger.info("platform.activation") { "检测到系统关机/登出 Apple Event（kAEQuitApplication），放行退出" }
                        }
                        if (exiting || systemQuit) {
                            exiting = true
                            root.close()
                            instance.close()
                            response.performQuit()
                        } else {
                            logger.info("platform.activation") { "Dock/Cmd+Q 退出已拦截：隐藏窗口，保持后台运行" }
                            response.cancelQuit()
                            SwingUtilities.invokeLater(::hide)
                        }
                    }
                    logger.info("platform.activation") { "已安装 NSApplication terminate: 拦截器（含系统关机检测）" }
                }
                onDispose { showWindow.set({}) }
            }

            // 原生系统托盘（AWT TrayIcon，对标原 Tauri NSStatusItem）：
            // setImageAutoSize(true) 解决图标模糊；macOS 左键点击弹出 PopupMenu。
            val tray = remember {
                io.github.yuanbaobaoo.petallink.platform.DesktopTray(
                    onShow = { SwingUtilities.invokeLater(::show) },
                    onQuit = { SwingUtilities.invokeLater(::quit) },
                ).also { it.install() }
            }
            DisposableEffect(Unit) {
                onDispose { tray.remove() }
            }
            // 同步状态到托盘（tooltip / 传输段）
            LaunchedEffect(state.syncStatus) { tray.tooltip = "PetalLink · ${state.syncStatus}" }
            LaunchedEffect(state.transfers) {
                // 仅展示进行中（Pending/Running）任务，对标原版 load_active_transfers 的 state IN (PENDING, RUNNING)
                tray.activeTransfers = state.transfers.filter {
                    it.state in setOf(
                        io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
                        io.github.yuanbaobaoo.petallink.sync.TransferState.Running,
                    )
                }
            }

            LaunchedEffect(visible) {
                if (visible) {
                    windowState.isMinimized = false
                    MacActivationPolicy.regularAndActivate()
                } else MacActivationPolicy.accessory()
            }

            LaunchedEffect(state.updateReadyToQuit) {
                if (state.updateReadyToQuit) quit()
            }

            Window(
                visible = visible,
                state = windowState,
                title = "PetalLink - 华为云盘客户端开源版",
                onCloseRequest = ::hide,
            ) {
                DisposableEffect(window) {
                    val listener = object : WindowAdapter() {
                        /**
                         * 窗口获得焦点时通知 viewModel，用于同步聚焦状态。
                         */
                        override fun windowGainedFocus(event: WindowEvent?) {
                            root.viewModel.onWindowFocused()
                        }
                    }
                    window.addWindowFocusListener(listener)
                    onDispose { window.removeWindowFocusListener(listener) }
                }
                LaunchedEffect(visible) {
                    if (visible) {
                        window.toFront()
                        window.requestFocus()
                    }
                }
                PetalLinkTheme {
                    Surface(modifier = Modifier.fillMaxSize()) {
                        Box(modifier = Modifier.fillMaxSize()) {
                            when {
                            !state.initialized -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator()
                            }
                            !state.loggedIn || state.page == AppPage.LOGIN -> LoginScreen(
                                loggingIn = state.loggingIn,
                                secretConfigured = state.secretConfigured,
                                errorMessage = state.errorMessage,
                                onLogin = root.viewModel::login,
                                onCancel = root.viewModel::cancelLogin,
                                onDismissError = root.viewModel::dismissError,
                            )
                            state.page == AppPage.SETTINGS -> SettingsScreen(
                                initialConfig = state.config,
                                launchAtLogin = state.launchAtLogin,
                                userInfo = state.userInfo,
                                appVersion = state.appVersion,
                                quotaUsed = state.quotaUsedBytes,
                                quotaTotal = state.quotaTotalBytes,
                                availableUpdate = state.availableUpdate,
                                updateStatus = state.updateStatus,
                                updateChecking = state.updatePhase == UpdaterPhase.CHECKING,
                                onLaunchAtLoginChange = root.viewModel::setLaunchAtLogin,
                                onBack = root.viewModel::openFiles,
                                onLogout = root.viewModel::logout,
                                onOpenLogs = root.viewModel::openLogs,
                                onExportConfig = root.viewModel::exportConfig,
                                onImportConfig = root.viewModel::importConfig,
                                onClearCache = root.viewModel::clearApplicationCache,
                                onCheckUpdate = root.viewModel::checkForUpdate,
                                onInstallUpdate = root.viewModel::installUpdate,
                                onSelectDir = root.viewModel::chooseMountDirectory,
                                onSave = root.viewModel::saveConfig,
                                updateDownloading = state.updatePhase == UpdaterPhase.DOWNLOADING,
                                updateDownloadProgress = state.updateDownloadProgress,
                                onShowUpdate = root.viewModel::showUpdateDialog,
                            )
                            state.page == AppPage.LOGS -> {
                                // 日志页 2 秒轮询刷新（对标原 Vue LogViewerPage）
                                LaunchedEffect(Unit) {
                                    while (true) {
                                        kotlinx.coroutines.delay(2_000)
                                        root.viewModel.reloadLogs()
                                    }
                                }
                                LogViewerScreen(
                                    records = state.logs,
                                    onBack = root.viewModel::openFiles,
                                    onExport = root.viewModel::exportLogs,
                                    onClear = root.viewModel::clearLogs,
                                )
                            }
                            else -> MainScreen(
                                browser = state.browser,
                                sync = state.sync,
                                setupPhase = state.setupPhase,
                                mountDir = state.config.mountDir,
                                userName = state.userName,
                                quotaText = state.quotaText,
                                transfers = state.transfers,
                                errorMessage = state.errorMessage,
                                availableUpdate = state.availableUpdate,
                                updateDownloading = state.updatePhase == UpdaterPhase.DOWNLOADING,
                                updateDownloadProgress = state.updateDownloadProgress,
                                updateAvailableVersion = if (state.updatePhase == UpdaterPhase.AVAILABLE) state.availableUpdate?.version else null,
                                onDismissUpdate = root.viewModel::dismissUpdateDialog,
                                fileListView = {
                                    FileListScreen(
                                        browser = state.browser,
                                        fileStatuses = state.fileStatuses,
                                        thumbnails = state.thumbnails,
                                        mountConfigured = state.config.mountConfigured,
                                        isIndexing = state.sync.isIndexing,
                                        onSort = root.viewModel::sort,
                                        onEnterFolder = root.viewModel::enterFolder,
                                        onOpenItem = root.viewModel::openItem,
                                        onThumbnailNeeded = root.viewModel::loadThumbnail,
                                        onDelete = root.viewModel::deleteItems,
                                        onPreviewFreeUp = root.viewModel::previewFreeUpItems,
                                        onFreeUp = root.viewModel::freeUpItems,
                                        onDownload = { files -> root.viewModel.downloadItems(files) },
                                        onSyncFolder = root.viewModel::syncFolder,
                                        onRename = { file, newName -> root.viewModel.renameItem(file, newName) },
                                        onMove = root.viewModel::moveItem,
                                        onCanFreeUp = root.viewModel::canFreeUp,
                                    )
                                },
                                onSearch = root.viewModel::search,
                                onNavigate = root.viewModel::enterFolderFromTree,
                                onNavigateCrumb = root.viewModel::navigateTo,
                                onEnterFolder = root.viewModel::enterFolder,
                                onOpenItem = root.viewModel::openItem,
                                onRefresh = root.viewModel::refresh,
                                onOpenFinder = root.viewModel::openMountInFinder,
                                onOpenSettings = root.viewModel::openSettings,
                                onRetryTransfer = root.viewModel::retryTransferWithResult,
                                onClearFinishedTransfers = root.viewModel::clearFinishedTransfers,
                                onClearCompletedTransfers = root.viewModel::clearCompletedTransfers,
                                onClearFailedTransfers = root.viewModel::clearFailedTransfers,
                                onSelectDir = {
                                    root.viewModel.chooseMountDirectory { selected ->
                                        val errors = root.viewModel.saveConfig(
                                            state.config.copy(mountDir = selected, mountConfigured = true),
                                        )
                                        if (errors.isNotEmpty()) {
                                            root.viewModel.showError(errors.joinToString("；"))
                                        }
                                    }
                                },
                                onFirstSync = root.viewModel::refresh,
                                onRetrySetup = root.viewModel::refresh,
                                onInstallUpdate = root.viewModel::installUpdate,
                                treeLoadingIds = state.browser.treeLoadingIds,
                                onExpandNode = root.viewModel::loadTreeChildren,
                                onShowUpdate = root.viewModel::showUpdateDialog,
                            )
                        }
                        // 全局更新对话框（覆盖所有页面，对标原 Vue App.vue 顶层 <UpdateDialog />）
                        UpdateDialogScreen(
                            phase = if (state.updateDialogDismissed) UpdaterPhase.IDLE else state.updatePhase,
                            manifest = state.availableUpdate,
                            downloadProgress = state.updateDownloadProgress,
                            errorMessage = state.errorMessage,
                            hasActiveTransfers = state.sync.hasActiveTransfer,
                            onStartUpdate = root.viewModel::installUpdate,
                            onRelaunch = root.viewModel::installUpdate,
                            onRetry = root.viewModel::installUpdate,
                            onDismiss = root.viewModel::dismissUpdateDialog,
                        )
                        // 下载进度遮罩（对标原 Vue「下载中」MateDialog）
                        state.downloadProgressText?.let { text ->
                            androidx.compose.ui.window.Dialog(onDismissRequest = {}) {
                                Surface(
                                    shape = androidx.compose.foundation.shape.RoundedCornerShape(10.dp),
                                    elevation = 8.dp,
                                ) {
                                    androidx.compose.foundation.layout.Row(
                                        modifier = Modifier.padding(20.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        CircularProgressIndicator(
                                            modifier = Modifier.size(24.dp),
                                            strokeWidth = 2.dp,
                                        )
                                        androidx.compose.foundation.layout.Spacer(Modifier.size(12.dp))
                                        androidx.compose.material.Text(text)
                                    }
                                }
                            }
                        }
                        // 全局对话框 / Toast 宿主
                        io.github.yuanbaobaoo.petallink.ui.components.mate.MateDialogHost()
                        io.github.yuanbaobaoo.petallink.ui.components.mate.MateToastHost()
                        }
                    }
                }
            }
        }
    } finally {
        instance.close()
    }
}
