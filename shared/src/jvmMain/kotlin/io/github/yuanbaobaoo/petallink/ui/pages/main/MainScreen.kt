@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSearchField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserViewModel
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SetupPhase
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncSnapshotUi
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi
import io.github.yuanbaobaoo.petallink.update.UpdateManifest

/**
 * 主界面（对标原 Vue MainPage.vue）。
 *
 * 左栏 Sidebar(220px) + 右侧主区（flex column）：
 * AppBar(56px) + info-area(SyncSetupBanner/SyncStatusBar/error) + Breadcrumb(32px) + 文件区 + TransferPopover 浮层。
 *
 * @param browser 文件浏览器状态（含目录树、面包屑、文件列表）
 * @param sync 同步快照
 * @param setupPhase 同步配置阶段
 * @param mountDir 挂载目录
 * @param userName 用户名
 * @param quotaText 配额文本
 * @param transfers 传输任务列表
 * @param errorMessage 错误消息
 * @param availableUpdate 可用更新
 * @param onSearch 搜索回调
 * @param onNavigate 面包屑/目录树导航
 * @param onEnterFolder 进入文件夹
 * @param onOpenItem 打开/下载文件
 * @param onRefresh 刷新（同步索引）
 * @param onOpenFinder 在 Finder 打开挂载目录
 * @param onOpenSettings 打开设置
 * @param onRetryTransfer 重试传输任务
 * @param onClearTransfers 清除传输历史
 * @param onSelectDir 选择同步目录（引导条）
 * @param onFirstSync 首次同步（引导条）
 * @param onInstallUpdate 安装更新
 */
@Composable
fun MainScreen(
    browser: io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserState,
    sync: SyncSnapshotUi,
    setupPhase: SetupPhase,
    mountDir: String,
    userName: String?,
    quotaText: String?,
    transfers: List<TransferTaskUi>,
    errorMessage: String?,
    availableUpdate: UpdateManifest?,
    fileListView: @Composable () -> Unit,
    onSearch: (String) -> Unit,
    onNavigate: (DriveFile) -> Unit,
    onNavigateCrumb: (BrowserBreadcrumb) -> Unit,
    onEnterFolder: (DriveFile) -> Unit,
    onOpenItem: (DriveFile) -> Unit,
    onRefresh: () -> Unit,
    onOpenFinder: () -> Unit,
    onOpenSettings: () -> Unit,
    onRetryTransfer: (Long) -> Unit,
    onClearFinishedTransfers: () -> Unit,
    onClearCompletedTransfers: () -> Unit,
    onClearFailedTransfers: () -> Unit,
    onSelectDir: () -> Unit,
    onFirstSync: () -> Unit,
    onRetrySetup: () -> Unit,
    onInstallUpdate: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    val mountConfigured = setupPhase == SetupPhase.ACTIVE || setupPhase == SetupPhase.NEEDS_FIRST_SYNC
    var showTransferPopover by remember { mutableStateOf(false) }
    var searchKeyword by remember { mutableStateOf("") }

    Row(modifier = Modifier.fillMaxSize()) {
        // 左栏 Sidebar
        Sidebar(
            rootChildren = browser.directoryChildren[FileBrowserViewModel.ROOT_KEY].orEmpty(),
            directoryChildren = browser.directoryChildren,
            selectedFolderId = browser.folderId,
            userName = userName,
            quotaText = quotaText,
            onNavigate = onNavigate,
        )

        // 右侧主区
        Column(modifier = Modifier.fillMaxHeight().background(semantic.bgContainer)) {
            // AppBar 56px
            Row(
                modifier = Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                MateSearchField(
                    value = searchKeyword,
                    onValueChange = { searchKeyword = it; onSearch(it) },
                    onSubmit = { onSearch(searchKeyword) },
                    modifier = Modifier.weight(1f),
                    placeholder = "搜索当前目录",
                )
                if (searchKeyword.isNotEmpty()) {
                    MateButton(
                        variant = MateButtonVariant.ICON,
                        icon = "x",
                        onClick = { searchKeyword = ""; onSearch("") },
                    )
                }
                Spacer(Modifier.width(4.dp))
                // 分隔线
                Box(Modifier.width(1.dp).height(24.dp).background(semantic.border))
                Spacer(Modifier.width(4.dp))
                if (mountConfigured) {
                    MateButton(
                        label = "同步索引",
                        variant = MateButtonVariant.ICON_TEXT,
                        icon = "refresh",
                        onClick = onRefresh,
                        loading = sync.isIndexing,
                        disabled = sync.isIndexing,
                    )
                }
                MateButton(
                    label = "传输队列",
                    variant = MateButtonVariant.ICON_TEXT,
                    icon = "transfer",
                    onClick = { showTransferPopover = !showTransferPopover },
                )
                if (mountConfigured) {
                    MateButton(
                        label = "Finder",
                        variant = MateButtonVariant.ICON_TEXT,
                        icon = "folder-open",
                        onClick = onOpenFinder,
                    )
                }
                MateButton(variant = MateButtonVariant.ICON, icon = "settings", onClick = onOpenSettings)
            }

            // 信息/错误提示区
            if (!mountConfigured || setupPhase == SetupPhase.NEEDS_FIRST_SYNC) {
                SyncSetupBanner(setupPhase, mountDir, errorMessage, onSelectDir, onFirstSync, onRetrySetup)
            } else if (mountConfigured) {
                SyncStatusBar(sync)
            }
            if (errorMessage != null && mountConfigured && setupPhase == SetupPhase.ACTIVE) {
                MateInfoBanner(message = errorMessage)
            }

            // 面包屑
            Breadcrumb(browser.breadcrumbs, onNavigateCrumb)

            // 文件区
            Box(modifier = Modifier.fillMaxSize()) {
                fileListView()
                if (browser.loading) {
                    Box(
                        modifier = Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.6f)),
                        contentAlignment = Alignment.Center,
                    ) { MateCircularProgress(size = 24.dp) }
                }
                if (showTransferPopover) {
                    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0f))) {
                        TransferPopoverScreen(
                            tasks = transfers,
                            onDismiss = { showTransferPopover = false },
                            onRetry = onRetryTransfer,
                            onClearCompleted = onClearCompletedTransfers,
                            onClearFailed = onClearFailedTransfers,
                            onClearFinished = onClearFinishedTransfers,
                        )
                    }
                }
            }
        }
    }
}
