package io.github.yuanbaobaoo.petallink

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.CircularProgressIndicator
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Menu
import androidx.compose.runtime.remember
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.painter.BitmapPainter
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.material.Surface
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.window.Tray
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.rememberWindowState
import androidx.compose.ui.window.application
import io.github.yuanbaobaoo.petallink.app.AppPage
import io.github.yuanbaobaoo.petallink.app.ApplicationRoot
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.platform.MacActivationPolicy
import io.github.yuanbaobaoo.petallink.platform.SingleInstanceCoordinator
import io.github.yuanbaobaoo.petallink.ui.pages.main.LoginScreen
import io.github.yuanbaobaoo.petallink.ui.pages.main.LogViewerScreen
import io.github.yuanbaobaoo.petallink.ui.pages.main.MainScreen
import io.github.yuanbaobaoo.petallink.ui.pages.main.SettingsScreen
import io.github.yuanbaobaoo.petallink.ui.pages.main.FileListScreen
import io.github.yuanbaobaoo.petallink.ui.pages.main.UpdateDialogScreen
import io.github.yuanbaobaoo.petallink.ui.viewmodel.UpdaterPhase
import io.github.yuanbaobaoo.petallink.ui.theme.PetalLinkTheme
import java.awt.Desktop
import java.awt.event.WindowAdapter
import java.awt.event.WindowEvent
import java.util.concurrent.atomic.AtomicReference
import javax.swing.SwingUtilities
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay

fun main(args: Array<String>) {
    val paths = AppPaths.fromEnvironment()
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
            val trayIcon = rememberTrayIcon()
            var trayTransfers by remember { mutableStateOf(state.transfers) }
            var lastTrayRebuild by remember { mutableStateOf(0L) }
            val transferSignature = state.transfers.joinToString("|") { "${it.fileName}:${it.state}:${it.progress}" }

            fun show() {
                visible = true
                MacActivationPolicy.regularAndActivate()
            }

            fun hide() {
                visible = false
                MacActivationPolicy.accessory()
            }

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
                        if (exiting || MacActivationPolicy.isSystemQuitAppleEvent()) {
                            exiting = true
                            root.close()
                            instance.close()
                            response.performQuit()
                        } else {
                            response.cancelQuit()
                            SwingUtilities.invokeLater(::hide)
                        }
                    }
                }
                onDispose { showWindow.set({}) }
            }

            LaunchedEffect(transferSignature) {
                val now = System.currentTimeMillis()
                val remaining = (5_000L - (now - lastTrayRebuild)).coerceAtLeast(0L)
                if (remaining > 0) delay(remaining)
                trayTransfers = state.transfers
                lastTrayRebuild = System.currentTimeMillis()
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

            Tray(
                icon = trayIcon,
                tooltip = "PetalLink · ${state.syncStatus}",
                onAction = ::show,
                menu = {
                    Item("显示 PetalLink", onClick = ::show)
                    Item("立即刷新", onClick = root.viewModel::refresh, enabled = state.loggedIn)
                    if (trayTransfers.isNotEmpty()) {
                        Separator()
                        trayTransfers.forEach { transfer ->
                            Item(transfer.fileName.take(20), onClick = {}, enabled = false)
                            Item(
                                "${if (transfer.direction == "upload") "上传" else "下载"} ${(transfer.progress * 100).toInt()}% · ${transfer.state}",
                                onClick = {}, enabled = false,
                            )
                        }
                    }
                    Separator()
                    Item("退出", onClick = ::quit)
                },
            )

            Window(
                visible = visible,
                state = windowState,
                title = "PetalLink - 华为云盘客户端开源版",
                onCloseRequest = ::hide,
            ) {
                DisposableEffect(window) {
                    val listener = object : WindowAdapter() {
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
                            )
                            state.page == AppPage.SETTINGS -> SettingsScreen(
                                initialConfig = state.config,
                                launchAtLogin = state.launchAtLogin,
                                userInfo = state.userInfo,
                                appVersion = state.appVersion,
                                quotaUsed = null,
                                quotaTotal = null,
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
                                onSave = root.viewModel::saveConfig,
                            )
                            state.page == AppPage.LOGS -> LogViewerScreen(
                                records = state.logs,
                                onBack = root.viewModel::openFiles,
                                onExport = root.viewModel::exportLogs,
                                onClear = root.viewModel::clearLogs,
                            )
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
                                        onFreeUp = root.viewModel::freeUpItems,
                                        onDownload = { files -> files.forEach(root.viewModel::openItem) },
                                        onSyncFolder = root.viewModel::syncFolder,
                                        onRename = { file, newName -> root.viewModel.renameItem(file, newName) },
                                        onShowProps = {},
                                        onCanFreeUp = root.viewModel::canFreeUp,
                                    )
                                },
                                onSearch = root.viewModel::search,
                                onNavigate = root.viewModel::enterFolder,
                                onNavigateCrumb = root.viewModel::navigateTo,
                                onEnterFolder = root.viewModel::enterFolder,
                                onOpenItem = root.viewModel::openItem,
                                onRefresh = root.viewModel::refresh,
                                onOpenFinder = root.viewModel::openMountInFinder,
                                onOpenSettings = root.viewModel::openSettings,
                                onRetryTransfer = root.viewModel::retryTransfer,
                                onClearFinishedTransfers = root.viewModel::clearFinishedTransfers,
                                onClearCompletedTransfers = root.viewModel::clearFinishedTransfers,
                                onClearFailedTransfers = root.viewModel::clearFinishedTransfers,
                                onSelectDir = {},
                                onFirstSync = root.viewModel::refresh,
                                onRetrySetup = root.viewModel::refresh,
                                onInstallUpdate = root.viewModel::installUpdate,
                            )
                        }
                        // 全局更新对话框（覆盖所有页面，对标原 Vue App.vue 顶层 <UpdateDialog />）
                        UpdateDialogScreen(
                            phase = state.updatePhase,
                            manifest = state.availableUpdate,
                            downloadProgress = state.updateDownloadProgress,
                            errorMessage = state.errorMessage,
                            hasActiveTransfers = state.sync.hasActiveTransfer,
                            onStartUpdate = root.viewModel::installUpdate,
                            onRelaunch = root.viewModel::installUpdate,
                            onRetry = root.viewModel::installUpdate,
                            onDismiss = root.viewModel::dismissUpdateDialog,
                        )
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

/**
 * 加载托盘图标 Painter（对标原 Tauri menubar-icon.png）。
 *
 * 从 resources/assets/menubar-icon.png 加载真实图标；加载失败回退 Material Menu 图标，
 * 保证托盘始终可用。
 */
@Composable
private fun rememberTrayIcon(): Painter {
    val pngIcon = remember {
        runCatching {
            val loader = Thread.currentThread().contextClassLoader ?: ClassLoader.getSystemClassLoader()
            loader.getResourceAsStream("assets/menubar-icon.png")?.use { it.readAllBytes() }
        }.getOrNull()
    }
    return if (pngIcon != null) {
        remember(pngIcon) {
            BitmapPainter(org.jetbrains.skia.Image.makeFromEncoded(pngIcon).toComposeImageBitmap())
        }
    } else {
        rememberVectorPainter(Icons.Default.Menu)
    }
}
