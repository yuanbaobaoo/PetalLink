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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateAppLogo
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLinearProgress
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandHover
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.drive.DriveFile

/**
 * 侧边栏（对标原 Vue Sidebar.vue）。
 *
 * 宽 220px，bg-page，右 0.5px border。三段式纵向 flex：
 * 1. Logo 区（56px 高，padding 0/16）
 * 2. 目录树（flex:1 scroll，padding 4/8）
 * 3. 账号栏（顶 0.5px border，padding 16，gap 12；28×28 圆形渐变头像 + 用户名 + 配额）
 *
 * @param rootChildren 根目录子文件夹列表
 * @param directoryChildren 各文件夹 ID → 子文件夹列表
 * @param selectedFolderId 当前选中文件夹 ID
 * @param userName 用户显示名
 * @param quotaText 配额文本（如 "1.2 GB / 5 GB"）
 * @param onNavigate 点击目录树节点导航
 */
@Composable
fun Sidebar(
    rootChildren: List<DriveFile>,
    directoryChildren: Map<String, List<DriveFile>>,
    selectedFolderId: String?,
    userName: String?,
    quotaText: String?,
    updateDownloading: Boolean,
    updateDownloadProgress: Float,
    updateAvailableVersion: String?,
    onDismissUpdate: () -> Unit,
    onNavigate: (DriveFile) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    Column(
        modifier = Modifier
            .width(220.dp)
            .fillMaxHeight()
            .background(semantic.bgPage)
            .then(Modifier.drawBehindBorder(semantic.border, isRight = true)),
    ) {
        // 1. Logo 区（56px）
        Box(
            modifier = Modifier.height(56.dp).padding(horizontal = 16.dp),
            contentAlignment = Alignment.CenterStart,
        ) { MateAppLogo(size = 26.dp) }

        // 2. 目录树（flex:1 scroll）
        Column(
            modifier = Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 8.dp, vertical = 4.dp),
        ) {
            SidebarTreeNode(
                folder = DriveFile(id = null, name = "全部文件", category = "folder"),
                depth = 0,
                selectedId = selectedFolderId,
                children = rootChildren,
                directoryChildren = directoryChildren,
                onSelect = onNavigate,
            )
        }

        // 账号栏顶分隔线
        MateHDivider()

        // 3. 账号栏（padding 16，gap 12）
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // 28×28 圆形渐变头像（品牌色，白色 initial 占位字，无 padding）
            Box(
                modifier = Modifier.size(28.dp).clip(CircleShape)
                    .background(Brush.linearGradient(listOf(BrandColor, BrandHover))),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    userName?.firstOrNull()?.toString() ?: "华",
                    color = Color.White,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    userName ?: "加载账号中…",
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                    color = semantic.textPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (quotaText != null) {
                    Text(
                        quotaText,
                        fontSize = 12.sp,
                        color = semantic.textSecondary,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }

        // 更新下载进度条（对标原 Vue .sidebar__update-progress）
        if (updateDownloading) {
            SidebarUpdateProgress(updateDownloadProgress)
        }
        // 更新提示横幅（对标原 Vue .sidebar__update-banner）
        if (updateAvailableVersion != null) {
            SidebarUpdateBanner(updateAvailableVersion, onDismissUpdate)
        }
    }
}

/** 更新下载进度条（账号栏下方，4px 渐变，点击重开 dialog）。 */
@Composable
private fun SidebarUpdateProgress(progress: Float) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("正在下载更新", fontSize = 12.sp, color = semantic.textSecondary)
            Text("${(progress * 100).toInt()}%", fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = BrandColor)
        }
        Spacer(Modifier.height(6.dp))
        MateLinearProgress(value = progress)
    }
}

/** 更新提示横幅（135° 品牌渐变，日志/× 按钮）。 */
@Composable
private fun SidebarUpdateBanner(version: String, onDismiss: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(6.dp))
            .background(Brush.linearGradient(listOf(BrandColor, BrandHover)))
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("新版本 $version", fontSize = 12.sp, fontWeight = FontWeight.Medium, color = Color.White, modifier = Modifier.weight(1f))
        // × 关闭按钮（18×18 圆形半透明白）
        Box(
            modifier = Modifier.size(18.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.25f))
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
            contentAlignment = Alignment.Center,
        ) {
            Text("×", color = Color.White, fontSize = 14.sp)
        }
    }
}

/**
 * 递归目录树节点（对标原 Vue SidebarTreeNode.vue）。
 *
 * 行高 28px，缩进 depth*14+8，gap 8，radius 3；
 * chevron(16px 宽，arrow 图标展开 rotate 90°)；文件夹图标 16px brand；名称 13px；
 * 三态：默认 secondary / hover bg-hover / 选中 brand-lighter+brand+medium。
 */
@Composable
private fun SidebarTreeNode(
    folder: DriveFile,
    depth: Int,
    selectedId: String?,
    children: List<DriveFile>,
    directoryChildren: Map<String, List<DriveFile>>,
    onSelect: (DriveFile) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    val isSelected = folder.id == selectedId
    var expanded by remember(folder.id) { mutableStateOf(depth == 0) }
    val name = folder.name ?: folder.fileName ?: "未命名"

    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(28.dp)
                .padding(start = (depth * 14 + 8).dp, end = 8.dp)
                .clip(RoundedCornerShape(3.dp))
                .background(if (isSelected) Color(0xFFF2F3FF) else Color.Transparent)
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) {
                    expanded = true
                    onSelect(folder)
                }
                .padding(end = 0.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // chevron（16px 宽命中区，arrow 图标展开 rotate 90°）
            Box(
                modifier = Modifier.size(16.dp),
                contentAlignment = Alignment.Center,
            ) {
                MateIcon(
                    name = "arrow",
                    size = 12.dp,
                    tint = semantic.textSecondary,
                    modifier = Modifier.rotate(if (expanded) 90f else 0f),
                )
            }
            // 文件夹图标（16px brand）
            MateIcon(name = "folder", size = 16.dp, tint = BrandColor)
            // 名称（13px，选中 brand+medium，默认 secondary）
            Text(
                name,
                fontSize = 13.sp,
                fontWeight = if (isSelected) FontWeight.Medium else FontWeight.Normal,
                color = if (isSelected) BrandColor else semantic.textSecondary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        // 递归子节点
        if (expanded) {
            children.forEach { child ->
                val childId = child.id ?: return@forEach
                SidebarTreeNode(
                    folder = child,
                    depth = depth + 1,
                    selectedId = selectedId,
                    children = directoryChildren[childId].orEmpty(),
                    directoryChildren = directoryChildren,
                    onSelect = onSelect,
                )
            }
        }
    }
}

/** 在 Modifier 上绘制 0.5px 边框线（右边或底边），对标 CSS border-right/bottom。 */
private fun Modifier.drawBehindBorder(color: Color, isRight: Boolean = false, isBottom: Boolean = false): Modifier =
    this.drawWithContent {
        drawContent()
        if (isRight) {
            drawRect(
                color = color,
                topLeft = Offset(size.width - 0.5f, 0f),
                size = Size(0.5f, size.height),
            )
        }
        if (isBottom) {
            drawRect(
                color = color,
                topLeft = Offset(0f, size.height - 0.5f),
                size = Size(size.width, 0.5f),
            )
        }
    }
