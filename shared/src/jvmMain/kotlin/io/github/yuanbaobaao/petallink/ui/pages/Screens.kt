package io.github.yuanbaobaao.petallink.ui.pages

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.onPointerEvent
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaao.petallink.config.UserConfig
import io.github.yuanbaobaao.petallink.config.ConfigValidator
import io.github.yuanbaobaao.petallink.config.DEFAULT_CALLBACK_PORT
import io.github.yuanbaobaao.petallink.core.logging.LogLevel
import io.github.yuanbaobaao.petallink.drive.DriveFile
import io.github.yuanbaobaao.petallink.sync.isFolder
import io.github.yuanbaobaao.petallink.ui.components.*
import io.github.yuanbaobaao.petallink.ui.theme.*
import io.github.yuanbaobaao.petallink.ui.viewmodel.BrowserBreadcrumb
import io.github.yuanbaobaao.petallink.ui.viewmodel.BrowserSortField
import io.github.yuanbaobaao.petallink.ui.viewmodel.FileBrowserState
import io.github.yuanbaobaao.petallink.ui.viewmodel.FileBrowserViewModel
import io.github.yuanbaobaao.petallink.update.UpdateManifest
import org.jetbrains.skia.Image as SkiaImage

// === LoginScreen ===
@Composable
fun LoginScreen(
    loggingIn: Boolean,
    secretConfigured: Boolean,
    errorMessage: String?,
    onLogin: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxSize().padding(48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text("PetalLink", fontSize = 32.sp, fontWeight = FontWeight.Bold, color = BrandColor)
        Spacer(Modifier.height(8.dp))
        Text("华为云盘 macOS 客户端", fontSize = 14.sp, color = Color.Gray)
        Spacer(Modifier.height(24.dp))
        Text(
            errorMessage ?: when {
                !secretConfigured -> "未配置 OAuth client secret，请先完成应用配置"
                loggingIn -> "正在浏览器中完成授权…"
                else -> "点击下方按钮登录华为账号"
            },
            fontSize = 13.sp,
            color = if (errorMessage == null) Color(0xFF666666) else ErrorColor,
        )
        Spacer(Modifier.height(16.dp))
        MateButton(
            text = if (loggingIn) "登录中..." else "登录华为账号",
            onClick = onLogin,
            enabled = !loggingIn && secretConfigured,
        )
        if (loggingIn) TextButton(onClick = onCancel) { Text("取消登录") }
    }
}

