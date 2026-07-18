package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material.MaterialTheme
import androidx.compose.material.darkColors
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color

/**
 * Material 浅色配置。
 */
private val MATERIAL_LIGHT_COLORS = lightColors(
    // Material 浅色主题主色。
    primary = DesignTokens.BRAND,
    // Material 浅色主题主色上的内容色。
    onPrimary = DesignTokens.ON_PRIMARY,
    // Material 浅色主题次色。
    secondary = DesignTokens.BRAND_HOVER,
    // Material 浅色主题页面背景色。
    background = DesignTokens.LIGHT_BG_PAGE,
    // Material 浅色主题容器表面色。
    surface = DesignTokens.LIGHT_BG_CONTAINER,
    // Material 浅色主题错误色。
    error = DesignTokens.ERROR,
)

/**
 * Material 深色配置。
 */
private val MATERIAL_DARK_COLORS = darkColors(
    // Material 深色主题主色。
    primary = DesignTokens.BRAND_HOVER,
    // Material 深色主题主色上的内容色。
    onPrimary = DesignTokens.ON_PRIMARY,
    // Material 深色主题次色。
    secondary = DesignTokens.BRAND,
    // Material 深色主题页面背景色。
    background = DesignTokens.DARK_BG_PAGE,
    // Material 深色主题容器表面色。
    surface = DesignTokens.DARK_BG_CONTAINER,
    // Material 深色主题错误色。
    error = DesignTokens.ERROR,
)

/**
 * 当前明暗主题下的语义别名集合。
 */
