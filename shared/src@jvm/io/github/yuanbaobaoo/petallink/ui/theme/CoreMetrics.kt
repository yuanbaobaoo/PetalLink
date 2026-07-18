package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.unit.Dp

/**
 * 按 UI 区域组织的尺寸 token 集合。
 */
data class PetalMetrics(
    /**
     * 按钮尺寸。
     */
    val button: ButtonMetrics,

    /**
     * 菜单尺寸。
     */
    val menu: MenuMetrics,

    /**
     * 表单尺寸。
     */
    val form: FormMetrics,

    /**
     * 导航尺寸。
     */
    val navigation: NavigationMetrics,

    /**
     * 提示组件尺寸。
     */
    val feedback: FeedbackMetrics,

    /**
     * 对话框尺寸。
     */
    val dialog: DialogMetrics,

    /**
     * 文件列表尺寸。
     */
    val fileList: FileListMetrics,

    /**
     * 基础品牌组件尺寸。
     */
    val basic: BasicMetrics,

    /**
     * 图标尺寸。
     */
    val icon: IconMetrics,

    /**
     * 分隔线尺寸。
     */
    val divider: DividerMetrics,

    /**
     * 组件目录尺寸。
     */
    val catalog: CatalogMetrics,

    /**
     * 同步状态栏尺寸。
     */
    val statusBar: StatusBarMetrics,

    /**
     * 同步设置横幅尺寸。
     */
    val syncSetup: SyncSetupMetrics,

    /**
     * 登录页尺寸。
     */
    val login: LoginMetrics,

    /**
     * 日志查看器尺寸。
     */
    val logViewer: LogViewerMetrics,

    /**
     * 主页面尺寸。
     */
    val mainPage: MainPageMetrics,

    /**
     * 浮层组件尺寸。
     */
    val overlay: OverlayMetrics,

    /**
     * 设置页尺寸。
     */
    val settings: SettingsMetrics,

    /**
     * 更新弹窗尺寸。
     */
    val updateDialog: UpdateDialogMetrics,

    /**
     * 侧边栏尺寸。
     */
    val sidebar: SidebarMetrics,

    /**
     * 传输弹窗尺寸。
     */
    val transferPopover: TransferPopoverMetrics,
)

/**
 * 按钮各变体的独立尺寸。
 */
data class ButtonMetrics(
    /**
     * 主按钮高度。
     */
    val primaryHeight: Dp,

    /**
     * 软色按钮高度。
     */
    val softHeight: Dp,

    /**
     * 文字按钮高度。
     */
    val textHeight: Dp,

    /**
     * 图标文字按钮高度。
     */
    val iconTextHeight: Dp,

    /**
     * 图标按钮尺寸。
     */
    val iconButtonSize: Dp,

    /**
     * 主按钮圆角。
     */
    val primaryRadius: Dp,

    /**
     * 软色按钮圆角。
     */
    val softRadius: Dp,

    /**
     * 文字按钮圆角。
     */
    val textRadius: Dp,

    /**
     * 图标文字按钮圆角。
     */
    val iconTextRadius: Dp,

    /**
     * 图标按钮图标尺寸。
     */
    val iconVariantIconSize: Dp,

    /**
     * 图标文字按钮图标尺寸。
     */
    val iconTextVariantIconSize: Dp,

    /**
     * 软色按钮图标尺寸。
     */
    val softVariantIconSize: Dp,

    /**
     * 主按钮图标尺寸。
     */
    val primaryVariantIconSize: Dp,

    /**
     * 文字按钮图标尺寸。
     */
    val textVariantIconSize: Dp,

    /**
     * 图标文字按钮水平内边距。
     */
    val iconTextHorizontalPadding: Dp,

    /**
     * 文字按钮水平内边距。
     */
    val textHorizontalPadding: Dp,

    /**
     * 软色按钮水平内边距。
     */
    val softHorizontalPadding: Dp,

    /**
     * 主按钮水平内边距。
     */
    val primaryHorizontalPadding: Dp,

    /**
     * 主按钮阴影高度。
     */
    val primaryShadowElevation: Dp,

    /**
     * 加载指示器尺寸。
     */
    val loadingSpinnerSize: Dp,

    /**
     * 加载指示器与文字间距。
     */
    val loadingLabelSpacing: Dp,

    /**
     * 图标与文字间距。
     */
    val iconLabelSpacing: Dp,

    /**
     * 角标起始内边距。
     */
    val badgeStartPadding: Dp,

    /**
     * 角标水平内边距。
     */
    val badgeHorizontalPadding: Dp,

    /**
     * 角标垂直内边距。
     */
    val badgeVerticalPadding: Dp,

    /**
     * 角标高度。
     */
    val badgeHeight: Dp,

    /**
     * 主按钮无图标时的占位尺寸。
     */
    val primaryWithoutIconSize: Dp,

    /**
     * 文字按钮无图标时的占位尺寸。
     */
    val textWithoutIconSize: Dp,

    /**
     * 图标按钮水平内边距。
     */
    val iconHorizontalPadding: Dp,

    /**
     * 危险按钮按下背景透明度。
     */
    val dangerPressedAlpha: Float,

    /**
     * 主按钮阴影透明度。
     */
    val primaryShadowAlpha: Float,

    /**
     * 禁用按钮整体透明度。
     */
    val disabledAlpha: Float,

    /**
     * 按钮加载指示器轨道透明度。
     */
    val spinnerTrackAlpha: Float,

    /**
     * 按钮加载指示器旋转一周时长。
     */
    val spinnerRotationDurationMillis: Int,
)

