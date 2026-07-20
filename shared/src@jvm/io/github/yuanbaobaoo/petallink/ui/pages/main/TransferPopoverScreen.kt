@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLinearProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MatePopupItem
import io.github.yuanbaobaoo.petallink.ui.components.mate.MatePopupMenu
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTag
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagSize
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagTheme
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateToastVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.showToast
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi

/**
 * 传输状态元信息（对标原 Vue stateMeta）。
 */
private data class StateMeta(val icon: String, val label: String, val color: Color, val spin: Boolean)

/**
 * 9 态 stateMeta 映射（对标原 Vue TransferPopover stateMeta，逻辑与文案不变）。
 *
 * v2：中性灰由硬编码改为语义色（[neutral] 传 semantic.textSecondary，浅色下与原值一致）。
 */
@Composable
private fun stateMeta(state: TransferState, neutral: Color): StateMeta = when (state) {
    TransferState.Pending -> StateMeta("clock", "等待调度", neutral, false)
    TransferState.Running -> StateMeta("sync", "传输中", PetalTheme.colors.brand, true)
    TransferState.WaitingForNetwork -> StateMeta("clock", "等待网络", PetalTheme.colors.warning, false)
    TransferState.BackingOff -> StateMeta("clock", "等待重试", PetalTheme.colors.warning, false)
    TransferState.VerifyingRemote -> StateMeta("sync", "核验远端", PetalTheme.colors.brand, true)
    TransferState.RestartRequired -> StateMeta("refresh", "等待重新规划", PetalTheme.colors.warning, false)
    TransferState.Completed -> StateMeta("check", "已完成", PetalTheme.colors.success, false)
    TransferState.Failed -> StateMeta("x", "失败", PetalTheme.colors.error, false)
    TransferState.Canceled -> StateMeta("x", "已取消", neutral, false)
}

/**
 * 方向图标（对标原 Vue dirIcon）。
 */
private fun dirIcon(direction: String): String = when (direction) {
    "download" -> "download"
    "download_update" -> "refresh"
    "delete" -> "trash"
    else -> "transfer"
}

/**
 * 方向标签（对标原 Vue DIR_LABEL）。
 */
private fun dirLabel(direction: String): String = when (direction) {
    "upload" -> "上传"
    "download" -> "下载"
    "download_update" -> "下载"
    "delete" -> "删除"
    else -> "—"
}

/**
 * 进度条颜色（对标原 Vue progressColor）。
 *
 * v2：等待类由硬编码灰改为语义色（[waitingColor] 传 semantic.textPlaceholder）。
 */
@Composable
private fun progressColor(state: TransferState, waitingColor: Color): Color = when (state) {
    TransferState.Completed -> PetalTheme.colors.success
    TransferState.Failed -> PetalTheme.colors.error
    TransferState.Pending, TransferState.WaitingForNetwork,
    TransferState.BackingOff, TransferState.RestartRequired -> waitingColor
    else -> PetalTheme.colors.brand
}

/**
 * 是否可重试（对标原 Vue canRetryTransferTask）。
 */
private fun canRetry(task: TransferTaskUi): Boolean {
    val stateOk = task.state == TransferState.Failed || task.state == TransferState.RestartRequired
    // operation ∈ CREATE/UPDATE/DOWNLOAD/DOWNLOAD_UPDATE；这里用 direction 近似（upload/download 可重试，delete 不可）
    val dirOk = task.direction == "upload" || task.direction == "download"
    return stateOk && dirOk
}

/**
 * 传输队列弹窗（v2 视觉，对标 design/v2/04-transfer.html，原 Vue TransferPopover.vue）。
 *
 * 440×580，radius-xl(12)，shadow-modal，border 0.5px；贴 AppBar 下右侧（top 64 / end 20）；
 * header(60) + stats(stat-pill 卡片行) + body(flex scroll，顶分隔线)；
 * 任务行 minHeight 68 padding 10/20：方向色块(36×36 radius8) + 信息区(dir chip + name 14.5 medium + 进度/错误) + 状态区(80) + 重试按钮。
 *
 * @param tasks 传输任务列表
 * @param onDismiss 关闭回调
 * @param onRetry 重试回调（传 taskId）
 * @param onClearCompleted 清除已完成
 * @param onClearFailed 清除失败历史
 * @param onClearFinished 清除完成+失败
 */
