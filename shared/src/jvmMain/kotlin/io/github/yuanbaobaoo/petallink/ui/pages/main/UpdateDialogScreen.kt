@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradient
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.viewmodel.UpdaterPhase
import io.github.yuanbaobaoo.petallink.update.UpdateManifest

/**
 * 更新对话框（v2 重设计，对标原 Vue UpdateDialog.vue / design/v2/08-update.html）。
 *
 * 9 phase 状态机：visible 仅在 available/downloading/downloaded/waitingTransfer/ready/error 显示。
 * 自绘 overlay（rgba 0.36），不复用 MateDialog（与原 Vue 一致，独立状态机）。
 * dialog 宽 440，radius 12，24dp 柔影（v2：去 0.5dp 描边），fade-in 0.15s；
 * header 为 40×40 BrandLighter 图标徽章 + 18sp 标题（参照 MateDialogHost）。
 *
 * @param phase 当前更新阶段
 * @param manifest 可用更新信息（version/notes），available 态显示版本号和日志
 * @param downloadProgress 下载进度 0..1
 * @param errorMessage 错误消息（error 态）
 * @param hasActiveTransfers 是否有活跃传输（downloaded/waitingTransfer 时禁用立即重启）
 * @param onStartUpdate 立即更新（available → 开始下载）
 * @param onRelaunch 立即重启（ready/downloaded → 重启安装）
 * @param onRetry 重试（error → 重新下载）
 * @param onDismiss 关闭（稍后/后台下载/后台等待）
 */
@Composable
fun UpdateDialogScreen(
    phase: UpdaterPhase,
    manifest: UpdateManifest?,
    downloadProgress: Float,
    errorMessage: String?,
    hasActiveTransfers: Boolean,
    onStartUpdate: () -> Unit,
    onRelaunch: () -> Unit,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
) {
    // visible 计算：仅这些 phase 显示弹窗（对标原 Vue visible computed）
    val visible = phase in setOf(
        UpdaterPhase.AVAILABLE,
        UpdaterPhase.DOWNLOADING,
        UpdaterPhase.READY,
        UpdaterPhase.FAILED,
        UpdaterPhase.WAITING_TRANSFERS,
    )
    if (!visible) return

    val semantic = LocalSemanticColors.current
    val title = when (phase) {
        UpdaterPhase.AVAILABLE -> "发现新版本"
        UpdaterPhase.DOWNLOADING -> "正在下载更新…"
        UpdaterPhase.WAITING_TRANSFERS -> "下载完成"
        UpdaterPhase.READY -> "更新就绪"
        UpdaterPhase.FAILED -> "更新失败"
        else -> ""
    }

    // overlay（fixed inset 0，bg rgba 0.36，居中）
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.36f))
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
            ) { onDismiss() },
        contentAlignment = Alignment.Center,
    ) {
        // dialog：宽 440，radius 12，24dp 柔影（v2：去 0.5dp 描边）
        Column(
            modifier = Modifier
                .width(440.dp)
                .shadow(24.dp, RoundedCornerShape(12.dp))
                .clip(RoundedCornerShape(12.dp))
                .background(semantic.bgContainer)
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) { /* 阻止点击穿透到 overlay */ },
        ) {
            // header：40×40 BrandLighter 图标徽章 + 标题（v2，参照 MateDialogHost）
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = 32.dp, end = 32.dp, top = 32.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(40.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(BrandLighter),
                    contentAlignment = Alignment.Center,
                ) {
                    MateIcon(name = "download", size = 20.dp, tint = BrandColor)
                }
                Text(title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = semantic.textPrimary)
            }
            // 版本号（available 态显示）
            if (phase == UpdaterPhase.AVAILABLE && manifest != null) {
                Text(
                    "v${manifest.version}",
                    fontSize = 16.sp,
                    fontWeight = FontWeight.Bold,
                    color = BrandColor,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            // body
            Box(
                modifier = Modifier.fillMaxWidth().padding(start = 32.dp, end = 32.dp, top = 12.dp, bottom = 32.dp),
            ) {
                when (phase) {
                    UpdaterPhase.AVAILABLE -> UpdateAvailableBody(manifest)
                    UpdaterPhase.DOWNLOADING -> UpdateDownloadingBody(downloadProgress)
                    UpdaterPhase.WAITING_TRANSFERS, UpdaterPhase.READY -> UpdateWaitingBody(hasActiveTransfers, phase)
                    UpdaterPhase.FAILED -> Text(
                        errorMessage ?: "更新失败，请稍后重试。",
                        fontSize = 15.sp,
                        color = ErrorColor,
                    )
                    else -> {}
                }
            }
            // footer
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 16.dp),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                UpdateFooter(
                    phase = phase,
                    hasActiveTransfers = hasActiveTransfers,
                    onStartUpdate = onStartUpdate,
                    onRelaunch = onRelaunch,
                    onRetry = onRetry,
                    onDismiss = onDismiss,
                )
            }
        }
    }
}

