package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * 基础品牌组件尺寸。
 */
data class BasicMetrics(
    /**
     * 紧凑 Logo 默认尺寸。
     */
    val compactLogoSize: Dp,

    /**
     * 登录页 Logo 尺寸。
     */
    val largeLogoSize: Dp,

    /**
     * 紧凑 Logo 与文字间距。
     */
    val compactLogoTextSpacing: Dp,

    /**
     * 完整 Logo 默认高度。
     */
    val fullLogoHeight: Dp,

    /**
     * 完整 Logo 与文字间距。
     */
    val fullLogoTextSpacing: Dp,

    /**
     * 竖分隔线默认高度。
     */
    val verticalSeparatorHeight: Dp,

    /**
     * 竖分隔线宽度。
     */
    val verticalSeparatorWidth: Dp,

    /**
     * 底部分隔线厚度。
     */
    val bottomBorderThickness: Dp,
)

/**
 * 图标基础尺寸。
 */
data class IconMetrics(
    /**
     * 图标默认尺寸。
     */
    val defaultSize: Dp,

    /**
     * 图标旋转一周时长。
     */
    val spinDurationMillis: Int,
)

/**
 * 分隔线尺寸。
 */
data class DividerMetrics(
    /**
     * 横分隔线默认厚度。
     */
    val horizontalThickness: Dp,

    /**
     * 竖分隔线默认高度。
     */
    val verticalHeight: Dp,

    /**
     * 竖分隔线宽度。
     */
    val verticalWidth: Dp,
)

/**
 * 组件目录页面尺寸。
 */
data class CatalogMetrics(
    /**
     * 页面内边距。
     */
    val pagePadding: Dp,

    /**
     * 区块间距。
     */
    val sectionSpacing: Dp,

    /**
     * 示例项间距。
     */
    val itemSpacing: Dp,

    /**
     * 图标预览尺寸。
     */
    val iconPreviewSize: Dp,

    /**
     * 输入框预览宽度。
     */
    val fieldPreviewWidth: Dp,

    /**
     * 进度条预览宽度。
     */
    val progressPreviewWidth: Dp,

    /**
     * 环形进度预览尺寸。
     */
    val circularProgressSize: Dp,

    /**
     * 纵向示例间距。
     */
    val verticalGroupSpacing: Dp,

    /**
     * 紧凑示例间距。
     */
    val compactItemSpacing: Dp,

    /**
     * 弹窗示例宽度。
     */
    val dialogPreviewWidth: Dp,
)

/**
 * 同步状态栏尺寸。
 */
data class StatusBarMetrics(
    /**
     * 最小高度。
     */
    val minimumHeight: Dp,

    /**
     * 水平内边距。
     */
    val horizontalPadding: Dp,

    /**
     * 垂直内边距。
     */
    val verticalPadding: Dp,

    /**
     * 状态内容间距。
     */
    val statusContentSpacing: Dp,

    /**
     * 同步图标尺寸。
     */
    val syncingIconSize: Dp,

    /**
     * 空闲指示点尺寸。
     */
    val idleIndicatorSize: Dp,

    /**
     * 操作区水平间距。
     */
    val actionHorizontalSpacing: Dp,

    /**
     * 操作区垂直间距。
     */
    val actionVerticalSpacing: Dp,
)

/**
 * 同步设置横幅尺寸。
 */
data class SyncSetupMetrics(
    /**
     * 水平内边距。
     */
    val horizontalPadding: Dp,

    /**
     * 垂直内边距。
     */
    val verticalPadding: Dp,
)

/**
 * 表单控件内部尺寸；字段只影响名称对应的具体职责。
 */
