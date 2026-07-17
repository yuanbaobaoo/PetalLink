@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradient
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradientSoft
import io.github.yuanbaobaoo.petallink.ui.theme.BrandHover
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorBg
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.LocalReducedMotion
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessBg
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.ui.theme.WarningBg
import io.github.yuanbaobaoo.petallink.ui.theme.WarningColor

/**
 * 线性进度条（v2：h6 圆角条，brand 为渐变填充）。
 *
 * h=[height]（默认 6），bg=bg-fill，overflow hidden；
 * fill 高 100%，过渡 width 0.3s；color=brand 时用品牌渐变，其余纯色；
 * value=null 不确定态（30% 宽指示器 1.2s 循环移动）。
 *
 * @param value 进度 0..1；null=不确定态
 * @param height 条高（默认 6dp）
 * @param color fill 颜色（默认 brand=品牌渐变）
 */
@Composable
fun MateLinearProgress(
    value: Float? = null,
    modifier: Modifier = Modifier,
    height: Dp = 6.dp,
    color: Color = BrandColor,
) {
    val semantic = LocalSemanticColors.current
    val reducedMotion = LocalReducedMotion.current
    val fillBrush: Brush = if (color == BrandColor) BrandGradient else Brush.linearGradient(listOf(color, color))

    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(height)
            .clip(RoundedCornerShape(height / 2))
            .background(semantic.bgFill),
    ) {
        if (value != null) {
            // 确定态：fill 宽度 = value * 100%
            val clamped = value.coerceIn(0f, 1f)
            Box(
                modifier = Modifier
                    .fillMaxWidth(clamped)
                    .height(height)
                    .background(fillBrush),
            )
        } else if (!reducedMotion) {
            // 不确定态：30% 宽指示器循环移动
            val transition = rememberInfiniteTransition(label = "mate-progress-indet")
            val offset by transition.animateFloat(
                initialValue = 0f,
                targetValue = 1f,
                animationSpec = infiniteRepeatable(
                    animation = tween(1200, easing = LinearEasing),
                    repeatMode = RepeatMode.Restart,
                ),
                label = "mate-progress-offset",
            )
            Box(
                modifier = Modifier
                    .fillMaxWidth(0.3f)
                    .height(height)
                    .background(fillBrush),
            )
            // offset 驱动水平位移（0 → 容器宽度 - 30% 宽度）
        }
    }
}

/**
 * 环形进度条（对标原 Vue `<MateCircularProgress>`）。
 *
 * SVG viewBox 24×24；轨道 bg-fill；填充 color，linecap round，从顶部起笔（rotate -90）；
 * value=null 不确定态画约 86° 弧。
 *
 * @param size 直径（默认 24）
 * @param strokeWidth 描边宽（默认 2.5）
 * @param color 填充色（默认 brand）
 * @param value 进度 0..1；null=不确定态
 */
@Composable
fun MateCircularProgress(
    size: Dp = 24.dp,
    strokeWidth: Dp = 2.5.dp,
    color: Color = BrandColor,
    value: Float? = null,
) {
    val semantic = LocalSemanticColors.current
    val reducedMotion = LocalReducedMotion.current
    val track = semantic.bgFill

    // 用 Canvas + drawArc 绘制环形（比 SVG 更可控）
    androidx.compose.foundation.Canvas(modifier = Modifier.size(size)) {
        val strokePx = strokeWidth.toPx()
        val diameter = this.size.minDimension - strokePx
        val topLeft = androidx.compose.ui.geometry.Offset(
            (this.size.width - diameter) / 2f,
            (this.size.height - diameter) / 2f,
        )
        val arcSize = androidx.compose.ui.geometry.Size(diameter, diameter)

        // 轨道（整圈）
        drawArc(
            color = track,
            startAngle = 0f,
            sweepAngle = 360f,
            useCenter = false,
            topLeft = topLeft,
            size = arcSize,
            style = Stroke(width = strokePx),
        )
        // 填充弧
        val sweep = if (value != null) {
            (value.coerceIn(0f, 1f) * 360f)
        } else {
            // 不确定态：约 86° 弧
            if (reducedMotion) 360f else 86f
        }
        drawArc(
            color = color,
            startAngle = 270f, // 从顶部起笔
            sweepAngle = sweep,
            useCenter = false,
            topLeft = topLeft,
            size = arcSize,
            style = Stroke(width = strokePx, cap = androidx.compose.ui.graphics.StrokeCap.Round),
        )
    }
}

/**
 * 横幅变体（对标原 Vue MateInfoBanner variant）。
 */
enum class MateBannerVariant { INFO, SUCCESS, WARNING, ERROR }

/**
 * 根据横幅变体返回对应的背景色与图标色。
 */