data class ThemeSemanticColors(
    /**
     * 品牌主色。
     */
    val brand: Color,

    /**
     * 品牌悬停色。
     */
    val brandHover: Color,

    /**
     * 品牌按下色。
     */
    val brandActive: Color,

    /**
     * 品牌浅色。
     */
    val brandLight: Color,

    /**
     * 品牌 100 色阶。
     */
    val brand100: Color,

    /**
     * 品牌最浅色。
     */
    val brandLighter: Color,

    /**
     * 品牌渐变。
     */
    val brandGradient: Brush,

    /**
     * 品牌浅色渐变。
     */
    val brandGradientSoft: Brush,

    /**
     * 成功色。
     */
    val success: Color,

    /**
     * 成功背景色。
     */
    val successBg: Color,

    /**
     * 警告色。
     */
    val warning: Color,

    /**
     * 警告背景色。
     */
    val warningBg: Color,

    /**
     * 错误色。
     */
    val error: Color,

    /**
     * 错误背景色。
     */
    val errorBg: Color,

    /**
     * 信息色。
     */
    val info: Color,

    /**
     * 信息背景色。
     */
    val infoBg: Color,

    /**
     * 文件夹色。
     */
    val folder: Color,

    /**
     * 文件夹背景色。
     */
    val folderBg: Color,

    /**
     * 文档色。
     */
    val document: Color,

    /**
     * 文档背景色。
     */
    val documentBg: Color,

    /**
     * 图片色。
     */
    val image: Color,

    /**
     * 图片背景色。
     */
    val imageBg: Color,

    /**
     * 视频色。
     */
    val video: Color,

    /**
     * 视频背景色。
     */
    val videoBg: Color,

    /**
     * 表格色。
     */
    val sheet: Color,

    /**
     * 表格背景色。
     */
    val sheetBg: Color,

    /**
     * 关闭状态开关轨道色。
     */
    val switchOffTrack: Color,

    /**
     * 页面背景色。
     */
    val bgPage: Color,

    /**
     * 容器背景色。
     */
    val bgContainer: Color,

    /**
     * 填充背景色。
     */
    val bgFill: Color,

    /**
     * 悬停背景色。
     */
    val bgHover: Color,

    /**
     * 激活背景色。
     */
    val bgActive: Color,

    /**
     * 边框色。
     */
    val border: Color,

    /**
     * 悬停边框色。
     */
    val borderHover: Color,

    /**
     * 主要文字色。
     */
    val textPrimary: Color,

    /**
     * 次要文字色。
     */
    val textSecondary: Color,

    /**
     * 占位文字色。
     */
    val textPlaceholder: Color,

    /**
     * 紧凑 Logo 文字色。
     */
    val appLogoCompactText: Color,

    /**
     * 完整 Logo 文字色。
     */
    val appLogoFullText: Color,

    /**
     * 图标默认颜色。
     */
    val defaultIconTint: Color,

    /**
     * 文件列表批量操作栏背景色。
     */
    val fileListBulkBackground: Color,

    /**
     * 批量危险操作文字色。
     */
    val fileListBulkDangerText: Color,

    /**
     * 批量危险操作图标色。
     */
    val fileListBulkDangerIcon: Color,

    /**
     * 批量危险操作悬停背景色。
     */
    val fileListBulkDangerHoverBackground: Color,

    /**
     * 成功 Toast 图标色。
     */
    val toastSuccessIcon: Color,

    /**
     * 错误 Toast 图标色。
     */
    val toastErrorIcon: Color,

    /**
     * Toast 背景色。
     */
    val toastBackground: Color,

    /**
     * 主页面加载遮罩色。
     */
    val mainLoadingScrim: Color,

    /**
     * 侧边栏账号头像文字色。
     */
    val sidebarAccountAvatarText: Color,

    /**
     * 侧边栏更新卡文字色。
     */
    val sidebarUpdateText: Color,

    /**
     * 侧边栏更新卡进度条色。
     */
    val sidebarUpdateProgress: Color,

    /**
     * 侧边栏更新卡关闭按钮背景色。
     */
    val sidebarDismissBackground: Color,

    /**
     * 侧边栏更新卡关闭按钮文字色。
     */
    val sidebarDismissText: Color,

    /**
     * 侧边栏立即更新按钮背景色。
     */
    val sidebarInstallBackground: Color,

    /**
     * 设置页账号头像文字色。
     */
    val settingsAccountAvatarText: Color,

    /**
     * 文件列表批量选择摘要文字色。
     */
    val fileListBulkSelectionText: Color,

    /**
     * 文件列表批量普通操作文字色。
     */
    val fileListBulkActionText: Color,

    /**
     * 文件列表批量普通操作图标色。
     */
    val fileListBulkActionIcon: Color,

    /**
     * 文件列表批量普通操作悬停背景色。
     */
    val fileListBulkActionHoverBackground: Color,

    /**
     * 文件列表批量关闭图标色。
     */
    val fileListBulkCloseIcon: Color,

    /**
     * 文件列表批量关闭悬停图标色。
     */
    val fileListBulkCloseHoverIcon: Color,

    /**
     * 文件列表批量关闭悬停背景色。
     */
    val fileListBulkCloseHoverBackground: Color,

    /**
     * 更新弹窗遮罩色。
     */
    val updateDialogScrim: Color,

    /**
     * 通用对话框遮罩色。
     */
    val overlayDialogScrim: Color,

    /**
     * 默认 Toast 图标色。
     */
    val toastDefaultIcon: Color,

    /**
     * Toast 文字色。
     */
    val toastText: Color,

    /**
     * 主按钮文字色。
     */
    val buttonPrimaryText: Color,

    /**
     * 禁用主按钮文字色。
     */
    val buttonDisabledPrimaryText: Color,

    /**
     * 危险按钮文字色。
     */
    val buttonDangerText: Color,

    /**
     * 按钮角标文字色。
     */
    val buttonBadgeText: Color,

    /**
     * 紧凑 Logo 图标色。
     */
    val appLogoCompactIcon: Color,

    /**
     * 开关滑块色。
     */
    val switchKnob: Color,

    /**
     * 复选框标记色。
     */
    val checkboxMark: Color,
)

/**
 * 默认浅色语义配色。
 */
