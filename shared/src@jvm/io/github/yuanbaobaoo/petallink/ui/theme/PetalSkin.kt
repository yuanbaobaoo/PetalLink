package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.material.Colors
import androidx.compose.material.Typography
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf

/**
 * PetalLink 皮肤的数据结构。每个属性都以 UI 职责命名，不以数值档位命名。
 *
 * 两个职责即使当前数值一样，也必须保留两个独立 token。例如按钮文字和菜单文字
 * 都是 14sp，但分别由 `button.primaryLabel` 和 `menu.itemLabel` 控制。
 */
data class PetalSkin(
    /**
     * 皮肤名称。
     */
    val name: String,

    /**
     * 浅色 Material 配色。
     */
    val lightMaterialColors: Colors,

    /**
     * 深色 Material 配色。
     */
    val darkMaterialColors: Colors,

    /**
     * 浅色语义配色。
     */
    val lightSemanticColors: ThemeSemanticColors,

    /**
     * 深色语义配色。
     */
    val darkSemanticColors: ThemeSemanticColors,

    /**
     * 按 UI 职责拆分的字体样式。
     */
    val typography: PetalTypography,

    /**
     * 按 UI 职责拆分的尺寸。
     */
    val metrics: PetalMetrics,

    /**
     * Material 组件默认字体配置。
     */
    val materialTypography: Typography = Typography(),
)

/**
 * 当前组合树中的完整皮肤。
 */
internal val LOCAL_PETAL_SKIN = staticCompositionLocalOf<PetalSkin> {
    error("PetalSkin is not provided. Wrap the UI in PetalLinkTheme.")
}

/**
 * UI 中读取当前皮肤的唯一入口。
 */
object PetalTheme {
    /**
     * 当前完整皮肤。
     */
    val skin: PetalSkin
        @Composable
        @ReadOnlyComposable
        get() = LOCAL_PETAL_SKIN.current

    /**
     * 当前字体 token。
     */
    val typography: PetalTypography
        @Composable
        @ReadOnlyComposable
        get() = LOCAL_PETAL_SKIN.current.typography

    /**
     * 当前尺寸 token。
     */
    val metrics: PetalMetrics
        @Composable
        @ReadOnlyComposable
        get() = LOCAL_PETAL_SKIN.current.metrics

    /**
     * 当前明暗模式下的语义颜色。
     */
    val colors: ThemeSemanticColors
        @Composable
        @ReadOnlyComposable
        get() = LOCAL_SEMANTIC_COLORS.current
}