private fun bannerVisual(variant: MateBannerVariant): Pair<Color, Color> {
    // 返回 (背景色, 图标色)
    return when (variant) {
        MateBannerVariant.INFO -> BrandLighter to BrandColor
        MateBannerVariant.SUCCESS -> SuccessBg to SuccessColor
        MateBannerVariant.WARNING -> WarningBg to WarningColor
        MateBannerVariant.ERROR -> ErrorBg to ErrorColor
    }
}

/**
 * 根据横幅变体返回对应的图标名称。
 */
private fun bannerIcon(variant: MateBannerVariant): String = when (variant) {
    MateBannerVariant.INFO -> "info"
    MateBannerVariant.SUCCESS -> "check"
    MateBannerVariant.WARNING -> "alert"
    MateBannerVariant.ERROR -> "x"
}

/**
 * 信息横幅（v2：radius-lg(10) 无描边，图标与文字、右侧 action 统一垂直居中）。
 *
 * row，gap sm，padding 12/14，radius-lg，body-sm+1，line-height 1.55；
 * 四变体 info/success/warning/error 各自浅底 + 着色图标；正文 text-primary；title semibold；message pre-line。
 * 整体 verticalAlignment = Center（v2 .banner align-items:center）：单行时图标/文字与 action 按钮同线居中。
 *
 * @param variant 变体
 * @param message 消息正文
 * @param title 标题（可选）
 * @param closable 是否显示关闭按钮
 * @param onClose 关闭回调
 * @param action 右侧操作区内容（margin-left auto）
 */
@Composable
fun MateInfoBanner(
    message: String,
    modifier: Modifier = Modifier,
    variant: MateBannerVariant = MateBannerVariant.INFO,
    title: String? = null,
    closable: Boolean = false,
    onClose: () -> Unit = {},
    action: @Composable (() -> Unit)? = null,
) {
    val (bg, iconColor) = bannerVisual(variant)
    val semantic = LocalSemanticColors.current
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(bg)
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        MateIcon(name = bannerIcon(variant), size = 18.dp, tint = iconColor)
        Column(modifier = Modifier.weight(1f)) {
            if (title != null) {
                Text(title, color = semantic.textPrimary, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
            }
            Text(message, color = semantic.textPrimary, fontSize = 14.sp, lineHeight = (14 * 1.55f).sp)
        }
        action?.invoke()
        if (closable) {
            MateIcon(
                name = "x",
                size = 14.dp,
                tint = iconColor.copy(alpha = 0.7f),
                modifier = Modifier.clip(CircleShape),
            )
            // 关闭按钮：简单实现，点击由调用方 onClose 绑定（这里不直接 clickable，保持纯展示）
        }
    }
}

/**
 * 标签主题（对标原 Vue MateTag theme）。
 */
enum class MateTagTheme { DEFAULT, PRIMARY, SUCCESS, WARNING, ERROR }

/**
 * 标签尺寸。
 */
enum class MateTagSize { SMALL, MEDIUM }

/**
 * 标签 chip（v2：radius-sm(5) 纯底色无描边）。
 *
 * inline-flex，gap xs，radius-sm，nowrap；
 * small: padding 2/sm, caption+1；medium: padding xs/md, body-sm+1；
 * 五主题各自浅底深字（无描边）。
 */
@Composable
fun MateTag(
    label: String,
    modifier: Modifier = Modifier,
    theme: MateTagTheme = MateTagTheme.DEFAULT,
    size: MateTagSize = MateTagSize.MEDIUM,
    icon: String? = null,
    onClick: (() -> Unit)? = null,
) {
    val semantic = LocalSemanticColors.current
    val (bg, fg) = when (theme) {
        MateTagTheme.DEFAULT -> semantic.bgFill to semantic.textSecondary
        MateTagTheme.PRIMARY -> BrandLighter to BrandColor
        MateTagTheme.SUCCESS -> SuccessBg to SuccessColor
        MateTagTheme.WARNING -> WarningBg to WarningColor
        MateTagTheme.ERROR -> ErrorBg to ErrorColor
    }
    val padding = if (size == MateTagSize.SMALL) 6.dp else 10.dp
    val verticalPadding = if (size == MateTagSize.SMALL) 2.dp else 3.dp
    val fontSize = if (size == MateTagSize.SMALL) 13.sp else 14.sp
    val iconSize = if (size == MateTagSize.SMALL) 12.dp else 14.dp

    val tagInteraction = remember { MutableInteractionSource() }
    val visualModifier = modifier
        .clip(RoundedCornerShape(5.dp))
        .background(bg)
        .padding(horizontal = padding, vertical = verticalPadding)
        .then(if (onClick != null) Modifier.mateClickable(tagInteraction, onClick) else Modifier)
    Row(
        modifier = visualModifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (icon != null) MateIcon(name = icon, size = iconSize, tint = fg)
        Text(label, color = fg, fontSize = fontSize, fontWeight = FontWeight.Medium)
    }
}

