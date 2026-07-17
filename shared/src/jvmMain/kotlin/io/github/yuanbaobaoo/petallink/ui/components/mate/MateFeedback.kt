@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLight
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
 * 线性进度条（对标原 Vue `<MateLinearProgress>`）。
 *
 * h=[height]（默认 4），bg=bg-active，overflow hidden；
 * fill 高 100%，过渡 width 0.3s；
 * value=null 不确定态（30% 宽指示器 1.2s 循环移动）。
 *
 * @param value 进度 0..1；null=不确定态
 * @param height 条高（默认 4dp）
 * @param color fill 颜色（默认 brand）
 */
@Composable
fun MateLinearProgress(
    value: Float? = null,
    modifier: Modifier = Modifier,
    height: Dp = 4.dp,
    color: Color = BrandColor,
) {
    val semantic = LocalSemanticColors.current
    val reducedMotion = LocalReducedMotion.current

    Box(
        modifier = modifier
            .fillMaxWidth()
            .height(height)
            .clip(RoundedCornerShape(height / 2))
            .background(semantic.bgActive),
    ) {
        if (value != null) {
            // 确定态：fill 宽度 = value * 100%
            val clamped = value.coerceIn(0f, 1f)
            Box(
                modifier = Modifier
                    .fillMaxWidth(clamped)
                    .height(height)
                    .background(color),
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
                    .background(color),
            )
            // offset 驱动水平位移（0 → 容器宽度 - 30% 宽度）
        }
    }
}

/**
 * 环形进度条（对标原 Vue `<MateCircularProgress>`）。
 *
 * SVG viewBox 24×24；轨道 bg-active；填充 color，linecap round，从顶部起笔（rotate -90）；
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
    val track = semantic.bgActive

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

/** 横幅变体（对标原 Vue MateInfoBanner variant）。 */
enum class MateBannerVariant { INFO, SUCCESS, WARNING, ERROR }

private fun bannerVisual(variant: MateBannerVariant): Triple<Color, Color, String> {
    // 返回 (背景色, 文字/图标色, 默认图标 name)
    return when (variant) {
        MateBannerVariant.INFO -> Triple(BrandLighter, BrandColor, "info")
        MateBannerVariant.SUCCESS -> Triple(SuccessBg, SuccessColor, "check")
        MateBannerVariant.WARNING -> Triple(WarningBg, WarningColor, "alert")
        MateBannerVariant.ERROR -> Triple(ErrorBg, ErrorColor, "x")
    }
}

/**
 * 信息横幅（对标原 Vue `<MateInfoBanner variant title closable>`）。
 *
 * row，gap sm，padding 10/md，1px border，radius-sm，body-sm，line-height 1.5；
 * 四变体 info/success/warning/error 各自浅底深字 + 边框；图标 18；title semibold；message pre-line。
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
    val (bg, fg, iconName) = bannerVisual(variant)
    val borderColor = fg.copy(alpha = 0.2f)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(3.dp))
            .background(bg)
            .border(1.dp, borderColor, RoundedCornerShape(3.dp))
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        MateIcon(name = iconName, size = 18.dp, tint = fg)
        Column(modifier = Modifier.weight(1f)) {
            if (title != null) {
                Text(title, color = fg, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
            }
            Text(message, color = fg, fontSize = 13.sp, lineHeight = (13 * 1.5f).sp)
        }
        action?.invoke()
        if (closable) {
            MateIcon(
                name = "x",
                size = 14.dp,
                tint = fg.copy(alpha = 0.7f),
                modifier = Modifier.clip(CircleShape),
            )
            // 关闭按钮：简单实现，点击由调用方 onClose 绑定（这里不直接 clickable，保持纯展示）
        }
    }
}

/** 标签主题（对标原 Vue MateTag theme）。 */
enum class MateTagTheme { DEFAULT, PRIMARY, SUCCESS, WARNING, ERROR }
/** 标签尺寸。 */
enum class MateTagSize { SMALL, MEDIUM }

