package io.github.yuanbaobaoo.petallink.ui.theme

/**
 * 设计系统 Token（对标原项目 `app/styles/tokens.css` + `docs/09-设计系统.md`）。
 *
 * 视觉学派：Modern Minimal（Tech Utility）· TDesign Enterprise × macOS Native。
 * 这里集中定义颜色、间距、圆角、字号、字体栈等不变量；随明暗主题切换的语义别名见 [SemanticColors]。
 */
object DesignTokens {

    // ------------------------------------------------------------------
    // 品牌色（对标 TDesign brand）
    // ------------------------------------------------------------------
    const val BRAND = "#0052D9"
    const val BRAND_HOVER = "#366EF4"
    const val BRAND_ACTIVE = "#003CAB"
    const val BRAND_LIGHT = "#D9E1FF"
    const val BRAND_LIGHTER = "#F2F3FF"

    // ------------------------------------------------------------------
    // 功能色（语义色主体）
    // ------------------------------------------------------------------
    const val SUCCESS = "#2BA471"
    const val SUCCESS_BG = "#E3F9E9"
    const val WARNING = "#E37318"
    const val WARNING_BG = "#FFF1E9"
    const val ERROR = "#D54941"
    const val ERROR_BG = "#FFF0ED"

    // ------------------------------------------------------------------
    // 灰阶（14 级，对标 TDesign gray，逐级加深）
    // ------------------------------------------------------------------
    const val GRAY_1 = "#F3F3F3"
    const val GRAY_2 = "#EEEEEE"
    const val GRAY_3 = "#E7E7E7"
    const val GRAY_4 = "#DCDCDC"
    const val GRAY_5 = "#C9C9C9"
    const val GRAY_6 = "#B5B5B5"
    const val GRAY_7 = "#9C9C9C"
    const val GRAY_8 = "#888888"
    const val GRAY_9 = "#777777"
    const val GRAY_10 = "#666666"
    const val GRAY_11 = "#555555"
    const val GRAY_12 = "#444444"
    const val GRAY_13 = "#333333"
    const val GRAY_14 = "#222222"

    // ------------------------------------------------------------------
    // macOS 窗口专用色
    // ------------------------------------------------------------------
    const val TITLEBAR_BG = "#EBEBEB"
    const val TITLEBAR_BORDER = "#D0D0D0"
    const val WINDOW_OUTER_BG = "#5A5A5A"
    const val CLOSE_BTN = "#FF5F57"
    const val MINIMIZE_BTN = "#FFBD2E"
    const val MAXIMIZE_BTN = "#28C840"

    // ------------------------------------------------------------------
    // 间距（4 栅格，px）
    // ------------------------------------------------------------------
    const val SPACING_XS = 4
    const val SPACING_SM = 8
    const val SPACING_MD = 12
    const val SPACING_LG = 16
    const val SPACING_XL = 24
    const val SPACING_XXL = 32

    // ------------------------------------------------------------------
    // 圆角（px）
    // ------------------------------------------------------------------
    const val RADIUS_SM = 3
    const val RADIUS_MD = 6
    const val RADIUS_LG = 9

    // ------------------------------------------------------------------
    // 字号阶梯（px）
    // ------------------------------------------------------------------
    const val FONT_DISPLAY = 36
    const val FONT_TITLE_LG = 24
    const val FONT_TITLE_MD = 20
    const val FONT_TITLE_SM = 16
    const val FONT_BODY = 14
    const val FONT_BODY_SM = 13
    const val FONT_CAPTION = 12

    // ------------------------------------------------------------------
    // 字重
    // ------------------------------------------------------------------
    const val FW_REGULAR = 400
    const val FW_MEDIUM = 500
    const val FW_SEMIBOLD = 600

    // ------------------------------------------------------------------
    // 字体栈
    // ------------------------------------------------------------------
    const val FONT_FAMILY = "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"PingFang SC\", \"Helvetica Neue\", sans-serif"
    const val FONT_MONO = "\"SF Mono\", \"Menlo\", \"Monaco\", \"Consolas\", monospace"

