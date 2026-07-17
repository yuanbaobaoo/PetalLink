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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors

/**
 * 应用 Logo（对标原 Vue `<MateAppLogo size text container>`）。
 *
 * 从 resources 加载 PNG 图标；container 模式包 64×64 圆角容器 + 品牌阴影。
 * text 非空时在右侧显示「PetalLink」文字（字号 = round(size*0.42)，semibold）。
 *
 * @param size 图标尺寸（dp），默认 26
 * @param text 附加文字，空串则隐藏
 * @param container 是否包 64×64 容器（登录页用）
 */
@Composable
fun MateAppLogo(
    size: Dp = 26.dp,
    text: String = "PetalLink",
    container: Boolean = false,
) {
    if (container) {
        // container 模式：64×64，padding 5，radius 16，品牌阴影
        Box(
            modifier = Modifier
                .size(64.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(Color.White)
                .padding(5.dp),
            contentAlignment = Alignment.Center,
        ) { LogoImage(size = 54.dp) }
        return
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        LogoImage(size = size)
        if (text.isNotEmpty()) {
            Spacer(Modifier.width(6.dp))
            // 文字字号 = round(size_px * 0.42)；近似用 size.value*0.42。
            Text(
                text,
                fontSize = (size.value * 0.42f).sp,
                fontWeight = FontWeight.SemiBold,
                color = Color(0xFF181818),
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
fun MateLogoWithText(height: Dp = 32.dp) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        LogoImage(size = height)
        Spacer(Modifier.width(6.dp))
        Text(
            "PetalLink",
            fontSize = (height.value * 0.36f).sp,
            fontWeight = FontWeight.SemiBold,
            color = Color(0xFF181818),
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
            modifier = Modifier.size(size).clip(RoundedCornerShape(size * 0.2f)),
        )
    } else {
        // 回退：品牌色圆角方块 + 云朵图标（仅在 logo.png 加载失败时出现）
        Box(
            modifier = Modifier
                .size(size)
                .clip(RoundedCornerShape(size * 0.2f))
                .background(BrandColor),
            contentAlignment = Alignment.Center,
        ) {
            io.github.yuanbaobaoo.petallink.ui.components.MateIcon(
                name = "cloud",
                size = size * 0.6f,
                tint = Color.White,
            )
        }
    }
}

/** 加载 resources/assets/logo.png 为 ImageBitmap；失败返回 null。 */
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
    val semantic = LocalSemanticColors.current
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

/** 竖分隔线（对标原 Vue `<MateVerticalSeparator height>`）。w 1px，bg border。 */
@Composable
fun MateVerticalSeparator(height: Dp = 20.dp) {
    val semantic = LocalSemanticColors.current
    Box(
        modifier = Modifier
            .width(1.dp)
            .height(height)
            .background(semantic.border),
    )
}

/** 底部分隔线容器（对标原 Vue `<MateBottomDivider>`）。border-bottom 0.5px。 */
@Composable
fun MateBottomDivider(
    content: @Composable () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    Box(
        modifier = Modifier.fillMaxSize(),
    ) {
        content()
        // 底部 0.5px 分隔线（Compose 用 0.5.dp 近似 hairline）
        Box(
            modifier = Modifier
                .fillMaxSize()
                .height(0.5.dp)
                .background(semantic.border),
        )
    }
}