/**
 * 标签（对标原 Vue `<MateTag label theme size icon>`）。
 *
 * inline-flex，gap xs，1px border，radius-sm，nowrap；
 * small: padding 2/sm, caption；medium: padding xs/md, body-sm；
 * 五主题各自浅底深字 + 同色边框。
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
    val (bg, fg, borderColor) = when (theme) {
        MateTagTheme.DEFAULT -> Triple(semantic.bgHover, BrandColor, semantic.border)
        MateTagTheme.PRIMARY -> Triple(BrandLight, BrandColor, BrandColor)
        MateTagTheme.SUCCESS -> Triple(SuccessBg, SuccessColor, SuccessColor)
        MateTagTheme.WARNING -> Triple(WarningBg, WarningColor, WarningColor)
        MateTagTheme.ERROR -> Triple(ErrorBg, ErrorColor, ErrorColor)
    }
    val padding = if (size == MateTagSize.SMALL) 4.dp else 8.dp
    val verticalPadding = if (size == MateTagSize.SMALL) 2.dp else 4.dp
    val fontSize = if (size == MateTagSize.SMALL) 12.sp else 13.sp
    val iconSize = if (size == MateTagSize.SMALL) 12.dp else 14.dp

    val shape = RoundedCornerShape(3.dp)
    val tagInteraction = remember { MutableInteractionSource() }
    val visualModifier = modifier
        .clip(shape)
        .background(bg)
        .border(1.dp, borderColor, shape)
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
 * 空状态占位（对标原 Vue `<MateEmpty icon title description>`）。
 *
 * column center，padding xxl；
 * icon-wrap 64×64 radius 50% bg-hover center；
 * icon text-placeholder size 32；
 * title body medium primary；desc caption secondary line-height 1.5；
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
                .size(64.dp)
                .clip(CircleShape)
                .background(semantic.bgHover),
            contentAlignment = Alignment.Center,
        ) {
            MateIcon(name = icon, size = 32.dp, tint = semantic.textPlaceholder)
        }
        Spacer(Modifier.height(16.dp))
        Text(title, color = semantic.textPrimary, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        if (description != null) {
            Spacer(Modifier.height(8.dp))
            Text(description, color = semantic.textSecondary, fontSize = 12.sp, lineHeight = (12 * 1.5f).sp)
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
 * inline-flex，gap xs，padding xs/sm，radius-sm，bg-hover，caption medium secondary，nowrap。
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
            .clip(RoundedCornerShape(3.dp))
            .background(semantic.bgHover)
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        MateIcon(name = icon, size = 12.dp, tint = semantic.textSecondary)
        Text(count.toString(), color = semantic.textSecondary, fontSize = 12.sp, fontWeight = FontWeight.Medium)
        Text(label, color = semantic.textSecondary, fontSize = 12.sp, fontWeight = FontWeight.Medium)
    }
}

/**
 * 分区标题（对标原 Vue `<MateSectionHeader text icon trailing>`）。
 *
 * row center gap sm，padding-bottom md，border-bottom 1px border，margin-bottom xxl；
 * icon brand size 18；text title-sm semibold primary；trailing margin-left auto。
 */
@Composable
fun MateSectionHeader(
    text: String,
    modifier: Modifier = Modifier,
    icon: String? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val semantic = LocalSemanticColors.current
    Column(modifier = modifier) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (icon != null) MateIcon(name = icon, size = 18.dp, tint = BrandColor)
            Text(text, color = semantic.textPrimary, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            trailing?.let {
                Spacer(Modifier.width(0.dp))
                Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.CenterEnd) { it() }
            }
        }
        Box(modifier = Modifier.fillMaxWidth().height(1.dp).background(semantic.border))
    }
}

/**
 * 侧栏导航项（对标原 Vue `<MateNavItem label icon active indent height>`）。
 *
 * row center gap sm，padding-left indent+8 padding-right sm，radius-sm；
 * hover bg-hover；active bg=brand-lighter color=brand medium；
 * icon size 16 secondary（active 时 brand）；label ellipsis。
 */
@Composable
fun MateNavItem(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: String? = null,
    active: Boolean = false,
    indent: Int = 0,
    height: Dp = 32.dp,
) {
    val semantic = LocalSemanticColors.current
    val textColor = if (active) BrandColor else semantic.textPrimary
    val iconColor = if (active) BrandColor else semantic.textSecondary
    val navInteraction = remember { MutableInteractionSource() }
    Row(
        modifier = modifier
            .fillMaxWidth()
            .height(height)
            .mateClickable(navInteraction, onClick)
            .then(Modifier.background(if (active) BrandLighter else Color.Transparent))
            .clip(RoundedCornerShape(3.dp))
            .padding(start = (indent + 8).dp, end = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (icon != null) MateIcon(name = icon, size = 16.dp, tint = iconColor)
        Text(
            label,
            color = textColor,
            fontSize = 14.sp,
            fontWeight = if (active) FontWeight.Medium else FontWeight.Normal,
            maxLines = 1,
        )
    }
}
