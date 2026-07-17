@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSearchField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateVDivider
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
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
 * 左栏 Sidebar(220px) + 右侧主区：
 * AppBar(56px) + info-area + Breadcrumb(32px+底分隔线) + 文件区（搜索结果 / 文件列表）+ TransferPopover 浮层。
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
    // 更新下载进度（侧边栏更新横幅用）
    updateDownloading: Boolean,
    updateDownloadProgress: Float,
    updateAvailableVersion: String?,
    onDismissUpdate: () -> Unit,
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
    // 搜索关键词：仅回车提交才触发远端搜索；输入过程只更新本地显示
    var searchKeyword by remember { mutableStateOf("") }
    val isSearching = remember(searchKeyword) { mutableStateOf(false) }
    val searchResults = remember { mutableStateOf<List<DriveFile>>(emptyList()) }

    Row(modifier = Modifier.fillMaxSize()) {
        // 左栏 Sidebar
        Sidebar(
            rootChildren = browser.directoryChildren[FileBrowserViewModel.ROOT_KEY].orEmpty(),
            directoryChildren = browser.directoryChildren,
            selectedFolderId = browser.folderId,
            userName = userName,
            quotaText = quotaText,
            updateDownloading = updateDownloading,
            updateDownloadProgress = updateDownloadProgress,
            updateAvailableVersion = updateAvailableVersion,
            onDismissUpdate = onDismissUpdate,
            onNavigate = onNavigate,
        )

        // 右侧主区
        Column(modifier = Modifier.fillMaxHeight().background(semantic.bgContainer)) {
            // AppBar 56px
            Row(
                modifier = Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // 左侧：搜索框 + 清除按钮（flex:1）
                MateSearchField(
                    value = searchKeyword,
                    onValueChange = { searchKeyword = it },
                    onSubmit = {
                        // 仅回车提交才触发搜索（对标原 Vue @submit）
                        if (searchKeyword.isNotBlank()) {
                            isSearching.value = true
                            onSearch(searchKeyword)
                        }
                    },
                    modifier = Modifier.weight(1f),
                    placeholder = "搜索文件和文件夹...",
                )
                if (searchKeyword.isNotEmpty()) {
                    MateButton(
                        variant = MateButtonVariant.ICON,
                        icon = "x",
                        onClick = {
                            searchKeyword = ""
                            searchResults.value = emptyList()
                            onSearch("")
                        },
                    )
                }
                Spacer(Modifier.width(4.dp))
                // 分隔线（app-bar__sep，1px×24px）
                MateVDivider(height = 24.dp)
                Spacer(Modifier.width(4.dp))
                // 工具组（gap 4px）
                if (mountConfigured) {
                    MateButton(
                        label = "同步索引",
                        variant = MateButtonVariant.ICON_TEXT,
                        icon = "refresh",
                        onClick = onRefresh,
                        loading = sync.isIndexing,
                        disabled = sync.isIndexing,
                    )
                    Spacer(Modifier.width(4.dp))
                }
                MateButton(
                    label = "传输队列",
                    variant = MateButtonVariant.ICON_TEXT,
                    icon = "transfer",
                    onClick = { showTransferPopover = !showTransferPopover },
                )
                if (mountConfigured) {
                    Spacer(Modifier.width(4.dp))
                    MateButton(
                        label = "Finder",
                        variant = MateButtonVariant.ICON_TEXT,
                        icon = "folder-open",
                        onClick = onOpenFinder,
                    )
                }
                Spacer(Modifier.width(4.dp))
                MateButton(variant = MateButtonVariant.ICON, icon = "settings", onClick = onOpenSettings)
            }
            // AppBar 底分隔线
            MateHDivider()

            // info-area
            if (!mountConfigured || setupPhase == SetupPhase.NEEDS_FIRST_SYNC) {
                SyncSetupBanner(setupPhase, mountDir, errorMessage, onSelectDir, onFirstSync, onRetrySetup)
            } else if (mountConfigured) {
                SyncStatusBar(sync)
            }

            // 面包屑（含底分隔线）
            Breadcrumb(browser.breadcrumbs, onNavigateCrumb)

            // 文件区
            Box(modifier = Modifier.fillMaxSize()) {
                if (searchKeyword.isNotEmpty()) {
                    // 搜索结果区（对标原 Vue .search-results）
                    SearchResults(
                        keyword = searchKeyword,
                        results = searchResults.value,
                        searching = isSearching.value,
                        onEnterFolder = { file ->
                            onEnterFolder(file)
                            searchKeyword = ""
                            searchResults.value = emptyList()
                        },
                    )
                } else {
                    fileListView()
                }
                if (browser.loading) {
                    Box(
                        modifier = Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.6f)),
                        contentAlignment = Alignment.Center,
                    ) { MateCircularProgress(size = 24.dp) }
                }
                // TransferPopover 浮层（贴 AppBar 下右侧，点击外部关闭）
                if (showTransferPopover) {
                    Box(
                        modifier = Modifier.fillMaxSize().clickable(
                            interactionSource = remember { MutableInteractionSource() },
                            indication = null,
                        ) { showTransferPopover = false },
                    )
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.TopEnd) {
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

/** 搜索结果区（对标原 Vue .search-results）。 */
@Composable
private fun SearchResults(
    keyword: String,
    results: List<DriveFile>,
    searching: Boolean,
    onEnterFolder: (DriveFile) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxSize()) {
        // header
        Box(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(
                if (searching) "搜索中…" else "搜索：$keyword",
                fontSize = 13.sp,
                fontWeight = androidx.compose.ui.text.font.FontWeight.Medium,
                color = semantic.textSecondary,
            )
        }
        if (results.isEmpty() && !searching) {
            MateEmpty(title = "无匹配结果", icon = "search", description = "试试其他关键词")
        } else {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(results, key = { it.id ?: it.name ?: "" }) { file ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(
                                interactionSource = remember { MutableInteractionSource() },
                                indication = null,
                            ) { if (file.isFolder()) onEnterFolder(file) }
                            .padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        MateIcon(
                            name = if (file.isFolder()) "folder" else "file",
                            size = 20.dp,
                            tint = if (file.isFolder()) BrandColor else semantic.textPlaceholder,
                        )
                        Column {
                            Text(
                                file.name ?: file.fileName ?: "未命名",
                                fontSize = 14.sp,
                                color = semantic.textPrimary,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                if (file.isFolder()) "文件夹" else "${file.sizeBytes} 字节",
                                fontSize = 13.sp,
                                color = semantic.textSecondary,
                            )
                        }
                    }
                }
            }
        }
    }
}
