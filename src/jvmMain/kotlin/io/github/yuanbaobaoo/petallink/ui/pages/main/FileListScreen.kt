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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
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
import io.github.yuanbaobaoo.petallink.ui.components.mate.confirmDialog
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.FolderAmber
import io.github.yuanbaobaoo.petallink.ui.theme.FolderAmberBg
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.theme.TileDoc
import io.github.yuanbaobaoo.petallink.ui.theme.TileDocBg
import io.github.yuanbaobaoo.petallink.ui.theme.TileImage
import io.github.yuanbaobaoo.petallink.ui.theme.TileImageBg
import io.github.yuanbaobaoo.petallink.ui.theme.TileSheet
import io.github.yuanbaobaoo.petallink.ui.theme.TileSheetBg
import io.github.yuanbaobaoo.petallink.ui.theme.TileVideo
import io.github.yuanbaobaoo.petallink.ui.theme.TileVideoBg
import io.github.yuanbaobaoo.petallink.config.SortField
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
    // v2 列宽（FileListColumns）：size 110 / time 160
    var sizeWidth by remember { mutableStateOf(110.dp) }
    var timeWidth by remember { mutableStateOf(160.dp) }
    val files = browser.visibleFiles
    val checkedCount = checked.size

    // 对话框状态
    var renameTarget by remember { mutableStateOf<DriveFile?>(null) }
    var renameValue by remember { mutableStateOf("") }
    var propsTarget by remember { mutableStateOf<DriveFile?>(null) }

    Column(modifier = Modifier.fillMaxSize()) {
        // 批量操作栏（选中>0 时；v2 05-file-ops：深色浮动条 h44 radius10，
        // margin 10/12/0 叠加 file-table 容器 padding 12 → 水平内缩 24）
        if (checkedCount > 0) {
            val bulkBusy = isIndexing
            Row(
                modifier = Modifier.fillMaxWidth()
                    .padding(start = 24.dp, top = 10.dp, end = 24.dp)
                    .height(44.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(Color(0xF01C1C1E))
                    .padding(start = 16.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text("已选 $checkedCount 项", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Color.White)
                Spacer(Modifier.weight(1f))
                BulkBarButton(label = "批量下载", icon = "download", disabled = bulkBusy,
                    onClick = { val s = files.filter { it.id in checked }; onDownload(s) })
                BulkBarButton(label = "释放空间", icon = "cloud",
                    onClick = { val s = files.filter { it.id in checked }; onFreeUp(s) })
                if (mountConfigured) {
                    BulkBarButton(label = "批量删除", icon = "trash", danger = true, disabled = bulkBusy,
                        onClick = { val s = files.filter { it.id in checked }; onDelete(s); checked = emptySet() })
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
            Column(modifier = Modifier.fillMaxWidth().weight(1f).padding(horizontal = 12.dp)) {
                // 表头（v2：38px，12.5sp semibold textSecondary，底部分隔线）
                Row(
                    modifier = Modifier.fillMaxWidth().height(38.dp).padding(horizontal = 12.dp),
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
                    HeaderCell("名称", browser.sortField == SortField.Name, browser.ascending, Modifier.weight(1f)) {
                        onSort(SortField.Name)
                    }
                    HeaderCell("大小", browser.sortField == SortField.Size, browser.ascending, Modifier.width(sizeWidth),
                        resizable = true, onResize = { delta -> sizeWidth = (sizeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                        onSort(SortField.Size)
                    }
                    HeaderCell("修改时间", browser.sortField == SortField.ModifiedTime, browser.ascending, Modifier.width(timeWidth),
                        resizable = true, onResize = { delta -> timeWidth = (timeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                        onSort(SortField.ModifiedTime)
                    }
                    // v2 列宽：状态 72 / 操作 44
                    Box(modifier = Modifier.width(72.dp), contentAlignment = Alignment.Center) {
                        Text("状态", fontSize = 12.5.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary)
                    }
                    Box(modifier = Modifier.width(44.dp), contentAlignment = Alignment.Center) {
                        Text("操作", fontSize = 12.5.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary)
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
                            onShowProps = { propsTarget = file; onShowProps(file) },
                            onDelete = { onDelete(listOf(file)) },
                            onFreeUp = { onFreeUp(listOf(file)) },
                            onCanFreeUp = onCanFreeUp,
                        )
                    }
                }
                // 底部信息（v2 file-footer：h36，13sp textPlaceholder）
                Box(modifier = Modifier.fillMaxWidth().height(36.dp), contentAlignment = Alignment.Center) {
                    Text("${files.size} 项 · 已全部加载", fontSize = 13.sp, color = semantic.textPlaceholder)
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
    val contentColor = if (danger) Color(0xFFFDA4AF) else Color.White.copy(alpha = 0.85f)
    val iconColor = if (danger) Color(0xFFFDA4AF) else Color.White.copy(alpha = 0.7f)
    val bg = when {
        disabled -> Color.Transparent
        hovered && danger -> Color(0xFFFDA4AF).copy(alpha = 0.18f)
        hovered -> Color.White.copy(alpha = 0.12f)
        else -> Color.Transparent
    }
    Row(
        modifier = Modifier.height(32.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(bg)
            .alpha(if (disabled) 0.4f else 1f)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, enabled = !disabled, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        MateIcon(name = icon, size = 16.dp, tint = iconColor)
        Text(label, fontSize = 15.sp, fontWeight = FontWeight.Medium, color = contentColor)
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
        modifier = Modifier.size(32.dp)
            .clip(CircleShape)
            .background(if (hovered) Color.White.copy(alpha = 0.12f) else Color.Transparent)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        MateIcon(name = "x", size = 16.dp, tint = if (hovered) Color.White else Color.White.copy(alpha = 0.7f))
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
            // v2：表头 12.5sp semibold textSecondary
            Text(title, fontSize = 12.5.sp, color = semantic.textSecondary, fontWeight = FontWeight.SemiBold)
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

/**
 * 文件行（v2：56px，radius 8，hover bgHover，selected BrandLighter，双击触发，右键菜单条件渲染）。
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
    onShowProps: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
    onCanFreeUp: (DriveFile, (Boolean) -> Unit) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    var menuExpanded by remember { mutableStateOf(false) }
    var canFree by remember { mutableStateOf<Boolean?>(null) }
    // v2：hover 态接入 hoverable（此前 hovered 变量未接线，hover 背景从未生效）
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val bg = when {
        selected -> BrandLighter
        hovered -> semantic.bgHover
        else -> Color.Transparent
    }
    // 双击检测：用 pointerInput detectTapGestures(onDoubleTap)
    Column {
        Row(
            modifier = Modifier.fillMaxWidth().height(56.dp)
                .clip(RoundedCornerShape(8.dp))
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
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // checkbox 列
            Box(modifier = Modifier.width(40.dp)) {
                if (showCheckbox) MateCheckbox(checked = checked, onCheckedChange = { c -> if (c != null) onCheckedChange(c) })
            }
            // name 列（v2：图标 32×32 色块 tile，间距 12）
            Row(modifier = Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                LaunchedEffect(file.id, file.thumbnailLink) { onThumbnailNeeded() }
                FileTypeTile(file = file, thumbnail = thumbnail)
                Text(file.displayName(), fontSize = 15.sp, color = semantic.textPrimary,
                    maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            // size 列
            Text(if (file.isFolder()) "—" else formatFileSize(file.sizeBytes), fontSize = 14.sp, color = semantic.textSecondary, modifier = Modifier.width(sizeWidth))
            // time 列
            Text(file.modifiedTime.orEmpty().replace("T", " ").take(16), fontSize = 14.sp, color = semantic.textSecondary, modifier = Modifier.width(timeWidth))
            // status 列（v2 列宽 72）
            Box(modifier = Modifier.width(72.dp), contentAlignment = Alignment.Center) {
                val (statusIcon, statusColor) = when (status) {
                    "synced" -> "local" to SuccessColor
                    "placeholder" -> "cloud" to semantic.textSecondary
                    "folder" -> "folder" to BrandColor
                    else -> "cloud" to semantic.textPlaceholder
                }
                MateIcon(name = statusIcon, size = 16.dp, tint = statusColor)
            }
            // actions 列（v2 列宽 44；操作按钮 → 右键菜单，锚点为本 Box）
            Box(modifier = Modifier.width(44.dp), contentAlignment = Alignment.Center) {
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
                                .width(200.dp)
                                .shadow(16.dp, RoundedCornerShape(10.dp))
                                .clip(RoundedCornerShape(10.dp))
                                .background(semantic.bgContainer)
                                .border(0.5.dp, semantic.border, RoundedCornerShape(10.dp))
                                .padding(6.dp),
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
    val semantic = LocalSemanticColors.current
    val type = fileTypeIcon(file)
    if (thumbnail != null && !file.isFolder()) {
        val bitmap = remember(thumbnail) {
            runCatching { org.jetbrains.skia.Image.makeFromEncoded(thumbnail).toComposeImageBitmap() }.getOrNull()
        }
        if (bitmap != null) {
            androidx.compose.foundation.Image(bitmap, null, Modifier.size(32.dp).clip(RoundedCornerShape(6.dp)))
            return
        }
    }
    val (bg, tint) = when (type) {
        "folder" -> FolderAmberBg to FolderAmber
        "file-text" -> TileDocBg to TileDoc
        "image" -> TileImageBg to TileImage
        "video" -> TileVideoBg to TileVideo
        "chart" -> TileSheetBg to TileSheet
        else -> semantic.bgFill to semantic.textSecondary
    }
    Box(
        modifier = Modifier.size(32.dp).clip(RoundedCornerShape(6.dp)).background(bg),
        contentAlignment = Alignment.Center,
    ) {
        MateIcon(name = type, size = 18.dp, tint = tint)
    }
}

/**
 * 右键菜单项（v2 .menu__item：h36 radius8，hover bgFill，padding 0 12，gap 10；
 * icon 16（默认 textSecondary，danger ErrorColor），文字 15sp（默认 textPrimary，danger ErrorColor）；
 * enabled=false 时文字/图标 textPlaceholder 且不响应点击与 hover。
 */
@Composable
private fun CtxItem(label: String, icon: String, danger: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val semantic = LocalSemanticColors.current
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val contentColor = when {
        !enabled -> semantic.textPlaceholder
        danger -> ErrorColor
        else -> semantic.textPrimary
    }
    val iconColor = when {
        !enabled -> semantic.textPlaceholder
        danger -> ErrorColor
        else -> semantic.textSecondary
    }
    Row(
        modifier = Modifier.fillMaxWidth().height(36.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(if (hovered && enabled) semantic.bgFill else Color.Transparent)
            .hoverable(interaction)
            .clickable(interactionSource = interaction, indication = null, enabled = enabled, onClick = onClick)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        MateIcon(name = icon, size = 16.dp, tint = iconColor)
        Text(label, color = contentColor, fontSize = 15.sp)
    }
}

/**
 * 右键菜单分隔线（v2 .menu__sep：0.5px，margin 8/4，bg border）。
 */
@Composable
private fun CtxDivider() {
    Box(
        modifier = Modifier.fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 4.dp)
            .height(0.5.dp)
            .background(LocalSemanticColors.current.border),
    )
}

/**
 * 重命名对话框（对标原 Vue MateDialog 重命名）。
 */
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
