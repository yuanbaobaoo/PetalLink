package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * 更新弹窗尺寸。
 */
data class UpdateDialogMetrics(
    /**
     * 弹窗宽度。
     */
    val dialogWidth: Dp,

    /**
     * 弹窗阴影高度。
     */
    val dialogShadowElevation: Dp,

    /**
     * 弹窗圆角。
     */
    val dialogRadius: Dp,

    /**
     * 标题区水平内边距。
     */
    val headerHorizontalPadding: Dp,

    /**
     * 标题区顶部内边距。
     */
    val headerTopPadding: Dp,

    /**
     * 标题区底部内边距。
     */
    val headerBottomPadding: Dp,

    /**
     * 标题图标与文字间距。
     */
    val headerContentSpacing: Dp,

    /**
     * 标题图标徽章尺寸。
     */
    val headerBadgeSize: Dp,

    /**
     * 标题图标徽章圆角。
     */
    val headerBadgeRadius: Dp,

    /**
     * 标题图标尺寸。
     */
    val headerIconSize: Dp,

    /**
     * 版本号水平内边距。
     */
    val versionHorizontalPadding: Dp,

    /**
     * 正文水平内边距。
     */
    val bodyHorizontalPadding: Dp,

    /**
     * 正文顶部内边距。
     */
    val bodyTopPadding: Dp,

    /**
     * 正文底部内边距。
     */
    val bodyBottomPadding: Dp,

    /**
     * 操作区水平内边距。
     */
    val footerHorizontalPadding: Dp,

    /**
     * 操作区顶部内边距。
     */
    val footerTopPadding: Dp,

    /**
     * 操作区底部内边距。
     */
    val footerBottomPadding: Dp,

    /**
     * 更新内容标签与正文间距。
     */
    val releaseNotesLabelSpacing: Dp,

    /**
     * 更新内容最大高度。
     */
    val releaseNotesMaximumHeight: Dp,

    /**
     * 更新内容容器圆角。
     */
    val releaseNotesRadius: Dp,

    /**
     * 更新内容容器内边距。
     */
    val releaseNotesPadding: Dp,

    /**
     * 下载进度条与百分比间距。
     */
    val progressContentSpacing: Dp,

    /**
     * 下载进度轨道高度。
     */
    val progressTrackHeight: Dp,

    /**
     * 下载进度轨道圆角。
     */
    val progressTrackRadius: Dp,

    /**
     * 下载进度填充高度。
     */
    val progressFillHeight: Dp,

    /**
     * 下载进度填充圆角。
     */
    val progressFillRadius: Dp,

    /**
     * 等待状态图标与文字间距。
     */
    val waitingContentSpacing: Dp,

    /**
     * 等待状态旋转图标容器尺寸。
     */
    val spinnerContainerSize: Dp,

    /**
     * 等待状态旋转图标顶部校正量。
     */
    val spinnerTopPadding: Dp,

    /**
     * 等待状态旋转圆环尺寸。
     */
    val spinnerRingSize: Dp,

    /**
     * 等待状态旋转圆环线宽。
     */
    val spinnerRingStrokeWidth: Dp,

    /**
     * 操作区相邻按钮间距。
     */
    val footerActionSpacing: Dp,

    /**
     * 等待状态旋转图标旋转一周时长。
     */
    val spinnerRotationDurationMillis: Int,
)

/**
 * 创建更新弹窗默认尺寸。
 */
