package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material.MaterialTheme
import androidx.compose.material.darkColors
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

// 品牌色（对标 DesignTokens）—— Compose Color 形式，供组件直接使用。
val BrandColor = Color(0xFF0052D9)
val BrandHover = Color(0xFF366EF4)
val BrandActive = Color(0xFF003CAB)
val BrandLight = Color(0xFFD9E1FF)
val BrandLighter = Color(0xFFF2F3FF)
val SuccessColor = Color(0xFF2BA471)
val SuccessBg = Color(0xFFE3F9E9)
val WarningColor = Color(0xFFE37318)
val WarningBg = Color(0xFFFFF1E9)
val ErrorColor = Color(0xFFD54941)
val ErrorBg = Color(0xFFFFF0ED)

// 语义别名 Color 形式（浅色默认值；随主题切换通过 LocalSemanticColors 注入）。
object LightPalette {
    val BgPage = Color(0xFFF5F5F5)
    val BgContainer = Color(0xFFFFFFFF)
    val BgHover = Color(0xFFF3F3F3)
    val BgActive = Color(0xFFE8E8E8)
    val Border = Color(0xFFDDDDDD)
    val BorderHover = Color(0xFFC6C6C6)
    val TextPrimary = Color(0xE6000000)
    val TextSecondary = Color(0x99000000)
    val TextPlaceholder = Color(0x59000000)
}

object DarkPalette {
    val BgPage = Color(0xFF181818)
    val BgContainer = Color(0xFF242424)
    val BgHover = Color(0xFF2C2C2C)
    val BgActive = Color(0xFF2C2C2C)
    val Border = Color(0xFF3E3E3E)
    val BorderHover = Color(0xFF5E5E5E)
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

/** 当前明暗主题下的语义别名集合（对标 tokens.css 的 `:root` 与 `@media dark`）。 */
class ThemeSemanticColors(
    val bgPage: Color,
    val bgContainer: Color,
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
    bgHover = DarkPalette.BgHover,
    bgActive = DarkPalette.BgActive,
    border = DarkPalette.Border,
    borderHover = DarkPalette.BorderHover,
    textPrimary = DarkPalette.TextPrimary,
    textSecondary = DarkPalette.TextSecondary,
    textPlaceholder = DarkPalette.TextPlaceholder,
)

/** 当前主题语义别名；由 [PetalLinkTheme] 按 `isSystemInDarkTheme` 注入。 */
val LocalSemanticColors = staticCompositionLocalOf { lightSemantic }

val LocalReducedMotion = compositionLocalOf { false }

private val systemReducedMotion: Boolean by lazy {
    System.getProperty("petallink.reduceMotion")?.toBooleanStrictOrNull()
        ?: runCatching {
            java.awt.Toolkit.getDefaultToolkit().getDesktopProperty("apple.awt.reduceMotion") as? Boolean
        }.getOrNull()
        ?: false
}

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
