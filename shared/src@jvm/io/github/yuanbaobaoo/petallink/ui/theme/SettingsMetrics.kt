package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * 设置页尺寸。
 */
data class SettingsMetrics(
    /**
     * 顶部标题栏高度。
     */
    val headerHeight: Dp,

    /**
     * 顶部标题栏水平内边距。
     */
    val headerHorizontalPadding: Dp,

    /**
     * 顶部标题栏内容间距。
     */
    val headerContentSpacing: Dp,

    /**
     * 左侧导航宽度。
     */
    val navigationWidth: Dp,

    /**
     * 左侧导航水平内边距。
     */
    val navigationHorizontalPadding: Dp,

    /**
     * 左侧导航垂直内边距。
     */
    val navigationVerticalPadding: Dp,

    /**
     * 左侧导航项间距。
     */
    val navigationItemSpacing: Dp,

    /**
     * 导航与内容区分隔线宽度。
     */
    val navigationBorderWidth: Dp,

    /**
     * 设置内容水平内边距。
     */
    val bodyHorizontalPadding: Dp,

    /**
     * 设置内容垂直内边距。
     */
    val bodyVerticalPadding: Dp,

    /**
     * 并发设置控件间距。
     */
    val concurrencyContentSpacing: Dp,

    /**
     * 跳过文件输入框宽度。
     */
    val skipPatternFieldWidth: Dp,

    /**
     * OAuth 提示顶部间距。
     */
    val oauthBannerTopPadding: Dp,

    /**
     * OAuth 提示底部间距。
     */
    val oauthBannerBottomPadding: Dp,

    /**
     * 日志面板内边距。
     */
    val logPanelPadding: Dp,

    /**
     * 日志面板内容间距。
     */
    val logPanelContentSpacing: Dp,

    /**
     * 底部操作栏分隔线宽度。
     */
    val footerBorderWidth: Dp,

    /**
     * 底部操作栏高度。
     */
    val footerHeight: Dp,

    /**
     * 底部操作栏水平内边距。
     */
    val footerHorizontalPadding: Dp,

    /**
     * 底部操作项间距。
     */
    val footerActionSpacing: Dp,

    /**
     * 保存成功内容间距。
     */
    val savedIndicatorSpacing: Dp,

    /**
     * 保存成功指示点尺寸。
     */
    val savedIndicatorSize: Dp,

    /**
     * 设置卡片默认水平内边距。
     */
    val panelHorizontalPadding: Dp,

    /**
     * 设置卡片默认垂直内边距。
     */
    val panelVerticalPadding: Dp,

    /**
     * 设置卡片默认内容间距。
     */
    val panelDefaultContentSpacing: Dp,

    /**
     * 设置卡片圆角。
     */
    val panelRadius: Dp,

    /**
     * 设置卡片边框宽度。
     */
    val panelBorderWidth: Dp,

    /**
     * 首个分组标题顶部间距。
     */
    val firstGroupTopPadding: Dp,

    /**
     * 后续分组标题顶部间距。
     */
    val groupTopPadding: Dp,

    /**
     * 分组标题底部间距。
     */
    val groupBottomPadding: Dp,

    /**
     * 设置行垂直内边距。
     */
    val settingRowVerticalPadding: Dp,

    /**
     * 设置行内容间距。
     */
    val settingRowContentSpacing: Dp,

    /**
     * 设置说明顶部间距。
     */
    val settingDescriptionTopPadding: Dp,

    /**
     * 设置行分隔线宽度。
     */
    val settingRowDividerWidth: Dp,

    /**
     * 同步目录卡片圆角。
     */
    val mountPanelRadius: Dp,

    /**
     * 已配置目录边框宽度。
     */
    val configuredMountBorderWidth: Dp,

    /**
     * 未配置目录边框宽度。
     */
    val emptyMountBorderWidth: Dp,

    /**
     * 同步目录卡片水平内边距。
     */
    val mountPanelHorizontalPadding: Dp,

    /**
     * 已配置目录卡片垂直内边距。
     */
    val configuredMountVerticalPadding: Dp,

    /**
     * 未配置目录卡片垂直内边距。
     */
    val emptyMountVerticalPadding: Dp,

    /**
     * 同步目录卡片内容间距。
     */
    val mountPanelContentSpacing: Dp,

    /**
     * 未配置目录徽章尺寸。
     */
    val emptyMountBadgeSize: Dp,

    /**
     * 未配置目录徽章圆角。
     */
    val emptyMountBadgeRadius: Dp,

    /**
     * 未配置目录图标尺寸。
     */
    val emptyMountIconSize: Dp,

    /**
     * 已配置目录徽章尺寸。
     */
    val configuredMountBadgeSize: Dp,

    /**
     * 已配置目录徽章圆角。
     */
    val configuredMountBadgeRadius: Dp,

    /**
     * 已配置目录图标尺寸。
     */
    val configuredMountIconSize: Dp,

    /**
     * 同步目录路径背景圆角。
     */
    val mountPathRadius: Dp,

    /**
     * 同步目录路径水平内边距。
     */
    val mountPathHorizontalPadding: Dp,

    /**
     * 同步目录路径垂直内边距。
     */
    val mountPathVerticalPadding: Dp,

    /**
     * 同步目录提示间距。
     */
    val mountBannerSpacing: Dp,

    /**
     * 账号卡片水平内边距。
     */
    val accountPanelHorizontalPadding: Dp,

    /**
     * 账号卡片垂直内边距。
     */
    val accountPanelVerticalPadding: Dp,

    /**
     * 账号头像与名称间距。
     */
    val accountContentSpacing: Dp,

    /**
     * 账号头像尺寸。
     */
    val accountAvatarSize: Dp,

    /**
     * 账号区块间距。
     */
    val accountSectionSpacing: Dp,

    /**
     * 详情行垂直内边距。
     */
    val detailRowVerticalPadding: Dp,

    /**
     * 详情标签宽度。
     */
    val detailLabelWidth: Dp,

    /**
     * 详情内容底部间距。
     */
    val detailContentSpacing: Dp,

    /**
     * 详情分隔线宽度。
     */
    val detailDividerWidth: Dp,

    /**
     * 关于卡片内边距。
     */
    val aboutPanelPadding: Dp,

    /**
     * 关于卡片内容间距。
     */
    val aboutPanelContentSpacing: Dp,

    /**
     * 关于区域 Logo 高度。
     */
    val aboutLogoHeight: Dp,

    /**
     * 版本信息间距。
     */
    val versionContentSpacing: Dp,

    /**
     * 外链之间的间距。
     */
    val externalLinksSpacing: Dp,

    /**
     * 外链垂直内边距。
     */
    val externalLinkVerticalPadding: Dp,

    /**
     * 外链图标文字间距。
     */
    val externalLinkContentSpacing: Dp,

    /**
     * 外链图标尺寸。
     */
    val externalLinkIconSize: Dp,
)