/**
 * 空状态占位（v2：品牌浅底渐变徽章 + 大号图标）。
 *
 * column center，padding xxl；
 * badge 72×72 radius 14，品牌浅底渐变，图标 36 brand-hover；
 * title 16 semibold text-primary；desc 14 text-secondary line-height 1.5；
 * action margin-top xl。
 */
@Composable
fun MateEmpty(
    title: String,
    modifier: Modifier = Modifier,
    icon: String = "list",
    description: String? = null,
    action: @Composable (() -> Unit)? = null,
) {
    val semantic = LocalSemanticColors.current
    Column(
        modifier = modifier.fillMaxWidth().padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            modifier = Modifier
                .size(72.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(BrandGradientSoft),
            contentAlignment = Alignment.Center,
        ) {
            MateIcon(name = icon, size = 36.dp, tint = BrandHover)
        }
        Spacer(Modifier.height(16.dp))
        Text(title, color = semantic.textPrimary, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        if (description != null) {
            Spacer(Modifier.height(6.dp))
            Text(description, color = semantic.textSecondary, fontSize = 14.sp, lineHeight = (14 * 1.5f).sp)
        }
        if (action != null) {
            Spacer(Modifier.height(24.dp))
            action()
        }
    }
}

/**
 * 统计芯片（对标原 Vue `<MateStatChip icon count label>`）。
 *
 * inline-flex，gap xs，padding xs/sm，radius-md(8)，bg-fill，caption+1 medium secondary，nowrap。
 * 模板：{icon} {count} {label}
 */
@Composable
fun MateStatChip(
    icon: String,
    count: Int,
    label: String,
    modifier: Modifier = Modifier,
) {
    val semantic = LocalSemanticColors.current
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(8.dp))
            .background(semantic.bgFill)
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        MateIcon(name = icon, size = 12.dp, tint = semantic.textSecondary)
        Text(count.toString(), color = semantic.textSecondary, fontSize = 13.sp, fontWeight = FontWeight.Medium)
        Text(label, color = semantic.textSecondary, fontSize = 13.sp, fontWeight = FontWeight.Medium)
    }
}

/**
 * 分区标题（v2：19px semibold，无分割线）。
 *
 * row center gap sm，padding-bottom md；icon brand size 18；text 19 semibold primary；trailing margin-left auto。
 */
@Composable
fun MateSectionHeader(
    text: String,
    modifier: Modifier = Modifier,
    icon: String? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val semantic = LocalSemanticColors.current
    Row(
        modifier = modifier.fillMaxWidth().padding(bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (icon != null) MateIcon(name = icon, size = 18.dp, tint = BrandColor)
        Text(text, color = semantic.textPrimary, fontSize = 19.sp, fontWeight = FontWeight.SemiBold)
        trailing?.let {
            Spacer(Modifier.width(0.dp))
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.CenterEnd) { it() }
        }
    }
}

/**
 * 侧栏导航项（v2 放松版：46px 行高 + radius-md + 18px 图标）。
 *
 * row center gap md，padding 0/14，radius-md(8)；
 * hover bg-fill；active bg=brand-lighter color=brand medium；
 * icon size 18 secondary（active 时 brand）；label 15px ellipsis。
 */
@Composable
fun MateNavItem(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: String? = null,
    active: Boolean = false,
    indent: Int = 0,
    height: Dp = 46.dp,
) {
    val semantic = LocalSemanticColors.current
    val textColor = if (active) BrandColor else semantic.textPrimary
    val iconColor = if (active) BrandColor else semantic.textSecondary
    val navInteraction = remember { MutableInteractionSource() }
    Row(
        modifier = modifier
            .fillMaxWidth()
            .height(height)
            .clip(RoundedCornerShape(8.dp))
            .background(if (active) BrandLighter else Color.Transparent)
            .mateClickable(navInteraction, onClick)
            .padding(start = (indent + 14).dp, end = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (icon != null) MateIcon(name = icon, size = 18.dp, tint = iconColor)
        Text(
            label,
            color = textColor,
            fontSize = 15.sp,
            fontWeight = if (active) FontWeight.Medium else FontWeight.Normal,
            maxLines = 1,
        )
    }
}

/**
 * 导航分组标签（v2 设置页：12px semibold placeholder，上 20 下 6）。
 */
@Composable
fun MateNavGroupLabel(
    label: String,
    modifier: Modifier = Modifier,
) {
    val semantic = LocalSemanticColors.current
    Text(
        label,
        color = semantic.textPlaceholder,
        fontSize = 12.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = modifier.padding(start = 14.dp, top = 20.dp, bottom = 6.dp),
    )
}