// === MainScreen ===
@Composable
fun MainScreen(
    syncStatus: String,
    isOnline: Boolean,
    browser: FileBrowserState,
    thumbnails: Map<String, ByteArray>,
    fileStatuses: Map<String, String>,
    quotaText: String?,
    transferItems: List<MateTransferItemData>,
    availableUpdate: UpdateManifest?,
    onSearch: (String) -> Unit,
    onSort: (BrowserSortField) -> Unit,
    onNavigate: (BrowserBreadcrumb) -> Unit,
    onEnterFolder: (DriveFile) -> Unit,
    onOpenItem: (DriveFile) -> Unit,
    onThumbnailNeeded: (DriveFile) -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
    onDelete: (List<DriveFile>) -> Unit,
    onFreeUp: (List<DriveFile>) -> Unit,
    onCreateFolder: (String) -> Unit,
    onRename: (DriveFile, String) -> Unit,
    onMove: (DriveFile, String) -> Unit,
    onSyncFolder: (DriveFile) -> Unit,
    onUpload: () -> Unit,
    onOpenFinder: () -> Unit,
    onLoadQuota: () -> Unit,
    onLoadMore: () -> Unit,
    onRetryTransfer: (Long) -> Unit,
    onClearTransfers: () -> Unit,
    onInstallUpdate: () -> Unit,
    onRefresh: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenLogs: () -> Unit,
) {
    var selected by remember(browser.folderId) { mutableStateOf<Set<String>>(emptySet()) }
    var pendingDelete by remember { mutableStateOf<List<DriveFile>>(emptyList()) }
    var pendingFreeUp by remember { mutableStateOf<List<DriveFile>>(emptyList()) }
    var showCreate by remember { mutableStateOf(false) }
    var createName by remember { mutableStateOf("") }
    var renameTarget by remember { mutableStateOf<DriveFile?>(null) }
    var renameName by remember { mutableStateOf("") }
    var moveTarget by remember { mutableStateOf<DriveFile?>(null) }
    var moveParentId by remember { mutableStateOf("") }
    var showUpdate by remember(availableUpdate?.version) { mutableStateOf(false) }
    val visible = browser.visibleFiles

    MateDialog(
        visible = pendingDelete.isNotEmpty(), title = "确认双端删除",
        content = "将删除选中的 ${pendingDelete.size} 项；已同步内容会同时从云端删除，此操作不可撤销。",
        confirmText = "删除", onConfirm = { onDelete(pendingDelete); pendingDelete = emptyList(); selected = emptySet() },
        onDismiss = { pendingDelete = emptyList() },
    )
    availableUpdate?.let { update ->
        MateDialog(
            visible = showUpdate,
            title = "发现新版本 ${update.version}",
            content = update.notes.ifBlank { "更新包会在传输任务空闲后下载、校验并安装。" },
            confirmText = "更新并重启",
            onConfirm = { showUpdate = false; onInstallUpdate() },
            onDismiss = { showUpdate = false },
        )
    }
    if (showCreate) AlertDialog(
        onDismissRequest = { showCreate = false },
        title = { Text("新建文件夹") },
        text = { OutlinedTextField(createName, { createName = it }, label = { Text("名称") }, singleLine = true) },
        confirmButton = { TextButton(onClick = { if (createName.isNotBlank()) onCreateFolder(createName); showCreate = false; createName = "" }) { Text("创建") } },
        dismissButton = { TextButton(onClick = { showCreate = false }) { Text("取消") } },
    )
    renameTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { renameTarget = null },
            title = { Text("重命名") },
            text = { OutlinedTextField(renameName, { renameName = it }, label = { Text("新名称") }, singleLine = true) },
            confirmButton = { TextButton(onClick = { if (renameName.isNotBlank()) onRename(target, renameName); renameTarget = null }) { Text("保存") } },
            dismissButton = { TextButton(onClick = { renameTarget = null }) { Text("取消") } },
        )
    }
    moveTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { moveTarget = null },
            title = { Text("移动到云端目录") },
            text = { OutlinedTextField(moveParentId, { moveParentId = it }, label = { Text("目标 parent fileId") }, singleLine = true) },
            confirmButton = { TextButton(onClick = { if (moveParentId.isNotBlank()) onMove(target, moveParentId); moveTarget = null }) { Text("移动") } },
            dismissButton = { TextButton(onClick = { moveTarget = null }) { Text("取消") } },
        )
    }
    MateDialog(
        visible = pendingFreeUp.isNotEmpty(), title = "确认释放空间",
        content = "将校验远端身份并把 ${pendingFreeUp.size} 项转换为按需下载占位符。",
        confirmText = "释放", onConfirm = { onFreeUp(pendingFreeUp); pendingFreeUp = emptyList(); selected = emptySet() },
        onDismiss = { pendingFreeUp = emptyList() },
    )

    Row(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier.width(200.dp).fillMaxHeight().padding(8.dp),
        ) {
            MateSectionHeader("PetalLink", subtitle = syncStatus)
            Spacer(Modifier.height(8.dp))
            Text(if (isOnline) "🟢 在线" else "🔴 离线", fontSize = 12.sp)
            Spacer(Modifier.height(16.dp))
            TextButton(onClick = { onNavigate(BrowserBreadcrumb(null, "全部文件")) }) { Text("📁 全部文件") }
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                BrowserFolderTree(
                    browser.directoryChildren[FileBrowserViewModel.ROOT_KEY].orEmpty(),
                    browser.directoryChildren,
                    browser.folderId,
                    0,
                    onEnterFolder,
                )
            }
            TextButton(onClick = onLoadQuota) { Text(quotaText?.let { "云盘 $it" } ?: "查看云盘容量", fontSize = 11.sp) }
            availableUpdate?.let { update ->
                TextButton(onClick = { showUpdate = true }) { Text("⬆ 更新到 ${update.version}", fontSize = 11.sp) }
            }
            TextButton(onClick = onOpenLogs) { Text("运行日志") }
            TextButton(onClick = onOpenSettings) { Text("设置") }
        }

        Column(modifier = Modifier.weight(1f).fillMaxHeight().padding(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) {
                    MateSearchField(browser.query, onSearch, "搜索当前目录")
                }
                TextButton(onClick = { showCreate = true }) { Text("新建文件夹") }
                TextButton(onClick = onUpload) { Text("上传") }
                TextButton(onClick = onOpenFinder) { Text("Finder") }
                TextButton(onClick = onRefresh) { Text("刷新") }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                browser.breadcrumbs.forEachIndexed { index, crumb ->
                    TextButton(onClick = { onNavigate(crumb) }) { Text(crumb.name, fontSize = 12.sp) }
                    if (index != browser.breadcrumbs.lastIndex) Text("/")
                }
                if (browser.loading) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
            }

            if (selected.isNotEmpty()) {
                val chosen = visible.filter { it.id in selected }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("已选择 ${selected.size} 项", fontSize = 12.sp, modifier = Modifier.weight(1f))
                    TextButton(onClick = { chosen.forEach(onOpenItem) }) { Text("下载") }
                    TextButton(onClick = { pendingFreeUp = chosen }) { Text("释放空间") }
                    TextButton(onClick = { pendingDelete = chosen }) { Text("删除", color = ErrorColor) }
                }
            }

            if (visible.isEmpty() && !browser.loading) {
                MateEmpty("📂", "暂无文件", "点击刷新加载文件列表")
            } else {
                Row(modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp)) {
                    Checkbox(
                        checked = visible.isNotEmpty() && visible.all { it.id in selected },
                        onCheckedChange = { all -> selected = if (all) visible.mapNotNull { it.id }.toSet() else emptySet() },
                        modifier = Modifier.width(40.dp),
                    )
                    HeaderButton("名称", Modifier.weight(1f)) { onSort(BrowserSortField.NAME) }
                    HeaderButton("大小", Modifier.width(100.dp)) { onSort(BrowserSortField.SIZE) }
                    HeaderButton("修改时间", Modifier.width(150.dp)) { onSort(BrowserSortField.MODIFIED_TIME) }
                    Text("状态", fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(64.dp))
                    Text("操作", fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(44.dp))
                }
                Divider()
                LazyColumn(modifier = Modifier.weight(1f)) {
                    items(visible, key = { it.id ?: "${it.name}-${it.editedTime}" }) { file ->
                        FileBrowserRow(
                            file = file,
                            thumbnail = file.id?.let(thumbnails::get),
                            checked = file.id in selected,
                            onChecked = { checked ->
                                file.id?.let { id -> selected = if (checked) selected + id else selected - id }
                            },
                            onOpen = { onOpenItem(file) },
                            onEnter = { onEnterFolder(file) },
                            onThumbnailNeeded = { onThumbnailNeeded(file) },
                            status = file.id?.let(fileStatuses::get) ?: if (file.isFolder()) "folder" else "not_synced",
                            onCanFreeUp = { callback -> onCanFreeUp(file, callback) },
                            onRename = { renameTarget = file; renameName = file.name ?: file.fileName.orEmpty() },
                            onMove = { moveTarget = file; moveParentId = "" },
                            onSyncFolder = { onSyncFolder(file) },
                            onDelete = { pendingDelete = listOf(file) },
                            onFreeUp = { pendingFreeUp = listOf(file) },
                        )
                    }
                    if (browser.nextCursor != null) {
                        item { TextButton(onClick = onLoadMore, modifier = Modifier.fillMaxWidth()) { Text("加载更多") } }
                    }
                }
            }

            if (transferItems.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                Divider()
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("传输任务 (${transferItems.size})", fontSize = 14.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(8.dp).weight(1f))
                    TextButton(onClick = onClearTransfers) { Text("清除已结束") }
                }
                transferItems.takeLast(5).forEach { item ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.weight(1f)) { MateTransferItem(item) }
                        if (item.stateText in setOf("Failed", "RestartRequired")) {
                            item.id?.let { id -> TextButton(onClick = { onRetryTransfer(id) }) { Text("重试") } }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun BrowserFolderTree(
    folders: List<DriveFile>,
    children: Map<String, List<DriveFile>>,
    selectedId: String?,
    indent: Int,
    onSelect: (DriveFile) -> Unit,
) {
    folders.forEach { folder ->
        val id = folder.id ?: return@forEach
        var expanded by remember(id) { mutableStateOf(id == selectedId || children[id]?.isNotEmpty() == true) }
        Row(
            Modifier.fillMaxWidth().clickable { expanded = true; onSelect(folder) }
                .background(if (id == selectedId) BrandColor.copy(alpha = 0.08f) else Color.Transparent)
                .padding(start = (8 + indent * 14).dp, top = 5.dp, bottom = 5.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(if (expanded) "▾" else "▸", fontSize = 11.sp)
            Spacer(Modifier.width(4.dp))
            Text("📁 ${folder.name ?: folder.fileName ?: "未命名"}", fontSize = 12.sp, maxLines = 1)
        }
        if (expanded) BrowserFolderTree(children[id].orEmpty(), children, selectedId, indent + 1, onSelect)
    }
}

@Composable
private fun HeaderButton(text: String, modifier: Modifier, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = modifier, contentPadding = PaddingValues(0.dp)) {
        Text(text, fontSize = 12.sp, color = Color.Gray)
    }
}

@Composable
@OptIn(ExperimentalComposeUiApi::class)
private fun FileBrowserRow(
    file: DriveFile,
    thumbnail: ByteArray?,
    checked: Boolean,
    onChecked: (Boolean) -> Unit,
    onOpen: () -> Unit,
    onEnter: () -> Unit,
    onThumbnailNeeded: () -> Unit,
    status: String,
    onCanFreeUp: ((Boolean) -> Unit) -> Unit,
    onRename: () -> Unit,
    onMove: () -> Unit,
    onSyncFolder: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    var canFree by remember { mutableStateOf<Boolean?>(null) }
    LaunchedEffect(file.id, file.thumbnailLink) { onThumbnailNeeded() }
    Row(
        modifier = Modifier.fillMaxWidth()
            .onPointerEvent(PointerEventType.Press) { event ->
                if (event.buttons.isSecondaryPressed) {
                    canFree = null
                    menu = true
                    onCanFreeUp { canFree = it }
                }
            }
            .pointerInput(file.id) {
                detectTapGestures(
                    onDoubleTap = { if (file.isFolder()) onEnter() else onOpen() },
                    onLongPress = { canFree = null; menu = true; onCanFreeUp { canFree = it } },
                )
            }
            .padding(horizontal = 8.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked, onChecked, Modifier.width(40.dp))
        if (thumbnail != null && !file.isFolder()) {
            val bitmap = remember(thumbnail) { runCatching { SkiaImage.makeFromEncoded(thumbnail).toComposeImageBitmap() }.getOrNull() }
            if (bitmap != null) Image(bitmap, null, Modifier.size(24.dp)) else Text("📄")
        } else Text(if (file.isFolder()) "📁" else "📄")
        Spacer(Modifier.width(8.dp))
        TextButton(onClick = { if (file.isFolder()) onEnter() else onOpen() }, modifier = Modifier.weight(1f)) {
            Text(file.name ?: file.fileName ?: "未命名", maxLines = 1)
        }
        Text(if (file.isFolder()) "—" else formatBytes(file.sizeBytes), fontSize = 12.sp, modifier = Modifier.width(100.dp))
        Text(file.modifiedTime.orEmpty().replace("T", " ").take(16), fontSize = 12.sp, modifier = Modifier.width(150.dp))
        Text(statusLabel(status), fontSize = 12.sp, modifier = Modifier.width(64.dp))
        Box(Modifier.width(44.dp)) {
            IconButton(onClick = { canFree = null; menu = true; onCanFreeUp { canFree = it } }) { Text("⋯") }
            // Compose DropdownMenu 自带窗口边界钳制，等价于原前端 8px 视口钳制。
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(onClick = { menu = false; onOpen() }) { Text(if (file.isFolder()) "进入目录" else "下载/打开") }
                if (file.isFolder()) DropdownMenuItem(onClick = { menu = false; onSyncFolder() }) { Text("双端对齐") }
                DropdownMenuItem(onClick = { menu = false; onRename() }) { Text("重命名") }
                DropdownMenuItem(onClick = { menu = false; onMove() }) { Text("移动") }
                DropdownMenuItem(onClick = { if (canFree == true) { menu = false; onFreeUp() } }, enabled = canFree == true) {
                    Text(if (canFree == null) "检查释放条件…" else "释放空间")
                }
                DropdownMenuItem(onClick = { menu = false; onDelete() }) { Text("删除", color = ErrorColor) }
            }
        }
    }
    Divider()
}

