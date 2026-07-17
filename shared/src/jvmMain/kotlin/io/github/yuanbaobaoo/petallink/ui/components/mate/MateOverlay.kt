@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorBg
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.WarningColor
import kotlinx.coroutines.delay

/**
 * 弹出菜单项（对标原 Vue PopupItem）。
 */
data class MatePopupItem(
    val value: String,
    val label: String = value,
    val icon: String? = null,
    val danger: Boolean = false,
    val divider: Boolean = false,
)

/**
 * 弹出菜单（v2：radius-lg(10) 浮层 + radius-md(8) 菜单项）。
 *
 * trigger + Popup + menu；menu min-width=menuWidth(默认 168)，radius-lg，shadow-dropdown；
 * 边界自动翻转（Popup 自带窗口边界钳制，贴 trigger 左下）；
 * item row gap sm，h36，radius-md，hover bg-fill；danger color error；divider 0.5px。
 *
 * @param items 菜单项列表
 * @param menuWidth 菜单宽度（默认 168）
 * @param onDismiss 关闭回调（点击外部或选择后）
 * @param onSelect 选中回调（传 item.value）
 * @param trigger 触发器内容
 */
@Composable
fun MatePopupMenu(
    items: List<MatePopupItem>,
    onDismiss: () -> Unit,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    menuWidth: Int = 168,
    disabled: Boolean = false,
    trigger: @Composable () -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    val semantic = LocalSemanticColors.current
    val triggerInteraction = remember { MutableInteractionSource() }

    Box(
        modifier = modifier.wrapContentSize(Alignment.TopStart),
    ) {
        Box(modifier = Modifier.mateClickable(triggerInteraction) {
            if (!disabled) expanded = true
        }) {
            trigger()
        }
        if (expanded && !disabled) {
            Popup(
                onDismissRequest = {
                    expanded = false
                    onDismiss()
                },
                properties = PopupProperties(focusable = true),
            ) {
                Column(
                    modifier = Modifier
                        .width(menuWidth.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(semantic.bgContainer)
                        .border(0.5.dp, semantic.border, RoundedCornerShape(10.dp))
                        .padding(6.dp),
                ) {
                    items.forEach { item ->
                        if (item.divider) {
                            // 分隔线：0.5px border-top
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 8.dp, vertical = 4.dp)
                                    .height(0.5.dp)
                                    .background(semantic.border),
                            )
                        } else {
                            val itemInteraction = remember { MutableInteractionSource() }
                            var itemHovered by remember { mutableStateOf(false) }
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(36.dp)
                                    .clip(RoundedCornerShape(8.dp))
                                    .background(if (itemHovered) semantic.bgFill else Color.Transparent)
                                    .mateClickable(itemInteraction) {
                                        expanded = false
                                        onSelect(item.value)
                                        onDismiss()
                                    }
                                    .padding(horizontal = 12.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                if (item.icon != null) {
                                    MateIcon(
                                        name = item.icon,
                                        size = 16.dp,
                                        tint = if (item.danger) ErrorColor else semantic.textSecondary,
                                    )
                                }
                                Text(
                                    item.label,
                                    color = if (item.danger) ErrorColor else semantic.textPrimary,
                                    fontSize = 15.sp,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

// ============================================================
// Dialog（v2：radius-xl(12) + 图标徽章标题）
// ============================================================

/**
 * 对话框配置（对标原 Vue DialogOptions / ConfirmOptions）。
 */
data class MateDialogOptions(
    val title: String = "",
    val titleIcon: String? = null,
    val danger: Boolean = false,
    val content: String = "",
    val closeOnOverlay: Boolean = true,
    val width: Int = 460,
    val cancelText: String = "取消",
    val confirmText: String = "确定",
)

/**
 * 全局对话框状态（模块级，供 useDialog 管理）。
 */
private val globalDialogState = mutableStateOf<Pair<MateDialogOptions, ((Boolean) -> Unit)?>?>(null)

/**
 * 显示对话框（非确认型，无 resolver）。
 */
fun openDialog(options: MateDialogOptions) {
    globalDialogState.value = options to null
}

/**
 * 确认对话框（suspend 风格的回调形式；resolver 收到 true=确认/false=取消）。
 */
fun confirmDialog(options: MateDialogOptions, onResult: (Boolean) -> Unit) {
    globalDialogState.value = options to onResult
}

/**
 * 关闭对话框。
 */
fun closeDialog(value: Boolean = false) {
    globalDialogState.value?.second?.invoke(value)
    globalDialogState.value = null
}

/**
 * 对话框宿主（v2）。
 *
 * 绑定 [globalDialogState]；confirm 时 footer 为「取消(ghost) + 确认(渐变)」两按钮。
 * overlay fixed inset 0，bg rgba(0,0,0,0.36)；dialog radius-xl(12)/shadow-modal；
 * header 带 40×40 radius-lg 图标徽章（danger→err-bg/err，默认 brand-lighter/brand）。
 */
@Composable
fun MateDialogHost() {
    val state = globalDialogState.value
    if (state == null) return
    val (options, resolver) = state
    val semantic = LocalSemanticColors.current

    Box(
        modifier = Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.36f)),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier
                .width(options.width.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(semantic.bgContainer),
        ) {
            // header：图标徽章 + 标题
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 24.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (options.titleIcon != null) {
                    Box(
                        modifier = Modifier
                            .size(40.dp)
                            .clip(RoundedCornerShape(10.dp))
                            .background(if (options.danger) ErrorBg else BrandLighter),
                        contentAlignment = Alignment.Center,
                    ) {
                        MateIcon(
                            name = options.titleIcon,
                            size = 20.dp,
                            tint = if (options.danger) ErrorColor else BrandColor,
                        )
                    }
                }
                Text(
                    options.title,
                    color = semantic.textPrimary,
                    fontSize = 18.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
            // body
            Text(
                options.content,
                color = semantic.textSecondary,
                fontSize = 15.sp,
                modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 20.dp),
                lineHeight = (15 * 1.65f).sp,
            )
            // footer
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, bottom = 20.dp),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (resolver != null) {
                    MateButton(
                        label = options.cancelText,
                        variant = MateButtonVariant.ICON_TEXT,
                        onClick = { closeDialog(false) },
                    )
                    Spacer(Modifier.width(10.dp))
                    MateButton(
                        label = options.confirmText,
                        variant = MateButtonVariant.PRIMARY,
                        danger = options.danger,
                        onClick = { closeDialog(true) },
                    )
                } else {
                    MateButton(label = options.confirmText, onClick = { closeDialog(false) })
                }
            }
        }
    }
}

// ============================================================
// Toast（v2：深色浮条 + 状态图标）
// ============================================================

/**
 * Toast 变体（对标原 Vue ToastVariant）。
 */
enum class MateToastVariant { DEFAULT, SUCCESS, WARNING, ERROR }

/**
 * Toast 通知条目：消息文本及其展示样式变体。
 */
private data class ToastEntry(val message: String, val variant: MateToastVariant)

private val globalToastState = mutableStateOf<ToastEntry?>(null)

/**
 * 显示 Toast（单条语义：新 toast 清空旧的）。
 *
 * @param message 消息
 * @param variant 变体（default/success/warning/error）
 * @param duration 显示时长（默认 2000ms）
 */
fun showToast(
    message: String,
    variant: MateToastVariant = MateToastVariant.DEFAULT,
    duration: Long = 2000L,
) {
    globalToastState.value = ToastEntry(message, variant)
}

/**
 * Toast 宿主（v2：深色浮条）。
 *
 * 底部居中，max-w 480，padding 10/18，radius-lg(10)，bg rgba(28,28,30,0.92)；
 * 图标按 variant 着色（success 绿 / warning 橙 / error 粉红 / default 白）。
 * 自动 2 秒后清除（单条语义）。
 */
@Composable
fun MateToastHost() {
    val entry = globalToastState.value ?: return
    val (iconName, iconColor) = when (entry.variant) {
        MateToastVariant.DEFAULT -> "info" to Color.White
        MateToastVariant.SUCCESS -> "check" to Color(0xFF4ADE80)
        MateToastVariant.WARNING -> "alert" to WarningColor
        MateToastVariant.ERROR -> "alert" to Color(0xFFFB7185)
    }
    LaunchedEffect(entry) {
        delay(2000L)
        globalToastState.value = null
    }
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.BottomCenter,
    ) {
        Row(
            modifier = Modifier
                .padding(48.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(Color(0xEB1C1C1E))
                .padding(horizontal = 18.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MateIcon(name = iconName, size = 16.dp, tint = iconColor)
            Text(entry.message, color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        }
    }
}
