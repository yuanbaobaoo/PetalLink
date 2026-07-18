@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import androidx.compose.ui.window.Dialog
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.displayName
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCheckbox
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateDialogOptions
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTextField
import io.github.yuanbaobaoo.petallink.ui.components.mate.confirmDialog
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme
import io.github.yuanbaobaoo.petallink.config.SortField
import io.github.yuanbaobaoo.petallink.commands.FreeableItem
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserState

/**
 * v2：文档类扩展名 → file-text tile（对标原型 .docx/.md/.pdf）。
 */
private val DOC_TILE_EXTS = setOf("doc", "docx", "txt", "md", "pdf", "rtf", "odt", "pages")

/**
 * v2：表格/图表类扩展名 → chart tile（对标原型 .xlsx）。
 */
private val SHEET_TILE_EXTS = setOf("xls", "xlsx", "csv", "ods", "numbers", "et")

/**
 * 文件类型图标（对标原 Vue driveApi.fileTypeIcon）。
 *
 * v2 扩展返回 tile 类型（folder / file-text / image / video / chart / file），
 * 名称列色块依此取色；image/video 判断逻辑与原有一致，doc/sheet 为原 "file" 桶的细分。
 */
private fun fileTypeIcon(file: DriveFile): String {
    if (file.isFolder()) return "folder"
    val mime = file.mimeType.orEmpty()
    val ext = (file.name ?: file.fileName).orEmpty().substringAfterLast('.', "").lowercase()
    return when {
        file.category == "images" || mime.startsWith("image/") -> "image"
        file.category == "videos" || mime.startsWith("video/") -> "video"
        // 表格/图表类（category 或扩展名）→ sheet tile
        file.category == "sheets" || ext in SHEET_TILE_EXTS -> "chart"
        // 文档类（text/*、pdf、常见文档扩展名）→ doc tile
        file.category == "docs" || mime.startsWith("text/") || mime == "application/pdf" || ext in DOC_TILE_EXTS -> "file-text"
        else -> "file"
    }
}

/**
 * 文件大小格式化（对标原 Vue formatFileSize）。
 */
fun formatFileSize(bytes: Long): String = when {
    bytes < 1024 -> "$bytes B"
    bytes < 1_048_576 -> "%.1f KB".format(bytes / 1024.0)
    bytes < 1_073_741_824 -> "%.1f MB".format(bytes / 1_048_576.0)
    else -> "%.2f GB".format(bytes / 1_073_741_824.0)
}

/**
 * 文件列表（对标原 Vue FileListView.vue；视觉按 v2 原型 design/v2/02-main.html + 05-file-ops.html）。
 *
 * 6 列 + 拖拽列宽 + hover/selected 态 + 右键菜单条件渲染 + 双击行 +
 * 批量操作栏（v2 深色浮动条）+ 4 个对话框（重命名/属性/下载进度/释放空间预览）。
 */