@Composable
fun TransferPopoverScreen(
    tasks: List<TransferTaskUi>,
    onDismiss: () -> Unit,
    onRetry: (Long, (Boolean) -> Unit) -> Unit,
    onClearCompleted: () -> Unit,
    onClearFailed: () -> Unit,
    onClearFinished: () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val metrics = PetalTheme.metrics.transferPopover
    // 重试防抖：单任务重试期间禁用重复点击，完成后 toast 反馈（对标原 Vue retryingId）
    var retryingId by remember { mutableStateOf<Long?>(null) }
    val requestRetry: (Long) -> Unit = { id ->
        if (retryingId == null) {
            retryingId = id
            onRetry(id) { ok ->
                retryingId = null
                if (ok) showToast("已重新提交传输任务", MateToastVariant.SUCCESS)
                else showToast("重试失败，请稍后再试", MateToastVariant.ERROR)
            }
        }
    }
    val processing = tasks.count {
        it.state in setOf(TransferState.Running, TransferState.VerifyingRemote, TransferState.Pending)
    }
    val waiting = tasks.count {
        it.state in setOf(TransferState.WaitingForNetwork, TransferState.BackingOff, TransferState.RestartRequired)
    }
    val completed = tasks.count { it.state == TransferState.Completed }
    val failed = tasks.count { it.state == TransferState.Failed }

    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.TopEnd,
    ) {
        Column(
            modifier = Modifier
                .width(metrics.panelWidth)
                .height(metrics.panelHeight)
                .padding(top = metrics.panelTopOffset, end = metrics.panelEndOffset)
                .shadow(elevation = metrics.panelShadowElevation, shape = RoundedCornerShape(metrics.panelRadius))
                .clip(RoundedCornerShape(metrics.panelRadius))
                .background(semantic.bgContainer)
                .border(metrics.panelBorderWidth, semantic.border, RoundedCornerShape(metrics.panelRadius)),
        ) {
            // header 60（v2：transfer 图标 18 brand + 标题 18 semibold + ICON 关闭）
            Row(
                modifier = Modifier.fillMaxWidth().height(metrics.headerHeight).padding(
                    start = metrics.headerStartPadding,
                    end = metrics.headerEndPadding,
                ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(metrics.headerContentSpacing),
            ) {
                MateIcon(name = "transfer", size = metrics.headerIconSize, tint = PetalTheme.colors.brand)
                Text(
                    "传输队列",
                    style = PetalTheme.typography.transfer.panelTitle,
                    color = semantic.textPrimary,
                    modifier = Modifier.weight(1f),
                )
                MateButton(variant = MateButtonVariant.ICON, icon = "x", onClick = onDismiss)
            }
            // stats（v2：stat-pill 卡片行，padding 0/20/14，gap 8；右侧清空菜单 trigger 保持）
            Row(
                modifier = Modifier.fillMaxWidth().padding(
                    start = metrics.summaryHorizontalPadding,
                    end = metrics.summaryHorizontalPadding,
                    bottom = metrics.summaryBottomPadding,
                ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(metrics.summaryItemSpacing),
            ) {
                StatPill(num = processing, label = "处理中", modifier = Modifier.weight(1f))
                StatPill(num = waiting, label = "等待中", modifier = Modifier.weight(1f))
                StatPill(num = completed, label = "已完成", modifier = Modifier.weight(1f))
                if (failed > 0) {
                    StatPill(num = failed, label = "历史失败", error = true, modifier = Modifier.weight(1f))
                }
                MatePopupMenu(
                    items = listOf(
                        MatePopupItem("completed", label = "清除已完成", icon = "check"),
                        MatePopupItem("failed", label = "清除失败历史", icon = "x", danger = true),
                        MatePopupItem("finished", label = "清除完成+失败历史", icon = "transfer"),
                    ),
                    onDismiss = {},
                    onSelect = { value ->
                        when (value) {
                            "completed" -> onClearCompleted()
                            "failed" -> onClearFailed()
                            "finished" -> onClearFinished()
                        }
                    },
                ) {
                    MateButton(variant = MateButtonVariant.ICON, icon = "transfer", onClick = {})
                }
            }
            // body（v2：列表区顶部分隔线）
            MateHDivider()
            if (tasks.isEmpty()) {
                MateEmpty(title = "暂无传输任务", icon = "cloud")
            } else {
                LazyColumn(modifier = Modifier.weight(1f)) {
                    items(tasks, key = { it.id }) { task ->
                        TransferItemRow(task, retrying = retryingId == task.id, onRetry = requestRetry)
                    }
                }
            }
        }
    }
}

/**
 * stat-pill 统计卡片（v2：bgFill radius-md(8)，padding 8/10，上数字下标签）。
 *
 * 数字 17 bold（tabular-nums），标签 12 textSecondary；
 * error 变体（历史失败）：PetalTheme.colors.errorBg 底 + PetalTheme.colors.error 数字。
 */
@Composable
private fun StatPill(num: Int, label: String, modifier: Modifier = Modifier, error: Boolean = false) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val metrics = PetalTheme.metrics.transferPopover
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(metrics.summaryRadius))
            .background(if (error) PetalTheme.colors.errorBg else semantic.bgFill)
            .padding(
                horizontal = metrics.summaryHorizontalContentPadding,
                vertical = metrics.summaryVerticalContentPadding,
            ),
        verticalArrangement = Arrangement.spacedBy(metrics.summaryTextSpacing),
    ) {
        Text(
            "$num",
            style = PetalTheme.typography.transfer.summaryValue.copy(fontFeatureSettings = "tnum"),
            color = if (error) PetalTheme.colors.error else semantic.textPrimary,
        )
        Text(label, style = PetalTheme.typography.transfer.summaryLabel, color = semantic.textSecondary)
    }
}

