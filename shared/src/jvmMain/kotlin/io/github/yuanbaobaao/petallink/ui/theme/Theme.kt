package io.github.yuanbaobaao.petallink.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material.MaterialTheme
import androidx.compose.material.darkColors
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// 品牌色（对标 DesignTokens）
val BrandColor = Color(0xFF0052D9)
val BrandHover = Color(0xFF366EF4)
val SuccessColor = Color(0xFF2BA471)
val WarningColor = Color(0xFFE37318)
val ErrorColor = Color(0xFFD54941)

private val LightColors = lightColors(
    primary = BrandColor,
    onPrimary = Color.White,
    secondary = BrandHover,
    background = Color(0xFFF5F5F5),
    surface = Color.White,
    error = ErrorColor,
)

private val DarkColors = darkColors(
    primary = BrandHover,
    onPrimary = Color.White,
    secondary = BrandColor,
    background = Color(0xFF1E1E1E),
    surface = Color(0xFF2B2B2B),
    error = ErrorColor,
)

@Composable
fun PetalLinkTheme(content: @Composable () -> Unit) {
    val colors = if (isSystemInDarkTheme()) DarkColors else LightColors
    MaterialTheme(colors = colors, content = content)
}