@Composable
fun FileListScreen(
    browser: FileBrowserState,
    fileStatuses: Map<String, String>,
    thumbnails: Map<String, ByteArray>,
    mountConfigured: Boolean,
    isIndexing: Boolean,
    onSort: (SortField) -> Unit,
    onEnterFolder: (DriveFile) -> Unit,
    onOpenItem: (DriveFile) -> Unit,
    onThumbnailNeeded: (DriveFile) -> Unit,
    onDelete: (List<DriveFile>) -> Unit,
    onPreviewFreeUp: (List<DriveFile>, (List<FreeableItem>) -> Unit) -> Unit,
    onFreeUp: (List<FreeableItem>) -> Unit,
    onDownload: (List<DriveFile>) -> Unit,
    onSyncFolder: (DriveFile) -> Unit,
    onRename: (DriveFile, String) -> Unit,
    onMove: (DriveFile, String) -> Unit,
    onShowProps: (DriveFile) -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val fileListControls = PetalTheme.metrics.fileList.controls
    var checked by remember(browser.folderId) { mutableStateOf<Set<String>>(emptySet()) }
    var showCheckboxes by remember { mutableStateOf(false) }
    var selectedId by remember { mutableStateOf<String?>(null) }
    // v2 列宽（FileListColumns）：size 110 / time 160
    var sizeWidth by remember { mutableStateOf(fileListControls.sizeColumnInitialWidth) }
    var timeWidth by remember { mutableStateOf(fileListControls.timeColumnInitialWidth) }
    val files = browser.visibleFiles
    val checkedCount = checked.size

    // 对话框状态
    var renameTarget by remember { mutableStateOf<DriveFile?>(null) }
    var renameValue by remember { mutableStateOf("") }
    var moveTarget by remember { mutableStateOf<DriveFile?>(null) }
    var moveParentId by remember { mutableStateOf<String?>(null) }
    var propsTarget by remember { mutableStateOf<DriveFile?>(null) }
    val knownFolders = browser.directoryChildren.values.flatten()
        .filter { it.isFolder() }
        .distinctBy { it.id }
    val requestDelete: (List<DriveFile>, () -> Unit) -> Unit = { selection, afterDelete ->
        val names = selection.take(5).joinToString("、") { it.displayName() }
        val suffix = if (selection.size > 5) " 等 ${selection.size} 项" else ""
        confirmDialog(
            MateDialogOptions(
                title = "确认删除",
                content = "将从云端删除：$names$suffix。此操作会同步到本地，且不能在应用内撤销。",
                confirmText = "删除",
                danger = true,
                titleIcon = "trash",
            ),
        ) { confirmed ->
            if (confirmed) {
                onDelete(selection)
                afterDelete()
            }
        }
    }
    val requestFreeUp: (List<DriveFile>) -> Unit = { selection ->
        onPreviewFreeUp(selection) { items ->
            val totalBytes = items.sumOf { it.size }
            val content = if (items.isEmpty()) {
                "所选内容中没有通过远端校验、可安全释放的本地文件。"
            } else {
                "将释放 ${items.size} 个本地文件，共 ${formatFileSize(totalBytes)}。云端内容会保留，本地文件将变为占位符。"
            }
            confirmDialog(
                MateDialogOptions(
                    title = if (items.isEmpty()) "无法释放空间" else "释放空间预览",
                    content = content,
                    confirmText = if (items.isEmpty()) "关闭" else "确认释放",
                    danger = items.isNotEmpty(),
                    titleIcon = "cloud",
                ),
            ) { confirmed ->
                if (confirmed && items.isNotEmpty()) onFreeUp(items)
            }
        }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        // 批量操作栏（选中>0 时；v2 05-file-ops：深色浮动条 h44 radius10，
        // margin 10/12/0 叠加 file-table 容器 padding 12 → 水平内缩 24）
        if (checkedCount > 0) {
            val bulkBusy = isIndexing
            Row(
                modifier = Modifier.fillMaxWidth()
                    .padding(
                        start = PetalTheme.metrics.fileList.controls.bulkBarHorizontalMargin,
                        top = PetalTheme.metrics.fileList.controls.bulkBarTopMargin,
                        end = PetalTheme.metrics.fileList.controls.bulkBarHorizontalMargin,
                    )
                    .height(PetalTheme.metrics.fileList.controls.bulkBarHeight)
                    .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.bulkBarRadius))
                    .background(PetalTheme.colors.fileListBulkBackground)
                    .padding(
                        start = PetalTheme.metrics.fileList.controls.bulkBarStartPadding,
                        end = PetalTheme.metrics.fileList.controls.bulkBarEndPadding,
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.fileList.controls.bulkBarContentSpacing),
            ) {
                Text(
                    "已选 $checkedCount 项",
                    style = PetalTheme.typography.fileList.selectionSummary,
                    color = PetalTheme.colors.fileListBulkSelectionText,
                )
                Spacer(Modifier.weight(1f))
                BulkBarButton(label = "批量下载", icon = "download", disabled = bulkBusy,
                    onClick = { val s = files.filter { it.id in checked }; onDownload(s) })
                BulkBarButton(label = "释放空间", icon = "cloud",
                    onClick = { requestFreeUp(files.filter { it.id in checked }) })
                if (mountConfigured) {
                    BulkBarButton(label = "批量删除", icon = "trash", danger = true, disabled = bulkBusy,
                        onClick = {
                            val selection = files.filter { it.id in checked }
                            requestDelete(selection) { checked = emptySet() }
                        })
                }
                BulkBarCloseButton(onClick = { checked = emptySet(); showCheckboxes = false })
            }
        }

        // 空状态
        if (files.isEmpty() && !browser.loading) {
            MateEmpty(title = "此文件夹为空", icon = "folder-open", description = "上传或拖入文件即可同步到云端")
        }

        if (files.isNotEmpty()) {
            // v2：file-table 容器 padding 0 12
            Column(modifier = Modifier.fillMaxWidth().weight(1f).padding(horizontal = PetalTheme.metrics.fileList.controls.tableHorizontalPadding)) {
                // 表头（v2：38px，12.5sp semibold textSecondary，底部分隔线）
                Row(
                    modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.fileList.controls.headerHeight)
                        .padding(horizontal = PetalTheme.metrics.fileList.controls.headerHorizontalPadding),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.checkboxColumnWidth), contentAlignment = Alignment.CenterStart) {
                        if (showCheckboxes) {
                            val headerCheck: Boolean? = if (checkedCount == 0) false
                            else if (checkedCount == files.size) true else null
                            MateCheckbox(checked = headerCheck, onCheckedChange = { _ ->
                                checked = if (checkedCount == files.size) emptySet()
                                else files.mapNotNull { it.id }.toSet()
                            })
                        } else {
                            MateButton(variant = MateButtonVariant.ICON, icon = "check", onClick = { showCheckboxes = true })
                        }
                    }
                    HeaderCell("名称", browser.sortField == SortField.Name, browser.ascending, Modifier.weight(1f)) {
                        onSort(SortField.Name)
                    }
                    HeaderCell("大小", browser.sortField == SortField.Size, browser.ascending, Modifier.width(sizeWidth),
                        resizable = true, onResize = { delta -> sizeWidth = (sizeWidth + delta).coerceIn(
                            fileListControls.resizableColumnMinimumWidth,
                            fileListControls.resizableColumnMaximumWidth,
                        ) }) {
                        onSort(SortField.Size)
                    }
                    HeaderCell("修改时间", browser.sortField == SortField.ModifiedTime, browser.ascending, Modifier.width(timeWidth),
                        resizable = true, onResize = { delta -> timeWidth = (timeWidth + delta).coerceIn(
                            fileListControls.resizableColumnMinimumWidth,
                            fileListControls.resizableColumnMaximumWidth,
                        ) }) {
                        onSort(SortField.ModifiedTime)
                    }
                    // v2 列宽：状态 72 / 操作 44
                    Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.statusColumnWidth), contentAlignment = Alignment.Center) {
                        Text("状态", style = PetalTheme.typography.fileList.statusColumnHeader, color = semantic.textSecondary)
                    }
                    Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.actionColumnWidth), contentAlignment = Alignment.Center) {
                        Text("操作", style = PetalTheme.typography.fileList.actionColumnHeader, color = semantic.textSecondary)
                    }
                }
                MateHDivider()

                LazyColumn(modifier = Modifier.weight(1f)) {
                    items(files, key = { it.id ?: "${it.name}-${it.editedTime}" }) { file ->
                        FileRow(
                            file = file,
                            checked = file.id in checked,
                            selected = file.id == selectedId,
                            showCheckbox = showCheckboxes,
                            status = fileStatuses[file.id] ?: "not_synced",
                            thumbnail = file.id?.let(thumbnails::get),
                            mountConfigured = mountConfigured,
                            isIndexing = isIndexing,
                            sizeWidth = sizeWidth,
                            timeWidth = timeWidth,
                            onCheckedChange = { c ->
                                val id = file.id ?: return@FileRow
                                checked = if (c) checked + id else checked - id
                            },
                            onClick = { selectedId = file.id },
                            onDoubleClick = { if (file.isFolder()) onEnterFolder(file) else onOpenItem(file) },
                            onThumbnailNeeded = { onThumbnailNeeded(file) },
                            onSync = { onSyncFolder(file) },
                            onRename = { renameTarget = file; renameValue = file.name ?: file.fileName ?: "" },
                            onMove = { moveTarget = file; moveParentId = null },
                            onShowProps = { propsTarget = file; onShowProps(file) },
                            onDelete = { requestDelete(listOf(file)) {} },
                            onFreeUp = { requestFreeUp(listOf(file)) },
                            onCanFreeUp = onCanFreeUp,
                        )
                    }
                }
                // 底部信息（v2 file-footer：h36，13sp textPlaceholder）
                Box(modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.fileList.controls.footerHeight), contentAlignment = Alignment.Center) {
                    Text("${files.size} 项 · 已全部加载", style = PetalTheme.typography.fileList.loadedSummary, color = semantic.textPlaceholder)
                }
            }
        }
    }

    // 重命名对话框
    renameTarget?.let { target ->
        RenameDialog(
            target = target,
            value = renameValue,
            onValueChange = { renameValue = it },
            onConfirm = {
                val newName = renameValue.trim()
                if (newName.isNotBlank() && newName != (target.name ?: target.fileName)) {
                    onRename(target, newName)
                }
                renameTarget = null
            },
            onDismiss = { renameTarget = null },
        )
    }

    moveTarget?.let { target ->
        MoveDialog(
            target = target,
            folders = knownFolders.filter { it.id != target.id },
            selectedParentId = moveParentId,
            onSelect = { moveParentId = it },
            onConfirm = {
                moveParentId?.let { onMove(target, it) }
                moveTarget = null
            },
            onDismiss = { moveTarget = null },
        )
    }

    // 属性对话框
    propsTarget?.let { target ->
        PropsDialog(target = target, onDismiss = { propsTarget = null })
    }
}