private fun statusLabel(status: String): String = when (status.lowercase()) {
    "synced", "0" -> "本地"
    "placeholder", "1" -> "占位"
    "folder" -> "目录"
    "syncing", "3" -> "同步中"
    "error", "failed", "8" -> "异常"
    else -> "云端"
}

// === SettingsScreen ===
@Composable
fun SettingsScreen(
    initialConfig: UserConfig,
    launchAtLogin: Boolean,
    userName: String?,
    appVersion: String,
    availableUpdate: UpdateManifest?,
    updateStatus: String,
    onLaunchAtLoginChange: (Boolean) -> Boolean,
    onBack: () -> Unit,
    onLogout: () -> Unit,
    onOpenLogs: () -> Unit,
    onExportConfig: () -> Unit,
    onImportConfig: () -> Unit,
    onClearCache: () -> Unit,
    onCheckUpdate: () -> Unit,
    onInstallUpdate: () -> Unit,
    onSave: (UserConfig) -> List<String>,
) {
    var mountDir by remember(initialConfig) { mutableStateOf(initialConfig.mountDir) }
    var concurrency by remember(initialConfig) { mutableStateOf(initialConfig.concurrency.toString()) }
    var pollInterval by remember(initialConfig) { mutableStateOf(initialConfig.pollIntervalSec.toString()) }
    var debounce by remember(initialConfig) { mutableStateOf(initialConfig.debounceSec.toString()) }
    var oauthPort by remember(initialConfig) { mutableStateOf(initialConfig.oauthCallbackPort.toString()) }
    var launchEnabled by remember(launchAtLogin) { mutableStateOf(launchAtLogin) }
    var errors by remember { mutableStateOf<List<String>>(emptyList()) }
    var tab by remember { mutableStateOf(0) }
    val tabs = listOf("同步目录", "传输", "高级", "账户", "日志", "关于")

    Column(modifier = Modifier.fillMaxSize().padding(24.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("← 返回") }
            Text("设置", fontSize = 20.sp, fontWeight = FontWeight.Bold)
        }
        Row(Modifier.fillMaxWidth()) {
            tabs.forEachIndexed { index, label ->
                TextButton(onClick = { tab = index }) {
                    Text(label, color = if (tab == index) BrandColor else Color.Gray)
                }
            }
        }
        Divider()
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(top = 20.dp)) {
            when (tab) {
                0 -> {
                    MateTextField(mountDir, { mountDir = it }, "挂载目录")
                    Spacer(Modifier.height(12.dp))
                    Text("切换目录后会停止旧 watcher，并以新目录执行一次可信全量索引。", fontSize = 12.sp, color = Color.Gray)
                }
                1 -> MateTextField(concurrency, { concurrency = it }, "并发传输数（1-20）")
                2 -> {
                    MateTextField(pollInterval, { pollInterval = it }, "增量轮询间隔（秒，0=禁用）")
                    Spacer(Modifier.height(12.dp))
                    MateTextField(debounce, { debounce = it }, "文件监听去抖（秒）")
                    Spacer(Modifier.height(12.dp))
                    MateTextField(oauthPort, { oauthPort = it }, "OAuth 回调端口")
                    Spacer(Modifier.height(12.dp))
                    MateSwitch(
                        checked = launchEnabled,
                        onCheckedChange = { requested ->
                            if (onLaunchAtLoginChange(requested)) launchEnabled = requested
                            else errors = listOf("开机自动启动设置失败")
                        },
                        label = "开机自动启动",
                    )
                    Spacer(Modifier.height(20.dp))
                    Row {
                        MateButton("导出配置", onExportConfig, primary = false)
                        Spacer(Modifier.width(8.dp))
                        MateButton("导入配置", onImportConfig, primary = false)
                        Spacer(Modifier.width(8.dp))
                        MateButton("清空应用数据", onClearCache, primary = false)
                    }
                }
                3 -> {
                    Text(userName ?: "华为账号", fontWeight = FontWeight.Bold)
                    Spacer(Modifier.height(12.dp))
                    MateButton("退出登录", onLogout, primary = false)
                }
                4 -> {
                    Text("运行日志使用共享 1000 条 ring buffer，并保留 30 天滚动文件。")
                    Spacer(Modifier.height(12.dp))
                    MateButton("打开日志查看器", onOpenLogs)
                }
                5 -> {
                    MateAppLogo(32.dp)
                    Spacer(Modifier.height(12.dp))
                    Text("版本 $appVersion")
                    Text("纯 Compose Multiplatform macOS 客户端", color = Color.Gray)
                    Spacer(Modifier.height(16.dp))
                    Text(updateStatus, fontSize = 12.sp, color = Color.Gray)
                    if (availableUpdate == null) MateButton("检查更新", onCheckUpdate, primary = false)
                    else MateButton("安装 ${availableUpdate.version}", onInstallUpdate)
                }
            }
        }
        errors.forEach { err -> Text("⚠️ $err", fontSize = 12.sp, color = ErrorColor) }
        if (tab <= 2) {
            MateButton("保存设置", onClick = {
                val config = UserConfig(
                    oauthRedirectUri = initialConfig.oauthRedirectUri,
                    mountDir = mountDir,
                    mountConfigured = mountDir.isNotBlank(),
                    concurrency = concurrency.toIntOrNull() ?: 6,
                    pollIntervalSec = pollInterval.toLongOrNull() ?: 60L,
                    debounceSec = debounce.toLongOrNull() ?: 3L,
                    oauthCallbackPort = oauthPort.toIntOrNull() ?: DEFAULT_CALLBACK_PORT,
                    skipPatterns = initialConfig.skipPatterns,
                    sortField = initialConfig.sortField,
                    sortOrder = initialConfig.sortOrder,
                )
                errors = onSave(config)
            })
        }
    }
}

