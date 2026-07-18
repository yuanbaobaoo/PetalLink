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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.displayName
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSearchField
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserViewModel
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SetupPhase
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncSnapshotUi
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi
import io.github.yuanbaobaoo.petallink.update.UpdateManifest

/**
 * 主界面（对标原 Vue MainPage.vue）。
 *
 * 左栏 Sidebar(248px) + 右侧主区：
 * AppBar(64px，v2 .toolbar) + info-area + Breadcrumb(40px+底分隔线) + 文件区（搜索结果 / 文件列表）+ TransferPopover 浮层。
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
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val mountConfigured = setupPhase == SetupPhase.ACTIVE || setupPhase == SetupPhase.NEEDS_FIRST_SYNC
    var showTransferPopover by remember { mutableStateOf(false) }
    // 搜索关键词：仅回车提交才触发远端搜索；输入过程只更新本地显示
    var searchKeyword by remember { mutableStateOf("") }
    var submittedSearch by remember { mutableStateOf("") }

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
            onInstallUpdate = onInstallUpdate,
            onNavigate = onNavigate,
        )

        // 右侧主区
        Column(modifier = Modifier.fillMaxHeight().background(semantic.bgContainer)) {
            // AppBar 64px（v2 .toolbar：h64，padding 0 20px，gap 8px）
            Row(
                modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.mainPage.appBarHeight)
                    .padding(horizontal = PetalTheme.metrics.mainPage.appBarHorizontalPadding),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // 左侧：搜索框 + 清除按钮（v2：flex:1，max-width 420px；weight fill=false 让剩余空间留在按钮前）
                MateSearchField(
                    value = searchKeyword,
                    onValueChange = { searchKeyword = it },
                    onSubmit = {
                        // 仅回车提交才触发搜索（对标原 Vue @submit）
                        if (searchKeyword.isNotBlank()) {
                            submittedSearch = searchKeyword
                            onSearch(searchKeyword)
                        }
                    },
                    modifier = Modifier.weight(1f, fill = false).widthIn(max = PetalTheme.metrics.mainPage.searchMaximumWidth),
                    placeholder = "搜索文件和文件夹...",
                )
                if (searchKeyword.isNotEmpty()) {
                    MateButton(
                        variant = MateButtonVariant.ICON,
                        icon = "x",
                        onClick = {
                            searchKeyword = ""
                            submittedSearch = ""
                            onSearch("")
                        },
                    )
                }
                Spacer(Modifier.weight(1f))
                // 工具组（v2：无分隔线，按钮直接排，gap 8px；整体右对齐——weight(1f) 弹簧把按钮组推到右侧）
                if (mountConfigured) {
                    // 「同步索引」：v2 主按钮（PRIMARY 品牌渐变）
                    MateButton(
                        label = "同步索引",
                        variant = MateButtonVariant.PRIMARY,
                        icon = "refresh",
                        onClick = onRefresh,
                        loading = sync.isIndexing,
                        disabled = sync.isIndexing,
                    )
                    Spacer(Modifier.width(PetalTheme.metrics.mainPage.appBarActionSpacing))
                }
                // 「传输队列」：v2 软色按钮（SOFT 浅蓝底）
                MateButton(
                    label = "传输队列",
                    variant = MateButtonVariant.SOFT,
                    icon = "transfer",
                    onClick = { showTransferPopover = !showTransferPopover },
                )
                if (mountConfigured) {
                    Spacer(Modifier.width(PetalTheme.metrics.mainPage.appBarActionSpacing))
                    MateButton(
                        label = "Finder",
                        variant = MateButtonVariant.ICON_TEXT,
                        icon = "folder-open",
                        onClick = onOpenFinder,
                    )
                }
                Spacer(Modifier.width(PetalTheme.metrics.mainPage.appBarActionSpacing))
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
                if (submittedSearch.isNotEmpty()) {
                    // 搜索结果区（对标原 Vue .search-results）
                    SearchResults(
                        keyword = submittedSearch,
                        results = browser.visibleFiles,
                        searching = browser.loading,
                        onEnterFolder = { file ->
                            onEnterFolder(file)
                            searchKeyword = ""
                            submittedSearch = ""
                        },
                    )
                } else {
                    fileListView()
                }
                if (browser.loading) {
                    Box(
                        modifier = Modifier.fillMaxSize().background(PetalTheme.colors.mainLoadingScrim),
                        contentAlignment = Alignment.Center,
                    ) { MateCircularProgress(size = PetalTheme.metrics.mainPage.loadingSize) }
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

/**
 * 搜索结果区（v2：对标 02-main.html 场景 3，header + 56px 结果行 + 32px 色块 tile）。
 */
@Composable
private fun SearchResults(
    keyword: String,
    results: List<DriveFile>,
    searching: Boolean,
    onEnterFolder: (DriveFile) -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Column(modifier = Modifier.fillMaxSize()) {
        // header（v2：13.5sp semibold textSecondary，padding 14/12/10）
        Box(
            modifier = Modifier.fillMaxWidth().padding(
                start = PetalTheme.metrics.mainPage.searchPanelStartPadding,
                top = PetalTheme.metrics.mainPage.searchPanelTopPadding,
                end = PetalTheme.metrics.mainPage.searchPanelEndPadding,
                bottom = PetalTheme.metrics.mainPage.searchPanelBottomPadding,
            ),
        ) {
            Text(
                if (searching) "搜索中…" else "搜索：$keyword",
                style = PetalTheme.typography.main.searchHeader,
                color = semantic.textSecondary,
            )
        }
        if (results.isEmpty() && !searching) {
            MateEmpty(title = "无匹配结果", icon = "search", description = "试试其他关键词")
        } else {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(results, key = { it.id ?: it.name ?: "" }) { file ->
                    // 结果行（v2 .file-row：h56，padding 0 12px，gap 12px）
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(PetalTheme.metrics.mainPage.searchResultHeight)
                            .clickable(
                                interactionSource = remember { MutableInteractionSource() },
                                indication = null,
                            ) { if (file.isFolder()) onEnterFolder(file) }
                            .padding(horizontal = PetalTheme.metrics.mainPage.searchResultHorizontalPadding),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.mainPage.searchResultContentSpacing),
                    ) {
                        // 32×32 radius 6 色块 tile（v2 .ftile：文件夹 PetalTheme.colors.folderBg/PetalTheme.colors.folder，文件 bgFill/textSecondary）
                        Box(
                            modifier = Modifier
                                .size(PetalTheme.metrics.mainPage.searchResultIconContainerSize)
                                .clip(RoundedCornerShape(PetalTheme.metrics.mainPage.searchResultIconRadius))
                                .background(if (file.isFolder()) PetalTheme.colors.folderBg else semantic.bgFill),
                            contentAlignment = Alignment.Center,
                        ) {
                            MateIcon(
                                name = if (file.isFolder()) "folder" else "file",
                                size = PetalTheme.metrics.mainPage.searchResultIconSize,
                                tint = if (file.isFolder()) PetalTheme.colors.folder else semantic.textSecondary,
                            )
                        }
                        Column {
                            Text(
                                file.displayName(),
                                style = PetalTheme.typography.main.searchResultName,
                                color = semantic.textPrimary,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                if (file.isFolder()) "文件夹" else "${file.sizeBytes} 字节",
                                style = PetalTheme.typography.main.searchResultDescription,
                                color = semantic.textSecondary,
                            )
                        }
                    }
                }
            }
        }
    }
}