internal fun createUpdateDialogMetrics() = UpdateDialogMetrics(
    // 弹窗宽度。
    dialogWidth = 440.dp,
    // 弹窗阴影高度。
    dialogShadowElevation = 24.dp,
    // 弹窗圆角。
    dialogRadius = 12.dp,
    // 标题区水平内边距。
    headerHorizontalPadding = 32.dp,
    // 标题区顶部内边距。
    headerTopPadding = 32.dp,
    // 标题区底部内边距。
    headerBottomPadding = 4.dp,
    // 标题图标与文字间距。
    headerContentSpacing = 8.dp,
    // 标题图标徽章尺寸。
    headerBadgeSize = 40.dp,
    // 标题图标徽章圆角。
    headerBadgeRadius = 10.dp,
    // 标题图标尺寸。
    headerIconSize = 20.dp,
    // 版本号水平内边距。
    versionHorizontalPadding = 32.dp,
    // 正文水平内边距。
    bodyHorizontalPadding = 32.dp,
    // 正文顶部内边距。
    bodyTopPadding = 12.dp,
    // 正文底部内边距。
    bodyBottomPadding = 32.dp,
    // 操作区水平内边距。
    footerHorizontalPadding = 16.dp,
    // 操作区顶部内边距。
    footerTopPadding = 8.dp,
    // 操作区底部内边距。
    footerBottomPadding = 16.dp,
    // 更新内容标签与正文间距。
    releaseNotesLabelSpacing = 4.dp,
    // 更新内容最大高度。
    releaseNotesMaximumHeight = 180.dp,
    // 更新内容容器圆角。
    releaseNotesRadius = 8.dp,
    // 更新内容容器内边距。
    releaseNotesPadding = 12.dp,
    // 下载进度条与百分比间距。
    progressContentSpacing = 12.dp,
    // 下载进度轨道高度。
    progressTrackHeight = 8.dp,
    // 下载进度轨道圆角。
    progressTrackRadius = 4.dp,
    // 下载进度填充高度。
    progressFillHeight = 8.dp,
    // 下载进度填充圆角。
    progressFillRadius = 4.dp,
    // 等待状态图标与文字间距。
    waitingContentSpacing = 12.dp,
    // 等待状态旋转图标容器尺寸。
    spinnerContainerSize = 20.dp,
    // 等待状态旋转图标顶部校正量。
    spinnerTopPadding = 2.dp,
    // 等待状态旋转圆环尺寸。
    spinnerRingSize = 20.dp,
    // 等待状态旋转圆环线宽。
    spinnerRingStrokeWidth = 2.5.dp,
    // 操作区相邻按钮间距。
    footerActionSpacing = 8.dp,
    // 等待状态旋转图标旋转一周时长。
    spinnerRotationDurationMillis = 800,
)

/**
 * 侧边栏尺寸。
 */
data class SidebarMetrics(
    /**
     * 侧边栏整体宽度。
     */
    val width: Dp,

    /**
     * Logo 区高度。
     */
    val logoHeaderHeight: Dp,

    /**
     * Logo 区水平内边距。
     */
    val logoHeaderHorizontalPadding: Dp,

    /**
     * Logo 尺寸。
     */
    val logoSize: Dp,

    /**
     * 分组标签起始内边距。
     */
    val sectionLabelStartPadding: Dp,

    /**
     * 分组标签顶部内边距。
     */
    val sectionLabelTopPadding: Dp,

    /**
     * 分组标签底部内边距。
     */
    val sectionLabelBottomPadding: Dp,

    /**
     * 目录树水平内边距。
     */
    val treeHorizontalPadding: Dp,

    /**
     * 目录树垂直内边距。
     */
    val treeVerticalPadding: Dp,

    /**
     * 账号卡片外边距。
     */
    val accountOuterPadding: Dp,

    /**
     * 账号卡片圆角。
     */
    val accountRadius: Dp,

    /**
     * 账号卡片边框宽度。
     */
    val accountBorderWidth: Dp,

    /**
     * 账号卡片内边距。
     */
    val accountInnerPadding: Dp,

    /**
     * 账号头像与信息间距。
     */
    val accountContentSpacing: Dp,

    /**
     * 账号头像尺寸。
     */
    val accountAvatarSize: Dp,

    /**
     * 配额文字顶部校正量。
     */
    val accountQuotaTopPadding: Dp,

    /**
     * 配额文字与进度条间距。
     */
    val accountQuotaProgressSpacing: Dp,

    /**
     * 配额进度条高度。
     */
    val accountQuotaProgressHeight: Dp,

    /**
     * 更新卡片水平外边距。
     */
    val updateCardHorizontalMargin: Dp,

    /**
     * 更新卡片底部外边距。
     */
    val updateCardBottomMargin: Dp,

    /**
     * 更新卡片圆角。
     */
    val updateCardRadius: Dp,

    /**
     * 更新卡片内边距。
     */
    val updateCardPadding: Dp,

    /**
     * 下载信息与进度条间距。
     */
    val downloadProgressSpacing: Dp,

    /**
     * 关闭更新按钮尺寸。
     */
    val dismissButtonSize: Dp,

    /**
     * 更新提示与操作按钮间距。
     */
    val availableActionSpacing: Dp,

    /**
     * 立即更新按钮高度。
     */
    val installButtonHeight: Dp,

    /**
     * 立即更新按钮圆角。
     */
    val installButtonRadius: Dp,

    /**
     * 目录树节点高度。
     */
    val treeNodeHeight: Dp,

    /**
     * 目录树每层缩进量。
     */
    val treeDepthIndent: Dp,

    /**
     * 目录树首层起始内边距。
     */
    val treeNodeStartPadding: Dp,

    /**
     * 目录树节点结束内边距。
     */
    val treeNodeEndPadding: Dp,

    /**
     * 目录树节点圆角。
     */
    val treeNodeRadius: Dp,

    /**
     * 目录树节点内容间距。
     */
    val treeNodeContentSpacing: Dp,

    /**
     * 目录树展开按钮命中区尺寸。
     */
    val treeExpanderSize: Dp,

    /**
     * 目录树展开箭头尺寸。
     */
    val treeArrowIconSize: Dp,

    /**
     * 目录树文件夹图标尺寸。
     */
    val treeFolderIconSize: Dp,
)

