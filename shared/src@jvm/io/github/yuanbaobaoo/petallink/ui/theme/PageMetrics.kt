package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * 登录页尺寸。
 */
data class LoginMetrics(
    /**
     * 顶部装饰横向偏移。
     */
    val topDecorationOffsetX: Dp,

    /**
     * 顶部装饰纵向偏移。
     */
    val topDecorationOffsetY: Dp,

    /**
     * 顶部装饰尺寸。
     */
    val topDecorationSize: Dp,

    /**
     * 底部装饰横向偏移。
     */
    val bottomDecorationOffsetX: Dp,

    /**
     * 底部装饰纵向偏移。
     */
    val bottomDecorationOffsetY: Dp,

    /**
     * 底部装饰尺寸。
     */
    val bottomDecorationSize: Dp,

    /**
     * 中部装饰尺寸。
     */
    val centerDecorationSize: Dp,

    /**
     * 登录卡片宽度。
     */
    val cardWidth: Dp,

    /**
     * 登录卡片阴影高度。
     */
    val cardShadowElevation: Dp,

    /**
     * 登录卡片圆角。
     */
    val cardRadius: Dp,

    /**
     * 登录卡片水平内边距。
     */
    val cardHorizontalPadding: Dp,

    /**
     * 登录卡片垂直内边距。
     */
    val cardVerticalPadding: Dp,

    /**
     * Logo 与标题间距。
     */
    val logoTitleSpacing: Dp,

    /**
     * 标题与副标题间距。
     */
    val subtitleSpacing: Dp,

    /**
     * 标题强调线宽度。
     */
    val accentWidth: Dp,

    /**
     * 标题强调线高度。
     */
    val accentHeight: Dp,

    /**
     * 标题强调线圆角。
     */
    val accentRadius: Dp,

    /**
     * 标题强调线底部间距。
     */
    val accentBottomSpacing: Dp,

    /**
     * 登录消息间距。
     */
    val messageSpacing: Dp,

    /**
     * 登录内容底部间距。
     */
    val contentBottomSpacing: Dp,

    /**
     * 授权状态栏高度。
     */
    val authorizingHeight: Dp,

    /**
     * 授权状态栏圆角。
     */
    val authorizingRadius: Dp,

    /**
     * 授权状态栏水平内边距。
     */
    val authorizingHorizontalPadding: Dp,

    /**
     * 授权状态栏内容间距。
     */
    val authorizingContentSpacing: Dp,

    /**
     * 授权旋转图标尺寸。
     */
    val authorizingSpinnerSize: Dp,

    /**
     * 授权旋转图标线宽。
     */
    val authorizingSpinnerStroke: Dp,

    /**
     * 错误提示与操作间距。
     */
    val errorActionSpacing: Dp,

    /**
     * 登录按钮高度。
     */
    val loginButtonHeight: Dp,

    /**
     * 页脚顶部间距。
     */
    val footerSpacing: Dp,

    /**
     * 顶部装饰透明度。
     */
    val topDecorationAlpha: Float,

    /**
     * 底部装饰透明度。
     */
    val bottomDecorationAlpha: Float,

    /**
     * 中部装饰透明度。
     */
    val centerDecorationAlpha: Float,
)

/**
 * 日志查看器尺寸。
 */
data class LogViewerMetrics(
    /**
     * 内嵌模式标题栏高度。
     */
    val inlineHeaderHeight: Dp,

    /**
     * 内嵌模式标题栏水平内边距。
     */
    val inlineHeaderHorizontalPadding: Dp,

    /**
     * 内嵌模式标题栏内容间距。
     */
    val inlineHeaderContentSpacing: Dp,

    /**
     * 独立模式标题栏水平内边距。
     */
    val standaloneHeaderHorizontalPadding: Dp,

    /**
     * 独立模式标题栏垂直内边距。
     */
    val headerVerticalPadding: Dp,

    /**
     * 独立模式标题栏内容间距。
     */
    val headerContentSpacing: Dp,

    /**
     * 加载指示器尺寸。
     */
    val loadingSize: Dp,

    /**
     * 内嵌模式内容内边距。
     */
    val inlineContentPadding: Dp,

    /**
     * 独立模式内容内边距。
     */
    val standaloneContentPadding: Dp,

    /**
     * 日志列表圆角。
     */
    val listRadius: Dp,

    /**
     * 日志列表边框宽度。
     */
    val listBorderWidth: Dp,

    /**
     * 日志记录水平内边距。
     */
    val recordHorizontalPadding: Dp,

    /**
     * 日志记录垂直内边距。
     */
    val recordVerticalPadding: Dp,

    /**
     * 日志记录内容间距。
     */
    val recordContentSpacing: Dp,

    /**
     * 日志元信息顶部内边距。
     */
    val metadataTopPadding: Dp,
)

