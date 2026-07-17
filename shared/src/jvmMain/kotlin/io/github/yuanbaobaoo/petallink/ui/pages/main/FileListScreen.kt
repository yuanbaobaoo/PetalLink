@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
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
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
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
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
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
 * 6 列：checkbox(40) / name(flex) / size(100 可拖 64-400) / time(150 可拖) / status(60) / actions(40)；
 * 表头 36px bg-hover 底 1px border；行 44px 底 0.5px border；hover bg-hover，选中 brand-lighter。
 * 批量操作栏（选中>0 时，36px brand-lighter 底）；右键菜单（min-width 168，视口钳制）。
 *
 * @param browser 文件浏览器状态
 * @param fileStatuses fileId → 同步状态（synced/placeholder/folder/not_synced）
 * @param thumbnails fileId → 缩略图字节
 * @param onSort 排序
 * @param onEnterFolder 进入文件夹
 * @param onOpenItem 打开文件
 * @param onThumbnailNeeded 需要加载缩略图
 * @param onDelete 批量/单个删除
 * @param onFreeUp 批量/单个释放空间
 * @param onDownload 批量/单个下载
 * @param onSyncFolder 双端对齐
 * @param onRename 重命名
 * @param onShowProps 属性
 */
@Composable
fun FileListScreen(
    browser: FileBrowserState,
    fileStatuses: Map<String, String>,
    thumbnails: Map<String, ByteArray>,
    onSort: (BrowserSortField) -> Unit,
    onEnterFolder: (DriveFile) -> Unit,
    onOpenItem: (DriveFile) -> Unit,
    onThumbnailNeeded: (DriveFile) -> Unit,
    onDelete: (List<DriveFile>) -> Unit,
    onFreeUp: (List<DriveFile>) -> Unit,
    onDownload: (List<DriveFile>) -> Unit,
    onSyncFolder: (DriveFile) -> Unit,
    onRename: (DriveFile) -> Unit,
    onShowProps: (DriveFile) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    var checked by remember(browser.folderId) { mutableStateOf<Set<String>>(emptySet()) }
    var showCheckboxes by remember { mutableStateOf(false) }
    var sizeWidth by remember { mutableStateOf(100.dp) }
    var timeWidth by remember { mutableStateOf(150.dp) }
    val files = browser.visibleFiles
    val checkedCount = checked.size

    Column(modifier = Modifier.fillMaxSize()) {
        // 批量操作栏（选中>0 时，36px brand-lighter 底）
        if (checkedCount > 0) {
            Row(
                modifier = Modifier.fillMaxWidth().height(36.dp)
                    .background(Color(0xFFF2F3FF))
                    .padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("已选 $checkedCount 项", fontSize = 13.sp, fontWeight = FontWeight.Medium, color = BrandColor)
                Spacer(Modifier.weight(1f))
                MateButton(label = "批量下载", variant = MateButtonVariant.TEXT, icon = "download", onClick = {
                    val selected = files.filter { it.id in checked }; onDownload(selected)
                })
                MateButton(label = "释放空间", variant = MateButtonVariant.TEXT, icon = "cloud", onClick = {
                    val selected = files.filter { it.id in checked }; onFreeUp(selected)
                })
                MateButton(label = "批量删除", variant = MateButtonVariant.TEXT, icon = "trash", danger = true, onClick = {
                    val selected = files.filter { it.id in checked }; onDelete(selected); checked = emptySet()
                })
                MateButton(variant = MateButtonVariant.ICON, icon = "x", onClick = { checked = emptySet(); showCheckboxes = false })
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
                // checkbox 列 40px
                Box(modifier = Modifier.width(40.dp), contentAlignment = Alignment.CenterStart) {
                    if (showCheckboxes) {
                        val headerCheck: Boolean? = if (checkedCount == 0) false
                        else if (checkedCount == files.size) true
                        else null
                        MateCheckbox(checked = headerCheck, onCheckedChange = { _ ->
                            checked = if (checkedCount == files.size) emptySet()
                            else files.mapNotNull { it.id }.toSet()
                        })
                    } else {
                        MateButton(variant = MateButtonVariant.ICON, icon = "check", onClick = { showCheckboxes = true })
                    }
                }
                // name 列（flex）
                HeaderCell("名称", browser.sortField == BrowserSortField.NAME, browser.ascending, Modifier.weight(1f)) {
                    onSort(BrowserSortField.NAME)
                }
                // size 列（可拖拽）
                HeaderCell("大小", browser.sortField == BrowserSortField.SIZE, browser.ascending, Modifier.width(sizeWidth), resizable = true, onResize = { delta -> sizeWidth = (sizeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                    onSort(BrowserSortField.SIZE)
                }
                // time 列（可拖拽）
                HeaderCell("修改时间", browser.sortField == BrowserSortField.MODIFIED_TIME, browser.ascending, Modifier.width(timeWidth), resizable = true, onResize = { delta -> timeWidth = (timeWidth + delta).coerceIn(64.dp, 400.dp) }) {
                    onSort(BrowserSortField.MODIFIED_TIME)
                }
                Text("状态", fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.width(60.dp))
                Text("操作", fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.width(40.dp))
            }
            // 文件行
            LazyColumn(modifier = Modifier.weight(1f)) {
                items(files, key = { it.id ?: "${it.name}-${it.editedTime}" }) { file ->
                    FileRow(
                        file = file,
                        checked = file.id in checked,
                        showCheckbox = showCheckboxes,
                        status = fileStatuses[file.id] ?: "not_synced",
                        thumbnail = file.id?.let(thumbnails::get),
                        sizeWidth = sizeWidth,
                        timeWidth = timeWidth,
                        onCheckedChange = { c ->
                            val id = file.id ?: return@FileRow
                            checked = if (c) checked + id else checked - id
                        },
                        onDoubleClick = { if (file.isFolder()) onEnterFolder(file) else onOpenItem(file) },
                        onThumbnailNeeded = { onThumbnailNeeded(file) },
                        onSync = { onSyncFolder(file) },
                        onRename = { onRename(file) },
                        onShowProps = { onShowProps(file) },
                        onDelete = { onDelete(listOf(file)) },
                        onFreeUp = { onFreeUp(listOf(file)) },
                    )
                }
            }
            // 底部信息
            Box(
                modifier = Modifier.fillMaxWidth().height(32.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text("${files.size} 项 · 已全部加载", fontSize = 12.sp, color = semantic.textSecondary)
            }
        }
    }
}

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
                MateIcon(
                    name = "arrow",
                    size = 12.dp,
                    tint = semantic.textSecondary,
                    modifier = Modifier.rotate(if (ascending) 0f else 90f),
                )
            }
        }
        if (resizable && onResize != null) {
            // resize-handle：右侧 6px col-resize 热区
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

/** 文件行（44px）。 */
@Composable
private fun FileRow(
    file: DriveFile,
    checked: Boolean,
    showCheckbox: Boolean,
    status: String,
    thumbnail: ByteArray?,
    sizeWidth: androidx.compose.ui.unit.Dp,
    timeWidth: androidx.compose.ui.unit.Dp,
    onCheckedChange: (Boolean) -> Unit,
    onDoubleClick: () -> Unit,
    onThumbnailNeeded: () -> Unit,
    onSync: () -> Unit,
    onRename: () -> Unit,
    onShowProps: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    Row(
        modifier = Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // checkbox 列
        Box(modifier = Modifier.width(40.dp)) {
            if (showCheckbox) MateCheckbox(checked = checked, onCheckedChange = { c -> if (c != null) onCheckedChange(c) })
        }
        // name 列（图标/缩略图 + 名称）
        Row(
            modifier = Modifier.weight(1f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // 缩略图或图标
            // 触发缩略图加载
            androidx.compose.runtime.LaunchedEffect(file.id, file.thumbnailLink) { onThumbnailNeeded() }
            if (thumbnail != null && !file.isFolder()) {
                // 缩略图渲染（20×20，radius 3）
                val bitmap = remember(thumbnail) {
                    runCatching {
                        org.jetbrains.skia.Image.makeFromEncoded(thumbnail).toComposeImageBitmap()
                    }.getOrNull()
                }
                if (bitmap != null) {
                    androidx.compose.foundation.Image(bitmap, null, Modifier.width(20.dp).height(20.dp).clip(RoundedCornerShape(3.dp)))
                } else {
                    MateIcon(name = fileTypeIcon(file), size = 20.dp, tint = if (file.isFolder()) BrandColor else semantic.textPlaceholder)
                }
            } else {
                MateIcon(name = fileTypeIcon(file), size = 20.dp, tint = if (file.isFolder()) BrandColor else semantic.textPlaceholder)
            }
            Text(
                file.name ?: file.fileName ?: "未命名",
                fontSize = 14.sp,
                color = semantic.textPrimary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                    onClick = onDoubleClick,
                ),
            )
        }
        // size 列
        Text(
            if (file.isFolder()) "—" else formatFileSize(file.sizeBytes),
            fontSize = 13.sp,
            color = semantic.textSecondary,
            modifier = Modifier.width(sizeWidth),
        )
        // time 列
        Text(
            file.modifiedTime.orEmpty().replace("T", " ").take(16),
            fontSize = 13.sp,
            color = semantic.textSecondary,
            modifier = Modifier.width(timeWidth),
        )
        // status 列（60px）
        Box(modifier = Modifier.width(60.dp), contentAlignment = Alignment.Center) {
            val (statusIcon, statusColor) = when (status) {
                "synced" -> "local" to SuccessColor
                "placeholder" -> "cloud" to semantic.textSecondary
                "folder" -> "folder" to BrandColor
                else -> "cloud" to semantic.textPlaceholder
            }
            MateIcon(name = statusIcon, size = 16.dp, tint = statusColor)
        }
        // actions 列（40px）
        Box(modifier = Modifier.width(40.dp), contentAlignment = Alignment.Center) {
            FileContextMenu(
                file = file,
                onSync = onSync,
                onRename = onRename,
                onShowProps = onShowProps,
                onDelete = onDelete,
                onFreeUp = onFreeUp,
            )
        }
    }
}

/** 文件右键菜单/操作按钮（对标原 Vue ctx-menu）。 */
@Composable
private fun FileContextMenu(
    file: DriveFile,
    onSync: () -> Unit,
    onRename: () -> Unit,
    onShowProps: () -> Unit,
    onDelete: () -> Unit,
    onFreeUp: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    var expanded by remember { mutableStateOf(false) }
    Box {
        MateButton(variant = MateButtonVariant.ICON, icon = "list", onClick = { expanded = true })
        androidx.compose.material.DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            // Compose DropdownMenu 自带窗口边界钳制，等价原前端 8px 视口钳制
            FileMenuItem("执行双端对齐", "sync", onClick = { expanded = false; onSync() })
            FileMenuItem("释放空间", "cloud", onClick = { expanded = false; onFreeUp() })
            FileMenuItem("重命名", "edit", onClick = { expanded = false; onRename() })
            FileMenuItem("属性", "info", onClick = { expanded = false; onShowProps() })
            FileMenuItem("删除", "trash", danger = true, onClick = { expanded = false; onDelete() })
        }
    }
}

@Composable
private fun FileMenuItem(label: String, icon: String, danger: Boolean = false, onClick: () -> Unit) {
    val semantic = LocalSemanticColors.current
    androidx.compose.material.DropdownMenuItem(
        onClick = onClick,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            MateIcon(name = icon, size = 16.dp, tint = if (danger) ErrorColor else semantic.textPrimary)
            Text(label, color = if (danger) ErrorColor else semantic.textPrimary, fontSize = 14.sp)
        }
    }
}