/**
 * 批量栏文字按钮（v2 .bulk-bar .btn-ghost：h32 radius8，白字 85% + 图标 70%，
 * hover 白 12% 底；danger 文字 #FDA4AF、hover 红 18% 底；disabled 整体 40% 透明）。
 */
@Composable
private fun BulkBarButton(
    label: String,
    icon: String,
    danger: Boolean = false,
    disabled: Boolean = false,
    onClick: () -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val contentColor = if (danger) PetalTheme.colors.fileListBulkDangerText else PetalTheme.colors.fileListBulkActionText
    val iconColor = if (danger) PetalTheme.colors.fileListBulkDangerIcon else PetalTheme.colors.fileListBulkActionIcon
    val bg = when {
        disabled -> Color.Transparent
        hovered && danger -> PetalTheme.colors.fileListBulkDangerHoverBackground
        hovered -> PetalTheme.colors.fileListBulkActionHoverBackground
        else -> Color.Transparent
    }
    Row(
        modifier = Modifier.height(PetalTheme.metrics.fileList.controls.bulkActionHeight)
            .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.bulkActionRadius))
            .background(bg)
            .alpha(if (disabled) PetalTheme.metrics.fileList.controls.bulkActionDisabledAlpha else 1f)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, enabled = !disabled, onClick = onClick)
            .padding(horizontal = PetalTheme.metrics.fileList.controls.bulkActionHorizontalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.fileList.controls.bulkActionContentSpacing),
    ) {
        MateIcon(name = icon, size = PetalTheme.metrics.fileList.controls.bulkActionIconSize, tint = iconColor)
        Text(label, style = PetalTheme.typography.fileList.toolbarAction, color = contentColor)
    }
}

