package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * 文件列表内部尺寸。
 */
data class FileListControlMetrics(
    /**
     * 文件大小列初始宽度。
     */
    val sizeColumnInitialWidth: Dp,

    /**
     * 修改时间列初始宽度。
     */
    val timeColumnInitialWidth: Dp,

    /**
     * 可调整列的最小宽度。
     */
    val resizableColumnMinimumWidth: Dp,

    /**
     * 可调整列的最大宽度。
     */
    val resizableColumnMaximumWidth: Dp,

    /**
     * 批量操作栏水平外边距。
     */
    val bulkBarHorizontalMargin: Dp,

    /**
     * 批量操作栏顶部外边距。
     */
    val bulkBarTopMargin: Dp,

    /**
     * 批量操作栏高度。
     */
    val bulkBarHeight: Dp,

    /**
     * 批量操作栏圆角。
     */
    val bulkBarRadius: Dp,

    /**
     * 批量操作栏起始内边距。
     */
    val bulkBarStartPadding: Dp,

    /**
     * 批量操作栏结束内边距。
     */
    val bulkBarEndPadding: Dp,

    /**
     * 批量操作栏内容间距。
     */
    val bulkBarContentSpacing: Dp,

    /**
     * 表格水平内边距。
     */
    val tableHorizontalPadding: Dp,

    /**
     * 表头高度。
     */
    val headerHeight: Dp,

    /**
     * 表头单元格水平内边距。
     */
    val headerHorizontalPadding: Dp,

    /**
     * 复选框列宽度。
     */
    val checkboxColumnWidth: Dp,

    /**
     * 状态列宽度。
     */
    val statusColumnWidth: Dp,

    /**
     * 操作列宽度。
     */
    val actionColumnWidth: Dp,

    /**
     * 表格底栏高度。
     */
    val footerHeight: Dp,

    /**
     * 批量操作按钮高度。
     */
    val bulkActionHeight: Dp,

    /**
     * 批量操作按钮圆角。
     */
    val bulkActionRadius: Dp,

    /**
     * 批量操作按钮水平内边距。
     */
    val bulkActionHorizontalPadding: Dp,

    /**
     * 批量操作按钮内容间距。
     */
    val bulkActionContentSpacing: Dp,

    /**
     * 批量操作按钮图标尺寸。
     */
    val bulkActionIconSize: Dp,

    /**
     * 批量操作栏关闭按钮尺寸。
     */
    val bulkCloseSize: Dp,

    /**
     * 批量操作栏关闭图标尺寸。
     */
    val bulkCloseIconSize: Dp,

    /**
     * 表头文字与排序图标间距。
     */
    val headerSortSpacing: Dp,

    /**
     * 表头排序图标尺寸。
     */
    val headerSortIconSize: Dp,

    /**
     * 列宽拖动手柄宽度。
     */
    val resizeHandleWidth: Dp,

    /**
     * 文件行高度。
     */
    val rowHeight: Dp,

    /**
     * 文件行圆角。
     */
    val rowRadius: Dp,

    /**
     * 文件行水平内边距。
     */
    val rowHorizontalPadding: Dp,

    /**
     * 文件图标与名称间距。
     */
    val rowNameContentSpacing: Dp,

    /**
     * 文件状态图标尺寸。
     */
    val rowStatusIconSize: Dp,

    /**
     * 右键菜单宽度。
     */
    val contextMenuWidth: Dp,

    /**
     * 右键菜单阴影高度。
     */
    val contextMenuShadowElevation: Dp,

    /**
     * 右键菜单圆角。
     */
    val contextMenuRadius: Dp,

    /**
     * 右键菜单边框宽度。
     */
    val contextMenuBorderWidth: Dp,

    /**
     * 右键菜单内边距。
     */
    val contextMenuPadding: Dp,

    /**
     * 文件缩略图尺寸。
     */
    val thumbnailSize: Dp,

    /**
     * 文件缩略图圆角。
     */
    val thumbnailRadius: Dp,

    /**
     * 文件类型图标尺寸。
     */
    val fileTypeIconSize: Dp,

    /**
     * 右键菜单操作项高度。
     */
    val contextActionHeight: Dp,

    /**
     * 右键菜单操作项圆角。
     */
    val contextActionRadius: Dp,

    /**
     * 右键菜单操作项水平内边距。
     */
    val contextActionHorizontalPadding: Dp,

    /**
     * 右键菜单操作项内容间距。
     */
    val contextActionContentSpacing: Dp,

    /**
     * 右键菜单操作项图标尺寸。
     */
    val contextActionIconSize: Dp,

    /**
     * 右键菜单分隔线水平内边距。
     */
    val contextDividerHorizontalPadding: Dp,

    /**
     * 右键菜单分隔线垂直内边距。
     */
    val contextDividerVerticalPadding: Dp,

    /**
     * 右键菜单分隔线高度。
     */
    val contextDividerHeight: Dp,

    /**
     * 禁用批量操作按钮透明度。
     */
    val bulkActionDisabledAlpha: Float,
)

