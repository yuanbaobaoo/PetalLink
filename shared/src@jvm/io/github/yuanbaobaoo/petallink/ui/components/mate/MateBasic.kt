@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme

/**
 * 应用 Logo（v2：直接使用真实 logo.png，品牌蓝 squircle）。
 *
 * 从 resources 加载 PNG 图标；container 模式显示 64×64 大图（登录页用），不再包白色容器。
 * text 非空时在右侧显示「PetalLink」文字（字号 = round(size*0.42)，semibold）。
 *
 * @param size 图标尺寸（dp），默认 26
 * @param text 附加文字，空串则隐藏
 * @param container 是否 64×64 大图模式（登录页用）
 */
@Composable
fun MateAppLogo(
    size: Dp = PetalTheme.metrics.basic.compactLogoSize,
    text: String = "PetalLink",
    container: Boolean = false,
) {
    if (container) {
        // container 模式：64×64 真实 logo（自带品牌蓝 squircle 底）
        LogoImage(size = PetalTheme.metrics.basic.largeLogoSize)
        return
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        LogoImage(size = size)
        if (text.isNotEmpty()) {
            Spacer(Modifier.width(PetalTheme.metrics.basic.compactLogoTextSpacing))
            // 文字字号 = round(size_px * 0.42)；近似用 size.value*0.42。
            Text(
                text,
                style = PetalTheme.typography.brand.compactLogoLabel,
                color = PetalTheme.colors.appLogoCompactText,
            )
        }
    }
}

/**
 * Logo + 文字组合（对标原 Vue `<MateLogoWithText height>`）。
 *
 * 内部复用 [MateAppLogo]，PNG 加载失败回退纯文字。
 *
 * @param height 整体高度（dp），默认 32；图标 = height，文字字号 = round(height*0.36)
 */
@Composable
fun MateLogoWithText(height: Dp = PetalTheme.metrics.basic.fullLogoHeight) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        LogoImage(size = height)
        Spacer(Modifier.width(PetalTheme.metrics.basic.fullLogoTextSpacing))
        Text(
            "PetalLink",
            style = PetalTheme.typography.brand.fullLogoLabel,
            color = PetalTheme.colors.appLogoFullText,
        )
    }
}

/**
 * 从 resources/assets/logo.png 加载真实 Logo（对标原 Vue `@assets/logo.png`）。
 *
 * 进程级 ImageBitmap 缓存，避免每次重组重新解码；PNG 缺失才回退品牌色圆角方块占位。
 */
@Composable
private fun LogoImage(size: Dp) {
    // 进程级缓存：logo.png 只解码一次（1024×1024 RGBA，解码开销不宜重复）。
    val bitmap = remember { loadLogoBitmap() }
    if (bitmap != null) {
        Image(
            bitmap = bitmap,
            contentDescription = "PetalLink",
            modifier = Modifier.size(size).clip(RoundedCornerShape(size * 0.225f)),
        )
    } else {
        // 回退：品牌渐变圆角方块 + 云朵图标（仅在 logo.png 加载失败时出现）
        Box(
            modifier = Modifier
                .size(size)
                .clip(RoundedCornerShape(size * 0.225f))
                .background(io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme.colors.brandGradient),
            contentAlignment = Alignment.Center,
        ) {
            io.github.yuanbaobaoo.petallink.ui.components.MateIcon(
                name = "cloud",
                size = size * 0.6f,
                tint = PetalTheme.colors.appLogoCompactIcon,
            )
        }
    }
}

/**
 * 加载 resources/assets/logo.png 为 ImageBitmap；失败返回 null。
 */
private fun loadLogoBitmap(): ImageBitmap? = runCatching {
    val loader = Thread.currentThread().contextClassLoader ?: ClassLoader.getSystemClassLoader()
    val stream = loader.getResourceAsStream("assets/logo.png") ?: return null
    val bytes = stream.use { it.readAllBytes() }
    org.jetbrains.skia.Image.makeFromEncoded(bytes).toComposeImageBitmap()
}.getOrNull()

/**
 * 页面脚手架（对标原 Vue `<MateScaffold flush>`）。
 *
 * 全屏 column 容器；默认 bg=bg-page，flush 时 bg=bg-container。
 */
@Composable
fun MateScaffold(
    flush: Boolean = false,
    content: @Composable () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Column(
        modifier = Modifier.fillMaxSize().background(if (flush) semantic.bgContainer else semantic.bgPage),
    ) { content() }
}

/**
 * 悬停探测器（对标原 Vue `<MateHover>` scoped slot）。
 *
 * 用 [hoverable] + [collectIsHoveredAsState] 暴露 hovered 状态给 content。
 * 鼠标指针样式（Hand/Default）由调用方按需在 content 外层 Modifier 控制。
 *
 * @param content 接收 hovered 的内容 lambda
 */
@Composable
fun MateHover(
    content: @Composable (hovered: Boolean) -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val hovered by interaction.collectIsHoveredAsState()
    Box(modifier = Modifier.hoverable(interaction)) { content(hovered) }
}

/**
 * 竖分隔线（对标原 Vue `<MateVerticalSeparator height>`）。w 1px，bg border。
 */
@Composable
fun MateVerticalSeparator(height: Dp = PetalTheme.metrics.basic.verticalSeparatorHeight) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Box(
        modifier = Modifier
            .width(PetalTheme.metrics.basic.verticalSeparatorWidth)
            .height(height)
            .background(semantic.border),
    )
}

/**
 * 底部分隔线容器（对标原 Vue `<MateBottomDivider>`）。border-bottom 0.5px。
 */
@Composable
fun MateBottomDivider(
    content: @Composable () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Box(
        modifier = Modifier.fillMaxSize(),
    ) {
        content()
        // 底部 0.5px 分隔线（Compose 用 0.5.dp 近似 hairline）
        Box(
            modifier = Modifier
                .fillMaxSize()
                .height(PetalTheme.metrics.basic.bottomBorderThickness)
                .background(semantic.border),
        )
    }
}
