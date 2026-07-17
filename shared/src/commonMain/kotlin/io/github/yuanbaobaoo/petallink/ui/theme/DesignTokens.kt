package io.github.yuanbaobaoo.petallink.ui.theme

/**
 * 设计系统 Token（v2 重设计：logo 品牌蓝 + 小圆角 + 卡片化）。
 *
 * 视觉学派：品牌蓝（取自 app logo 主色 #0053DB）× macOS Native。
 * 这里集中定义颜色、间距、圆角、字号、字体栈等不变量；随明暗主题切换的语义别名见 [SemanticColors]。
 */
object DesignTokens {

    // ------------------------------------------------------------------
    // 品牌色（取自 logo 主色 #0053DB 及其派生色阶）
    // ------------------------------------------------------------------
    const val BRAND = "#0053DB"
    const val BRAND_HOVER = "#4A8BF0"
    const val BRAND_ACTIVE = "#0047B8"
    const val BRAND_LIGHT = "#B7D0F7"
    const val BRAND_100 = "#DCE8FC"
    const val BRAND_LIGHTER = "#EFF4FE"

    // ------------------------------------------------------------------
    // 功能色（语义色主体）
    // ------------------------------------------------------------------
    const val SUCCESS = "#0CA678"
    const val SUCCESS_BG = "#E3F5EE"
    const val WARNING = "#F08C00"
    const val WARNING_BG = "#FFF3DE"
    const val ERROR = "#E5484D"
    const val ERROR_BG = "#FDECEC"
    const val INFO = "#3B82F6"
    const val INFO_BG = "#E8F0FE"

    // ------------------------------------------------------------------
    // 文件类型 tile 类别色（文件夹琥珀 / 文档靛蓝 / 图片粉 / 视频紫 / 表格绿）
    // ------------------------------------------------------------------
    const val FOLDER_AMBER = "#F0A63C"
    const val FOLDER_AMBER_BG = "#FFF4DE"
    const val TILE_DOC = "#6366F1"
    const val TILE_DOC_BG = "#EEF2FF"
    const val TILE_IMAGE = "#EC4899"
    const val TILE_IMAGE_BG = "#FDE7F3"
    const val TILE_VIDEO = "#8B5CF6"
    const val TILE_VIDEO_BG = "#F3E8FF"
    const val TILE_SHEET = "#10B981"
    const val TILE_SHEET_BG = "#E6F7EE"

    // ------------------------------------------------------------------
    // 灰阶（14 级，逐级加深）
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
    // 圆角（px，整体收小）
    // ------------------------------------------------------------------
    const val RADIUS_SM = 5
    const val RADIUS_MD = 8
    const val RADIUS_LG = 10
    const val RADIUS_XL = 12

    // ------------------------------------------------------------------
    // 字号阶梯（px，整体 +1，避免桌面端文字发虚）
    // ------------------------------------------------------------------
    const val FONT_DISPLAY = 37
    const val FONT_TITLE_LG = 25
    const val FONT_TITLE_MD = 21
    const val FONT_TITLE_SM = 17
    const val FONT_BODY = 15
    const val FONT_BODY_SM = 14
    const val FONT_CAPTION = 13

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
    const val TITLEBAR_HEIGHT = 44
    const val APPBAR_HEIGHT = 64
    const val SIDEBAR_WIDTH = 248
    const val SETTINGS_NAV_WIDTH = 240
    const val BREADCRUMB_HEIGHT = 40
    const val SYNC_BAR_HEIGHT = 44
    const val FILE_HEADER_HEIGHT = 38
    const val FILE_ROW_HEIGHT = 56
    const val TRANSFER_POPOVER_WIDTH = 440
    const val TRANSFER_POPOVER_HEIGHT = 580
}

/**
 * 随明暗主题切换的语义别名（对标 v2 设计的 `:root` 语义变量）。
 *
 * 浅色用 [LightSemanticColors]，深色用 [DarkSemanticColors]，
 * 由 PetalLinkTheme 按 `isSystemInDarkTheme` 注入 LocalSemanticColors。
 */
data class SemanticColors(
    // 页面与容器背景
    val bgPage: String,
    val bgContainer: String,
    val bgFill: String,
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
    bgPage = "#F5F5F7",
    bgContainer = "#FFFFFF",
    bgFill = "#F1F1F3",
    bgHover = "#F7F7F9",
    bgActive = "#ECECEF",
    border = "rgba(0,0,0,0.06)",
    borderHover = "rgba(0,0,0,0.1)",
    textPrimary = "rgba(0,0,0,0.9)",
    textSecondary = "rgba(0,0,0,0.6)",
    textPlaceholder = "rgba(0,0,0,0.35)",
    brandLight = DesignTokens.BRAND_LIGHT,
    brandLighter = DesignTokens.BRAND_LIGHTER,
    shadowCard = "0 1px 2px rgba(16,24,40,0.05)",
    shadowDropdown = "0 24px 64px -12px rgba(16,24,40,0.22), 0 0 0 0.5px rgba(0,0,0,0.05)",
    shadowModal = "0 24px 64px -12px rgba(16,24,40,0.22)",
)

/** 深色主题语义别名。 */
val DarkSemanticColors = SemanticColors(
    bgPage = "#181818",
    bgContainer = "#242424",
    bgFill = "#2C2C2C",
    bgHover = "#2C2C2C",
    bgActive = "#333333",
    border = "rgba(255,255,255,0.08)",
    borderHover = "rgba(255,255,255,0.16)",
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
 * 文件列表列定义（6 列）。
 *
 * checkbox 固定 40px，actions 固定 44px，status 固定 72px；
 * size/time 列宽可拖拽（默认 110/160，范围 64–400）。
 */
object FileListColumns {
    data class Column(val key: String, val title: String, val defaultWidth: Int, val minWidth: Int, val maxWidth: Int)

    val COLUMNS: List<Column> = listOf(
        Column("checkbox", "", 40, 40, 40),
        Column("name", "名称", 240, 80, 2000),
        Column("size", "大小", 110, 64, 400),
        Column("modified", "修改时间", 160, 64, 400),
        Column("status", "状态", 72, 60, 80),
        Column("actions", "操作", 44, 40, 48),
    )
}
