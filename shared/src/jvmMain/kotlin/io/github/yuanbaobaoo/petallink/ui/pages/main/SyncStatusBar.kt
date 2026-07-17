@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateDialogOptions
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTag
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagSize
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagTheme
import io.github.yuanbaobaoo.petallink.ui.components.mate.confirmDialog
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncSnapshotUi
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * 同步状态条（v2 视觉：对标 design/v2/02-main.html、03-sync-states.html 的 .sync-bar）。
 *
 * minHeight 44、padding 6/20（内容超高可换行）；
 * 左：状态指示（活动态 spin sync 图标 BrandColor；空闲 failed>0 红色 8×8 圆点；空闲正常绿色 8×8 圆点）
 *   + statusText（14sp medium，9 种 syncPhase 文案）+ 上次同步时间（13.5sp text-secondary）；
 * 右：标签区右对齐换行（MateTag chip：上传/下载 PRIMARY、等待网络/编辑中/冲突 WARNING、同步失败 ERROR 可点）。
 * 失败弹窗：列出 failedItems(path+error)。底部 MateHDivider 分隔线。
 *
 * @param snap 完整同步快照
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SyncStatusBar(
    snap: SyncSnapshotUi,
) {
    val semantic = LocalSemanticColors.current
    val isIdle = snap.isIdle
    val statusText = statusTextFor(snap)
    var showFailed by remember { mutableStateOf(false) }

    if (snap.failed > 0 && showFailed) {
        confirmDialog(
            MateDialogOptions(
                title = "同步失败项 (${snap.failedItems.size})",
                titleIcon = "alert",
                danger = true,
                confirmText = "关闭",
                content = snap.failedItems.joinToString("\n\n") {
                    val err = it.errorMessage?.let { e -> "\n$e" } ?: ""
                    "${it.relativePath}$err"
                }.ifBlank { "暂无失败项详情" },
            ),
        ) { showFailed = false }
    }

    Column {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 44.dp)
            .background(semantic.bgContainer)
            .padding(horizontal = 20.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 左侧：状态指示 + 文案 + 时间（v2 .sync-bar__left：flex:1，gap 10）
        Row(
            modifier = Modifier.weight(1f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (!isIdle) {
                // 活动态：spin 的 sync 图标（v2 场景 4）
                MateIcon(name = "sync", size = 16.dp, tint = BrandColor, spin = true)
            } else {
                // 空闲态：8×8 状态圆点（v2 .dot：failed>0 红色，否则绿色）
                Box(
                    Modifier
                        .size(8.dp)
                        .clip(CircleShape)
                        .background(if (snap.failed > 0) ErrorColor else SuccessColor),
                )
            }
            Text(statusText, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = semantic.textPrimary, maxLines = 1)
            if (isIdle && snap.lastSyncTime != null && snap.lastSyncTime > 0) {
                val time = SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(snap.lastSyncTime))
                Text("· 上次同步 $time", fontSize = 13.5.sp, color = semantic.textSecondary)
            }
        }
        // 右侧标签区（v2 .sync-bar__tags：右对齐、flex-wrap 换行、gap 6，chip 用 MateTag SMALL）
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.End),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (snap.uploading > 0) MateTag("上传 ${snap.uploading}", theme = MateTagTheme.PRIMARY, size = MateTagSize.SMALL)
            if (snap.downloading > 0) MateTag("下载 ${snap.downloading}", theme = MateTagTheme.PRIMARY, size = MateTagSize.SMALL)
            if (snap.waitingNetwork > 0) MateTag("等待网络 ${snap.waitingNetwork}", theme = MateTagTheme.WARNING, size = MateTagSize.SMALL)
            if (snap.editing > 0) MateTag("编辑中 ${snap.editing}", theme = MateTagTheme.WARNING, size = MateTagSize.SMALL)
            if (snap.conflict > 0) MateTag("冲突 ${snap.conflict}", theme = MateTagTheme.WARNING, size = MateTagSize.SMALL)
            if (snap.failed > 0) {
                MateTag(
                    "同步失败 ${snap.failed}",
                    theme = MateTagTheme.ERROR,
                    size = MateTagSize.SMALL,
                    onClick = { showFailed = true },
                )
            }
        }
    }
    // 底分隔线（v2 .sync-bar border-bottom: 1px var(--line)）
    MateHDivider()
    }
}

/** 9 种 syncPhase 文案 + 空闲细分（对标原 Vue statusText computed）。 */
private fun statusTextFor(snap: SyncSnapshotUi): String = when (snap.syncPhase) {
    "indexing-startup" -> "正在读取云端索引（首次）…"
    "indexing-manual" -> "正在读取云端索引…"
    "indexing-auto-full" -> "正在读取云端索引（全量纠偏）…"
    "querying-changes" -> "正在查询云端变更…"
    "syncing-auto-incremental" -> "正在同步云端变更…"
    "syncing-local" -> "正在同步本地变更…"
    "syncing-manual" -> "正在同步…"
    "syncing-retry" -> "正在重试失败项…"
    "syncing-startup" -> "正在同步（启动恢复）…"
    else -> when {
        snap.uploading > 0 || snap.downloading > 0 -> "同步中"
        snap.waitingNetwork > 0 -> "等待网络恢复…"
        snap.failed > 0 -> "同步存在失败项"
        else -> "同步完成"
    }
}