/**
 * 主页面尺寸。
 */
data class MainPageMetrics(
    /**
     * 顶部应用栏高度。
     */
    val appBarHeight: Dp,

    /**
     * 顶部应用栏水平内边距。
     */
    val appBarHorizontalPadding: Dp,

    /**
     * 搜索框最大宽度。
     */
    val searchMaximumWidth: Dp,

    /**
     * 顶部应用栏操作间距。
     */
    val appBarActionSpacing: Dp,

    /**
     * 页面加载指示器尺寸。
     */
    val loadingSize: Dp,

    /**
     * 搜索结果面板起始内边距。
     */
    val searchPanelStartPadding: Dp,

    /**
     * 搜索结果面板顶部内边距。
     */
    val searchPanelTopPadding: Dp,

    /**
     * 搜索结果面板结束内边距。
     */
    val searchPanelEndPadding: Dp,

    /**
     * 搜索结果面板底部内边距。
     */
    val searchPanelBottomPadding: Dp,

    /**
     * 搜索结果项高度。
     */
    val searchResultHeight: Dp,

    /**
     * 搜索结果项水平内边距。
     */
    val searchResultHorizontalPadding: Dp,

    /**
     * 搜索结果项内容间距。
     */
    val searchResultContentSpacing: Dp,

    /**
     * 搜索结果图标容器尺寸。
     */
    val searchResultIconContainerSize: Dp,

    /**
     * 搜索结果图标容器圆角。
     */
    val searchResultIconRadius: Dp,

    /**
     * 搜索结果图标尺寸。
     */
    val searchResultIconSize: Dp,
)

/**
 * 弹出菜单、对话框和 Toast 的内部尺寸。
 */
data class OverlayMetrics(
    /**
     * 弹出菜单边框宽度。
     */
    val menuBorderWidth: Dp,

    /**
     * 弹出菜单内边距。
     */
    val menuPadding: Dp,

    /**
     * 菜单分隔线水平内边距。
     */
    val menuDividerHorizontalPadding: Dp,

    /**
     * 菜单分隔线垂直内边距。
     */
    val menuDividerVerticalPadding: Dp,

    /**
     * 菜单分隔线高度。
     */
    val menuDividerHeight: Dp,

    /**
     * 菜单项水平内边距。
     */
    val menuItemHorizontalPadding: Dp,

    /**
     * 菜单项内容间距。
     */
    val menuItemContentSpacing: Dp,

    /**
     * 菜单项图标尺寸。
     */
    val menuItemIconSize: Dp,

    /**
     * 对话框标题区水平内边距。
     */
    val dialogHeaderHorizontalPadding: Dp,

    /**
     * 对话框标题区顶部内边距。
     */
    val dialogHeaderTopPadding: Dp,

    /**
     * 对话框标题区底部内边距。
     */
    val dialogHeaderBottomPadding: Dp,

    /**
     * 对话框标题区内容间距。
     */
    val dialogHeaderContentSpacing: Dp,

    /**
     * 对话框标题图标尺寸。
     */
    val dialogTitleIconSize: Dp,

    /**
     * 对话框正文水平内边距。
     */
    val dialogBodyHorizontalPadding: Dp,

    /**
     * 对话框正文顶部内边距。
     */
    val dialogBodyTopPadding: Dp,

    /**
     * 对话框正文底部内边距。
     */
    val dialogBodyBottomPadding: Dp,

    /**
     * 对话框操作区水平内边距。
     */
    val dialogFooterHorizontalPadding: Dp,

    /**
     * 对话框操作区底部内边距。
     */
    val dialogFooterBottomPadding: Dp,

    /**
     * 对话框操作按钮间距。
     */
    val dialogActionSpacing: Dp,

    /**
     * Toast 外边距。
     */
    val toastOuterPadding: Dp,

    /**
     * Toast 水平内边距。
     */
    val toastHorizontalPadding: Dp,

    /**
     * Toast 垂直内边距。
     */
    val toastVerticalPadding: Dp,

    /**
     * Toast 内容间距。
     */
    val toastContentSpacing: Dp,

    /**
     * Toast 图标尺寸。
     */
    val toastIconSize: Dp,
)