data class FormControlMetrics(
    /**
     * 文本输入框边框宽度。
     */
    val textFieldBorderWidth: Dp,

    /**
     * 文本输入框水平内边距。
     */
    val textFieldHorizontalPadding: Dp,

    /**
     * 文本输入框内容间距。
     */
    val textFieldContentSpacing: Dp,

    /**
     * 文本输入框前缀图标尺寸。
     */
    val textFieldPrefixIconSize: Dp,

    /**
     * 数字输入框边框宽度。
     */
    val numberFieldBorderWidth: Dp,

    /**
     * 数字输入框水平内边距。
     */
    val numberFieldHorizontalPadding: Dp,

    /**
     * 数字输入框内容间距。
     */
    val numberFieldContentSpacing: Dp,

    /**
     * 数字输入区域宽度。
     */
    val numberFieldInputWidth: Dp,

    /**
     * 步进器容器圆角。
     */
    val stepperRadius: Dp,

    /**
     * 步进器容器内边距。
     */
    val stepperPadding: Dp,

    /**
     * 步进器减号图标尺寸。
     */
    val stepperMinusIconSize: Dp,

    /**
     * 步进器数值区域宽度。
     */
    val stepperValueWidth: Dp,

    /**
     * 步进器按钮尺寸。
     */
    val stepperButtonSize: Dp,

    /**
     * 步进器按钮按下阴影高度。
     */
    val stepperButtonShadowElevation: Dp,

    /**
     * 步进器按钮静止阴影高度。
     */
    val stepperButtonIdleElevation: Dp,

    /**
     * 步进器按钮圆角。
     */
    val stepperButtonRadius: Dp,

    /**
     * 搜索框不限制宽度时的占位宽度。
     */
    val searchUnboundedWidth: Dp,

    /**
     * 开关选中时滑块偏移。
     */
    val switchCheckedKnobOffset: Dp,

    /**
     * 开关未选中时滑块偏移。
     */
    val switchUncheckedKnobOffset: Dp,

    /**
     * 开关宽度。
     */
    val switchWidth: Dp,

    /**
     * 开关高度。
     */
    val switchHeight: Dp,

    /**
     * 开关轨道圆角。
     */
    val switchRadius: Dp,

    /**
     * 开关滑块尺寸。
     */
    val switchKnobSize: Dp,

    /**
     * 开关滑块阴影高度。
     */
    val switchKnobShadowElevation: Dp,

    /**
     * 复选框默认尺寸。
     */
    val checkboxDefaultSize: Dp,

    /**
     * 复选框圆角。
     */
    val checkboxRadius: Dp,

    /**
     * 复选框边框宽度。
     */
    val checkboxBorderWidth: Dp,

    /**
     * 复选框勾选标记内缩量。
     */
    val checkboxCheckInset: Dp,

    /**
     * 复选框不确定标记内缩量。
     */
    val checkboxIndeterminateInset: Dp,

    /**
     * 复选框不确定标记高度。
     */
    val checkboxIndeterminateHeight: Dp,

    /**
     * 复选框不确定标记圆角。
     */
    val checkboxIndeterminateRadius: Dp,

    /**
     * 单选框默认尺寸。
     */
    val radioDefaultSize: Dp,

    /**
     * 单选框边框宽度。
     */
    val radioBorderWidth: Dp,

    /**
     * 禁用文本输入框透明度。
     */
    val textFieldDisabledAlpha: Float,

    /**
     * 禁用步进器按钮透明度。
     */
    val stepperDisabledAlpha: Float,

    /**
     * 禁用开关透明度。
     */
    val switchDisabledAlpha: Float,

    /**
     * 禁用复选框透明度。
     */
    val checkboxDisabledAlpha: Float,

    /**
     * 禁用单选框透明度。
     */
    val radioDisabledAlpha: Float,
)

/**
 * 反馈组件内部尺寸；每个字段只控制名称对应的视觉职责。
 */
