@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLinearProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MatePopupItem
import io.github.yuanbaobaoo.petallink.ui.components.mate.MatePopupMenu
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.theme.WarningColor
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi

/** 传输状态元信息（对标原 Vue stateMeta）。 */
private data class StateMeta(val icon: String, val label: String, val color: Color, val spin: Boolean)

/** 9 态 stateMeta 映射（对标原 Vue TransferPopover stateMeta）。 */
private fun stateMeta(state: TransferState): StateMeta = when (state) {
    TransferState.Pending -> StateMeta("clock", "等待调度", Color(0x99000000), false)
    TransferState.Running -> StateMeta("sync", "传输中", BrandColor, true)
    TransferState.WaitingForNetwork -> StateMeta("clock", "等待网络", WarningColor, false)
    TransferState.BackingOff -> StateMeta("clock", "等待重试", WarningColor, false)
    TransferState.VerifyingRemote -> StateMeta("sync", "核验远端", BrandColor, true)
    TransferState.RestartRequired -> StateMeta("refresh", "等待重新规划", WarningColor, false)
    TransferState.Completed -> StateMeta("check", "已完成", SuccessColor, false)
    TransferState.Failed -> StateMeta("x", "失败", ErrorColor, false)
    TransferState.Canceled -> StateMeta("x", "已取消", Color(0x99000000), false)
}

/** 方向图标（对标原 Vue dirIcon）。 */
private fun dirIcon(direction: String): String = when (direction) {
    "download" -> "download"
    "download_update" -> "refresh"
    "delete" -> "trash"
    else -> "transfer"
}

/** 方向标签（对标原 Vue DIR_LABEL）。 */
private fun dirLabel(direction: String): String = when (direction) {
    "upload" -> "上传"
    "download" -> "下载"
    "download_update" -> "下载"
    "delete" -> "删除"
    else -> "—"
}

/** 进度条颜色（对标原 Vue progressColor）。 */
private fun progressColor(state: TransferState): Color = when (state) {
    TransferState.Completed -> SuccessColor
    TransferState.Failed -> ErrorColor
    TransferState.Pending, TransferState.WaitingForNetwork,
    TransferState.BackingOff, TransferState.RestartRequired -> Color(0xFFC6C6C6)
    else -> BrandColor
}

/** 是否可重试（对标原 Vue canRetryTransferTask）。 */
private fun canRetry(task: TransferTaskUi): Boolean {
    val stateOk = task.state == TransferState.Failed || task.state == TransferState.RestartRequired
    // operation ∈ CREATE/UPDATE/DOWNLOAD/DOWNLOAD_UPDATE；这里用 direction 近似（upload/download 可重试，delete 不可）
    val dirOk = task.direction == "upload" || task.direction == "download"
    return stateOk && dirOk
}