/**
 * 创建登录页默认尺寸。
 */
internal fun createLoginMetrics() = LoginMetrics(
    // 顶部装饰横向偏移。
    topDecorationOffsetX = 80.dp,
    // 顶部装饰纵向偏移。
    topDecorationOffsetY = (-100).dp,
    // 顶部装饰尺寸。
    topDecorationSize = 400.dp,
    // 底部装饰横向偏移。
    bottomDecorationOffsetX = (-80).dp,
    // 底部装饰纵向偏移。
    bottomDecorationOffsetY = 60.dp,
    // 底部装饰尺寸。
    bottomDecorationSize = 300.dp,
    // 中部装饰尺寸。
    centerDecorationSize = 200.dp,
    // 登录卡片宽度。
    cardWidth = 480.dp,
    // 登录卡片阴影高度。
    cardShadowElevation = 24.dp,
    // 登录卡片圆角。
    cardRadius = 12.dp,
    // 登录卡片水平内边距。
    cardHorizontalPadding = 24.dp,
    // 登录卡片垂直内边距。
    cardVerticalPadding = 32.dp,
    // Logo 与标题间距。
    logoTitleSpacing = 12.dp,
    // 标题与副标题间距。
    subtitleSpacing = 4.dp,
    // 标题强调线宽度。
    accentWidth = 40.dp,
    // 标题强调线高度。
    accentHeight = 2.dp,
    // 标题强调线圆角。
    accentRadius = 1.dp,
    // 标题强调线底部间距。
    accentBottomSpacing = 12.dp,
    // 登录消息间距。
    messageSpacing = 12.dp,
    // 登录内容底部间距。
    contentBottomSpacing = 24.dp,
    // 授权状态栏高度。
    authorizingHeight = 40.dp,
    // 授权状态栏圆角。
    authorizingRadius = 8.dp,
    // 授权状态栏水平内边距。
    authorizingHorizontalPadding = 16.dp,
    // 授权状态栏内容间距。
    authorizingContentSpacing = 8.dp,
    // 授权旋转图标尺寸。
    authorizingSpinnerSize = 16.dp,
    // 授权旋转图标线宽。
    authorizingSpinnerStroke = 2.dp,
    // 错误提示与操作间距。
    errorActionSpacing = 8.dp,
    // 登录按钮高度。
    loginButtonHeight = 46.dp,
    // 页脚顶部间距。
    footerSpacing = 12.dp,
    // 顶部装饰透明度。
    topDecorationAlpha = 0.06f,
    // 底部装饰透明度。
    bottomDecorationAlpha = 0.06f,
    // 中部装饰透明度。
    centerDecorationAlpha = 0.05f,
)

/**
 * 创建日志查看器默认尺寸。
 */
internal fun createLogViewerMetrics() = LogViewerMetrics(
    // 内嵌模式标题栏高度。
    inlineHeaderHeight = 56.dp,
    // 内嵌模式标题栏水平内边距。
    inlineHeaderHorizontalPadding = 16.dp,
    // 内嵌模式标题栏内容间距。
    inlineHeaderContentSpacing = 8.dp,
    // 独立模式标题栏水平内边距。
    standaloneHeaderHorizontalPadding = 20.dp,
    // 独立模式标题栏垂直内边距。
    headerVerticalPadding = 14.dp,
    // 独立模式标题栏内容间距。
    headerContentSpacing = 8.dp,
    // 加载指示器尺寸。
    loadingSize = 24.dp,
    // 内嵌模式内容内边距。
    inlineContentPadding = 0.dp,
    // 独立模式内容内边距。
    standaloneContentPadding = 20.dp,
    // 日志列表圆角。
    listRadius = 10.dp,
    // 日志列表边框宽度。
    listBorderWidth = 0.5.dp,
    // 日志记录水平内边距。
    recordHorizontalPadding = 16.dp,
    // 日志记录垂直内边距。
    recordVerticalPadding = 12.dp,
    // 日志记录内容间距。
    recordContentSpacing = 12.dp,
    // 日志元信息顶部内边距。
    metadataTopPadding = 3.dp,
)