// === LogViewerScreen ===
@Composable
fun LogViewerScreen(
    records: List<LogRecordDisplay>,
    onBack: () -> Unit,
    onReload: () -> Unit,
    onExport: () -> Unit,
    onClear: () -> Unit,
) {
    var filterLevel by remember { mutableStateOf<LogLevel?>(null) }
    val levels = listOf(null, LogLevel.ERROR, LogLevel.WARN, LogLevel.INFO, LogLevel.DEBUG)

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("← 返回") }
            Text("日志查看", fontSize = 18.sp, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
            TextButton(onClick = onReload) { Text("刷新") }
            TextButton(onClick = onExport) { Text("导出") }
            TextButton(onClick = onClear) { Text("清除") }
        }
        Spacer(Modifier.height(8.dp))

        // 级别过滤栏
        Row {
            levels.forEach { level ->
                TextButton(onClick = { filterLevel = level }) {
                    Text(
                        level?.name ?: "全部",
                        color = if (filterLevel == level) BrandColor else Color.Gray,
                        fontSize = 12.sp,
                    )
                }
            }
        }
        Divider()

        val filtered = if (filterLevel != null) {
            records.filter { it.level.severity >= filterLevel!!.severity }
        } else records

        Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
            filtered.forEach { record ->
                Row(modifier = Modifier.fillMaxWidth().padding(2.dp)) {
                    Text(record.level.name, fontSize = 10.sp, color = colorForLevel(record.level), modifier = Modifier.width(60.dp))
                    Text("[${record.target}] ${record.message}", fontSize = 10.sp, color = Color(0xFF444444))
                }
            }
        }
    }
}

data class LogRecordDisplay(
    val timestampMs: Long,
    val level: LogLevel,
    val target: String,
    val message: String,
)

private fun colorForLevel(level: LogLevel): Color = when (level) {
    LogLevel.ERROR -> ErrorColor
    LogLevel.WARN -> WarningColor
    LogLevel.INFO -> SuccessColor
    LogLevel.DEBUG -> Color.Gray
    LogLevel.TRACE -> Color.LightGray
}
