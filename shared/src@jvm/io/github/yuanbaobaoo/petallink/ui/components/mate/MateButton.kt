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
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandActive
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradient
import io.github.yuanbaobaoo.petallink.ui.theme.Brand100
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.DesignTokens
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.LocalReducedMotion

/**
 * 按钮形态（v2：主按钮 / 文字 / 图标 / 图标文字 / 软色）。
 */
enum class MateButtonVariant { PRIMARY, TEXT, ICON, ICON_TEXT, SOFT }

/**
 * 统一按钮（v2 重设计）。
 *
 * 五变体视觉严格遵守 v2 设计系统：
 * - primary：品牌渐变底（135° #4A8BF0→#0053DB），#fff，radius-md(8)，h36，带品牌色柔影；hover 降透明；danger→error 纯色
 * - soft：brand-lighter 浅蓝底，brand 字，radius-md(8)，h36；hover→brand-100
 * - text：透明底，brand 字，radius-sm(5)；hover→brand-lighter；danger color error
 * - icon：透明，32×32 正圆形；hover→bg-fill + text-primary；danger color error
 * - icon-text：透明，h36，radius-md(8)；hover→bg-fill
 *
 * hover 仅背景色过渡，无 ripple（clickable indication=null）；loading 时 primary 显 spinner，其余图标 spin + 禁用点击。
 *
 * @param label 按钮文字（icon 变体可空）
 * @param variant 按钮形态
 * @param onClick 点击回调
 * @param icon 图标 name（可选；primary/text 用 14px，icon-text/soft 16px，icon 18px）
 * @param danger 危险态（红）
 * @param disabled 禁用
 * @param loading 加载中（primary 显 spinner，其余图标 spin + 禁用点击）
 * @param fullWidth 100% 宽（仅 primary/text/soft 生效）
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

    // 形状：icon 为正圆，text 为 radius-sm，其余 radius-md
    val shape = when (variant) {
        MateButtonVariant.ICON -> CircleShape
        MateButtonVariant.TEXT -> RoundedCornerShape(5.dp)
        else -> RoundedCornerShape(8.dp)
    }

    // 背景与文字色按变体 × 状态组合计算（无 ripple，纯背景过渡）。
    // primary 返回的 bgColor 仅作 danger/disabled 兜底；正常态用渐变刷。
    val pair: Pair<Color, Color> = when (variant) {
        MateButtonVariant.PRIMARY -> when {
            effectiveDisabled -> semantic.border to Color.White
            danger -> (if (pressed) ErrorColor.copy(alpha = 0.85f) else ErrorColor) to Color.White
            else -> BrandColor to Color.White
        }
        MateButtonVariant.SOFT -> when {
            effectiveDisabled -> semantic.bgFill to semantic.textPlaceholder
            hovered -> Brand100 to BrandColor
            else -> BrandLighter to BrandColor
        }
        MateButtonVariant.TEXT -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            danger -> (if (hovered) BrandLighter else Color.Transparent) to ErrorColor
            hovered -> BrandLighter to BrandColor
            else -> Color.Transparent to if (danger) ErrorColor else BrandColor
        }
        MateButtonVariant.ICON -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            hovered -> semantic.bgFill to semantic.textPrimary
            else -> Color.Transparent to if (danger) ErrorColor else semantic.textSecondary
        }
        MateButtonVariant.ICON_TEXT -> when {
            effectiveDisabled -> Color.Transparent to semantic.textPlaceholder
            hovered -> semantic.bgFill to semantic.textPrimary
            else -> Color.Transparent to semantic.textPrimary
        }
    }
    val bgColor = pair.first
    val contentColor = pair.second

    val resolvedHeight = when {
        height > 0.dp -> height
        variant == MateButtonVariant.ICON -> 32.dp
        else -> 36.dp
    }
    val iconSize = when (variant) {
        MateButtonVariant.ICON -> 18.dp
        MateButtonVariant.ICON_TEXT, MateButtonVariant.SOFT -> 16.dp
        else -> if (icon != null) 14.dp else 0.dp
    }
    val horizontalPadding = when (variant) {
        MateButtonVariant.ICON -> 0.dp
        MateButtonVariant.ICON_TEXT -> 14.dp
        MateButtonVariant.TEXT -> 8.dp
        MateButtonVariant.SOFT -> 16.dp
        MateButtonVariant.PRIMARY -> 18.dp
    }

    // primary 正常态（非 danger 非禁用）用品牌渐变 + 品牌色柔影，其余用纯色背景。
    val useGradient = variant == MateButtonVariant.PRIMARY && !danger && !effectiveDisabled
    val base = modifier
        .then(
            if (fullWidth && variant != MateButtonVariant.ICON) Modifier.fillMaxWidth()
            else Modifier.wrapContentSize(),
        )
        .height(resolvedHeight)
        .then(
            if (useGradient) Modifier.shadow(
                elevation = 6.dp,
                shape = shape,
                clip = false,
                ambientColor = BrandColor.copy(alpha = 0.35f),
                spotColor = BrandColor.copy(alpha = 0.35f),
            ) else Modifier,
        )
        .clip(shape)
        .then(
            if (useGradient) Modifier.background(BrandGradient)
            else if (variant == MateButtonVariant.PRIMARY && pressed && !danger && !effectiveDisabled) Modifier.background(BrandActive)
            else Modifier.background(bgColor),
        )
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
            if (label != null) Spacer(Modifier.width(6.dp))
        }
        if (label != null) {
            val fontSize = if (variant == MateButtonVariant.TEXT) {
                DesignTokens.FONT_BODY_SM.sp
            } else {
                DesignTokens.FONT_BODY.sp
            }
            Text(text = label, color = contentColor, fontSize = fontSize, fontWeight = FontWeight.Medium)
        }
        if (badge > 0 && (variant == MateButtonVariant.ICON || variant == MateButtonVariant.ICON_TEXT)) {
            Box(modifier = Modifier.padding(start = 2.dp)) { MateBadge(count = badge) }
        }
    }
}

/**
 * primary 加载态的小 spinner（16×16，0.8s 线性旋转）。
 */
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
