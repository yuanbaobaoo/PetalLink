@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.wrapContentWidth
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateDialogOptions
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.confirmDialog
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorBg
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.theme.WarningBg
import io.github.yuanbaobaoo.petallink.ui.theme.WarningColor
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FailedItemUi
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncSnapshotUi
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * 同步状态条（对标原 Vue SyncStatusBar.vue）。
 *
 * 高 32px，左右 flex 3:4。
 * 左：statusIcon（三态着色 success/error/active，活动 spin）+ statusText（9 种 syncPhase 文案）+ 上次同步时间。
 * 右：标签区右对齐换行（上传/下载 primary、等待网络/编辑中/冲突 warning、同步失败 error 可点）。
 * 失败弹窗：列出 failedItems(path+error)。
 *
 * @param snap 完整同步快照
 */
@Composable
fun SyncStatusBar(
    snap: SyncSnapshotUi,
) {
    val semantic = LocalSemanticColors.current
    val isIdle = snap.isIdle
    val statusIcon = if (!isIdle) "sync" else if (snap.failed > 0) "alert" else "check"
    val iconColor = when {
        !isIdle -> BrandColor
        snap.failed > 0 -> ErrorColor
        else -> SuccessColor
    }
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
            .height(32.dp)
            .background(semantic.bgContainer)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 左侧：图标 + 文案 + 时间（flex:3）
        Row(
            modifier = Modifier.weight(3f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MateIcon(name = statusIcon, size = 16.dp, tint = iconColor, spin = !isIdle)
            Text(statusText, fontSize = 13.sp, color = semantic.textPrimary, maxLines = 1)
            if (isIdle && snap.lastSyncTime != null && snap.lastSyncTime > 0) {
                val time = SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(snap.lastSyncTime))
                Text("· 上次同步 $time", fontSize = 13.sp, color = semantic.textSecondary)
            }
        }
        // 右侧标签区（flex:4，右对齐换行）
        Row(
            modifier = Modifier.weight(4f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.End),
        ) {
            if (snap.uploading > 0) StatusTag("上传 ${snap.uploading}", BrandColor, Color(0xFFF2F3FF))
            if (snap.downloading > 0) StatusTag("下载 ${snap.downloading}", BrandColor, Color(0xFFF2F3FF))
            if (snap.waitingNetwork > 0) StatusTag("等待网络 ${snap.waitingNetwork}", WarningColor, WarningBg)
            if (snap.editing > 0) StatusTag("编辑中 ${snap.editing}", WarningColor, WarningBg)
            if (snap.conflict > 0) StatusTag("冲突 ${snap.conflict}", WarningColor, WarningBg)
            if (snap.failed > 0) {
                StatusTag(
                    "同步失败 ${snap.failed}",
                    ErrorColor,
                    ErrorBg,
                    onClick = { showFailed = true },
                )
            }
        }
    }
    // 底分隔线（对标 .sync-bar border-bottom: 0.5px）
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

/** 状态标签（padding 1/6，radius 3，12px medium，行高 18）。 */
@Composable
private fun StatusTag(
    text: String,
    textColor: Color,
    bgColor: Color,
    onClick: (() -> Unit)? = null,
) {
    val shape = RoundedCornerShape(3.dp)
    val base = Modifier.clip(shape).background(bgColor).padding(horizontal = 6.dp, vertical = 1.dp)
        .then(
            if (onClick != null) Modifier.clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onClick,
            ) else Modifier,
        )
    Text(
        text = text,
        color = textColor,
        fontSize = 12.sp,
        fontWeight = FontWeight.Medium,
        modifier = base,
    )
}
