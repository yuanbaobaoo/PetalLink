package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.ui.Modifier

/**
 * 无 ripple 的点击 Modifier（macOS 原生无水波纹）。
 *
 * 封装 `clickable(indication = null)`，保持调用点简洁。
 * 调用方负责 [interactionSource] 的 remember（避免在条件分支中调用 @Composable）。
 *
 * 示例：
 * ```
 * val interaction = remember { MutableInteractionSource() }
 * Modifier.mateClickable(interaction) { onClick() }
 * ```
 */
fun Modifier.mateClickable(
    interactionSource: MutableInteractionSource,
    onClick: () -> Unit,
): Modifier = this.clickable(
    interactionSource = interactionSource,
    indication = null,
    onClick = onClick,
)