/**
 * 创建侧边栏默认尺寸。
 */
internal fun createSidebarMetrics() = SidebarMetrics(
    // 侧边栏整体宽度。
    width = 248.dp,
    // Logo 区高度。
    logoHeaderHeight = 60.dp,
    // Logo 区水平内边距。
    logoHeaderHorizontalPadding = 18.dp,
    // Logo 尺寸。
    logoSize = 26.dp,
    // 分组标签起始内边距。
    sectionLabelStartPadding = 18.dp,
    // 分组标签顶部内边距。
    sectionLabelTopPadding = 12.dp,
    // 分组标签底部内边距。
    sectionLabelBottomPadding = 6.dp,
    // 目录树水平内边距。
    treeHorizontalPadding = 8.dp,
    // 目录树垂直内边距。
    treeVerticalPadding = 4.dp,
    // 账号卡片外边距。
    accountOuterPadding = 10.dp,
    // 账号卡片圆角。
    accountRadius = 10.dp,
    // 账号卡片边框宽度。
    accountBorderWidth = 0.5.dp,
    // 账号卡片内边距。
    accountInnerPadding = 12.dp,
    // 账号头像与信息间距。
    accountContentSpacing = 10.dp,
    // 账号头像尺寸。
    accountAvatarSize = 32.dp,
    // 配额文字顶部校正量。
    accountQuotaTopPadding = 1.dp,
    // 配额文字与进度条间距。
    accountQuotaProgressSpacing = 6.dp,
    // 配额进度条高度。
    accountQuotaProgressHeight = 4.dp,
    // 更新卡片水平外边距。
    updateCardHorizontalMargin = 10.dp,
    // 更新卡片底部外边距。
    updateCardBottomMargin = 10.dp,
    // 更新卡片圆角。
    updateCardRadius = 10.dp,
    // 更新卡片内边距。
    updateCardPadding = 12.dp,
    // 下载信息与进度条间距。
    downloadProgressSpacing = 8.dp,
    // 关闭更新按钮尺寸。
    dismissButtonSize = 20.dp,
    // 更新提示与操作按钮间距。
    availableActionSpacing = 8.dp,
    // 立即更新按钮高度。
    installButtonHeight = 28.dp,
    // 立即更新按钮圆角。
    installButtonRadius = 5.dp,
    // 目录树节点高度。
    treeNodeHeight = 32.dp,
    // 目录树每层缩进量。
    treeDepthIndent = 14.dp,
    // 目录树首层起始内边距。
    treeNodeStartPadding = 8.dp,
    // 目录树节点结束内边距。
    treeNodeEndPadding = 8.dp,
    // 目录树节点圆角。
    treeNodeRadius = 6.dp,
    // 目录树节点内容间距。
    treeNodeContentSpacing = 8.dp,
    // 目录树展开按钮命中区尺寸。
    treeExpanderSize = 16.dp,
    // 目录树展开箭头尺寸。
    treeArrowIconSize = 12.dp,
    // 目录树文件夹图标尺寸。
    treeFolderIconSize = 16.dp,
)

/**
 * 传输弹窗尺寸。
 */