data class FeedbackControlMetrics(
    /**
     * 线性进度条默认高度。
     */
    val linearProgressHeight: Dp,

    /**
     * 环形进度条默认尺寸。
     */
    val circularProgressSize: Dp,

    /**
     * 环形进度条线宽。
     */
    val circularProgressStrokeWidth: Dp,

    /**
     * 横幅水平内边距。
     */
    val bannerHorizontalPadding: Dp,

    /**
     * 横幅垂直内边距。
     */
    val bannerVerticalPadding: Dp,

    /**
     * 横幅内容间距。
     */
    val bannerContentSpacing: Dp,

    /**
     * 横幅状态图标尺寸。
     */
    val bannerIconSize: Dp,

    /**
     * 横幅关闭图标尺寸。
     */
    val bannerCloseIconSize: Dp,

    /**
     * 小标签水平内边距。
     */
    val smallTagHorizontalPadding: Dp,

    /**
     * 中标签水平内边距。
     */
    val mediumTagHorizontalPadding: Dp,

    /**
     * 小标签垂直内边距。
     */
    val smallTagVerticalPadding: Dp,

    /**
     * 中标签垂直内边距。
     */
    val mediumTagVerticalPadding: Dp,

    /**
     * 小标签图标尺寸。
     */
    val smallTagIconSize: Dp,

    /**
     * 中标签图标尺寸。
     */
    val mediumTagIconSize: Dp,

    /**
     * 标签图标与文字间距。
     */
    val tagContentSpacing: Dp,

    /**
     * 空状态整体内边距。
     */
    val emptyStatePadding: Dp,

    /**
     * 空状态图标尺寸。
     */
    val emptyStateIconSize: Dp,

    /**
     * 空状态图标与标题间距。
     */
    val emptyStateTitleSpacing: Dp,

    /**
     * 空状态标题与说明间距。
     */
    val emptyStateDescriptionSpacing: Dp,

    /**
     * 空状态说明与操作间距。
     */
    val emptyStateActionSpacing: Dp,

    /**
     * 统计标签圆角。
     */
    val statChipRadius: Dp,

    /**
     * 统计标签水平内边距。
     */
    val statChipHorizontalPadding: Dp,

    /**
     * 统计标签垂直内边距。
     */
    val statChipVerticalPadding: Dp,

    /**
     * 统计标签内容间距。
     */
    val statChipContentSpacing: Dp,

    /**
     * 统计标签图标尺寸。
     */
    val statChipIconSize: Dp,

    /**
     * 区块标题底部内边距。
     */
    val sectionHeaderBottomPadding: Dp,

    /**
     * 区块标题内容间距。
     */
    val sectionHeaderContentSpacing: Dp,

    /**
     * 区块标题图标尺寸。
     */
    val sectionHeaderIconSize: Dp,

    /**
     * 导航项高度。
     */
    val navigationItemHeight: Dp,

    /**
     * 导航项圆角。
     */
    val navigationItemRadius: Dp,

    /**
     * 导航项水平内边距。
     */
    val navigationItemHorizontalPadding: Dp,

    /**
     * 导航项每层缩进量。
     */
    val navigationItemIndentPerLevel: Dp,

    /**
     * 导航项内容间距。
     */
    val navigationItemContentSpacing: Dp,

    /**
     * 导航项图标尺寸。
     */
    val navigationItemIconSize: Dp,

    /**
     * 导航分组标签起始内边距。
     */
    val navigationGroupStartPadding: Dp,

    /**
     * 导航分组标签顶部内边距。
     */
    val navigationGroupTopPadding: Dp,

    /**
     * 导航分组标签底部内边距。
     */
    val navigationGroupBottomPadding: Dp,

    /**
     * 标签图标透明度。
     */
    val tagIconAlpha: Float,

    /**
     * 环形进度指示器旋转一周时长。
     */
    val circularProgressRotationDurationMillis: Int,
)

/**
 * 创建不复用职责的基础组件默认尺寸。
 */
internal fun createBasicMetrics() = BasicMetrics(
    // 紧凑 Logo 默认尺寸。
    compactLogoSize = 26.dp,
    // 登录页 Logo 尺寸。
    largeLogoSize = 64.dp,
    // 紧凑 Logo 与文字间距。
    compactLogoTextSpacing = 8.dp,
    // 完整 Logo 默认高度。
    fullLogoHeight = 32.dp,
    // 完整 Logo 与文字间距。
    fullLogoTextSpacing = 6.dp,
    // 竖分隔线默认高度。
    verticalSeparatorHeight = 20.dp,
    // 竖分隔线宽度。
    verticalSeparatorWidth = 1.dp,
    // 底部分隔线厚度。
    bottomBorderThickness = 0.5.dp,
)

/**
 * 创建图标默认尺寸。
 */
internal fun createIconMetrics() = IconMetrics(
    // 图标默认尺寸。
    defaultSize = 16.dp,
    // 图标旋转一周时长。
    spinDurationMillis = 1000,
)

/**
 * 创建分隔线默认尺寸。
 */
internal fun createDividerMetrics() = DividerMetrics(
    // 横分隔线默认厚度。
    horizontalThickness = 0.5.dp,
    // 竖分隔线默认高度。
    verticalHeight = 24.dp,
    // 竖分隔线宽度。
    verticalWidth = 1.dp,
)