/**
 * 传输队列弹窗（对标原 Vue TransferPopover.vue）。
 *
 * 420×560，radius-md，shadow-modal，border 0.5px；
 * header(48px) + stats(36px) + body(flex scroll)；
 * 任务行 60px：方向图标(16) + 信息区(name 13px medium + tag + 进度/错误) + 状态区(80px) + 重试按钮。
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
    onRetry: (Long) -> Unit,
    onClearCompleted: () -> Unit,
    onClearFailed: () -> Unit,
    onClearFinished: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
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
                .width(420.dp)
                .height(560.dp)
                .padding(top = 56.dp, end = 16.dp)
                .shadow(elevation = 16.dp, shape = RoundedCornerShape(6.dp))
                .clip(RoundedCornerShape(6.dp))
                .background(semantic.bgContainer)
                .border(0.5.dp, semantic.border, RoundedCornerShape(6.dp)),
        ) {
            // header 48px
            Row(
                modifier = Modifier.fillMaxWidth().height(48.dp).padding(start = 16.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                MateIcon(name = "transfer", size = 18.dp, tint = BrandColor)
                Text(
                    "传输队列",
                    fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = semantic.textPrimary,
                    modifier = Modifier.weight(1f),
                )
                MateButton(variant = MateButtonVariant.ICON, icon = "x", onClick = onDismiss)
            }
            // stats 36px
            Row(
                modifier = Modifier.fillMaxWidth().height(36.dp).padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("处理中 $processing", fontSize = 12.sp, color = semantic.textSecondary)
                Box(Modifier.width(1.dp).height(14.dp).background(semantic.border))
                Text("等待中 $waiting", fontSize = 12.sp, color = semantic.textSecondary)
                Box(Modifier.width(1.dp).height(14.dp).background(semantic.border))
                Text("已完成 $completed", fontSize = 12.sp, color = semantic.textSecondary)
                if (failed > 0) Text("历史失败 $failed", fontSize = 12.sp, color = ErrorColor)
                Spacer(Modifier.weight(1f))
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
            // body
            if (tasks.isEmpty()) {
                MateEmpty(title = "暂无传输任务", icon = "cloud")
            } else {
                LazyColumn(modifier = Modifier.weight(1f)) {
                    items(tasks, key = { it.id }) { task ->
                        TransferItemRow(task, onRetry)
                    }
                }
            }
        }
    }
}

/** 单个传输任务行（60px，含底分隔线）。 */
@Composable
private fun TransferItemRow(task: TransferTaskUi, onRetry: (Long) -> Unit) {
    val semantic = LocalSemanticColors.current
    val meta = stateMeta(task.state)
    Column {
        Row(
            modifier = Modifier.fillMaxWidth().height(60.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // 方向图标
            MateIcon(name = dirIcon(task.direction), size = 16.dp, tint = semantic.textSecondary)
            // 信息区（flex:1）
            Column(modifier = Modifier.weight(1f)) {
                // 名称行（13px medium + tag 标签）
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    // tag：padding 0/4，bg-hover，radius 3
                    Text(
                        dirLabel(task.direction),
                        fontSize = 12.sp,
                        color = semantic.textSecondary,
                        modifier = Modifier
                            .clip(RoundedCornerShape(3.dp))
                            .background(semantic.bgHover)
                            .padding(horizontal = 4.dp, vertical = 0.dp),
                    )
                    Text(
                        task.fileName,
                        fontSize = 13.sp,
                        fontWeight = FontWeight.Medium,
                        color = semantic.textPrimary,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.height(4.dp))
                // 第二行：错误 or 进度条（带字节） or 删除操作
                if ((task.state == TransferState.Failed || task.state == TransferState.RestartRequired) && task.errorMessage != null) {
                    Text(
                        task.errorMessage,
                        fontSize = 12.sp,
                        color = ErrorColor,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                } else if (task.direction != "delete" && task.bytesTotal > 0) {
                    // 进度条 + 百分比 + 已传/总字节（对标原 Vue tp-item__pct）
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        MateLinearProgress(
                            value = task.progress,
                            color = progressColor(task.state),
                            modifier = Modifier.weight(1f),
                        )
                        val pctText = "${(task.progress * 100).toInt()}% · ${formatFileSize(task.bytesDone)}/${formatFileSize(task.bytesTotal)}"
                        Text(pctText, fontSize = 12.sp, color = semantic.textSecondary)
                    }
                } else if (task.direction == "delete") {
                    Text("删除操作", fontSize = 12.sp, color = semantic.textSecondary)
                }
            }
            // 状态区（80px 右对齐）
            Row(
                modifier = Modifier.width(80.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(3.dp, Alignment.End),
            ) {
                MateIcon(name = meta.icon, size = 12.dp, tint = meta.color, spin = meta.spin)
                Text(meta.label, fontSize = 12.sp, fontWeight = FontWeight.Medium, color = meta.color)
            }
            // 重试按钮（条件）
            if (canRetry(task)) {
                MateButton(
                    variant = MateButtonVariant.ICON,
                    icon = "refresh",
                    onClick = { onRetry(task.id) },
                )
            }
        }
        // item 底分隔线（对标 .tp-item border-bottom: 0.5px）
        io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider()
    }
}
