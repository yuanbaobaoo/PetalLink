@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandActive
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandHover
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.LocalReducedMotion

/** 按钮形态（对标原 Vue MateButton variant）。 */
enum class MateButtonVariant { PRIMARY, TEXT, ICON, ICON_TEXT }

/**
 * 统一按钮（对标原 Vue `<MateButton variant danger disabled loading fullWidth icon badge height>`）。
 *
 * 四变体视觉严格遵守设计系统：
 * - primary：bg=brand，#fff，radius-sm，h36；hover→brand-hover；active→brand-active；danger→error
 * - text：透明底，color brand，radius-sm，h36；hover→brand-lighter；danger color error
 * - icon：透明，32×32，radius-sm；hover→bg-hover；danger color error
 * - icon-text：透明，h32，radius-sm；hover→bg-hover
 *
 * hover 仅背景色过渡（0.15s），无 ripple（clickable indication=null）；loading 时 primary 显 spinner，其余图标 spin + 禁用点击。
 *
 * @param label 按钮文字（icon 变体可空）
 * @param variant 按钮形态
 * @param onClick 点击回调
 * @param icon 图标 name（可选；primary/text 用 14px，icon-text 16px，icon 18px）
 * @param danger 危险态（红）
 * @param disabled 禁用
 * @param loading 加载中（primary 显 spinner，其余图标 spin + 禁用点击）
 * @param fullWidth 100% 宽（仅 primary/text 生效）
 * @param badge 角标数字（>0 显示，仅 icon/icon-text）
 * @param height 自定义高度（0 用变体默认）
 */
@Composable
fun MateButton(
    label: String? = null,
    variant: MateButtonVariant = MateButtonVariant.PRIMARY,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: String? = null,
    danger: Boolean = false,
    disabled: Boolean = false,
    loading: Boolean = false,
    fullWidth: Boolean = false,
    badge: Int = 0,
    height: Dp = 0.dp,
) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    val pressed by interaction.collectIsPressedAsState()
    val effectiveDisabled = disabled || loading
    val semantic = LocalSemanticColors.current
    val reducedMotion = LocalReducedMotion.current

    // 背景与文字色按变体 × 状态组合计算（无 ripple，纯背景过渡）。
    val pair: Pair<Color, Color> = when (variant) {
        MateButtonVariant.PRIMARY -> when {
            effectiveDisabled -> semantic.border to Color.White
            danger -> (if (pressed) ErrorColor.copy(alpha = 0.85f) else ErrorColor) to Color.White
            pressed -> BrandActive to Color.White
            hovered -> BrandHover to Color.White
            else -> BrandColor to Color.White
        }
        MateButtonVariant.TEXT -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            danger -> (if (hovered) BrandLighter else Color.Transparent) to ErrorColor
            hovered -> BrandLighter to BrandColor
            else -> Color.Transparent to if (danger) ErrorColor else BrandColor
        }
        MateButtonVariant.ICON -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            hovered -> semantic.bgHover to semantic.textPrimary
            else -> Color.Transparent to if (danger) ErrorColor else semantic.textSecondary
        }
        MateButtonVariant.ICON_TEXT -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            hovered -> semantic.bgHover to semantic.textPrimary
            else -> Color.Transparent to semantic.textPrimary
        }
    }
    val bgColor = pair.first
    val contentColor = pair.second

    val resolvedHeight = when {
        height > 0.dp -> height
        variant == MateButtonVariant.ICON || variant == MateButtonVariant.ICON_TEXT -> 32.dp
        else -> 36.dp
    }
    val shape = RoundedCornerShape(3.dp)
    val iconSize = when (variant) {
        MateButtonVariant.ICON -> 18.dp
        MateButtonVariant.ICON_TEXT -> 16.dp
        else -> if (icon != null) 14.dp else 0.dp
    }
    val horizontalPadding = when (variant) {
        MateButtonVariant.ICON -> 0.dp
        MateButtonVariant.ICON_TEXT -> 12.dp
        MateButtonVariant.TEXT -> 8.dp
        MateButtonVariant.PRIMARY -> 16.dp
    }

    // 无 ripple 点击：clickable indication=null，仅靠 hoverable 驱动背景色。
    val base = modifier
        .then(if (fullWidth && (variant == MateButtonVariant.PRIMARY || variant == MateButtonVariant.TEXT)) Modifier.fillMaxWidth() else Modifier.wrapContentSize())
        .height(resolvedHeight)
        .clip(shape)
        .background(bgColor)
        .alpha(if (effectiveDisabled) 0.5f else 1f)
        .hoverable(interaction)
        .then(
            if (effectiveDisabled) Modifier
            else Modifier.clickable(
                interactionSource = interaction,
                indication = null,
                onClick = onClick,
            ),
        )

    // icon 变体固定正方形（32×32 命中区）。
    val sized = if (variant == MateButtonVariant.ICON) base.size(32.dp) else base
    Row(
        modifier = sized.padding(horizontal = horizontalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        if (loading && variant == MateButtonVariant.PRIMARY) {
            MateButtonSpinner(size = 16.dp, color = contentColor, reducedMotion = reducedMotion)
            if (label != null) Spacer(Modifier.width(8.dp))
        } else if (icon != null) {
            val spin = loading && variant != MateButtonVariant.PRIMARY && !reducedMotion
            MateIcon(name = icon, size = iconSize, tint = contentColor, spin = spin)
            if (label != null) Spacer(Modifier.width(4.dp))
        }
        if (label != null) {
            val fontSize = if (variant == MateButtonVariant.TEXT || variant == MateButtonVariant.ICON_TEXT) 13.sp else 14.sp
            Text(text = label, color = contentColor, fontSize = fontSize, fontWeight = FontWeight.Medium)
        }
        if (badge > 0 && (variant == MateButtonVariant.ICON || variant == MateButtonVariant.ICON_TEXT)) {
            Box(modifier = Modifier.padding(start = 2.dp)) { MateBadge(count = badge) }
        }
    }
}

/** primary 加载态的小 spinner（16×16，0.8s 线性旋转）。 */
@Composable
private fun MateButtonSpinner(size: Dp, color: Color, reducedMotion: Boolean) {
    if (reducedMotion) {
        // 降级为静态半透明圆，提示「加载中」但不旋转。
        Box(Modifier.size(size).clip(CircleShape).background(color.copy(alpha = 0.3f)))
        return
    }
    val transition = rememberInfiniteTransition(label = "mate-btn-spin")
    val rotation by transition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(800, easing = LinearEasing),
            repeatMode = RepeatMode.Restart,
        ),
        label = "mate-btn-spin-r",
    )
    // 简化 spinner：旋转的环（外圈淡色轨道 + 内圈高亮弧，用两段 Box 近似）。
    Box(modifier = Modifier.size(size).rotate(rotation)) {
        Box(Modifier.size(size).clip(CircleShape).background(color.copy(alpha = 0.3f)))
    }
}

/**
 * 角标（badge）。
 *
 * min-w 18，h18，padding 0 5，radius 9，bg brand，#fff，caption semibold（>99 显示 99+）。
 */
@Composable
private fun MateBadge(count: Int) {
    val display = if (count > 99) "99+" else count.toString()
    Box(
        modifier = Modifier
            .clip(CircleShape)
            .background(BrandColor)
            .padding(horizontal = 5.dp, vertical = 1.dp)
            .height(16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(display, color = Color.White, fontSize = 11.sp, fontWeight = FontWeight.SemiBold)
    }
}