/**
 * 单个传输任务行（v2：minHeight 68，padding 10/20，含底分隔线）。
 */
@Composable
private fun TransferItemRow(task: TransferTaskUi, retrying: Boolean, onRetry: (Long) -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    val metrics = PetalTheme.metrics.transferPopover
    val meta = stateMeta(task.state, semantic.textSecondary)
    // v2 方向色块配色：上传 PetalTheme.colors.brandLighter/PetalTheme.colors.brand；下载 PetalTheme.colors.infoBg/PetalTheme.colors.info；删除 bgFill/textSecondary
    val (dirBg, dirFg) = when (task.direction) {
        "download", "download_update" -> PetalTheme.colors.infoBg to PetalTheme.colors.info
        "delete" -> semantic.bgFill to semantic.textSecondary
        else -> PetalTheme.colors.brandLighter to PetalTheme.colors.brand
    }
    Column {
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = metrics.taskMinimumHeight).padding(
                horizontal = metrics.taskHorizontalPadding,
                vertical = metrics.taskVerticalPadding,
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(metrics.taskContentSpacing),
        ) {
            // 方向色块（36×36 radius 8）
            Box(
                modifier = Modifier.size(metrics.directionBadgeSize)
                    .clip(RoundedCornerShape(metrics.directionBadgeRadius)).background(dirBg),
                contentAlignment = Alignment.Center,
            ) {
                MateIcon(name = dirIcon(task.direction), size = metrics.directionIconSize, tint = dirFg)
            }
            // 信息区（flex:1，v2 gap 5）
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(metrics.taskInfoSpacing)) {
                // 名称行（v2：dir 文字 chip MateTag SMALL + 文件名 14.5 medium）
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(metrics.taskNameSpacing),
                ) {
                    MateTag(
                        label = dirLabel(task.direction),
                        theme = if (task.direction == "upload") MateTagTheme.PRIMARY else MateTagTheme.DEFAULT,
                        size = MateTagSize.SMALL,
                    )
                    Text(
                        task.fileName,
                        style = PetalTheme.typography.transfer.taskName,
                        color = semantic.textPrimary,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                // 第二行：错误 or 进度条（带字节） or 删除操作
                if ((task.state == TransferState.Failed || task.state == TransferState.RestartRequired) && task.errorMessage != null) {
                    Text(
                        task.errorMessage,
                        style = PetalTheme.typography.transfer.taskDescription,
                        color = PetalTheme.colors.error,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                } else if (task.direction != "delete" && task.bytesTotal > 0) {
                    // 进度条 + 百分比 + 已传/总字节（对标原 Vue tp-item__pct；v2：12.5sp textSecondary tabular）
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(metrics.taskProgressSpacing),
                    ) {
                        MateLinearProgress(
                            value = task.progress,
                            color = progressColor(task.state, semantic.textPlaceholder),
                            modifier = Modifier.weight(1f),
                        )
                        val pctText = "${(task.progress * 100).toInt()}% · ${formatFileSize(task.bytesDone)}/${formatFileSize(task.bytesTotal)}"
                        Text(pctText, color = semantic.textSecondary, style = PetalTheme.typography.transfer.taskProgress.copy(fontFeatureSettings = "tnum"))
                    }
                } else if (task.direction == "delete") {
                    Text("删除操作", style = PetalTheme.typography.transfer.deleteOperation, color = semantic.textSecondary)
                }
            }
            // 状态区（80px 右对齐，stateMeta 九态映射，13sp medium）
            Row(
                modifier = Modifier.width(metrics.taskStateWidth),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(metrics.taskStateSpacing, Alignment.End),
            ) {
                MateIcon(name = meta.icon, size = metrics.taskStateIconSize, tint = meta.color, spin = meta.spin)
                Text(meta.label, style = PetalTheme.typography.transfer.taskState, color = meta.color)
            }
            // 重试按钮（条件；重试中显示进度指示并防抖）
            if (canRetry(task)) {
                if (retrying) {
                    MateCircularProgress(size = metrics.taskStateIconSize)
                } else {
                    MateButton(
                        variant = MateButtonVariant.ICON,
                        icon = "refresh",
                        onClick = { onRetry(task.id) },
                    )
                }
            }
        }
        // item 底分隔线（对标 .transfer-item border-bottom）
        MateHDivider()
    }
}