/**
 * 创建设置页默认尺寸。
 */
internal fun createSettingsMetrics() = SettingsMetrics(
    // 顶部标题栏高度。
    headerHeight = 56.dp,
    // 顶部标题栏水平内边距。
    headerHorizontalPadding = 16.dp,
    // 顶部标题栏内容间距。
    headerContentSpacing = 8.dp,
    // 左侧导航宽度。
    navigationWidth = 240.dp,
    // 左侧导航水平内边距。
    navigationHorizontalPadding = 12.dp,
    // 左侧导航垂直内边距。
    navigationVerticalPadding = 20.dp,
    // 左侧导航项间距。
    navigationItemSpacing = 6.dp,
    // 导航与内容区分隔线宽度。
    navigationBorderWidth = 0.5.dp,
    // 设置内容水平内边距。
    bodyHorizontalPadding = 32.dp,
    // 设置内容垂直内边距。
    bodyVerticalPadding = 28.dp,
    // 并发设置控件间距。
    concurrencyContentSpacing = 8.dp,
    // 跳过文件输入框宽度。
    skipPatternFieldWidth = 280.dp,
    // OAuth 提示顶部间距。
    oauthBannerTopPadding = 4.dp,
    // OAuth 提示底部间距。
    oauthBannerBottomPadding = 8.dp,
    // 日志面板内边距。
    logPanelPadding = 24.dp,
    // 日志面板内容间距。
    logPanelContentSpacing = 14.dp,
    // 底部操作栏分隔线宽度。
    footerBorderWidth = 0.5.dp,
    // 底部操作栏高度。
    footerHeight = 64.dp,
    // 底部操作栏水平内边距。
    footerHorizontalPadding = 32.dp,
    // 底部操作项间距。
    footerActionSpacing = 10.dp,
    // 保存成功内容间距。
    savedIndicatorSpacing = 4.dp,
    // 保存成功指示点尺寸。
    savedIndicatorSize = 6.dp,
    // 设置卡片默认水平内边距。
    panelHorizontalPadding = 24.dp,
    // 设置卡片默认垂直内边距。
    panelVerticalPadding = 4.dp,
    // 设置卡片默认内容间距。
    panelDefaultContentSpacing = 0.dp,
    // 设置卡片圆角。
    panelRadius = 10.dp,
    // 设置卡片边框宽度。
    panelBorderWidth = 0.5.dp,
    // 首个分组标题顶部间距。
    firstGroupTopPadding = 12.dp,
    // 后续分组标题顶部间距。
    groupTopPadding = 20.dp,
    // 分组标题底部间距。
    groupBottomPadding = 8.dp,
    // 设置行垂直内边距。
    settingRowVerticalPadding = 16.dp,
    // 设置行内容间距。
    settingRowContentSpacing = 24.dp,
    // 设置说明顶部间距。
    settingDescriptionTopPadding = 3.dp,
    // 设置行分隔线宽度。
    settingRowDividerWidth = 0.5.dp,
    // 同步目录卡片圆角。
    mountPanelRadius = 10.dp,
    // 已配置目录边框宽度。
    configuredMountBorderWidth = 1.dp,
    // 未配置目录边框宽度。
    emptyMountBorderWidth = 0.5.dp,
    // 同步目录卡片水平内边距。
    mountPanelHorizontalPadding = 24.dp,
    // 已配置目录卡片垂直内边距。
    configuredMountVerticalPadding = 32.dp,
    // 未配置目录卡片垂直内边距。
    emptyMountVerticalPadding = 40.dp,
    // 同步目录卡片内容间距。
    mountPanelContentSpacing = 12.dp,
    // 未配置目录徽章尺寸。
    emptyMountBadgeSize = 72.dp,
    // 未配置目录徽章圆角。
    emptyMountBadgeRadius = 14.dp,
    // 未配置目录图标尺寸。
    emptyMountIconSize = 48.dp,
    // 已配置目录徽章尺寸。
    configuredMountBadgeSize = 40.dp,
    // 已配置目录徽章圆角。
    configuredMountBadgeRadius = 10.dp,
    // 已配置目录图标尺寸。
    configuredMountIconSize = 20.dp,
    // 同步目录路径背景圆角。
    mountPathRadius = 12.dp,
    // 同步目录路径水平内边距。
    mountPathHorizontalPadding = 12.dp,
    // 同步目录路径垂直内边距。
    mountPathVerticalPadding = 4.dp,
    // 同步目录提示间距。
    mountBannerSpacing = 16.dp,
    // 账号卡片水平内边距。
    accountPanelHorizontalPadding = 24.dp,
    // 账号卡片垂直内边距。
    accountPanelVerticalPadding = 16.dp,
    // 账号头像与名称间距。
    accountContentSpacing = 16.dp,
    // 账号头像尺寸。
    accountAvatarSize = 56.dp,
    // 账号区块间距。
    accountSectionSpacing = 16.dp,
    // 详情行垂直内边距。
    detailRowVerticalPadding = 12.dp,
    // 详情标签宽度。
    detailLabelWidth = 96.dp,
    // 详情内容底部间距。
    detailContentSpacing = 12.dp,
    // 详情分隔线宽度。
    detailDividerWidth = 0.5.dp,
    // 关于卡片内边距。
    aboutPanelPadding = 24.dp,
    // 关于卡片内容间距。
    aboutPanelContentSpacing = 14.dp,
    // 关于区域 Logo 高度。
    aboutLogoHeight = 30.dp,
    // 版本信息间距。
    versionContentSpacing = 10.dp,
    // 外链之间的间距。
    externalLinksSpacing = 16.dp,
    // 外链垂直内边距。
    externalLinkVerticalPadding = 4.dp,
    // 外链图标文字间距。
    externalLinkContentSpacing = 4.dp,
    // 外链图标尺寸。
    externalLinkIconSize = 16.dp,
)
