package io.github.yuanbaobaoo.petallink.ui.theme

/**
 * 设计系统 Token（对标原项目 tokens.css + docs/09）。
 *
 * 详见 docs/09-设计系统.md、docs/10 阶段 6 item 30。
 * TDesign × macOS 色彩体系。
 */
object DesignTokens {

    // ------------------------------------------------------------------
    // 品牌色（对标 TDesign brand）
    // ------------------------------------------------------------------
    const val BRAND = "#0052D9"
    const val BRAND_HOVER = "#366EF4"
    const val BRAND_ACTIVE = "#003CAB"

    // ------------------------------------------------------------------
    // 功能色
    // ------------------------------------------------------------------
    const val SUCCESS = "#2BA471"
    const val WARNING = "#E37318"
    const val ERROR = "#D54941"

    // ------------------------------------------------------------------
    // 灰阶（14 级，对标 TDesign gray）
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
    // 间距（4 栅格）
    // ------------------------------------------------------------------
    const val SPACING_XS = 4   // 4pt
    const val SPACING_SM = 8
    const val SPACING_MD = 12
    const val SPACING_LG = 16
    const val SPACING_XL = 24
    const val SPACING_XXL = 32

    // ------------------------------------------------------------------
    // 圆角
    // ------------------------------------------------------------------
    const val RADIUS_SM = 3
    const val RADIUS_MD = 6
    const val RADIUS_LG = 9

    // ------------------------------------------------------------------
    // 深色模式覆盖（关键色）
    // ------------------------------------------------------------------
    object Dark {
        const val BG_PRIMARY = "#1E1E1E"
        const val BG_SECONDARY = "#2B2B2B"
        const val TEXT_PRIMARY = "#E8E8E8"
        const val TEXT_SECONDARY = "#9C9C9C"
        const val BRAND = "#366EF4"  // 深色模式品牌色提亮
    }
}

/**
 * 文件列表列定义（对标 FileListView 6 列）。
 * 详见 docs/08 §FileListView。
 */
object FileListColumns {
    data class Column(val key: String, val title: String, val defaultWidth: Int)

    val COLUMNS: List<Column> = listOf(
        Column("name", "名称", 240),
        Column("size", "大小", 80),
        Column("modified", "修改时间", 140),
        Column("status", "状态", 100),
        Column("owner", "所有者", 100),
        Column("actions", "操作", 80),
    )
}
