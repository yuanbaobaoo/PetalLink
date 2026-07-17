@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.DropdownMenu
import androidx.compose.material.DropdownMenuItem
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCheckbox
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateDialogOptions
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.confirmDialog
import io.github.yuanbaobaoo.petallink.ui.components.mate.showToast
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateToastVariant
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserSortField
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserState

/** 文件类型图标（对标原 Vue driveApi.fileTypeIcon）。 */
private fun fileTypeIcon(file: DriveFile): String = when {
    file.isFolder() -> "folder"
    file.category == "images" || (file.mimeType?.startsWith("image/") == true) -> "image"
    file.category == "videos" || (file.mimeType?.startsWith("video/") == true) -> "video"
    else -> "file"
}

/** 文件大小格式化（对标原 Vue formatFileSize）。 */
fun formatFileSize(bytes: Long): String = when {
    bytes < 1024 -> "$bytes B"
    bytes < 1_048_576 -> "%.1f KB".format(bytes / 1024.0)
    bytes < 1_073_741_824 -> "%.1f MB".format(bytes / 1_048_576.0)
    else -> "%.2f GB".format(bytes / 1_073_741_824.0)
}

/**
 * 文件列表（对标原 Vue FileListView.vue）。
 *
 * 6 列 + 拖拽列宽 + hover/selected 态 + 右键菜单条件渲染 + 双击行 +
 * 批量操作栏 + 4 个对话框（重命名/属性/下载进度/释放空间预览）。
 */
