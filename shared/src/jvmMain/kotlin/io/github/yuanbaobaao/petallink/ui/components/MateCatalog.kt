package io.github.yuanbaobaao.petallink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaao.petallink.ui.theme.BrandColor
import io.github.yuanbaobaao.petallink.ui.theme.LocalReducedMotion

@Composable
fun MateAppLogo(size: Dp = 26.dp, text: String = "PetalLink") {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text("✿", color = BrandColor, fontSize = size.value.sp)
        Spacer(Modifier.width(6.dp))
        Text(text, fontWeight = FontWeight.Bold, fontSize = (size.value * 0.65f).sp)
    }
}

@Composable
fun MateLogoWithText(height: Dp = 32.dp) = MateAppLogo(height, "PetalLink")

@Composable
fun MateRadio(value: String, selectedValue: String, onSelect: (String) -> Unit, disabled: Boolean = false) {
    Row(
        Modifier.clickable(enabled = !disabled) { onSelect(value) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected = value == selectedValue, onClick = { onSelect(value) }, enabled = !disabled)
        Text(value)
    }
}

@Composable
fun MateLinearProgress(value: Float?, modifier: Modifier = Modifier, height: Dp = 4.dp) {
    if (value == null && !LocalReducedMotion.current) {
        LinearProgressIndicator(modifier = modifier.fillMaxWidth().height(height))
    } else {
        LinearProgressIndicator(progress = (value ?: 0.5f).coerceIn(0f, 1f), modifier = modifier.fillMaxWidth().height(height))
    }
}

@Composable
fun MateCircularProgress(size: Dp = 24.dp, value: Float? = null) {
    if (value == null && !LocalReducedMotion.current) CircularProgressIndicator(Modifier.size(size), strokeWidth = 2.5.dp)
    else CircularProgressIndicator((value ?: 0.5f).coerceIn(0f, 1f), Modifier.size(size), strokeWidth = 2.5.dp)
}

@Composable
fun MateDialogHost(content: @Composable () -> Unit) = content()

@Composable
fun MateToastHost(content: @Composable () -> Unit) = content()

@Composable
fun MateNavItem(label: String, active: Boolean, indent: Int = 0, onClick: () -> Unit) {
    Text(
        label,
        color = if (active) BrandColor else MaterialTheme.colors.onSurface,
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick)
            .background(if (active) BrandColor.copy(alpha = 0.08f) else Color.Transparent)
            .padding(start = (8 + indent * 16).dp, top = 8.dp, bottom = 8.dp),
        fontSize = 13.sp,
    )
}

@Composable
fun MateSpinningIcon(symbol: String = "↻", size: Dp = 16.dp) {
    // Reduced Motion 下保持静态；常规模式由调用方的状态刷新提供动画帧。
    Text(symbol, fontSize = size.value.sp, color = BrandColor)
}

@Composable
fun MateHover(modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    Box(modifier.hoverable(remember { MutableInteractionSource() }), content = content)
}

@Composable
fun MateVerticalSeparator(height: Dp = 20.dp) {
    Divider(Modifier.width(1.dp).height(height))
}

@Composable
fun MateBottomDivider(background: Color = MaterialTheme.colors.surface) {
    Column { Spacer(Modifier.fillMaxWidth().height(1.dp).background(background)); Divider() }
}

@Composable
fun MateScaffold(flush: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().padding(if (flush) 0.dp else 16.dp), content = content)
}