/**
 * 批量栏关闭按钮（v2 .bulk-bar .btn-circle：32×32 正圆，白 70% 图标，hover 白 12% 底 + 纯白图标）。
 */
@Composable
private fun BulkBarCloseButton(onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    Box(
        modifier = Modifier.size(PetalTheme.metrics.fileList.controls.bulkCloseSize)
            .clip(CircleShape)
            .background(if (hovered) PetalTheme.colors.fileListBulkCloseHoverBackground else Color.Transparent)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        MateIcon(
            name = "x",
            size = PetalTheme.metrics.fileList.controls.bulkCloseIconSize,
            tint = if (hovered) PetalTheme.colors.fileListBulkCloseHoverIcon else PetalTheme.colors.fileListBulkCloseIcon,
        )
    }
}

/**
 * 表头单元格（排序指示 + 可选 resize-handle）。
 */
@Composable
private fun HeaderCell(
    title: String,
    active: Boolean,
    ascending: Boolean,
    modifier: Modifier,
    resizable: Boolean = false,
    onResize: ((androidx.compose.ui.unit.Dp) -> Unit)? = null,
    onClick: () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Box(modifier = modifier) {
        Row(
            modifier = Modifier.fillMaxSize().clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onClick,
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.fileList.controls.headerSortSpacing),
        ) {
            // v2：表头 12.5sp semibold textSecondary
            Text(title, style = PetalTheme.typography.fileList.genericColumnHeader, color = semantic.textSecondary)
            if (active) {
                MateIcon(name = "arrow", size = PetalTheme.metrics.fileList.controls.headerSortIconSize, tint = semantic.textSecondary,
                    modifier = Modifier.rotate(if (ascending) 0f else 90f))
            }
        }
        if (resizable && onResize != null) {
            Box(
                modifier = Modifier
                    .align(Alignment.CenterEnd)
                    .width(PetalTheme.metrics.fileList.controls.resizeHandleWidth)
                    .fillMaxHeight()
                    .pointerInput(Unit) {
                        detectDragGestures { change, dragAmount ->
                            change.consume()
                            onResize(dragAmount.x.toDp())
                        }
                    },
            )
        }
    }
}

