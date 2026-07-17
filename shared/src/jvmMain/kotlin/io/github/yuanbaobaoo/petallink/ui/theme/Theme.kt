package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material.MaterialTheme
import androidx.compose.material.darkColors
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color

// 品牌色（v2：logo 蓝 #0053DB 系）—— Compose Color 形式，供组件直接使用。
val BrandColor = Color(0xFF0053DB)
val BrandHover = Color(0xFF4A8BF0)
val BrandActive = Color(0xFF0047B8)
val BrandLight = Color(0xFFB7D0F7)
val Brand100 = Color(0xFFDCE8FC)
val BrandLighter = Color(0xFFEFF4FE)
val SuccessColor = Color(0xFF0CA678)
val SuccessBg = Color(0xFFE3F5EE)
val WarningColor = Color(0xFFF08C00)
val WarningBg = Color(0xFFFFF3DE)
val ErrorColor = Color(0xFFE5484D)
val ErrorBg = Color(0xFFFDECEC)
val InfoColor = Color(0xFF3B82F6)
val InfoBg = Color(0xFFE8F0FE)

// 文件类型 tile 类别色
val FolderAmber = Color(0xFFF0A63C)
val FolderAmberBg = Color(0xFFFFF4DE)
val TileDoc = Color(0xFF6366F1)
val TileDocBg = Color(0xFFEEF2FF)
val TileImage = Color(0xFFEC4899)
val TileImageBg = Color(0xFFFDE7F3)
val TileVideo = Color(0xFF8B5CF6)
val TileVideoBg = Color(0xFFF3E8FF)
val TileSheet = Color(0xFF10B981)
val TileSheetBg = Color(0xFFE6F7EE)

// 控件专用色
val SwitchOffTrack = Color(0xFFE3E3E6)

/**
 * 品牌渐变（135° 浅蓝 → logo 蓝），主按钮、Logo tile、进度条、更新卡片共用。
 */
val BrandGradient: Brush = Brush.linearGradient(listOf(BrandHover, BrandColor))

/**
 * 品牌浅底渐变（空状态徽章等轻量容器）。
 */
val BrandGradientSoft: Brush = Brush.linearGradient(listOf(BrandLighter, Brand100))

// 语义别名 Color 形式（浅色默认值；随主题切换通过 LocalSemanticColors 注入）。
object LightPalette {
    val BgPage = Color(0xFFF5F5F7)
    val BgContainer = Color(0xFFFFFFFF)
    val BgFill = Color(0xFFF1F1F3)
    val BgHover = Color(0xFFF7F7F9)
    val BgActive = Color(0xFFECECEF)
    val Border = Color(0x0F000000)
    val BorderHover = Color(0x1A000000)
    val TextPrimary = Color(0xE6000000)
    val TextSecondary = Color(0x99000000)
    val TextPlaceholder = Color(0x59000000)
}

/**
 * 深色主题下的语义别名集合，字段含义与 [LightPalette] 一一对应。
 */
object DarkPalette {
    val BgPage = Color(0xFF181818)
    val BgContainer = Color(0xFF242424)
    val BgFill = Color(0xFF2C2C2C)
    val BgHover = Color(0xFF2C2C2C)
    val BgActive = Color(0xFF333333)
    val Border = Color(0x14FFFFFF)
    val BorderHover = Color(0x29FFFFFF)
    val TextPrimary = Color(0xE6FFFFFF)
    val TextSecondary = Color(0x99FFFFFF)
    val TextPlaceholder = Color(0x59FFFFFF)
}

private val MaterialLightColors = lightColors(
    primary = BrandColor,
    onPrimary = Color.White,
    secondary = BrandHover,
    background = LightPalette.BgPage,
    surface = LightPalette.BgContainer,
    error = ErrorColor,
)

private val MaterialDarkColors = darkColors(
    primary = BrandHover,
    onPrimary = Color.White,
    secondary = BrandColor,
    background = DarkPalette.BgPage,
    surface = DarkPalette.BgContainer,
    error = ErrorColor,
)

/**
 * 当前明暗主题下的语义别名集合。
 */
class ThemeSemanticColors(
    val bgPage: Color,
    val bgContainer: Color,
    val bgFill: Color,
    val bgHover: Color,
    val bgActive: Color,
    val border: Color,
    val borderHover: Color,
    val textPrimary: Color,
    val textSecondary: Color,
    val textPlaceholder: Color,
)

private val lightSemantic = ThemeSemanticColors(
    bgPage = LightPalette.BgPage,
    bgContainer = LightPalette.BgContainer,
    bgFill = LightPalette.BgFill,
    bgHover = LightPalette.BgHover,
    bgActive = LightPalette.BgActive,
    border = LightPalette.Border,
    borderHover = LightPalette.BorderHover,
    textPrimary = LightPalette.TextPrimary,
    textSecondary = LightPalette.TextSecondary,
    textPlaceholder = LightPalette.TextPlaceholder,
)

private val darkSemantic = ThemeSemanticColors(
    bgPage = DarkPalette.BgPage,
    bgContainer = DarkPalette.BgContainer,
    bgFill = DarkPalette.BgFill,
    bgHover = DarkPalette.BgHover,
    bgActive = DarkPalette.BgActive,
    border = DarkPalette.Border,
    borderHover = DarkPalette.BorderHover,
    textPrimary = DarkPalette.TextPrimary,
    textSecondary = DarkPalette.TextSecondary,
    textPlaceholder = DarkPalette.TextPlaceholder,
)

/**
 * 当前主题语义别名；由 [PetalLinkTheme] 按 `isSystemInDarkTheme` 注入。
 */
val LocalSemanticColors = staticCompositionLocalOf { lightSemantic }

val LocalReducedMotion = compositionLocalOf { false }

private val systemReducedMotion: Boolean by lazy {
    System.getProperty("petallink.reduceMotion")?.toBooleanStrictOrNull()
        ?: runCatching {
            java.awt.Toolkit.getDefaultToolkit().getDesktopProperty("apple.awt.reduceMotion") as? Boolean
        }.getOrNull()
        ?: false
}

/**
 * 应用主题入口：按系统明暗模式注入 Material 配色、语义别名与减弱动效偏好。
 */
@Composable
fun PetalLinkTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) MaterialDarkColors else MaterialLightColors
    val semantic = if (dark) darkSemantic else lightSemantic
    CompositionLocalProvider(
        LocalReducedMotion provides systemReducedMotion,
        LocalSemanticColors provides semantic,
    ) {
        MaterialTheme(colors = colors, content = content)
    }
}
