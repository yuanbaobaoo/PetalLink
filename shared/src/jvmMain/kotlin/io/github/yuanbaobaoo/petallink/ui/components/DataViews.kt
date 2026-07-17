package io.github.yuanbaobaoo.petallink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.theme.*

// ========== 传输弹窗 ==========
data class TransferPopoverItem(
    val id: Long,
    val fileName: String,
    val direction: TransferDirection,
    val progress: Float,       // 0..1
    val stateText: String,
    val bytesDone: Long,
    val bytesTotal: Long,
    val errorMessage: String?,
)

enum class TransferDirection { UPLOAD, DOWNLOAD }

@OptIn(ExperimentalMaterialApi::class)
@Composable
fun TransferPopover(
    items: List<TransferPopoverItem>,
    visible: Boolean,
    onDismiss: () -> Unit,
    onRetry: ((Long) -> Unit)? = null,
    onClearCompleted: (() -> Unit)? = null,
) {
    if (!visible) return
    Box(modifier = Modifier.fillMaxSize()) {
        // 半透明背景（点击关闭）
        Surface(modifier = Modifier.fillMaxSize(), color = Color.Black.copy(alpha = 0.3f), onClick = onDismiss) {}

        // 弹窗内容
        Card(
            modifier = Modifier.align(Alignment.BottomEnd).width(360.dp).heightIn(max = 480.dp).padding(16.dp),
            shape = RoundedCornerShape(12.dp),
            elevation = 8.dp,
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                // 标题栏
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text("传输任务", fontSize = 15.sp, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                    val activeCount = items.count { it.progress < 1f }
                    if (activeCount > 0) MateStatChip(activeCount, "进行中")
                    Spacer(Modifier.width(8.dp))
                    TextButton(onClick = { onClearCompleted?.invoke(); onDismiss() }) {
                        Text("清除已完成", fontSize = 12.sp)
                    }
                }
                Spacer(Modifier.height(8.dp))
                Divider()

                if (items.isEmpty()) {
                    Box(modifier = Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
                        Text("暂无传输任务", color = Color.Gray, fontSize = 13.sp)
                    }
                } else {
                    LazyColumn {
                        items(items) { item ->
                            TransferPopoverItemRow(item, onRetry)
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun TransferPopoverItemRow(item: TransferPopoverItem, onRetry: ((Long) -> Unit)?) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp).clickable(enabled = false) {},
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (item.direction == TransferDirection.UPLOAD) "⬆️" else "⬇️",
                fontSize = 14.sp,
            )
            Spacer(Modifier.width(8.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(item.fileName, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    item.stateText,
                    fontSize = 11.sp,
                    color = when {
                        item.errorMessage != null -> ErrorColor
                        item.progress >= 1f -> SuccessColor
                        else -> Color.Gray
                    },
                )
            }
            Text(
                if (item.bytesTotal > 0) formatBytes(item.bytesDone) + " / " + formatBytes(item.bytesTotal)
                else "",
                fontSize = 11.sp,
                color = Color.Gray,
            )
        }
        // 进度条
        if (item.progress < 1f) {
            Spacer(Modifier.height(4.dp))
            LinearProgressIndicator(progress = item.progress, modifier = Modifier.fillMaxWidth().height(3.dp))
        }
        // 错误消息 + 重试按钮
        if (item.errorMessage != null && onRetry != null) {
            Spacer(Modifier.height(2.dp))
            Row {
                Text(item.errorMessage, fontSize = 11.sp, color = ErrorColor, maxLines = 1, modifier = Modifier.weight(1f))
                TextButton(onClick = { onRetry(item.id) }) {
                    Text("重试", fontSize = 11.sp)
                }
            }
        }
        Divider(modifier = Modifier.padding(top = 4.dp))
    }
}

// ========== 文件列表视图 ==========
enum class FileColumn { NAME, SIZE, MODIFIED, STATUS, OWNER, ACTIONS }

data class FileListItem(
    val relativePath: String,
    val name: String,
    val isFolder: Boolean,
    val size: Long,
    val modifiedTime: String,  // 格式化后的日期
    val status: FileSyncStatus,
    val cloudFileId: String?,
)

enum class FileSyncStatus { SYNCED, SYNCING, OFFLINE, PLACEHOLDER, ERROR, DELETED }

@Composable
fun FileListHeader(
    sortColumn: FileColumn,
    onSortClick: (FileColumn) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth().background(Color(0xFFF5F5F5)).padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        HeaderCell("名称", FileColumn.NAME, sortColumn, onSortClick, Modifier.weight(2f))
        HeaderCell("大小", FileColumn.SIZE, sortColumn, onSortClick, Modifier.width(80.dp))
        HeaderCell("修改时间", FileColumn.MODIFIED, sortColumn, onSortClick, Modifier.width(140.dp))
        HeaderCell("状态", FileColumn.STATUS, sortColumn, onSortClick, Modifier.width(60.dp))
    }
}

@Composable
private fun HeaderCell(
    title: String,
    column: FileColumn,
    sortColumn: FileColumn,
    onSortClick: (FileColumn) -> Unit,
    modifier: Modifier,
) {
    Row(
        modifier = modifier.clickable { onSortClick(column) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, fontSize = 12.sp, color = Color(0xFF888888))
        if (sortColumn == column) Text(" ▼", fontSize = 10.sp, color = BrandColor)
    }
}

@Composable
fun FileListRow(
    item: FileListItem,
    isSelected: Boolean,
    onClick: () -> Unit,
    onContextMenu: (() -> Unit)? = null,
) {
    val bgColor = if (isSelected) BrandColor.copy(alpha = 0.08f) else Color.Transparent
    Surface(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        color = bgColor,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // 图标
            Text(
                when {
                    item.isFolder -> "📁"
                    item.status == FileSyncStatus.PLACEHOLDER -> "☁️"
                    item.status == FileSyncStatus.SYNCING -> "🔄"
                    item.status == FileSyncStatus.ERROR -> "❌"
                    else -> "📄"
                },
                fontSize = 16.sp,
            )
            Spacer(Modifier.width(8.dp))

            // 文件名
            Text(
                item.name,
                fontSize = 13.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(2f),
                color = if (item.status == FileSyncStatus.DELETED) Color.Gray else Color(0xFF333333),
            )

            // 大小
            Text(
                if (item.isFolder) "—" else formatBytes(item.size),
                fontSize = 12.sp,
                color = Color.Gray,
                modifier = Modifier.width(80.dp),
            )

            // 修改时间
            Text(
                item.modifiedTime,
                fontSize = 12.sp,
                color = Color.Gray,
                modifier = Modifier.width(140.dp),
            )

            // 状态指示
            val (statusIcon, statusColor) = when (item.status) {
                FileSyncStatus.SYNCED -> "✅" to SuccessColor
                FileSyncStatus.SYNCING -> "🔄" to BrandColor
                FileSyncStatus.OFFLINE -> "⏸️" to Color.Gray
                FileSyncStatus.PLACEHOLDER -> "☁️" to Color.Gray
                FileSyncStatus.ERROR -> "⚠️" to ErrorColor
                FileSyncStatus.DELETED -> "🗑️" to Color.Gray
            }
            Text(statusIcon, fontSize = 13.sp, modifier = Modifier.width(60.dp))
        }
    }
}

/** 文件大小格式化（对标原项目前端 formatFileSize） */
fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val kb = bytes / 1024.0
    if (kb < 1024) return "%.1f KB".format(kb)
    val mb = kb / 1024.0
    if (mb < 1024) return "%.1f MB".format(mb)
    val gb = mb / 1024.0
    return "%.2f GB".format(gb)
}