/**
 * 弹出菜单尺寸。
 */
data class MenuMetrics(
    /**
     * 默认菜单宽度。
     */
    val defaultWidth: Dp,

    /**
     * 菜单容器圆角。
     */
    val containerRadius: Dp,

    /**
     * 菜单项高度。
     */
    val itemHeight: Dp,

    /**
     * 菜单项圆角。
     */
    val itemRadius: Dp,
)

/**
 * 表单控件尺寸。
 */
data class FormMetrics(
    /**
     * 文本框高度。
     */
    val textFieldHeight: Dp,

    /**
     * 文本框圆角。
     */
    val textFieldRadius: Dp,

    /**
     * 数值框高度。
     */
    val numberFieldHeight: Dp,

    /**
     * 数值框圆角。
     */
    val numberFieldRadius: Dp,

    /**
     * 搜索框高度。
     */
    val searchFieldHeight: Dp,

    /**
     * 步进器高度。
     */
    val stepperHeight: Dp,

    /**
     * 表单控件内部尺寸。
     */
    val controls: FormControlMetrics,
)

/**
 * 导航控件尺寸。
 */
data class NavigationMetrics(
    /**
     * 侧边栏项高度。
     */
    val sidebarItemHeight: Dp,

    /**
     * 侧边栏项圆角。
     */
    val sidebarItemRadius: Dp,

    /**
     * 面包屑高度。
     */
    val breadcrumbHeight: Dp,

    /**
     * 面包屑水平内边距。
     */
    val breadcrumbHorizontalPadding: Dp,

    /**
     * 面包屑项间距。
     */
    val breadcrumbItemSpacing: Dp,
)

/**
 * 提示类组件尺寸。
 */
data class FeedbackMetrics(
    /**
     * 横幅圆角。
     */
    val bannerRadius: Dp,

    /**
     * 小标签圆角。
     */
    val smallTagRadius: Dp,

    /**
     * 中标签圆角。
     */
    val mediumTagRadius: Dp,

    /**
     * 空状态徽章尺寸。
     */
    val emptyBadgeSize: Dp,

    /**
     * 空状态徽章圆角。
     */
    val emptyBadgeRadius: Dp,

    /**
     * 反馈组件内部尺寸。
     */
    val controls: FeedbackControlMetrics,
)

/**
 * 对话框和 Toast 尺寸。
 */
data class DialogMetrics(
    /**
     * 对话框圆角。
     */
    val containerRadius: Dp,

    /**
     * 标题图标徽章尺寸。
     */
    val iconBadgeSize: Dp,

    /**
     * 标题图标徽章圆角。
     */
    val iconBadgeRadius: Dp,

    /**
     * Toast 圆角。
     */
    val toastRadius: Dp,
)

/**
 * 文件列表专属尺寸。重命名和移动对话框即使当前视觉数值一致，也分别保留变量，
 * 修改其中一个不会意外改变另一个。
 */
data class FileListMetrics(
    /**
     * 重命名弹窗宽度。
     */
    val renameDialogWidth: Dp,

    /**
     * 重命名弹窗圆角。
     */
    val renameDialogRadius: Dp,

    /**
     * 重命名弹窗内边距。
     */
    val renameDialogPadding: Dp,

    /**
     * 重命名弹窗内容间距。
     */
    val renameDialogContentSpacing: Dp,

    /**
     * 重命名弹窗操作间距。
     */
    val renameDialogActionSpacing: Dp,

    /**
     * 移动弹窗宽度。
     */
    val moveDialogWidth: Dp,

    /**
     * 移动弹窗圆角。
     */
    val moveDialogRadius: Dp,

    /**
     * 移动弹窗内边距。
     */
    val moveDialogPadding: Dp,

    /**
     * 移动弹窗内容间距。
     */
    val moveDialogContentSpacing: Dp,

    /**
     * 目标目录列表高度。
     */
    val moveDialogFolderListHeight: Dp,

    /**
     * 目标目录项圆角。
     */
    val moveDialogFolderRadius: Dp,

    /**
     * 目标目录项内边距。
     */
    val moveDialogFolderPadding: Dp,

    /**
     * 目标目录项内容间距。
     */
    val moveDialogFolderContentSpacing: Dp,

    /**
     * 目标目录图标尺寸。
     */
    val moveDialogFolderIconSize: Dp,

    /**
     * 移动弹窗操作间距。
     */
    val moveDialogActionSpacing: Dp,

    /**
     * 文件列表内部尺寸。
     */
    val controls: FileListControlMetrics,
)