/**
 * 创建主页面默认尺寸。
 */
internal fun createMainPageMetrics() = MainPageMetrics(
    // 顶部应用栏高度。
    appBarHeight = 64.dp,
    // 顶部应用栏水平内边距。
    appBarHorizontalPadding = 20.dp,
    // 搜索框最大宽度。
    searchMaximumWidth = 420.dp,
    // 顶部应用栏操作间距。
    appBarActionSpacing = 8.dp,
    // 页面加载指示器尺寸。
    loadingSize = 24.dp,
    // 搜索结果面板起始内边距。
    searchPanelStartPadding = 12.dp,
    // 搜索结果面板顶部内边距。
    searchPanelTopPadding = 14.dp,
    // 搜索结果面板结束内边距。
    searchPanelEndPadding = 12.dp,
    // 搜索结果面板底部内边距。
    searchPanelBottomPadding = 10.dp,
    // 搜索结果项高度。
    searchResultHeight = 56.dp,
    // 搜索结果项水平内边距。
    searchResultHorizontalPadding = 12.dp,
    // 搜索结果项内容间距。
    searchResultContentSpacing = 12.dp,
    // 搜索结果图标容器尺寸。
    searchResultIconContainerSize = 32.dp,
    // 搜索结果图标容器圆角。
    searchResultIconRadius = 6.dp,
    // 搜索结果图标尺寸。
    searchResultIconSize = 18.dp,
)

/**
 * 创建浮层组件默认尺寸。
 */
internal fun createOverlayMetrics() = OverlayMetrics(
    // 弹出菜单边框宽度。
    menuBorderWidth = 0.5.dp,
    // 弹出菜单内边距。
    menuPadding = 6.dp,
    // 菜单分隔线水平内边距。
    menuDividerHorizontalPadding = 8.dp,
    // 菜单分隔线垂直内边距。
    menuDividerVerticalPadding = 4.dp,
    // 菜单分隔线高度。
    menuDividerHeight = 0.5.dp,
    // 菜单项水平内边距。
    menuItemHorizontalPadding = 12.dp,
    // 菜单项内容间距。
    menuItemContentSpacing = 10.dp,
    // 菜单项图标尺寸。
    menuItemIconSize = 16.dp,
    // 对话框标题区水平内边距。
    dialogHeaderHorizontalPadding = 24.dp,
    // 对话框标题区顶部内边距。
    dialogHeaderTopPadding = 24.dp,
    // 对话框标题区底部内边距。
    dialogHeaderBottomPadding = 8.dp,
    // 对话框标题区内容间距。
    dialogHeaderContentSpacing = 12.dp,
    // 对话框标题图标尺寸。
    dialogTitleIconSize = 20.dp,
    // 对话框正文水平内边距。
    dialogBodyHorizontalPadding = 24.dp,
    // 对话框正文顶部内边距。
    dialogBodyTopPadding = 8.dp,
    // 对话框正文底部内边距。
    dialogBodyBottomPadding = 20.dp,
    // 对话框操作区水平内边距。
    dialogFooterHorizontalPadding = 24.dp,
    // 对话框操作区底部内边距。
    dialogFooterBottomPadding = 20.dp,
    // 对话框操作按钮间距。
    dialogActionSpacing = 10.dp,
    // Toast 外边距。
    toastOuterPadding = 48.dp,
    // Toast 水平内边距。
    toastHorizontalPadding = 18.dp,
    // Toast 垂直内边距。
    toastVerticalPadding = 10.dp,
    // Toast 内容间距。
    toastContentSpacing = 8.dp,
    // Toast 图标尺寸。
    toastIconSize = 16.dp,
)