    // ------------------------------------------------------------------
    // 布局尺寸（px）
    // ------------------------------------------------------------------
    const val WINDOW_WIDTH = 1280
    const val WINDOW_HEIGHT = 800
    const val WINDOW_MIN_WIDTH = 700
    const val WINDOW_MIN_HEIGHT = 480
    const val TITLEBAR_HEIGHT = 38
    const val APPBAR_HEIGHT = 56
    const val SIDEBAR_WIDTH = 220
    const val SETTINGS_NAV_WIDTH = 200
    const val BREADCRUMB_HEIGHT = 32
    const val SYNC_BAR_HEIGHT = 32
    const val FILE_HEADER_HEIGHT = 36
    const val FILE_ROW_HEIGHT = 44
    const val TRANSFER_POPOVER_WIDTH = 420
    const val TRANSFER_POPOVER_HEIGHT = 560
}

/**
 * 随明暗主题切换的语义别名（对标 tokens.css 的 `:root` 与 `@media dark` 覆盖）。
 *
 * 浅色用 [Light]，深色用 [Dark]，由 [PetalLinkTheme] 按 `isSystemInDarkTheme` 注入 [LocalSemanticColors]。
 */
data class SemanticColors(
    // 页面与容器背景
    val bgPage: String,
    val bgContainer: String,
    val bgHover: String,
    val bgActive: String,
    // 边框
    val border: String,
    val borderHover: String,
    // 文字
    val textPrimary: String,
    val textSecondary: String,
    val textPlaceholder: String,
    // 品牌容器背景（深色下调暗）
    val brandLight: String,
    val brandLighter: String,
    // 阴影（box-shadow CSS 字符串）
    val shadowCard: String,
    val shadowDropdown: String,
    val shadowModal: String,
)

/** 浅色主题语义别名。 */
val LightSemanticColors = SemanticColors(
    bgPage = "#F5F5F5",
    bgContainer = "#FFFFFF",
    bgHover = "#F3F3F3",
    bgActive = "#E8E8E8",
    border = "#DDDDDD",
    borderHover = "#C6C6C6",
    textPrimary = "rgba(0,0,0,0.9)",
    textSecondary = "rgba(0,0,0,0.6)",
    textPlaceholder = "rgba(0,0,0,0.35)",
    brandLight = DesignTokens.BRAND_LIGHT,
    brandLighter = DesignTokens.BRAND_LIGHTER,
    shadowCard = "0 1px 4px rgba(0,0,0,0.08), 0 0 0 0.5px rgba(0,0,0,0.06)",
    shadowDropdown = "0 4px 12px rgba(0,0,0,0.12), 0 0 0 0.5px rgba(0,0,0,0.06)",
    shadowModal = "0 8px 24px rgba(0,0,0,0.16)",
)

/** 深色主题语义别名（对标 tokens.css `@media dark`）。 */
val DarkSemanticColors = SemanticColors(
    bgPage = "#181818",
    bgContainer = "#242424",
    bgHover = "#2C2C2C",
    bgActive = "#2C2C2C",
    border = "#3E3E3E",
    borderHover = "#5E5E5E",
    textPrimary = "rgba(255,255,255,0.9)",
    textSecondary = "rgba(255,255,255,0.6)",
    textPlaceholder = "rgba(255,255,255,0.35)",
    // 深色模式 brand 主色不变，仅容器背景调暗。
    brandLight = "#1A3A8A",
    brandLighter = "#1F2A4A",
    shadowCard = "0 1px 4px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.06)",
    shadowDropdown = "0 4px 12px rgba(0,0,0,0.4), 0 0 0 0.5px rgba(255,255,255,0.06)",
    shadowModal = "0 8px 24px rgba(0,0,0,0.5)",
)

/**
 * 文件列表列定义（对标 FileListView 6 列）。
 *
 * checkbox 固定 40px，actions 固定 40px，status 固定 60px；
 * size/time 列宽可拖拽（默认 100/150，范围 64–400）。
 */
object FileListColumns {
    data class Column(val key: String, val title: String, val defaultWidth: Int, val minWidth: Int, val maxWidth: Int)

    val COLUMNS: List<Column> = listOf(
        Column("checkbox", "", 40, 40, 40),
        Column("name", "名称", 240, 80, 2000),
        Column("size", "大小", 100, 64, 400),
        Column("modified", "修改时间", 150, 64, 400),
        Column("status", "状态", 60, 60, 60),
        Column("actions", "操作", 40, 40, 40),
    )
}
