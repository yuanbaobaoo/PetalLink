package io.github.yuanbaobaao.petallink

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.CircularProgressIndicator
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Menu
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
import io.github.yuanbaobaao.petallink.app.AppPage
import io.github.yuanbaobaao.petallink.app.ApplicationRoot
import io.github.yuanbaobaao.petallink.core.net_guard.NetState
import io.github.yuanbaobaao.petallink.core.AppPaths
import io.github.yuanbaobaao.petallink.platform.MacActivationPolicy
import io.github.yuanbaobaao.petallink.platform.SingleInstanceCoordinator
import io.github.yuanbaobaao.petallink.ui.pages.LoginScreen
import io.github.yuanbaobaao.petallink.ui.pages.LogViewerScreen
import io.github.yuanbaobaao.petallink.ui.pages.MainScreen
import io.github.yuanbaobaao.petallink.ui.pages.SettingsScreen
import io.github.yuanbaobaao.petallink.ui.theme.PetalLinkTheme
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
            val trayIcon = rememberVectorPainter(Icons.Default.Menu)
            var trayTransfers by remember { mutableStateOf(state.transfers) }
            var lastTrayRebuild by remember { mutableStateOf(0L) }
            val transferSignature = state.transfers.joinToString("|") { "${it.fileName}:${it.stateText}:${it.progress}" }

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
                                "${if (transfer.direction == "upload") "上传" else "下载"} ${(transfer.progress * 100).toInt()}% · ${transfer.stateText}",
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
                                userName = state.userName,
                                appVersion = state.appVersion,
                                availableUpdate = state.availableUpdate,
                                updateStatus = state.updateStatus,
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
                                onReload = root.viewModel::reloadLogs,
                                onExport = root.viewModel::exportLogs,
                                onClear = root.viewModel::clearLogs,
                            )
                            else -> MainScreen(
                                syncStatus = state.errorMessage ?: state.syncStatus,
                                isOnline = state.netState == NetState.ONLINE,
                                browser = state.browser,
                                thumbnails = state.thumbnails,
                                fileStatuses = state.fileStatuses,
                                quotaText = state.quotaText,
                                transferItems = state.transfers,
                                availableUpdate = state.availableUpdate,
                                onSearch = root.viewModel::search,
                                onSort = root.viewModel::sort,
                                onNavigate = root.viewModel::navigateTo,
                                onEnterFolder = root.viewModel::enterFolder,
                                onOpenItem = root.viewModel::openItem,
                                onThumbnailNeeded = root.viewModel::loadThumbnail,
                                onCanFreeUp = root.viewModel::canFreeUp,
                                onDelete = root.viewModel::deleteItems,
                                onFreeUp = root.viewModel::freeUpItems,
                                onCreateFolder = root.viewModel::createFolder,
                                onRename = root.viewModel::renameItem,
                                onMove = root.viewModel::moveItem,
                                onSyncFolder = root.viewModel::syncFolder,
                                onUpload = root.viewModel::chooseAndUpload,
                                onOpenFinder = root.viewModel::openMountInFinder,
                                onLoadQuota = root.viewModel::loadQuota,
                                onLoadMore = root.viewModel::loadMore,
                                onRetryTransfer = root.viewModel::retryTransfer,
                                onClearTransfers = root.viewModel::clearFinishedTransfers,
                                onInstallUpdate = root.viewModel::installUpdate,
                                onRefresh = root.viewModel::refresh,
                                onOpenSettings = root.viewModel::openSettings,
                                onOpenLogs = root.viewModel::openLogs,
                            )
                        }
                    }
                }
            }
        }
    } finally {
        instance.close()
    }
}