/** available 态正文：更新日志或提示（v2：notes 块 radius 8 + 13sp，对标原 Vue .update-dialog__notes）。 */
@Composable
private fun UpdateAvailableBody(manifest: UpdateManifest?) {
    val semantic = LocalSemanticColors.current
    val notes = manifest?.notes?.takeIf { it.isNotBlank() }
    if (notes != null) {
        Column {
            Text("更新内容", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary)
            Spacer(Modifier.height(4.dp))
            // notes 文本块：bg-page，radius 8，padding 12，max-height 180，scroll
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 180.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(semantic.bgPage)
                    .verticalScroll(rememberScrollState())
                    .padding(12.dp),
            ) {
                Text(notes, fontSize = 13.sp, color = semantic.textSecondary)
            }
        }
    } else {
        Text(
            "暂无更新日志。是否下载并安装此更新？",
            fontSize = 15.sp,
            color = semantic.textSecondary,
        )
    }
}

/** downloading 态正文：进度条 + 百分比（v2：fill 用 BrandGradient 品牌渐变，对标原 Vue .update-dialog__progress）。 */
@Composable
private fun UpdateDownloadingBody(progress: Float) {
    val semantic = LocalSemanticColors.current
    val pct = (progress * 100).toInt()
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        // 进度条轨道（h8，bg-page，radius 4）
        Box(
            modifier = Modifier
                .weight(1f)
                .height(8.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(semantic.bgPage),
        ) {
            // fill（BrandGradient 品牌渐变，width = progress）
            Box(
                modifier = Modifier
                    .fillMaxWidth(progress.coerceIn(0f, 1f))
                    .height(8.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(BrandGradient),
            )
        }
        Text("$pct%", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = semantic.textPrimary)
    }
}

/** downloaded/waitingTransfer/ready 态正文：spinner + 提示。 */
@Composable
private fun UpdateWaitingBody(hasActiveTransfers: Boolean, phase: UpdaterPhase) {
    val semantic = LocalSemanticColors.current
    Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        UpdateSpinner()
        val text = when {
            phase == UpdaterPhase.READY -> "更新已准备就绪，重启即可生效。"
            hasActiveTransfers -> "下载完成。等待所有文档上传/下载完成后自动重启…"
            else -> "准备安装…"
        }
        Text(text, fontSize = 15.sp, color = semantic.textSecondary)
    }
}

/** 旋转 spinner（20×20，2.5px border，border-top brand，0.8s 线性）。 */
@Composable
private fun UpdateSpinner() {
    val transition = rememberInfiniteTransition(label = "update-spinner")
    val rotation by transition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(800, easing = LinearEasing),
            repeatMode = RepeatMode.Restart,
        ),
        label = "update-spinner-r",
    )
    Box(
        modifier = Modifier
            .width(20.dp)
            .height(20.dp)
            .padding(top = 2.dp),
        contentAlignment = Alignment.Center,
    ) {
        // 外圈淡色轨道
        Box(
            modifier = Modifier
                .width(20.dp)
                .height(20.dp)
                .clip(RoundedCornerShape(50))
                .background(Color.Transparent)
                .border(2.5.dp, LocalSemanticColors.current.border, RoundedCornerShape(50)),
        )
        // 高亮弧（用 brand 色覆盖 3/4，留 1/4 形成 spinner 效果——这里用旋转的 brand 圆近似）
        Box(
            modifier = Modifier
                .width(20.dp)
                .height(20.dp)
                .clip(RoundedCornerShape(50))
                .border(2.5.dp, BrandColor, RoundedCornerShape(50))
                .rotate(rotation),
        )
    }
}

/** footer 按钮组合（按 phase 切换，v2：次要按钮 ICON_TEXT 幽灵灰，主按钮 PRIMARY 渐变+柔影）。 */
@Composable
private fun UpdateFooter(
    phase: UpdaterPhase,
    hasActiveTransfers: Boolean,
    onStartUpdate: () -> Unit,
    onRelaunch: () -> Unit,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
) {
    when (phase) {
        UpdaterPhase.AVAILABLE -> {
            MateButton(label = "稍后提醒", variant = MateButtonVariant.ICON_TEXT, onClick = onDismiss)
            Spacer(Modifier.width(8.dp))
            MateButton(label = "立即更新", icon = "download", onClick = onStartUpdate)
        }
        UpdaterPhase.FAILED -> {
            MateButton(label = "关闭", variant = MateButtonVariant.ICON_TEXT, onClick = onDismiss)
            Spacer(Modifier.width(8.dp))
            MateButton(label = "重试", icon = "refresh", onClick = onRetry)
        }
        UpdaterPhase.READY -> {
            MateButton(label = "稍后", variant = MateButtonVariant.ICON_TEXT, onClick = onDismiss)
            Spacer(Modifier.width(8.dp))
            MateButton(label = "立即重启", icon = "check", onClick = onRelaunch)
        }
        UpdaterPhase.WAITING_TRANSFERS -> {
            MateButton(label = "后台等待", variant = MateButtonVariant.ICON_TEXT, onClick = onDismiss)
            Spacer(Modifier.width(8.dp))
            MateButton(
                label = if (hasActiveTransfers) "等待传输完成…" else "立即重启",
                icon = "check",
                onClick = onRelaunch,
                disabled = hasActiveTransfers,
            )
        }
        UpdaterPhase.DOWNLOADING -> {
            MateButton(label = "后台下载", variant = MateButtonVariant.ICON_TEXT, onClick = onDismiss)
        }
        else -> {}
    }
}