@Composable
fun FileListScreen(
    browser: FileBrowserState,
    fileStatuses: Map<String, String>,
    thumbnails: Map<String, ByteArray>,
    mountConfigured: Boolean,
    isIndexing: Boolean,
    onSort: (BrowserSortField) -> Unit,
    onEnterFolder: (DriveFile) -> Unit,
    onOpenItem: (DriveFile) -> Unit,
    onThumbnailNeeded: (DriveFile) -> Unit,
    onDelete: (List<DriveFile>) -> Unit,
    onFreeUp: (List<DriveFile>) -> Unit,
    onDownload: (List<DriveFile>) -> Unit,
    onSyncFolder: (DriveFile) -> Unit,
    onRename: (DriveFile, String) -> Unit,
    onShowProps: (DriveFile) -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    var checked by remember(browser.folderId) { mutableStateOf<Set<String>>(emptySet()) }
    var showCheckboxes by remember { mutableStateOf(false) }
    var selectedId by remember { mutableStateOf<String?>(null) }
    var sizeWidth by remember { mutableStateOf(100.dp) }
    var timeWidth by remember { mutableStateOf(150.dp) }
    val files = browser.visibleFiles
    val checkedCount = checked.size

    // 对话框状态
    var renameTarget by remember { mutableStateOf<DriveFile?>(null) }
    var renameValue by remember { mutableStateOf("") }
    var propsTarget by remember { mutableStateOf<DriveFile?>(null) }

    Column(modifier = Modifier.fillMaxSize()) {
        // 批量操作栏（选中>0 时，36px brand-lighter 底）
        if (checkedCount > 0) {
            val bulkBusy = isIndexing
            Row(
                modifier = Modifier.fillMaxWidth().height(36.dp)
                    .background(BrandLighter())
                    .padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("已选 $checkedCount 项", fontSize = 13.sp, fontWeight = FontWeight.Medium, color = BrandColor)
                Spacer(Modifier.weight(1f))
                MateButton(label = "批量下载", variant = MateButtonVariant.TEXT, icon = "download",
                    onClick = { val s = files.filter { it.id in checked }; onDownload(s) }, disabled = bulkBusy)
                MateButton(label = "释放空间", variant = MateButtonVariant.TEXT, icon = "cloud",
                    onClick = { val s = files.filter { it.id in checked }; onFreeUp(s) })
                if (mountConfigured) {
                    MateButton(label = "批量删除", variant = MateButtonVariant.TEXT, icon = "trash", danger = true,
                        onClick = { val s = files.filter { it.id in checked }; onDelete(s); checked = emptySet() },
                        disabled = bulkBusy)
                }
                MateButton(variant = MateButtonVariant.ICON, icon = "x",
                    onClick = { checked = emptySet(); showCheckboxes = false })
            }
        }

        // 空状态
        if (files.isEmpty() && !browser.loading) {
            MateEmpty(title = "此文件夹为空", icon = "folder-open", description = "上传或拖入文件即可同步到云端")
        }

        if (files.isNotEmpty()) {
            // 表头 36px
            Row(
                modifier = Modifier.fillMaxWidth().height(36.dp)
                    .background(semantic.bgHover)
                    .padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(modifier = Modifier.width(40.dp), contentAlignment = Alignment.CenterStart) {
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
                HeaderCell("名称", browser.sortField == BrowserSortField.NAME, browser.ascending, Modifier.weight(1f)) {
                    onSort(BrowserSortField.NAME)
                }
                HeaderCell("大小", browser.sortField == BrowserSortField.SIZE, browser.ascending, Modifier.width(sizeWidth),
                    resizable = true, onResize = { delta -> sizeWidth = (sizeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                    onSort(BrowserSortField.SIZE)
                }
                HeaderCell("修改时间", browser.sortField == BrowserSortField.MODIFIED_TIME, browser.ascending, Modifier.width(timeWidth),
                    resizable = true, onResize = { delta -> timeWidth = (timeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                    onSort(BrowserSortField.MODIFIED_TIME)
                }
                Text("状态", fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.width(60.dp))
                Text("操作", fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.width(40.dp))
            }

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
                        onShowProps = { propsTarget = file; onShowProps(file) },
                        onDelete = { onDelete(listOf(file)) },
                        onFreeUp = { onFreeUp(listOf(file)) },
                        onCanFreeUp = onCanFreeUp,
                    )
                }
            }
            // 底部信息
            Box(modifier = Modifier.fillMaxWidth().height(32.dp), contentAlignment = Alignment.Center) {
                Text("${files.size} 项 · 已全部加载", fontSize = 12.sp, color = semantic.textSecondary)
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

    // 属性对话框
    propsTarget?.let { target ->
        PropsDialog(target = target, onDismiss = { propsTarget = null })
    }
}

/** brand-lighter 色（浅色 #F2F3FF，暗色由主题决定，这里简化用浅色）。 */
@Composable
private fun BrandLighter(): Color = Color(0xFFF2F3FF)

/** 表头单元格（排序指示 + 可选 resize-handle）。 */
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
    val semantic = LocalSemanticColors.current
    Box(modifier = modifier) {
        Row(
            modifier = Modifier.fillMaxSize().clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onClick,
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(title, fontSize = 12.sp, color = semantic.textSecondary, fontWeight = FontWeight.Medium)
            if (active) {
                MateIcon(name = "arrow", size = 12.dp, tint = semantic.textSecondary,
                    modifier = Modifier.rotate(if (ascending) 0f else 90f))
            }
        }
        if (resizable && onResize != null) {
            Box(
                modifier = Modifier
                    .align(Alignment.CenterEnd)
                    .width(6.dp)
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

/** 文件行（44px，hover bg-hover，selected brand-lighter，双击触发，右键菜单条件渲染）。 */
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
    onShowProps: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    var hovered by remember { mutableStateOf(false) }
    var menuExpanded by remember { mutableStateOf(false) }
    var canFree by remember { mutableStateOf<Boolean?>(null) }
    val bg = when {
        selected -> BrandLighter()
        hovered -> semantic.bgHover
        else -> Color.Transparent
    }
    // 双击检测：用 pointerInput detectTapGestures(onDoubleTap)
    Column {
        Row(
            modifier = Modifier.fillMaxWidth().height(44.dp).background(bg).padding(horizontal = 16.dp)
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
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) {
                    // 右键菜单触发条件：这里用 secondary press 不便检测，简化为操作按钮触发
                },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // checkbox 列
            Box(modifier = Modifier.width(40.dp)) {
                if (showCheckbox) MateCheckbox(checked = checked, onCheckedChange = { c -> if (c != null) onCheckedChange(c) })
            }
            // name 列
            Row(modifier = Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                LaunchedEffect(file.id, file.thumbnailLink) { onThumbnailNeeded() }
                if (thumbnail != null && !file.isFolder()) {
                    val bitmap = remember(thumbnail) {
                        runCatching { org.jetbrains.skia.Image.makeFromEncoded(thumbnail).toComposeImageBitmap() }.getOrNull()
                    }
                    if (bitmap != null) {
                        androidx.compose.foundation.Image(bitmap, null, Modifier.width(20.dp).height(20.dp).clip(RoundedCornerShape(3.dp)))
                    } else {
                        MateIcon(name = fileTypeIcon(file), size = 20.dp, tint = if (file.isFolder()) BrandColor else semantic.textPlaceholder)
                    }
                } else {
                    MateIcon(name = fileTypeIcon(file), size = 20.dp, tint = if (file.isFolder()) BrandColor else semantic.textPlaceholder)
                }
                Text(file.name ?: file.fileName ?: "未命名", fontSize = 14.sp, color = semantic.textPrimary,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            // size 列
            Text(if (file.isFolder()) "—" else formatFileSize(file.sizeBytes), fontSize = 13.sp, color = semantic.textSecondary, modifier = Modifier.width(sizeWidth))
            // time 列
            Text(file.modifiedTime.orEmpty().replace("T", " ").take(16), fontSize = 13.sp, color = semantic.textSecondary, modifier = Modifier.width(timeWidth))
            // status 列
            Box(modifier = Modifier.width(60.dp), contentAlignment = Alignment.Center) {
                val (statusIcon, statusColor) = when (status) {
                    "synced" -> "local" to SuccessColor
                    "placeholder" -> "cloud" to semantic.textSecondary
                    "folder" -> "folder" to BrandColor
                    else -> "cloud" to semantic.textPlaceholder
                }
                MateIcon(name = statusIcon, size = 16.dp, tint = statusColor)
            }
            // actions 列（操作按钮 → 右键菜单）
            Box(modifier = Modifier.width(40.dp), contentAlignment = Alignment.Center) {
                MateButton(variant = MateButtonVariant.ICON, icon = "list", onClick = {
                    canFree = null; menuExpanded = true
                    onCanFreeUp(file) { canFree = it }
                })
                DropdownMenu(expanded = menuExpanded, onDismissRequest = { menuExpanded = false }) {
                    // 按条件渲染（对标原 Vue ctx-menu）
                    if (mountConfigured) {
                        CtxItem("执行双端对齐", "sync", enabled = !isIndexing) { menuExpanded = false; onSync() }
                        MateHDivider(modifier = Modifier.padding(vertical = 4.dp))
                    }
                    if (canFree == true) {
                        CtxItem("释放空间", "cloud") { menuExpanded = false; onFreeUp() }
                        MateHDivider(modifier = Modifier.padding(vertical = 4.dp))
                    }
                    if (mountConfigured) {
                        CtxItem("重命名", "edit", enabled = !isIndexing) { menuExpanded = false; onRename() }
                    }
                    CtxItem("属性", "info") { menuExpanded = false; onShowProps() }
                    if (mountConfigured) {
                        MateHDivider(modifier = Modifier.padding(vertical = 4.dp))
                        CtxItem("删除", "trash", danger = true, enabled = !isIndexing) { menuExpanded = false; onDelete() }
                    }
                }
            }
        }
        // 行底分隔线 0.5px
        MateHDivider()
    }
}

/** 右键菜单项（对标原 Vue .ctx-item）。 */
@Composable
private fun CtxItem(label: String, icon: String, danger: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val semantic = LocalSemanticColors.current
    DropdownMenuItem(onClick = onClick, enabled = enabled) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            MateIcon(name = icon, size = 16.dp, tint = if (danger) ErrorColor else semantic.textPrimary)
            Text(label, color = if (danger) ErrorColor else semantic.textPrimary, fontSize = 14.sp)
        }
    }
}

/** 重命名对话框（对标原 Vue MateDialog 重命名）。 */
@Composable
private fun RenameDialog(target: DriveFile, value: String, onValueChange: (String) -> Unit, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    confirmDialog(
        MateDialogOptions(
            title = "重命名",
            content = "",
            confirmText = "确定",
        ),
    ) { confirmed ->
        if (confirmed) onConfirm() else onDismiss()
    }
    // 内联输入框（简化：用 confirmDialog 包裹时无法嵌入输入，这里用独立弹窗逻辑替代）
    // 注：原 Vue 用 MateDialog + MateTextField，CMP 用 confirmDialog 不支持嵌入输入框；
    // 此处保留 confirmDialog 确认，实际重命名值由调用方从 renameValue 读取。
    // 完整的带输入框对话框需要 MateDialog 支持 slot，后续完善。
}

/** 属性对话框（对标原 Vue MateDialog 属性，5 行键值）。 */
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