val LIGHT_SEMANTIC_COLORS = ThemeSemanticColors(
    // 品牌主色。
    brand = DesignTokens.BRAND,
    // 品牌悬停色。
    brandHover = DesignTokens.BRAND_HOVER,
    // 品牌按下色。
    brandActive = DesignTokens.BRAND_ACTIVE,
    // 品牌浅色。
    brandLight = DesignTokens.BRAND_LIGHT,
    // 品牌 100 色阶。
    brand100 = DesignTokens.BRAND_100,
    // 品牌最浅色。
    brandLighter = DesignTokens.BRAND_LIGHTER,
    // 品牌渐变。
    brandGradient = DesignTokens.BRAND_GRADIENT,
    // 品牌浅色渐变。
    brandGradientSoft = DesignTokens.BRAND_GRADIENT_SOFT,
    // 成功色。
    success = DesignTokens.SUCCESS,
    // 成功背景色。
    successBg = DesignTokens.SUCCESS_BACKGROUND,
    // 警告色。
    warning = DesignTokens.WARNING,
    // 警告背景色。
    warningBg = DesignTokens.WARNING_BACKGROUND,
    // 错误色。
    error = DesignTokens.ERROR,
    // 错误背景色。
    errorBg = DesignTokens.ERROR_BACKGROUND,
    // 信息色。
    info = DesignTokens.INFO,
    // 信息背景色。
    infoBg = DesignTokens.INFO_BACKGROUND,
    // 文件夹色。
    folder = DesignTokens.FOLDER,
    // 文件夹背景色。
    folderBg = DesignTokens.FOLDER_BACKGROUND,
    // 文档色。
    document = DesignTokens.DOCUMENT,
    // 文档背景色。
    documentBg = DesignTokens.DOCUMENT_BACKGROUND,
    // 图片色。
    image = DesignTokens.IMAGE,
    // 图片背景色。
    imageBg = DesignTokens.IMAGE_BACKGROUND,
    // 视频色。
    video = DesignTokens.VIDEO,
    // 视频背景色。
    videoBg = DesignTokens.VIDEO_BACKGROUND,
    // 表格色。
    sheet = DesignTokens.SHEET,
    // 表格背景色。
    sheetBg = DesignTokens.SHEET_BACKGROUND,
    // 关闭状态开关轨道色。
    switchOffTrack = DesignTokens.SWITCH_OFF_TRACK,
    // 页面背景色。
    bgPage = DesignTokens.LIGHT_BG_PAGE,
    // 容器背景色。
    bgContainer = DesignTokens.LIGHT_BG_CONTAINER,
    // 填充背景色。
    bgFill = DesignTokens.LIGHT_BG_FILL,
    // 悬停背景色。
    bgHover = DesignTokens.LIGHT_BG_HOVER,
    // 激活背景色。
    bgActive = DesignTokens.LIGHT_BG_ACTIVE,
    // 边框色。
    border = DesignTokens.LIGHT_BORDER,
    // 悬停边框色。
    borderHover = DesignTokens.LIGHT_BORDER_HOVER,
    // 主要文字色。
    textPrimary = DesignTokens.LIGHT_TEXT_PRIMARY,
    // 次要文字色。
    textSecondary = DesignTokens.LIGHT_TEXT_SECONDARY,
    // 占位文字色。
    textPlaceholder = DesignTokens.LIGHT_TEXT_PLACEHOLDER,
    // 紧凑 Logo 文字色。
    appLogoCompactText = DesignTokens.APP_LOGO_COMPACT_TEXT,
    // 完整 Logo 文字色。
    appLogoFullText = DesignTokens.APP_LOGO_FULL_TEXT,
    // 图标默认颜色。
    defaultIconTint = DesignTokens.DEFAULT_ICON_TINT,
    // 文件列表批量操作栏背景色。
    fileListBulkBackground = DesignTokens.FILE_LIST_BULK_BACKGROUND,
    // 批量危险操作文字色。
    fileListBulkDangerText = DesignTokens.FILE_LIST_BULK_DANGER_TEXT,
    // 批量危险操作图标色。
    fileListBulkDangerIcon = DesignTokens.FILE_LIST_BULK_DANGER_ICON,
    // 批量危险操作悬停背景色。
    fileListBulkDangerHoverBackground = DesignTokens.FILE_LIST_BULK_DANGER_HOVER_BACKGROUND,
    // 成功 Toast 图标色。
    toastSuccessIcon = DesignTokens.TOAST_SUCCESS_ICON,
    // 错误 Toast 图标色。
    toastErrorIcon = DesignTokens.TOAST_ERROR_ICON,
    // Toast 背景色。
    toastBackground = DesignTokens.TOAST_BACKGROUND,
    // 主页面加载遮罩色。
    mainLoadingScrim = Color.White.copy(alpha = 0.6f),
    // 侧边栏账号头像文字色。
    sidebarAccountAvatarText = Color.White,
    // 侧边栏更新卡文字色。
    sidebarUpdateText = Color.White,
    // 侧边栏更新卡进度条色。
    sidebarUpdateProgress = Color.White,
    // 侧边栏更新卡关闭按钮背景色。
    sidebarDismissBackground = Color.White.copy(alpha = 0.25f),
    // 侧边栏更新卡关闭按钮文字色。
    sidebarDismissText = Color.White,
    // 侧边栏立即更新按钮背景色。
    sidebarInstallBackground = Color.White.copy(alpha = 0.95f),
    // 设置页账号头像文字色。
    settingsAccountAvatarText = Color.White,
    // 文件列表批量选择摘要文字色。
    fileListBulkSelectionText = Color.White,
    // 文件列表批量普通操作文字色。
    fileListBulkActionText = Color.White.copy(alpha = 0.85f),
    // 文件列表批量普通操作图标色。
    fileListBulkActionIcon = Color.White.copy(alpha = 0.7f),
    // 文件列表批量普通操作悬停背景色。
    fileListBulkActionHoverBackground = Color.White.copy(alpha = 0.12f),
    // 文件列表批量关闭图标色。
    fileListBulkCloseIcon = Color.White.copy(alpha = 0.7f),
    // 文件列表批量关闭悬停图标色。
    fileListBulkCloseHoverIcon = Color.White,
    // 文件列表批量关闭悬停背景色。
    fileListBulkCloseHoverBackground = Color.White.copy(alpha = 0.12f),
    // 更新弹窗遮罩色。
    updateDialogScrim = Color.Black.copy(alpha = 0.36f),
    // 通用对话框遮罩色。
    overlayDialogScrim = Color.Black.copy(alpha = 0.36f),
    // 默认 Toast 图标色。
    toastDefaultIcon = Color.White,
    // Toast 文字色。
    toastText = Color.White,
    // 主按钮文字色。
    buttonPrimaryText = Color.White,
    // 禁用主按钮文字色。
    buttonDisabledPrimaryText = Color.White,
    // 危险按钮文字色。
    buttonDangerText = Color.White,
    // 按钮角标文字色。
    buttonBadgeText = Color.White,
    // 紧凑 Logo 图标色。
    appLogoCompactIcon = Color.White,
    // 开关滑块色。
    switchKnob = Color.White,
    // 复选框标记色。
    checkboxMark = Color.White,
)