/**
 * 文件行（v2：56px，radius 8，hover bgHover，selected PetalTheme.colors.brandLighter，双击触发，右键菜单条件渲染）。
 */
@Composable
private fun FileRow(
    file: DriveFile,
    checked: Boolean,
    selected: Boolean,
    showCheckbox: Boolean,
    status: String,
    thumbnail: ByteArray?,
    mountConfigured: Boolean,
    isIndexing: Boolean,
    sizeWidth: androidx.compose.ui.unit.Dp,
    timeWidth: androidx.compose.ui.unit.Dp,
    onCheckedChange: (Boolean) -> Unit,
    onClick: () -> Unit,
    onDoubleClick: () -> Unit,
    onThumbnailNeeded: () -> Unit,
    onSync: () -> Unit,
    onRename: () -> Unit,
    onMove: () -> Unit,
    onShowProps: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    var menuExpanded by remember { mutableStateOf(false) }
    var canFree by remember { mutableStateOf<Boolean?>(null) }
    // v2：hover 态接入 hoverable（此前 hovered 变量未接线，hover 背景从未生效）
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val bg = when {
        selected -> PetalTheme.colors.brandLighter
        hovered -> semantic.bgHover
        else -> Color.Transparent
    }
    // 双击检测：用 pointerInput detectTapGestures(onDoubleTap)
    Column {
        Row(
            modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.fileList.controls.rowHeight)
                .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.rowRadius))
                .background(bg)
                .hoverable(interaction)
                .pointerInput(file.id) {
                    detectTapGestures(
                        onTap = { onClick() },
                        onDoubleTap = { onDoubleClick() },
                        onLongPress = {
                            // 右键菜单替代：长按弹出
                            canFree = null; menuExpanded = true
                            onCanFreeUp(file) { canFree = it }
                        },
                    )
                }
                .clickable(
                    interactionSource = interaction,
                    indication = null,
                ) {
                    // 右键菜单触发条件：这里用 secondary press 不便检测，简化为操作按钮触发
                }
                .padding(horizontal = PetalTheme.metrics.fileList.controls.rowHorizontalPadding),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // checkbox 列
            Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.checkboxColumnWidth)) {
                if (showCheckbox) MateCheckbox(checked = checked, onCheckedChange = { c -> if (c != null) onCheckedChange(c) })
            }
            // name 列（v2：图标 32×32 色块 tile，间距 12）
            Row(modifier = Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.fileList.controls.rowNameContentSpacing)) {
                LaunchedEffect(file.id, file.thumbnailLink) { onThumbnailNeeded() }
                FileTypeTile(file = file, thumbnail = thumbnail)
                Text(file.displayName(), style = PetalTheme.typography.fileList.rowFileName, color = semantic.textPrimary,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            // size 列
            Text(if (file.isFolder()) "—" else formatFileSize(file.sizeBytes), style = PetalTheme.typography.fileList.rowFileSize, color = semantic.textSecondary, modifier = Modifier.width(sizeWidth))
            // time 列
            Text(file.modifiedTime.orEmpty().replace("T", " ").take(16), style = PetalTheme.typography.fileList.rowModifiedTime, color = semantic.textSecondary, modifier = Modifier.width(timeWidth))
            // status 列（v2 列宽 72）
            Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.statusColumnWidth), contentAlignment = Alignment.Center) {
                val (statusIcon, statusColor) = when (status) {
                    "synced" -> "local" to PetalTheme.colors.success
                    "placeholder" -> "cloud" to semantic.textSecondary
                    "folder" -> "folder" to PetalTheme.colors.brand
                    else -> "cloud" to semantic.textPlaceholder
                }
                MateIcon(name = statusIcon, size = PetalTheme.metrics.fileList.controls.rowStatusIconSize, tint = statusColor)
            }
            // actions 列（v2 列宽 44；操作按钮 → 右键菜单，锚点为本 Box）
            Box(modifier = Modifier.width(PetalTheme.metrics.fileList.controls.actionColumnWidth), contentAlignment = Alignment.Center) {
                MateButton(variant = MateButtonVariant.ICON, icon = "list", onClick = {
                    canFree = null; menuExpanded = true
                    onCanFreeUp(file) { canFree = it }
                })
                if (menuExpanded) {
                    // v2 自绘菜单（对标 05-file-ops .menu：w200 radius10 + 阴影 + 0.5px 描边，padding 6）
                    Popup(
                        onDismissRequest = { menuExpanded = false },
                        properties = PopupProperties(focusable = true),
                    ) {
                        Column(
                            modifier = Modifier
                                .width(PetalTheme.metrics.fileList.controls.contextMenuWidth)
                                .shadow(PetalTheme.metrics.fileList.controls.contextMenuShadowElevation, RoundedCornerShape(PetalTheme.metrics.fileList.controls.contextMenuRadius))
                                .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.contextMenuRadius))
                                .background(semantic.bgContainer)
                                .border(
                                    PetalTheme.metrics.fileList.controls.contextMenuBorderWidth,
                                    semantic.border,
                                    RoundedCornerShape(PetalTheme.metrics.fileList.controls.contextMenuRadius),
                                )
                                .padding(PetalTheme.metrics.fileList.controls.contextMenuPadding),
                        ) {
                            // 按条件渲染（对标原 Vue ctx-menu）
                            if (mountConfigured) {
                                CtxItem("执行双端对齐", "sync", enabled = !isIndexing) { menuExpanded = false; onSync() }
                                CtxDivider()
                            }
                            if (canFree == true) {
                                CtxItem("释放空间", "cloud") { menuExpanded = false; onFreeUp() }
                                CtxDivider()
                            }
                            if (mountConfigured) {
                                CtxItem("重命名", "edit", enabled = !isIndexing) { menuExpanded = false; onRename() }
                                CtxItem("移动到…", "folder-open", enabled = !isIndexing) { menuExpanded = false; onMove() }
                            }
                            CtxItem("属性", "info") { menuExpanded = false; onShowProps() }
                            if (mountConfigured) {
                                CtxDivider()
                                CtxItem("删除", "trash", danger = true, enabled = !isIndexing) { menuExpanded = false; onDelete() }
                            }
                        }
                    }
                }
            }
        }
        // 行底分隔线 0.5px
        MateHDivider()
    }
}