data class TransferPopoverMetrics(
    /**
     * 弹窗宽度。
     */
    val panelWidth: Dp,

    /**
     * 弹窗高度。
     */
    val panelHeight: Dp,

    /**
     * 弹窗顶部偏移。
     */
    val panelTopOffset: Dp,

    /**
     * 弹窗结束侧偏移。
     */
    val panelEndOffset: Dp,

    /**
     * 弹窗阴影高度。
     */
    val panelShadowElevation: Dp,

    /**
     * 弹窗圆角。
     */
    val panelRadius: Dp,

    /**
     * 弹窗边框宽度。
     */
    val panelBorderWidth: Dp,

    /**
     * 标题栏高度。
     */
    val headerHeight: Dp,

    /**
     * 标题栏起始内边距。
     */
    val headerStartPadding: Dp,

    /**
     * 标题栏结束内边距。
     */
    val headerEndPadding: Dp,

    /**
     * 标题栏内容间距。
     */
    val headerContentSpacing: Dp,

    /**
     * 标题栏图标尺寸。
     */
    val headerIconSize: Dp,

    /**
     * 统计区水平内边距。
     */
    val summaryHorizontalPadding: Dp,

    /**
     * 统计区底部内边距。
     */
    val summaryBottomPadding: Dp,

    /**
     * 统计卡片间距。
     */
    val summaryItemSpacing: Dp,

    /**
     * 统计卡片圆角。
     */
    val summaryRadius: Dp,

    /**
     * 统计卡片水平内边距。
     */
    val summaryHorizontalContentPadding: Dp,

    /**
     * 统计卡片垂直内边距。
     */
    val summaryVerticalContentPadding: Dp,

    /**
     * 统计数字与标签间距。
     */
    val summaryTextSpacing: Dp,

    /**
     * 任务行最小高度。
     */
    val taskMinimumHeight: Dp,

    /**
     * 任务行水平内边距。
     */
    val taskHorizontalPadding: Dp,

    /**
     * 任务行垂直内边距。
     */
    val taskVerticalPadding: Dp,

    /**
     * 任务行主要内容间距。
     */
    val taskContentSpacing: Dp,

    /**
     * 传输方向徽章尺寸。
     */
    val directionBadgeSize: Dp,

    /**
     * 传输方向徽章圆角。
     */
    val directionBadgeRadius: Dp,

    /**
     * 传输方向图标尺寸。
     */
    val directionIconSize: Dp,

    /**
     * 任务信息纵向间距。
     */
    val taskInfoSpacing: Dp,

    /**
     * 方向标签与文件名间距。
     */
    val taskNameSpacing: Dp,

    /**
     * 进度条与进度文字间距。
     */
    val taskProgressSpacing: Dp,

    /**
     * 状态区宽度。
     */
    val taskStateWidth: Dp,

    /**
     * 状态图标与文字间距。
     */
    val taskStateSpacing: Dp,

    /**
     * 状态图标尺寸。
     */
    val taskStateIconSize: Dp,
)

/**
 * 创建传输弹窗默认尺寸。
 */
internal fun createTransferPopoverMetrics() = TransferPopoverMetrics(
    // 弹窗宽度。
    panelWidth = 440.dp,
    // 弹窗高度。
    panelHeight = 580.dp,
    // 弹窗顶部偏移。
    panelTopOffset = 64.dp,
    // 弹窗结束侧偏移。
    panelEndOffset = 20.dp,
    // 弹窗阴影高度。
    panelShadowElevation = 16.dp,
    // 弹窗圆角。
    panelRadius = 12.dp,
    // 弹窗边框宽度。
    panelBorderWidth = 0.5.dp,
    // 标题栏高度。
    headerHeight = 60.dp,
    // 标题栏起始内边距。
    headerStartPadding = 20.dp,
    // 标题栏结束内边距。
    headerEndPadding = 12.dp,
    // 标题栏内容间距。
    headerContentSpacing = 10.dp,
    // 标题栏图标尺寸。
    headerIconSize = 18.dp,
    // 统计区水平内边距。
    summaryHorizontalPadding = 20.dp,
    // 统计区底部内边距。
    summaryBottomPadding = 14.dp,
    // 统计卡片间距。
    summaryItemSpacing = 8.dp,
    // 统计卡片圆角。
    summaryRadius = 8.dp,
    // 统计卡片水平内边距。
    summaryHorizontalContentPadding = 10.dp,
    // 统计卡片垂直内边距。
    summaryVerticalContentPadding = 8.dp,
    // 统计数字与标签间距。
    summaryTextSpacing = 2.dp,
    // 任务行最小高度。
    taskMinimumHeight = 68.dp,
    // 任务行水平内边距。
    taskHorizontalPadding = 20.dp,
    // 任务行垂直内边距。
    taskVerticalPadding = 10.dp,
    // 任务行主要内容间距。
    taskContentSpacing = 12.dp,
    // 传输方向徽章尺寸。
    directionBadgeSize = 36.dp,
    // 传输方向徽章圆角。
    directionBadgeRadius = 8.dp,
    // 传输方向图标尺寸。
    directionIconSize = 18.dp,
    // 任务信息纵向间距。
    taskInfoSpacing = 5.dp,
    // 方向标签与文件名间距。
    taskNameSpacing = 6.dp,
    // 进度条与进度文字间距。
    taskProgressSpacing = 10.dp,
    // 状态区宽度。
    taskStateWidth = 80.dp,
    // 状态图标与文字间距。
    taskStateSpacing = 3.dp,
    // 状态图标尺寸。
    taskStateIconSize = 12.dp,
)