/**
 * 默认深色语义配色。
 */
val DARK_SEMANTIC_COLORS = ThemeSemanticColors(
    // 品牌主色。
    brand = DesignTokens.BRAND_HOVER,
    // 品牌悬停色。
    brandHover = DesignTokens.BRAND,
    // 品牌按下色。
    brandActive = DesignTokens.BRAND_ACTIVE,
    // 品牌浅色。
    brandLight = DesignTokens.DARK_BRAND_LIGHT,
    // 品牌 100 色阶。
    brand100 = DesignTokens.DARK_BRAND_100,
    // 品牌最浅色。
    brandLighter = DesignTokens.DARK_BRAND_LIGHTER,
    // 品牌渐变。
    brandGradient = Brush.linearGradient(listOf(DesignTokens.BRAND_HOVER, DesignTokens.BRAND)),
    // 品牌浅色渐变。
    brandGradientSoft = Brush.linearGradient(listOf(DesignTokens.DARK_BRAND_LIGHTER, DesignTokens.DARK_BRAND_100)),
    // 成功色。
    success = DesignTokens.SUCCESS,
    // 成功背景色。
    successBg = DesignTokens.DARK_SUCCESS_BACKGROUND,
    // 警告色。
    warning = DesignTokens.WARNING,
    // 警告背景色。
    warningBg = DesignTokens.DARK_WARNING_BACKGROUND,
    // 错误色。
    error = DesignTokens.ERROR,
    // 错误背景色。
    errorBg = DesignTokens.DARK_ERROR_BACKGROUND,
    // 信息色。
    info = DesignTokens.INFO,
    // 信息背景色。
    infoBg = DesignTokens.DARK_INFO_BACKGROUND,
    // 文件夹色。
    folder = DesignTokens.FOLDER,
    // 文件夹背景色。
    folderBg = DesignTokens.DARK_FOLDER_BACKGROUND,
    // 文档色。
    document = DesignTokens.DOCUMENT,
    // 文档背景色。
    documentBg = DesignTokens.DARK_DOCUMENT_BACKGROUND,
    // 图片色。
    image = DesignTokens.IMAGE,
    // 图片背景色。
    imageBg = DesignTokens.DARK_IMAGE_BACKGROUND,
    // 视频色。
    video = DesignTokens.VIDEO,
    // 视频背景色。
    videoBg = DesignTokens.DARK_VIDEO_BACKGROUND,
    // 表格色。
    sheet = DesignTokens.SHEET,
    // 表格背景色。
    sheetBg = DesignTokens.DARK_SHEET_BACKGROUND,
    // 关闭状态开关轨道色。
    switchOffTrack = DesignTokens.DARK_SWITCH_OFF_TRACK,
    // 页面背景色。
    bgPage = DesignTokens.DARK_BG_PAGE,
    // 容器背景色。
    bgContainer = DesignTokens.DARK_BG_CONTAINER,
    // 填充背景色。
    bgFill = DesignTokens.DARK_BG_FILL,
    // 悬停背景色。
    bgHover = DesignTokens.DARK_BG_HOVER,
    // 激活背景色。
    bgActive = DesignTokens.DARK_BG_ACTIVE,
    // 边框色。
    border = DesignTokens.DARK_BORDER,
    // 悬停边框色。
    borderHover = DesignTokens.DARK_BORDER_HOVER,
    // 主要文字色。
    textPrimary = DesignTokens.DARK_TEXT_PRIMARY,
    // 次要文字色。
    textSecondary = DesignTokens.DARK_TEXT_SECONDARY,
    // 占位文字色。
    textPlaceholder = DesignTokens.DARK_TEXT_PLACEHOLDER,
    // 紧凑 Logo 文字色。
    appLogoCompactText = DesignTokens.APP_LOGO_COMPACT_TEXT,
    // 完整 Logo 文字色。
    appLogoFullText = DesignTokens.APP_LOGO_FULL_TEXT,
    // 图标默认颜色。
    defaultIconTint = DesignTokens.DEFAULT_ICON_TINT,
    // 文件列表批量操作栏背景色。
    fileListBulkBackground = DesignTokens.FILE_LIST_BULK_BACKGROUND,
    // 批量危险操作文字色。
    fileListBulkDangerText = DesignTokens.FILE_LIST_BULK_DANGER_TEXT,
    // 批量危险操作图标色。
    fileListBulkDangerIcon = DesignTokens.FILE_LIST_BULK_DANGER_ICON,
    // 批量危险操作悬停背景色。
    fileListBulkDangerHoverBackground = DesignTokens.FILE_LIST_BULK_DANGER_HOVER_BACKGROUND,
    // 成功 Toast 图标色。
    toastSuccessIcon = DesignTokens.TOAST_SUCCESS_ICON,
    // 错误 Toast 图标色。
    toastErrorIcon = DesignTokens.TOAST_ERROR_ICON,
    // Toast 背景色。
    toastBackground = DesignTokens.TOAST_BACKGROUND,
    // 主页面加载遮罩色。
    mainLoadingScrim = Color.White.copy(alpha = 0.6f),
    // 侧边栏账号头像文字色。
    sidebarAccountAvatarText = Color.White,
    // 侧边栏更新卡文字色。
    sidebarUpdateText = Color.White,
    // 侧边栏更新卡进度条色。
    sidebarUpdateProgress = Color.White,
    // 侧边栏更新卡关闭按钮背景色。
    sidebarDismissBackground = Color.White.copy(alpha = 0.25f),
    // 侧边栏更新卡关闭按钮文字色。
    sidebarDismissText = Color.White,
    // 侧边栏立即更新按钮背景色。
    sidebarInstallBackground = Color.White.copy(alpha = 0.95f),
    // 设置页账号头像文字色。
    settingsAccountAvatarText = Color.White,
    // 文件列表批量选择摘要文字色。
    fileListBulkSelectionText = Color.White,
    // 文件列表批量普通操作文字色。
    fileListBulkActionText = Color.White.copy(alpha = 0.85f),
    // 文件列表批量普通操作图标色。
    fileListBulkActionIcon = Color.White.copy(alpha = 0.7f),
    // 文件列表批量普通操作悬停背景色。
    fileListBulkActionHoverBackground = Color.White.copy(alpha = 0.12f),
    // 文件列表批量关闭图标色。
    fileListBulkCloseIcon = Color.White.copy(alpha = 0.7f),
    // 文件列表批量关闭悬停图标色。
    fileListBulkCloseHoverIcon = Color.White,
    // 文件列表批量关闭悬停背景色。
    fileListBulkCloseHoverBackground = Color.White.copy(alpha = 0.12f),
    // 更新弹窗遮罩色。
    updateDialogScrim = Color.Black.copy(alpha = 0.36f),
    // 通用对话框遮罩色。
    overlayDialogScrim = Color.Black.copy(alpha = 0.36f),
    // 默认 Toast 图标色。
    toastDefaultIcon = Color.White,
    // Toast 文字色。
    toastText = Color.White,
    // 主按钮文字色。
    buttonPrimaryText = Color.White,
    // 禁用主按钮文字色。
    buttonDisabledPrimaryText = Color.White,
    // 危险按钮文字色。
    buttonDangerText = Color.White,
    // 按钮角标文字色。
    buttonBadgeText = Color.White,
    // 紧凑 Logo 图标色。
    appLogoCompactIcon = Color.White,
    // 开关滑块色。
    switchKnob = Color.White,
    // 复选框标记色。
    checkboxMark = Color.White,
)