/**
 * 创建组件目录默认尺寸。
 */
internal fun createCatalogMetrics() = CatalogMetrics(
    // 页面内边距。
    pagePadding = 24.dp,
    // 区块间距。
    sectionSpacing = 24.dp,
    // 示例项间距。
    itemSpacing = 12.dp,
    // 图标预览尺寸。
    iconPreviewSize = 24.dp,
    // 输入框预览宽度。
    fieldPreviewWidth = 200.dp,
    // 进度条预览宽度。
    progressPreviewWidth = 200.dp,
    // 环形进度预览尺寸。
    circularProgressSize = 32.dp,
    // 纵向示例间距。
    verticalGroupSpacing = 8.dp,
    // 紧凑示例间距。
    compactItemSpacing = 8.dp,
    // 弹窗示例宽度。
    dialogPreviewWidth = 200.dp,
)

/**
 * 创建同步状态栏默认尺寸。
 */
internal fun createStatusBarMetrics() = StatusBarMetrics(
    // 最小高度。
    minimumHeight = 44.dp,
    // 水平内边距。
    horizontalPadding = 20.dp,
    // 垂直内边距。
    verticalPadding = 6.dp,
    // 状态内容间距。
    statusContentSpacing = 10.dp,
    // 同步图标尺寸。
    syncingIconSize = 16.dp,
    // 空闲指示点尺寸。
    idleIndicatorSize = 8.dp,
    // 操作区水平间距。
    actionHorizontalSpacing = 6.dp,
    // 操作区垂直间距。
    actionVerticalSpacing = 6.dp,
)

/**
 * 创建同步设置横幅默认尺寸。
 */
internal fun createSyncSetupMetrics() = SyncSetupMetrics(
    // 水平内边距。
    horizontalPadding = 20.dp,
    // 垂直内边距。
    verticalPadding = 8.dp,
)

/**
 * 创建表单控件内部默认尺寸。
 */
internal fun createFormControlMetrics() = FormControlMetrics(
    // 文本输入框边框宽度。
    textFieldBorderWidth = 2.dp,
    // 文本输入框水平内边距。
    textFieldHorizontalPadding = 12.dp,
    // 文本输入框内容间距。
    textFieldContentSpacing = 8.dp,
    // 文本输入框前缀图标尺寸。
    textFieldPrefixIconSize = 16.dp,
    // 数字输入框边框宽度。
    numberFieldBorderWidth = 2.dp,
    // 数字输入框水平内边距。
    numberFieldHorizontalPadding = 12.dp,
    // 数字输入框内容间距。
    numberFieldContentSpacing = 8.dp,
    // 数字输入区域宽度。
    numberFieldInputWidth = 120.dp,
    // 步进器容器圆角。
    stepperRadius = 8.dp,
    // 步进器容器内边距。
    stepperPadding = 3.dp,
    // 步进器减号图标尺寸。
    stepperMinusIconSize = 14.dp,
    // 步进器数值区域宽度。
    stepperValueWidth = 44.dp,
    // 步进器按钮尺寸。
    stepperButtonSize = 30.dp,
    // 步进器按钮按下阴影高度。
    stepperButtonShadowElevation = 1.dp,
    // 步进器按钮静止阴影高度。
    stepperButtonIdleElevation = 0.dp,
    // 步进器按钮圆角。
    stepperButtonRadius = 5.dp,
    // 搜索框不限制宽度时的占位宽度。
    searchUnboundedWidth = 0.dp,
    // 开关选中时滑块偏移。
    switchCheckedKnobOffset = 21.dp,
    // 开关未选中时滑块偏移。
    switchUncheckedKnobOffset = 3.dp,
    // 开关宽度。
    switchWidth = 46.dp,
    // 开关高度。
    switchHeight = 28.dp,
    // 开关轨道圆角。
    switchRadius = 14.dp,
    // 开关滑块尺寸。
    switchKnobSize = 22.dp,
    // 开关滑块阴影高度。
    switchKnobShadowElevation = 2.dp,
    // 复选框默认尺寸。
    checkboxDefaultSize = 18.dp,
    // 复选框圆角。
    checkboxRadius = 5.dp,
    // 复选框边框宽度。
    checkboxBorderWidth = 1.5.dp,
    // 复选框勾选标记内缩量。
    checkboxCheckInset = 5.dp,
    // 复选框不确定标记内缩量。
    checkboxIndeterminateInset = 9.dp,
    // 复选框不确定标记高度。
    checkboxIndeterminateHeight = 1.5.dp,
    // 复选框不确定标记圆角。
    checkboxIndeterminateRadius = 1.dp,
    // 单选框默认尺寸。
    radioDefaultSize = 16.dp,
    // 单选框边框宽度。
    radioBorderWidth = 1.dp,
    // 禁用文本输入框透明度。
    textFieldDisabledAlpha = 0.6f,
    // 禁用步进器按钮透明度。
    stepperDisabledAlpha = 0.4f,
    // 禁用开关透明度。
    switchDisabledAlpha = 0.5f,
    // 禁用复选框透明度。
    checkboxDisabledAlpha = 0.5f,
    // 禁用单选框透明度。
    radioDisabledAlpha = 0.5f,
)

