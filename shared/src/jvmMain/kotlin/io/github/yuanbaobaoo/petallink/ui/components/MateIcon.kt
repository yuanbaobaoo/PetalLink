package io.github.yuanbaobaoo.petallink.ui.components

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.theme.LocalReducedMotion
import java.util.concurrent.ConcurrentHashMap
import org.jetbrains.skia.Data
import org.jetbrains.skia.Picture
import org.jetbrains.skia.PictureRecorder
import org.jetbrains.skia.Rect
import org.jetbrains.skia.svg.SVGDOM

/**
 * 图标 name 注册表（对标原 Vue IconSprite.vue 的 32 个 `<symbol>`）。
 *
 * 与 resources/icons/ 下的 .svg 文件一一对应。
 */
public object MateIcons {
    /** 已注册的全部图标 name。 */
    public val NAMES: List<String> = listOf(
        "cloud", "local", "folder", "folder-open", "file", "file-text", "image", "chart",
        "search", "refresh", "transfer", "settings", "check", "sync", "alert", "clock",
        "copy", "info", "lock", "list", "arrow", "pause", "play", "x",
        "video", "edit", "archive", "download", "share", "trash", "github", "gitcode",
    )

    /** SVG viewBox 内在尺寸（所有图标统一 24×24）。 */
    public const val VIEWBOX_SIZE: Float = 24f
}

/**
 * SVG 字节与 Picture 缓存；进程级单例，避免每次重组重新解析。
 *
 * Picture 按 (name, pixelSize) 缓存：Skia SVGDOM 渲染结果与容器像素尺寸绑定，
 * 不同显示尺寸需要不同 Picture，但同尺寸图标只解析一次。
 */
internal object SvgIconCache {
    private val bytesCache = ConcurrentHashMap<String, ByteArray?>()

    // 加载 resources/icons/<name>.svg；文件缺失返回 null（渲染降级为空 Box）。
    private fun loadBytes(name: String): ByteArray? = bytesCache.computeIfAbsent(name) {
        javaClass.getResourceAsStream("/icons/$name.svg")?.use { it.readAllBytes() }
    }

    /**
     * 把指定 name 的 SVG 渲染为 [Picture]。
     *
     * 图标内部按 [pixelSize] 缩放，viewBox 24×24 等比映射。
     */
    fun renderPicture(name: String, pixelSize: Int): Picture? {
        if (pixelSize <= 0) return null
        val bytes = loadBytes(name) ?: return null
        return runCatching {
            // SVGDOM 解析 SVG；setContainerSize 后 render 才会按目标像素尺寸缩放。
            val dom = SVGDOM(Data.makeFromBytes(bytes))
            dom.setContainerSize(pixelSize.toFloat(), pixelSize.toFloat())
            val recorder = PictureRecorder()
            val canvas = recorder.beginRecording(
                Rect.makeXYWH(0f, 0f, pixelSize.toFloat(), pixelSize.toFloat()),
            )
            dom.render(canvas)
            recorder.finishRecordingAsPicture()
        }.getOrNull()
    }
}

/**
 * 矢量图标（对标原 Vue `<MateIcon name size spin>`）。
 *
 * 渲染流程：从 resources/icons/<name>.svg 加载 → Skia SVGDOM 解析为 Picture → Compose DrawScope 重放。
 * tint 通过 [ColorFilter.tint]（SrcIn 混合）实现，等价于原 SVG 的 `stroke="currentColor"`。
 * spin 用 Modifier.graphicsLayer 旋转，tint 始终生效。
 *
 * @param name 图标 name（不带扩展名），如 "cloud"、"folder-open"
 * @param size 图标显示尺寸（dp），默认 16
 * @param tint 着色，默认当前文字色；遵循原 `currentColor` 语义
 * @param spin 是否旋转（1s 线性循环），用于同步中图标；受 ReducedMotion 降级
 */
@Composable
public fun MateIcon(
    name: String,
    modifier: Modifier = Modifier,
    size: Dp = 16.dp,
    tint: Color = Color.Unspecified,
    spin: Boolean = false,
) {
    val density = LocalDensity.current.density
    // density 转像素后参与 Picture 缓存 key；DPI 变化时会重新解析。
    val pixelSize = remember(density, size) {
        (size.value * density).toInt().coerceAtLeast(1)
    }
    val picture = remember(name, pixelSize) {
        SvgIconCache.renderPicture(name, pixelSize)
    }
    if (picture == null) {
        // 图标缺失：渲染占位空盒，不抛异常（避免单个图标加载失败拖垮整页）。
        Canvas(modifier.size(size)) {}
        return
    }
    val resolvedTint = if (tint == Color.Unspecified) Color(0xFF181818) else tint
    val reducedMotion = LocalReducedMotion.current
    val effectiveSpin = spin && !reducedMotion
    val rotation = if (effectiveSpin) {
        val transition = rememberInfiniteTransition(label = "mate-icon-spin")
        transition.animateFloat(
            initialValue = 0f,
            targetValue = 360f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 1000, easing = LinearEasing),
                repeatMode = RepeatMode.Restart,
            ),
            label = "mate-icon-rotation",
        ).value
    } else {
        0f
    }

    val rotationModifier = if (effectiveSpin) {
        Modifier.iconRotate(rotation)
    } else {
        Modifier
    }
    Canvas(modifier.size(size).then(rotationModifier)) {
        drawSvgPicture(picture, resolvedTint)
    }
}

/** 重放 Picture 并叠加 tint（SrcIn：把图标非透明像素染成 tint）。 */
private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawSvgPicture(
    picture: Picture,
    tint: Color,
) {
    drawIntoCanvas { canvas ->
        // Skia Paint 携带 ColorFilter.makeBlend(SRC_IN)，把任意 currentColor 解析结果统一染成 tint。
        // makeBlend 的 color 是 ARGB int（Compose Color.toArgb 提供）。
        val paint = org.jetbrains.skia.Paint().apply {
            colorFilter = org.jetbrains.skia.ColorFilter.makeBlend(
                tint.toArgb(),
                org.jetbrains.skia.BlendMode.SRC_IN,
            )
        }
        canvas.nativeCanvas.drawPicture(picture, null, paint)
    }
}

/**
 * 用 graphicsLayer 旋转 Modifier（封装以保持调用点简洁）。
 *
 * Compose 的 rotate 是 Modifier 扩展（GPU 合成变换），旋转时 tint 仍通过 Picture 内 Paint 生效。
 */
private fun Modifier.iconRotate(degrees: Float): Modifier = this.rotate(degrees)