/**
 * 当前主题语义别名；由 [PetalLinkTheme] 按 `isSystemInDarkTheme` 注入。
 */
val LOCAL_SEMANTIC_COLORS = staticCompositionLocalOf { LIGHT_SEMANTIC_COLORS }

/**
 * 当前组合树是否减少动画。
 */
val LOCAL_REDUCED_MOTION = compositionLocalOf { false }

/**
 * 应用默认皮肤。
 */
val DEFAULT_PETAL_SKIN = PetalSkin(
    // 皮肤名称。
    name = "default",
    // 浅色 Material 配色。
    lightMaterialColors = MATERIAL_LIGHT_COLORS,
    // 深色 Material 配色。
    darkMaterialColors = MATERIAL_DARK_COLORS,
    // 浅色语义配色。
    lightSemanticColors = LIGHT_SEMANTIC_COLORS,
    // 深色语义配色。
    darkSemanticColors = DARK_SEMANTIC_COLORS,
    // 按 UI 职责拆分的字体样式。
    typography = DesignTokens.TYPOGRAPHY,
    // 按 UI 职责拆分的尺寸。
    metrics = DesignTokens.METRICS,
)

/**
 * 系统减少动态效果配置。
 */
private val SYSTEM_REDUCED_MOTION: Boolean by lazy {
    System.getProperty("petallink.reduceMotion")?.toBooleanStrictOrNull()
        ?: runCatching {
            java.awt.Toolkit.getDefaultToolkit().getDesktopProperty("apple.awt.reduceMotion") as? Boolean
        }.getOrNull()
        ?: false
}

/**
 * 应用主题入口：注入完整皮肤，再按系统明暗模式选择对应的配色。
 *
 * @param skin 当前应用皮肤。
 * @param content 使用该皮肤渲染的 UI 内容。
 */
@Composable
fun PetalLinkTheme(
    skin: PetalSkin = DEFAULT_PETAL_SKIN,
    content: @Composable () -> Unit,
) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) skin.darkMaterialColors else skin.lightMaterialColors
    val semantic = if (dark) skin.darkSemanticColors else skin.lightSemanticColors
    CompositionLocalProvider(
        LOCAL_PETAL_SKIN provides skin,
        LOCAL_REDUCED_MOTION provides SYSTEM_REDUCED_MOTION,
        LOCAL_SEMANTIC_COLORS provides semantic,
    ) {
        MaterialTheme(
            colors = colors,
            typography = skin.materialTypography,
            content = content,
        )
    }
}
