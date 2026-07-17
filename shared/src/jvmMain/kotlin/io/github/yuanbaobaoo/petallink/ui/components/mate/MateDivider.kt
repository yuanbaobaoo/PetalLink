@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors

/**
 * 水平 0.5px 分隔线（对标原 Vue `border-bottom/top: 0.5px solid var(--border)`）。
 *
 * Compose 用 0.5.dp 近似 hairline；颜色取当前主题 border。
 *
 * @param thickness 线宽（默认 0.5dp）
 */
@Composable
fun MateHDivider(modifier: Modifier = Modifier, thickness: Dp = 0.5.dp, color: Color? = null) {
    val border = color ?: LocalSemanticColors.current.border
    Box(modifier = modifier.fillMaxWidth().height(thickness).background(border))
}

/**
 * 垂直分隔线（对标原 Vue 1px 竖线，如 app-bar__sep、tp-stats__sep）。
 *
 * @param height 线高（默认 24dp）
 */
@Composable
fun MateVDivider(modifier: Modifier = Modifier, height: Dp = 24.dp, color: Color? = null) {
    val border = color ?: LocalSemanticColors.current.border
    Box(modifier = modifier.width(1.dp).height(height).background(border))
}