/**
 * 文件类型色块 tile（v2 .ftile：32×32 radius 6，柔和底色 + 彩色图标 18dp）。
 * 图片有缩略图时直接显示缩略图（同尺寸 clip radius 6），解码失败回退到色块图标。
 */
@Composable
private fun FileTypeTile(file: DriveFile, thumbnail: ByteArray?) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val type = fileTypeIcon(file)
    if (thumbnail != null && !file.isFolder()) {
        val bitmap = remember(thumbnail) {
            runCatching { org.jetbrains.skia.Image.makeFromEncoded(thumbnail).toComposeImageBitmap() }.getOrNull()
        }
        if (bitmap != null) {
            androidx.compose.foundation.Image(
                bitmap,
                null,
                Modifier.size(PetalTheme.metrics.fileList.controls.thumbnailSize)
                    .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.thumbnailRadius)),
            )
            return
        }
    }
    val (bg, tint) = when (type) {
        "folder" -> PetalTheme.colors.folderBg to PetalTheme.colors.folder
        "file-text" -> PetalTheme.colors.documentBg to PetalTheme.colors.document
        "image" -> PetalTheme.colors.imageBg to PetalTheme.colors.image
        "video" -> PetalTheme.colors.videoBg to PetalTheme.colors.video
        "chart" -> PetalTheme.colors.sheetBg to PetalTheme.colors.sheet
        else -> semantic.bgFill to semantic.textSecondary
    }
    Box(
        modifier = Modifier.size(PetalTheme.metrics.fileList.controls.thumbnailSize)
            .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.thumbnailRadius)).background(bg),
        contentAlignment = Alignment.Center,
    ) {
        MateIcon(name = type, size = PetalTheme.metrics.fileList.controls.fileTypeIconSize, tint = tint)
    }
}