/**
 * 创建文件列表内部默认尺寸。
 */
internal fun createFileListControlMetrics() = FileListControlMetrics(
    // 文件大小列初始宽度。
    sizeColumnInitialWidth = 110.dp,
    // 修改时间列初始宽度。
    timeColumnInitialWidth = 160.dp,
    // 可调整列的最小宽度。
    resizableColumnMinimumWidth = 64.dp,
    // 可调整列的最大宽度。
    resizableColumnMaximumWidth = 400.dp,
    // 批量操作栏水平外边距。
    bulkBarHorizontalMargin = 24.dp,
    // 批量操作栏顶部外边距。
    bulkBarTopMargin = 10.dp,
    // 批量操作栏高度。
    bulkBarHeight = 44.dp,
    // 批量操作栏圆角。
    bulkBarRadius = 10.dp,
    // 批量操作栏起始内边距。
    bulkBarStartPadding = 16.dp,
    // 批量操作栏结束内边距。
    bulkBarEndPadding = 8.dp,
    // 批量操作栏内容间距。
    bulkBarContentSpacing = 10.dp,
    // 表格水平内边距。
    tableHorizontalPadding = 12.dp,
    // 表头高度。
    headerHeight = 38.dp,
    // 表头单元格水平内边距。
    headerHorizontalPadding = 12.dp,
    // 复选框列宽度。
    checkboxColumnWidth = 40.dp,
    // 状态列宽度。
    statusColumnWidth = 72.dp,
    // 操作列宽度。
    actionColumnWidth = 44.dp,
    // 表格底栏高度。
    footerHeight = 36.dp,
    // 批量操作按钮高度。
    bulkActionHeight = 32.dp,
    // 批量操作按钮圆角。
    bulkActionRadius = 8.dp,
    // 批量操作按钮水平内边距。
    bulkActionHorizontalPadding = 14.dp,
    // 批量操作按钮内容间距。
    bulkActionContentSpacing = 6.dp,
    // 批量操作按钮图标尺寸。
    bulkActionIconSize = 16.dp,
    // 批量操作栏关闭按钮尺寸。
    bulkCloseSize = 32.dp,
    // 批量操作栏关闭图标尺寸。
    bulkCloseIconSize = 16.dp,
    // 表头文字与排序图标间距。
    headerSortSpacing = 4.dp,
    // 表头排序图标尺寸。
    headerSortIconSize = 12.dp,
    // 列宽拖动手柄宽度。
    resizeHandleWidth = 6.dp,
    // 文件行高度。
    rowHeight = 56.dp,
    // 文件行圆角。
    rowRadius = 8.dp,
    // 文件行水平内边距。
    rowHorizontalPadding = 12.dp,
    // 文件图标与名称间距。
    rowNameContentSpacing = 12.dp,
    // 文件状态图标尺寸。
    rowStatusIconSize = 16.dp,
    // 右键菜单宽度。
    contextMenuWidth = 200.dp,
    // 右键菜单阴影高度。
    contextMenuShadowElevation = 16.dp,
    // 右键菜单圆角。
    contextMenuRadius = 10.dp,
    // 右键菜单边框宽度。
    contextMenuBorderWidth = 0.5.dp,
    // 右键菜单内边距。
    contextMenuPadding = 6.dp,
    // 文件缩略图尺寸。
    thumbnailSize = 32.dp,
    // 文件缩略图圆角。
    thumbnailRadius = 6.dp,
    // 文件类型图标尺寸。
    fileTypeIconSize = 18.dp,
    // 右键菜单操作项高度。
    contextActionHeight = 36.dp,
    // 右键菜单操作项圆角。
    contextActionRadius = 8.dp,
    // 右键菜单操作项水平内边距。
    contextActionHorizontalPadding = 12.dp,
    // 右键菜单操作项内容间距。
    contextActionContentSpacing = 10.dp,
    // 右键菜单操作项图标尺寸。
    contextActionIconSize = 16.dp,
    // 右键菜单分隔线水平内边距。
    contextDividerHorizontalPadding = 8.dp,
    // 右键菜单分隔线垂直内边距。
    contextDividerVerticalPadding = 4.dp,
    // 右键菜单分隔线高度。
    contextDividerHeight = 0.5.dp,
    // 禁用批量操作按钮透明度。
    bulkActionDisabledAlpha = 0.4f,
)
