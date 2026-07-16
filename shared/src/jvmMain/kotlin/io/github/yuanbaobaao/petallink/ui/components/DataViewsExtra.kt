package io.github.yuanbaobaao.petallink.ui.components

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaao.petallink.ui.theme.*

// ========== Sidebar 递归树 ==========
data class SidebarTreeNode(
    val name: String,
    val fileId: String,
    val isFolder: Boolean,
    val children: List<SidebarTreeNode> = emptyList(),
    val indent: Int = 0,
)

@Composable
fun SidebarTree(
    nodes: List<SidebarTreeNode>,
    selectedFileId: String?,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.verticalScroll(rememberScrollState())) {
        nodes.forEach { node -> SidebarTreeNodeRow(node, selectedFileId, onSelect) }
    }
}

@Composable
private fun SidebarTreeNodeRow(
    node: SidebarTreeNode,
    selectedFileId: String?,
    onSelect: (String) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    val hasChildren = node.children.isNotEmpty()
    val isSelected = node.fileId == selectedFileId

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable {
                if (hasChildren) expanded = !expanded
                onSelect(node.fileId)
            }
            .background(if (isSelected) BrandColor.copy(alpha = 0.08f) else Color.Transparent)
            .padding(start = (node.indent * 16 + 8).dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // expand/collapse 箭头
        if (hasChildren) {
            Text(
                if (expanded) "▾" else "▸",
                fontSize = 12.sp,
                color = Color.Gray,
                modifier = Modifier.width(16.dp),
            )
        } else {
            Spacer(Modifier.width(16.dp))
        }
        // 图标
        Text(if (node.isFolder) "📁" else "📄", fontSize = 14.sp)
        Spacer(Modifier.width(6.dp))
        // 名称
        Text(
            node.name,
            fontSize = 13.sp,
            fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Normal,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
    }
    // 子节点（expanded 时递归渲染）
    if (expanded && hasChildren) {
        node.children.forEach { child ->
            SidebarTreeNodeRow(child, selectedFileId, onSelect)
        }
    }
}

// ========== 增强 FileListRow（含 tri-state checkbox + 右键菜单） ==========
@Composable
fun FileListRowEnhanced(
    item: io.github.yuanbaobaao.petallink.ui.components.FileListItem,
    isSelected: Boolean,
    onClick: () -> Unit,
    checked: Boolean,
    onCheckChange: (Boolean) -> Unit,
    contextMenuItems: List<Pair<String, () -> Unit>> = emptyList(),
) {
    var showMenu by remember { mutableStateOf(false) }

    Box {
        Surface(
            modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
            color = if (isSelected) BrandColor.copy(alpha = 0.08f) else Color.Transparent,
        ) {
            Row(
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // tri-state checkbox
                Checkbox(
                    checked = checked,
                    onCheckedChange = onCheckChange,
                    modifier = Modifier.size(20.dp),
                )
                Spacer(Modifier.width(4.dp))
                // 图标
                Text(iconFor(item), fontSize = 16.sp)
                Spacer(Modifier.width(6.dp))
                // 文件名
                Text(item.name, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(2f))
                // 大小
                Text(if (item.isFolder) "—" else formatBytes(item.size), fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(80.dp))
                // 修改时间
                Text(item.modifiedTime, fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(140.dp))
                // 状态
                Text(statusIcon(item.status), fontSize = 13.sp, modifier = Modifier.width(60.dp))
                // 右键菜单按钮
                if (contextMenuItems.isNotEmpty()) {
                    IconButton(onClick = { showMenu = true }, modifier = Modifier.size(24.dp)) {
                        Text("⋯", fontSize = 14.sp)
                    }
                }
            }
        }
        // 右键菜单（DropdownMenu）
        DropdownMenu(expanded = showMenu, onDismissRequest = { showMenu = false }) {
            contextMenuItems.forEach { (label, action) ->
                DropdownMenuItem(onClick = { showMenu = false; action() }) {
                    Text(label, fontSize = 13.sp)
                }
            }
        }
    }
}

/** 文件图标 */
private fun iconFor(item: io.github.yuanbaobaao.petallink.ui.components.FileListItem): String = when {
    item.isFolder -> "📁"
    item.status == io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.PLACEHOLDER -> "☁️"
    item.status == io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.ERROR -> "❌"
    else -> "📄"
}

/** 状态图标 */
private fun statusIcon(status: io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus): String = when (status) {
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.SYNCED -> "✅"
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.SYNCING -> "🔄"
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.OFFLINE -> "⏸️"
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.PLACEHOLDER -> "☁️"
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.ERROR -> "⚠️"
    io.github.yuanbaobaao.petallink.ui.components.FileSyncStatus.DELETED -> "🗑️"
}