/**
 * 右键菜单项（v2 .menu__item：h36 radius8，hover bgFill，padding 0 12，gap 10；
 * icon 16（默认 textSecondary，danger PetalTheme.colors.error），文字 15sp（默认 textPrimary，danger PetalTheme.colors.error）；
 * enabled=false 时文字/图标 textPlaceholder 且不响应点击与 hover。
 */
@Composable
private fun CtxItem(label: String, icon: String, danger: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val contentColor = when {
        !enabled -> semantic.textPlaceholder
        danger -> PetalTheme.colors.error
        else -> semantic.textPrimary
    }
    val iconColor = when {
        !enabled -> semantic.textPlaceholder
        danger -> PetalTheme.colors.error
        else -> semantic.textSecondary
    }
    Row(
        modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.fileList.controls.contextActionHeight)
            .clip(RoundedCornerShape(PetalTheme.metrics.fileList.controls.contextActionRadius))
            .background(if (hovered && enabled) semantic.bgFill else Color.Transparent)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, enabled = enabled, onClick = onClick)
            .padding(horizontal = PetalTheme.metrics.fileList.controls.contextActionHorizontalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.fileList.controls.contextActionContentSpacing),
    ) {
        MateIcon(name = icon, size = PetalTheme.metrics.fileList.controls.contextActionIconSize, tint = iconColor)
        Text(label, color = contentColor, style = PetalTheme.typography.fileList.secondaryAction)
    }
}

/**
 * 右键菜单分隔线（v2 .menu__sep：0.5px，margin 8/4，bg border）。
 */
@Composable
private fun CtxDivider() {
    Box(
        modifier = Modifier.fillMaxWidth()
            .padding(
                horizontal = PetalTheme.metrics.fileList.controls.contextDividerHorizontalPadding,
                vertical = PetalTheme.metrics.fileList.controls.contextDividerVerticalPadding,
            )
            .height(PetalTheme.metrics.fileList.controls.contextDividerHeight)
            .background(LOCAL_SEMANTIC_COLORS.current.border),
    )
}

/**
 * 重命名对话框（对标原 Vue MateDialog 重命名）。
 */