/**
 * 创建反馈组件内部默认尺寸。
 */
internal fun createFeedbackControlMetrics() = FeedbackControlMetrics(
    // 线性进度条默认高度。
    linearProgressHeight = 6.dp,
    // 环形进度条默认尺寸。
    circularProgressSize = 24.dp,
    // 环形进度条线宽。
    circularProgressStrokeWidth = 2.5.dp,
    // 横幅水平内边距。
    bannerHorizontalPadding = 14.dp,
    // 横幅垂直内边距。
    bannerVerticalPadding = 12.dp,
    // 横幅内容间距。
    bannerContentSpacing = 10.dp,
    // 横幅状态图标尺寸。
    bannerIconSize = 18.dp,
    // 横幅关闭图标尺寸。
    bannerCloseIconSize = 14.dp,
    // 小标签水平内边距。
    smallTagHorizontalPadding = 6.dp,
    // 中标签水平内边距。
    mediumTagHorizontalPadding = 10.dp,
    // 小标签垂直内边距。
    smallTagVerticalPadding = 2.dp,
    // 中标签垂直内边距。
    mediumTagVerticalPadding = 3.dp,
    // 小标签图标尺寸。
    smallTagIconSize = 12.dp,
    // 中标签图标尺寸。
    mediumTagIconSize = 14.dp,
    // 标签图标与文字间距。
    tagContentSpacing = 4.dp,
    // 空状态整体内边距。
    emptyStatePadding = 32.dp,
    // 空状态图标尺寸。
    emptyStateIconSize = 36.dp,
    // 空状态图标与标题间距。
    emptyStateTitleSpacing = 16.dp,
    // 空状态标题与说明间距。
    emptyStateDescriptionSpacing = 6.dp,
    // 空状态说明与操作间距。
    emptyStateActionSpacing = 24.dp,
    // 统计标签圆角。
    statChipRadius = 8.dp,
    // 统计标签水平内边距。
    statChipHorizontalPadding = 8.dp,
    // 统计标签垂直内边距。
    statChipVerticalPadding = 4.dp,
    // 统计标签内容间距。
    statChipContentSpacing = 4.dp,
    // 统计标签图标尺寸。
    statChipIconSize = 12.dp,
    // 区块标题底部内边距。
    sectionHeaderBottomPadding = 12.dp,
    // 区块标题内容间距。
    sectionHeaderContentSpacing = 8.dp,
    // 区块标题图标尺寸。
    sectionHeaderIconSize = 18.dp,
    // 导航项高度。
    navigationItemHeight = 46.dp,
    // 导航项圆角。
    navigationItemRadius = 8.dp,
    // 导航项水平内边距。
    navigationItemHorizontalPadding = 14.dp,
    // 导航项每层缩进量。
    navigationItemIndentPerLevel = 1.dp,
    // 导航项内容间距。
    navigationItemContentSpacing = 12.dp,
    // 导航项图标尺寸。
    navigationItemIconSize = 18.dp,
    // 导航分组标签起始内边距。
    navigationGroupStartPadding = 14.dp,
    // 导航分组标签顶部内边距。
    navigationGroupTopPadding = 20.dp,
    // 导航分组标签底部内边距。
    navigationGroupBottomPadding = 6.dp,
    // 标签图标透明度。
    tagIconAlpha = 0.7f,
    // 环形进度指示器旋转一周时长。
    circularProgressRotationDurationMillis = 1200,
)
