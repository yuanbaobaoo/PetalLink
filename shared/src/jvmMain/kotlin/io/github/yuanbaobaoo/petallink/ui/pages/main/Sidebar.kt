@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateAppLogo
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLinearProgress
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradient
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.FolderAmber
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.drive.DriveFile

/**
 * 侧边栏（v2：design/v2/02-main.html .sidebar）。
 *
 * 宽 248px，bg-page，右 0.5px border。纵向三段：
 * 1. Logo 区（60px 高，padding 0/18）
 * 2. section 标签「位置」（12sp semibold textPlaceholder，padding 12/18/6）
 * 3. 目录树（flex:1 scroll，padding 4/8）
 * 底部：悬浮账号卡（margin 10，bg-container radius 10 + 0.5px border，padding 12；
 * 32×32 圆形 BrandGradient 渐变头像 + 用户名 14sp semibold + 配额 12.5sp secondary + 4px 配额进度条），
 * 以及更新卡片（渐变底，见 [SidebarUpdateProgress] / [SidebarUpdateBanner]）。
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
    onInstallUpdate: () -> Unit = {},
    onNavigate: (DriveFile) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    Column(
        modifier = Modifier
            .width(248.dp)
            .fillMaxHeight()
            .background(semantic.bgPage)
            .then(Modifier.drawBehindBorder(semantic.border, isRight = true)),
    ) {
        // 1. Logo 区（60px，padding 0/18）
        Box(
            modifier = Modifier.height(60.dp).padding(horizontal = 18.dp),
            contentAlignment = Alignment.CenterStart,
        ) { MateAppLogo(size = 26.dp) }

        // 2. section 标签「位置」（12sp semibold textPlaceholder，padding 12/18/6）
        Text(
            "位置",
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
            color = semantic.textPlaceholder,
            letterSpacing = 0.4.sp,
            modifier = Modifier.padding(start = 18.dp, top = 12.dp, bottom = 6.dp),
        )

        // 3. 目录树（flex:1 scroll）
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

        // 4. 悬浮账号卡（margin 10，bg-container radius 10 + 0.5px border，padding 12，gap 10）
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(10.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(semantic.bgContainer)
                .border(0.5.dp, semantic.border, RoundedCornerShape(10.dp))
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            // 32×32 圆形 BrandGradient 渐变头像（白色 initial 占位字）
            Box(
                modifier = Modifier.size(32.dp).clip(CircleShape).background(BrandGradient),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    userName?.firstOrNull()?.toString() ?: "华",
                    color = Color.White,
                    fontSize = 15.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    userName ?: "加载账号中…",
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = semantic.textPrimary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (quotaText != null) {
                    Text(
                        quotaText,
                        fontSize = 12.5.sp,
                        color = semantic.textSecondary,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(top = 1.dp),
                    )
                    // 配额进度条（4px，品牌渐变自动；比例从 quotaText 解析，失败则不显示）
                    val quotaRatio = remember(quotaText) { parseQuotaRatio(quotaText) }
                    if (quotaRatio != null) {
                        Spacer(Modifier.height(6.dp))
                        MateLinearProgress(value = quotaRatio, height = 4.dp)
                    }
                }
            }
        }

        // 更新下载进度卡（v2 .sidebar__update 渐变卡片）
        if (updateDownloading) {
            SidebarUpdateProgress(updateDownloadProgress)
        }
        // 更新提示卡（v2 .sidebar__update 渐变卡片）
        if (updateAvailableVersion != null) {
            SidebarUpdateBanner(updateAvailableVersion, onDismissUpdate, onInstallUpdate)
        }
    }
}

/** 更新下载进度卡（v2：margin 0/10/10，BrandGradient 底 radius 10 padding 12，白字 + 白色进度条）。 */
@Composable
private fun SidebarUpdateProgress(progress: Float) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 10.dp, end = 10.dp, bottom = 10.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(BrandGradient)
            .padding(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("正在下载更新", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Color.White)
            Text("${(progress * 100).toInt()}%", fontSize = 14.sp, fontWeight = FontWeight.Bold, color = Color.White)
        }
        Spacer(Modifier.height(8.dp))
        MateLinearProgress(value = progress, color = Color.White)
    }
}

/** 更新提示卡（v2：margin 0/10/10，BrandGradient 底 radius 10 padding 12，白字标题 + 圆形半透明 × + 白底「立即更新」按钮）。 */
@Composable
private fun SidebarUpdateBanner(version: String, onDismiss: () -> Unit, onInstall: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 10.dp, end = 10.dp, bottom = 10.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(BrandGradient)
            .padding(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("新版本 $version", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Color.White)
            // × 关闭按钮（20×20 圆形半透明白）
            Box(
                modifier = Modifier.size(20.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.25f))
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
                contentAlignment = Alignment.Center,
            ) {
                Text("×", color = Color.White, fontSize = 14.sp)
            }
        }
        Spacer(Modifier.height(8.dp))
        // 「立即更新」按钮（白底 h28 radius 5，BrandColor 字，点击触发安装更新）
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(28.dp)
                .clip(RoundedCornerShape(5.dp))
                .background(Color.White.copy(alpha = 0.95f))
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onInstall),
            contentAlignment = Alignment.Center,
        ) {
            Text("立即更新", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = BrandColor)
        }
    }
}

/**
 * 递归目录树节点（v2：design/v2/02-main.html .tree-node）。
 *
 * 行高 32px，缩进 depth*14+8，gap 8，radius 6；
 * chevron(16px 宽，arrow 图标展开 rotate 90°)；文件夹图标 16px FolderAmber；名称 14px；
 * 三态：默认 secondary / hover bg-hover / 选中 BrandLighter 底 + BrandColor 字 + medium。
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
                .height(32.dp)
                .padding(start = (depth * 14 + 8).dp, end = 8.dp)
                .clip(RoundedCornerShape(6.dp))
                .background(if (isSelected) BrandLighter else Color.Transparent)
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
            // 文件夹图标（16px FolderAmber）
            MateIcon(name = "folder", size = 16.dp, tint = FolderAmber)
            // 名称（14px，选中 brand+medium，默认 secondary）
            Text(
                name,
                fontSize = 14.sp,
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

/**
 * 从配额文本（"36.5 GB / 200 GB"，ApplicationRoot.formatBytes 的产出格式）解析已用比例，
 * 仅用于账号卡配额进度条的显示；解析失败返回 null（调用方不显示进度条）。
 */
private fun parseQuotaRatio(quotaText: String): Float? {
    val parts = quotaText.split(" / ")
    if (parts.size != 2) return null
    val used = parseSizeBytes(parts[0]) ?: return null
    val total = parseSizeBytes(parts[1]) ?: return null
    if (total <= 0L) return null
    return (used.toDouble() / total.toDouble()).toFloat().coerceIn(0f, 1f)
}

/** 解析 "X.X GB/MB/KB" 或 "N B" 为字节数；格式不符返回 null。 */
private fun parseSizeBytes(text: String): Long? {
    val tokens = text.trim().split(' ')
    if (tokens.size != 2) return null
    val value = tokens[0].toDoubleOrNull() ?: return null
    val multiplier = when (tokens[1]) {
        "B" -> 1L
        "KB" -> 1024L
        "MB" -> 1024L * 1024
        "GB" -> 1024L * 1024 * 1024
        else -> return null
    }
    return (value * multiplier).toLong()
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