@Composable
private fun RenameDialog(target: DriveFile, value: String, onValueChange: (String) -> Unit, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val metrics = PetalTheme.metrics.fileList
    val currentName = target.name ?: target.fileName.orEmpty()
    val valid = value.trim().isNotEmpty() && value.trim() != currentName &&
        '/' !in value && value != "." && value != ".."
    Dialog(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier.width(metrics.renameDialogWidth)
                .clip(RoundedCornerShape(metrics.renameDialogRadius))
                .background(semantic.bgContainer)
                .padding(metrics.renameDialogPadding),
            verticalArrangement = Arrangement.spacedBy(metrics.renameDialogContentSpacing),
        ) {
            Text(
                text = "重命名",
                style = PetalTheme.typography.fileList.renameDialogTitle,
                color = semantic.textPrimary,
            )
            MateTextField(
                value = value,
                onValueChange = onValueChange,
                modifier = Modifier.fillMaxWidth(),
                error = value.isNotEmpty() && !valid,
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(
                    metrics.renameDialogActionSpacing,
                    Alignment.End,
                ),
            ) {
                MateButton(label = "取消", variant = MateButtonVariant.TEXT, onClick = onDismiss)
                MateButton(
                    label = "确定",
                    variant = MateButtonVariant.PRIMARY,
                    disabled = !valid,
                    onClick = onConfirm,
                )
            }
        }
    }
}

/**
 * 从已加载目录树选择目标文件夹的移动对话框。
 */
@Composable
private fun MoveDialog(
    target: DriveFile,
    folders: List<DriveFile>,
    selectedParentId: String?,
    onSelect: (String) -> Unit,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val metrics = PetalTheme.metrics.fileList
    Dialog(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier.width(metrics.moveDialogWidth)
                .clip(RoundedCornerShape(metrics.moveDialogRadius))
                .background(semantic.bgContainer)
                .padding(metrics.moveDialogPadding),
            verticalArrangement = Arrangement.spacedBy(metrics.moveDialogContentSpacing),
        ) {
            Text(
                "移动“${target.displayName()}”",
                color = semantic.textPrimary,
                style = PetalTheme.typography.fileList.moveDialogTitle,
            )
            if (folders.isEmpty()) {
                Text(
                    "当前已加载的目录树中没有可选目标，请先在侧边栏展开目标目录。",
                    color = semantic.textSecondary,
                    style = PetalTheme.typography.fileList.moveDialogDescription,
                )
            } else {
                LazyColumn(modifier = Modifier.fillMaxWidth().height(metrics.moveDialogFolderListHeight)) {
                    items(folders, key = { it.id.orEmpty() }) { folder ->
                        val id = folder.id ?: return@items
                        Row(
                            modifier = Modifier.fillMaxWidth()
                                .clip(RoundedCornerShape(metrics.moveDialogFolderRadius))
                                .background(if (id == selectedParentId) PetalTheme.colors.brandLighter else Color.Transparent)
                                .clickable { onSelect(id) }
                                .padding(metrics.moveDialogFolderPadding),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(metrics.moveDialogFolderContentSpacing),
                        ) {
                            MateIcon(name = "folder", size = metrics.moveDialogFolderIconSize, tint = PetalTheme.colors.folder)
                            Text(folder.displayName(), color = semantic.textPrimary, style = PetalTheme.typography.fileList.moveDialogFolder)
                        }
                    }
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(metrics.moveDialogActionSpacing, Alignment.End),
            ) {
                MateButton(label = "取消", variant = MateButtonVariant.TEXT, onClick = onDismiss)
                MateButton(
                    label = "移动",
                    variant = MateButtonVariant.PRIMARY,
                    disabled = selectedParentId == null,
                    onClick = onConfirm,
                )
            }
        }
    }
}

/**
 * 属性对话框（对标原 Vue MateDialog 属性，5 行键值）。
 */
@Composable
private fun PropsDialog(target: DriveFile, onDismiss: () -> Unit) {
    confirmDialog(
        MateDialogOptions(
            title = target.name ?: target.fileName ?: "",
            content = buildString {
                append("文件 ID：${target.id ?: "—"}\n")
                append("类型：${if (target.isFolder()) "文件夹" else (target.mimeType ?: "文件")}\n")
                append("大小：${if (target.isFolder()) "—" else formatFileSize(target.sizeBytes)}\n")
                append("修改时间：${target.modifiedTime.orEmpty()}\n")
                target.contentHash?.let { append("SHA256：$it") }
            },
            confirmText = "关闭",
        ),
    ) { onDismiss() }
}
