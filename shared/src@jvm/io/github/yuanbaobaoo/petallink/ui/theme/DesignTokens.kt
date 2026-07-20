package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * 构造默认字体 token，未指定的排版属性保持 Compose 默认值。
 */
private fun textStyle(
    size: Float,
    weight: FontWeight = FontWeight.Normal,
    lineHeight: Float? = null,
    letterSpacing: Float? = null,
): TextStyle = TextStyle(
    fontSize = size.sp,
    fontWeight = weight,
    lineHeight = lineHeight?.sp ?: TextUnit.Unspecified,
    letterSpacing = letterSpacing?.sp ?: TextUnit.Unspecified,
)

/**
 * PetalLink 默认设计 token。
 *
 * 所有字段按具体 UI 职责命名；即使默认值相同，也不得合并不同职责的字段。
 */
object DesignTokens {
    // ------ 品牌色 ------
    // 品牌主色。
    internal val BRAND = Color(0xFF0053DB)
    // 品牌控件悬停色。
    internal val BRAND_HOVER = Color(0xFF4A8BF0)
    // 品牌控件按下色。
    internal val BRAND_ACTIVE = Color(0xFF0047B8)
    // 品牌浅色强调。
    internal val BRAND_LIGHT = Color(0xFFB7D0F7)
    // 品牌 100 色阶背景。
    internal val BRAND_100 = Color(0xFFDCE8FC)
    // 品牌最浅背景。
    internal val BRAND_LIGHTER = Color(0xFFEFF4FE)
    // 品牌主渐变。
    internal val BRAND_GRADIENT: Brush = Brush.linearGradient(listOf(BRAND_HOVER, BRAND))
    // 品牌浅色渐变。
    internal val BRAND_GRADIENT_SOFT: Brush = Brush.linearGradient(listOf(BRAND_LIGHTER, BRAND_100))

    // ------ 功能色 ------
    // 成功状态前景色。
    internal val SUCCESS = Color(0xFF0CA678)
    // 浅色主题成功状态背景色。
    internal val SUCCESS_BACKGROUND = Color(0xFFE3F5EE)
    // 警告状态前景色。
    internal val WARNING = Color(0xFFF08C00)
    // 浅色主题警告状态背景色。
    internal val WARNING_BACKGROUND = Color(0xFFFFF3DE)
    // 错误状态前景色。
    internal val ERROR = Color(0xFFE5484D)
    // 浅色主题错误状态背景色。
    internal val ERROR_BACKGROUND = Color(0xFFFDECEC)
    // 信息状态前景色。
    internal val INFO = Color(0xFF3B82F6)
    // 浅色主题信息状态背景色。
    internal val INFO_BACKGROUND = Color(0xFFE8F0FE)

    // ------ 文件类型色 ------
    // 文件夹图标前景色。
    internal val FOLDER = Color(0xFFF0A63C)
    // 浅色主题文件夹图标背景色。
    internal val FOLDER_BACKGROUND = Color(0xFFFFF4DE)
    // 文档图标前景色。
    internal val DOCUMENT = Color(0xFF6366F1)
    // 浅色主题文档图标背景色。
    internal val DOCUMENT_BACKGROUND = Color(0xFFEEF2FF)
    // 图片图标前景色。
    internal val IMAGE = Color(0xFFEC4899)
    // 浅色主题图片图标背景色。
    internal val IMAGE_BACKGROUND = Color(0xFFFDE7F3)
    // 视频图标前景色。
    internal val VIDEO = Color(0xFF8B5CF6)
    // 浅色主题视频图标背景色。
    internal val VIDEO_BACKGROUND = Color(0xFFF3E8FF)
    // 表格图标前景色。
    internal val SHEET = Color(0xFF10B981)
    // 浅色主题表格图标背景色。
    internal val SHEET_BACKGROUND = Color(0xFFE6F7EE)

    // ------ 控件色 ------
    // 浅色主题关闭状态开关轨道色。
    internal val SWITCH_OFF_TRACK = Color(0xFFE3E3E6)

    // ------ 浅色主题 ------
    // 浅色主题页面背景色。
    internal val LIGHT_BG_PAGE = Color(0xFFF5F5F7)
    // 浅色主题容器背景色。
    internal val LIGHT_BG_CONTAINER = Color(0xFFFFFFFF)
    // 浅色主题填充背景色。
    internal val LIGHT_BG_FILL = Color(0xFFF1F1F3)
    // 浅色主题悬停背景色。
    internal val LIGHT_BG_HOVER = Color(0xFFF7F7F9)
    // 浅色主题激活背景色。
    internal val LIGHT_BG_ACTIVE = Color(0xFFECECEF)
    // 浅色主题默认边框色。
    internal val LIGHT_BORDER = Color(0x0F000000)
    // 浅色主题悬停边框色。
    internal val LIGHT_BORDER_HOVER = Color(0x1A000000)
    // 浅色主题主要文字色。
    internal val LIGHT_TEXT_PRIMARY = Color(0xE6000000)
    // 浅色主题次要文字色。
    internal val LIGHT_TEXT_SECONDARY = Color(0x99000000)
    // 浅色主题占位文字色。
    internal val LIGHT_TEXT_PLACEHOLDER = Color(0x59000000)

    // ------ 深色主题 ------
    // 深色主题页面背景色。
    internal val DARK_BG_PAGE = Color(0xFF181818)
    // 深色主题容器背景色。
    internal val DARK_BG_CONTAINER = Color(0xFF242424)
    // 深色主题填充背景色。
    internal val DARK_BG_FILL = Color(0xFF2C2C2C)
    // 深色主题悬停背景色。
    internal val DARK_BG_HOVER = Color(0xFF2C2C2C)
    // 深色主题激活背景色。
    internal val DARK_BG_ACTIVE = Color(0xFF333333)
    // 深色主题默认边框色。
    internal val DARK_BORDER = Color(0x14FFFFFF)
    // 深色主题悬停边框色。
    internal val DARK_BORDER_HOVER = Color(0x29FFFFFF)
    // 深色主题主要文字色。
    internal val DARK_TEXT_PRIMARY = Color(0xE6FFFFFF)
    // 深色主题次要文字色。
    internal val DARK_TEXT_SECONDARY = Color(0x99FFFFFF)
    // 深色主题占位文字色。
    internal val DARK_TEXT_PLACEHOLDER = Color(0x59FFFFFF)
    // 深色主题品牌浅色强调。
    internal val DARK_BRAND_LIGHT = Color(0xFF1A3A8A)
    // 深色主题品牌 100 色阶背景。
    internal val DARK_BRAND_100 = Color(0xFF233A66)
    // 深色主题品牌最浅背景。
    internal val DARK_BRAND_LIGHTER = Color(0xFF1F2A4A)
    // 深色主题成功状态背景色。
    internal val DARK_SUCCESS_BACKGROUND = Color(0xFF173A31)
    // 深色主题警告状态背景色。
    internal val DARK_WARNING_BACKGROUND = Color(0xFF3D2C12)
    // 深色主题错误状态背景色。
    internal val DARK_ERROR_BACKGROUND = Color(0xFF432326)
    // 深色主题信息状态背景色。
    internal val DARK_INFO_BACKGROUND = Color(0xFF1E2F4F)
    // 深色主题文件夹图标背景色。
    internal val DARK_FOLDER_BACKGROUND = Color(0xFF3D301A)
    // 深色主题文档图标背景色。
    internal val DARK_DOCUMENT_BACKGROUND = Color(0xFF292B50)
    // 深色主题图片图标背景色。
    internal val DARK_IMAGE_BACKGROUND = Color(0xFF4A2239)
    // 深色主题视频图标背景色。
    internal val DARK_VIDEO_BACKGROUND = Color(0xFF34264D)
    // 深色主题表格图标背景色。
    internal val DARK_SHEET_BACKGROUND = Color(0xFF183C31)
    // 深色主题关闭状态开关轨道色。
    internal val DARK_SWITCH_OFF_TRACK = Color(0xFF4A4A4D)

    // ------ 固定前景色 ------
    // Material 主色上的固定内容色。
    internal val ON_PRIMARY = Color.White
    // 紧凑 Logo 的固定文字色。
    internal val APP_LOGO_COMPACT_TEXT = Color(0xFF1C1C1E)
    // 完整 Logo 的固定文字色。
    internal val APP_LOGO_FULL_TEXT = Color(0xFF181818)
    // 图标未指定颜色时的默认色。
    internal val DEFAULT_ICON_TINT = Color(0xFF181818)
    // 文件列表批量操作栏背景色。
    internal val FILE_LIST_BULK_BACKGROUND = Color(0xF01C1C1E)
    // 文件列表批量危险操作文字色。
    internal val FILE_LIST_BULK_DANGER_TEXT = Color(0xFFFDA4AF)
    // 文件列表批量危险操作图标色。
    internal val FILE_LIST_BULK_DANGER_ICON = Color(0xFFFDA4AF)
    // 文件列表批量危险操作悬停背景色。
    internal val FILE_LIST_BULK_DANGER_HOVER_BACKGROUND = Color(0x2EFDA4AF)
    // 成功 Toast 图标色。
    internal val TOAST_SUCCESS_ICON = Color(0xFF4ADE80)
    // 错误 Toast 图标色。
    internal val TOAST_ERROR_ICON = Color(0xFFFB7185)
    // Toast 固定背景色。
    internal val TOAST_BACKGROUND = Color(0xEB1C1C1E)

    // ------ 默认字体 ------
    /**
     * 默认字体 token；每个字段只控制对应的 UI 职责。
     */
    val TYPOGRAPHY = PetalTypography(
        // 品牌标识字体。
        brand = BrandTypography(
            // 紧凑 Logo 文字。
            compactLogoLabel = textStyle(14f, FontWeight.SemiBold),
            // 完整 Logo 文字。
            fullLogoLabel = textStyle(14f, FontWeight.SemiBold),
        ),
        // 按钮字体。
        button = ButtonTypography(
            // 主按钮文字。
            primaryLabel = textStyle(12f, FontWeight.Medium),
            // 软色按钮文字。
            softLabel = textStyle(11f, FontWeight.Medium),
            // 文字按钮文字。
            textLabel = textStyle(12f, FontWeight.Medium),
            // 图标文字按钮文字。
            iconTextLabel = textStyle(12f, FontWeight.Medium),
            // 按钮角标文字。
            badgeLabel = textStyle(10f, FontWeight.SemiBold),
        ),
        // 菜单字体。
        menu = MenuTypography(
            // 菜单项文字。
            itemLabel = textStyle(13f),
        ),
        // 表单字体。
        form = FormTypography(
            // 文本框输入文字。
            textFieldInput = textStyle(13f),
            // 文本框占位文字。
            textFieldPlaceholder = textStyle(13f),
            // 数值框输入文字。
            numberFieldInput = textStyle(13f),
            // 数值框单位文字。
            numberFieldSuffix = textStyle(12f),
            // 步进器数值文字。
            stepperValue = textStyle(13f, FontWeight.Medium),
            // 步进器操作文字。
            stepperAction = textStyle(16f, FontWeight.Medium),
        ),
        // 提示和导航字体。
        feedback = FeedbackTypography(
            // 横幅标题。
            bannerTitle = textStyle(13f, FontWeight.SemiBold),
            // 横幅正文。
            bannerMessage = textStyle(13f, lineHeight = 21.7f),
            // 小标签文字。
            smallTagLabel = textStyle(12f, FontWeight.Medium),
            // 中标签文字。
            mediumTagLabel = textStyle(13f, FontWeight.Medium),
            // 空状态标题。
            emptyStateTitle = textStyle(15f, FontWeight.SemiBold),
            // 空状态说明。
            emptyStateDescription = textStyle(13f, lineHeight = 21f),
            // 统计数量。
            statChipCount = textStyle(12f, FontWeight.Medium),
            // 统计标签。
            statChipLabel = textStyle(12f, FontWeight.Medium),
            // 区块标题。
            sectionHeader = textStyle(18f, FontWeight.SemiBold),
            // 导航项文字。
            navigationItem = textStyle(14f),
            // 活动导航项文字。
            activeNavigationItem = textStyle(14f, FontWeight.Medium),
            // 导航分组文字。
            navigationGroupLabel = textStyle(12f, FontWeight.SemiBold),
        ),
        // 对话框字体。
        dialog = DialogTypography(
            // 对话框标题。
            title = textStyle(17f, FontWeight.SemiBold),
            // 对话框正文。
            body = textStyle(14f, lineHeight = 24.75f),
            // Toast 消息。
            toastMessage = textStyle(13f, FontWeight.Medium),
        ),
        // 侧边栏字体。
        sidebar = SidebarTypography(
            // 区块标签。
            sectionLabel = textStyle(11f, FontWeight.SemiBold, letterSpacing = 0.4f),
            // 账号头像文字。
            accountAvatar = textStyle(14f, FontWeight.SemiBold),
            // 账号名称。
            accountName = textStyle(14f, FontWeight.SemiBold),
            // 账号邮箱。
            accountEmail = textStyle(12f),
            // 配额说明。
            quotaDescription = textStyle(11.5f),
            // 更新下载提示。
            downloadUpdateLabel = textStyle(13f, FontWeight.SemiBold),
            // 更新下载进度。
            downloadUpdateProgress = textStyle(13f, FontWeight.Bold),
            // 可用更新提示。
            availableUpdateLabel = textStyle(13f, FontWeight.SemiBold),
            // 关闭更新操作。
            dismissUpdateAction = textStyle(13f),
            // 安装更新操作。
            installUpdateAction = textStyle(12f, FontWeight.SemiBold),
            // 目录树节点。
            treeNodeLabel = textStyle(13f),
            // 选中目录树节点。
            selectedTreeNodeLabel = textStyle(13f, FontWeight.Medium),
        ),
        // 面包屑字体。
        breadcrumb = BreadcrumbTypography(
            // 分隔符。
            separator = textStyle(12f),
            // 普通路径项。
            item = textStyle(13f),
            // 当前路径项。
            currentItem = textStyle(13f, FontWeight.SemiBold),
        ),
        // 状态栏字体。
        statusBar = StatusBarTypography(
            // 当前同步状态。
            currentStatus = textStyle(13f, FontWeight.Medium),
            // 最近同步时间。
            lastSyncTime = textStyle(12.5f),
        ),
        // 文件列表字体。
        fileList = FileListTypography(
            // 选中项汇总。
            selectionSummary = textStyle(13f, FontWeight.SemiBold),
            // 状态列标题。
            statusColumnHeader = textStyle(11.5f, FontWeight.SemiBold),
            // 操作列标题。
            actionColumnHeader = textStyle(11.5f, FontWeight.SemiBold),
            // 已加载项汇总。
            loadedSummary = textStyle(12f),
            // 工具栏操作。
            toolbarAction = textStyle(13f, FontWeight.Medium),
            // 通用列标题。
            genericColumnHeader = textStyle(11.5f, FontWeight.SemiBold),
            // 文件名称。
            rowFileName = textStyle(14f),
            // 文件大小。
            rowFileSize = textStyle(13f),
            // 修改时间。
            rowModifiedTime = textStyle(13f),
            // 次要操作。
            secondaryAction = textStyle(13f),
            // 重命名弹窗标题。
            renameDialogTitle = textStyle(16f, FontWeight.SemiBold),
            // 移动弹窗标题。
            moveDialogTitle = textStyle(16f),
            // 移动弹窗说明。
            moveDialogDescription = textStyle(12f),
            // 移动目标目录。
            moveDialogFolder = textStyle(13f),
        ),
        // 设置页字体。
        settings = SettingsTypography(
            // 页面标题。
            pageTitle = textStyle(16f, FontWeight.SemiBold),
            // 数值范围提示。
            numberRangeHint = textStyle(13f),
            // 日志保留说明。
            logRetentionDescription = textStyle(14f, lineHeight = 24f),
            // 校验错误。
            validationError = textStyle(12f),
            // 保存成功提示。
            saveSuccess = textStyle(12f),
            // 设置分组标题。
            groupHeader = textStyle(12f, FontWeight.SemiBold),
            // 设置项标题。
            optionTitle = textStyle(14f, FontWeight.Medium),
            // 设置项说明。
            optionDescription = textStyle(12f),
            // 未挂载标题。
            emptyMountTitle = textStyle(14f, FontWeight.SemiBold),
            // 未挂载说明。
            emptyMountDescription = textStyle(13f),
            // 当前挂载标题。
            currentMountTitle = textStyle(14f, FontWeight.SemiBold),
            // 当前挂载路径。
            currentMountPath = textStyle(12f),
            // 账号头像文字。
            accountAvatar = textStyle(22f, FontWeight.SemiBold),
            // 账号名称。
            accountName = textStyle(16f, FontWeight.SemiBold),
            // 详情标签。
            detailLabel = textStyle(13f),
            // 详情内容。
            detailValue = textStyle(13f),
            // 版本号。
            version = textStyle(12f),
            // 更新状态。
            updateStatus = textStyle(12f),
            // 关于说明。
            aboutDescription = textStyle(12f),
            // 外部链接。
            externalLink = textStyle(13f),
        ),
        // 登录页字体。
        login = LoginTypography(
            // 登录标题。
            title = textStyle(20f, FontWeight.SemiBold, letterSpacing = -0.2f),
            // 授权状态提示。
            authorizingMessage = textStyle(14f, FontWeight.Medium),
            // 页脚提示。
            footerHint = textStyle(12f),
        ),
        // 传输面板字体。
        transfer = TransferTypography(
            // 面板标题。
            panelTitle = textStyle(17f, FontWeight.SemiBold),
            // 汇总数值。
            summaryValue = textStyle(16f, FontWeight.Bold),
            // 汇总标签。
            summaryLabel = textStyle(11f),
            // 任务名称。
            taskName = textStyle(13.5f, FontWeight.Medium),
            // 任务说明。
            taskDescription = textStyle(12f, lineHeight = 18.85f),
            // 任务进度。
            taskProgress = textStyle(11.5f),
            // 删除操作提示。
            deleteOperation = textStyle(12f),
            // 任务状态。
            taskState = textStyle(12f, FontWeight.Medium),
        ),
        // 更新弹窗字体。
        update = UpdateTypography(
            // 弹窗标题。
            dialogTitle = textStyle(17f, FontWeight.SemiBold),
            // 版本号。
            version = textStyle(15f, FontWeight.Bold),
            // 失败提示。
            failureMessage = textStyle(14f),
            // 更新说明标签。
            releaseNotesLabel = textStyle(12f, FontWeight.SemiBold),
            // 更新说明正文。
            releaseNotesBody = textStyle(12f),
            // 无更新说明提示。
            noReleaseNotesMessage = textStyle(14f),
            // 下载进度。
            progress = textStyle(12f, FontWeight.SemiBold),
            // 等待提示。
            waitingMessage = textStyle(14f),
        ),
        // 日志查看器字体。
        logViewer = LogViewerTypography(
            // 页面标题。
            pageTitle = textStyle(16f, FontWeight.SemiBold),
            // 日志消息。
            recordMessage = textStyle(13.5f),
            // 日志元数据。
            recordMetadata = textStyle(11.5f),
        ),
        // 主页面字体。
        main = MainTypography(
            // 搜索区域标题。
            searchHeader = textStyle(12.5f, FontWeight.SemiBold),
            // 搜索结果名称。
            searchResultName = textStyle(14f),
            // 搜索结果说明。
            searchResultDescription = textStyle(12f),
        ),
        // 图标目录字体。
        catalog = CatalogTypography(
            // 图标名称。
            iconName = textStyle(9f),
        ),
    )

    // ------ 默认尺寸 ------
    /**
     * 默认尺寸 token；相同数值的不同职责保持独立字段。
     */
    val METRICS = PetalMetrics(
        // 按钮尺寸。
        button = ButtonMetrics(
            // 主按钮高度。
            primaryHeight = 36.dp,
            // 软色按钮高度。
            softHeight = 36.dp,
            // 文字按钮高度。
            textHeight = 36.dp,
            // 图标文字按钮高度。
            iconTextHeight = 36.dp,
            // 图标按钮尺寸。
            iconButtonSize = 32.dp,
            // 主按钮圆角。
            primaryRadius = 8.dp,
            // 软色按钮圆角。
            softRadius = 8.dp,
            // 文字按钮圆角。
            textRadius = 5.dp,
            // 图标文字按钮圆角。
            iconTextRadius = 8.dp,
            // 图标按钮图标尺寸。
            iconVariantIconSize = 18.dp,
            // 图标文字按钮图标尺寸。
            iconTextVariantIconSize = 16.dp,
            // 软色按钮图标尺寸。
            softVariantIconSize = 16.dp,
            // 主按钮图标尺寸。
            primaryVariantIconSize = 14.dp,
            // 文字按钮图标尺寸。
            textVariantIconSize = 14.dp,
            // 图标文字按钮水平内边距。
            iconTextHorizontalPadding = 14.dp,
            // 文字按钮水平内边距。
            textHorizontalPadding = 8.dp,
            // 软色按钮水平内边距。
            softHorizontalPadding = 16.dp,
            // 主按钮水平内边距。
            primaryHorizontalPadding = 18.dp,
            // 主按钮阴影高度。
            primaryShadowElevation = 6.dp,
            // 加载指示器尺寸。
            loadingSpinnerSize = 16.dp,
            // 加载指示器与文字间距。
            loadingLabelSpacing = 8.dp,
            // 图标与文字间距。
            iconLabelSpacing = 6.dp,
            // 角标起始内边距。
            badgeStartPadding = 2.dp,
            // 角标水平内边距。
            badgeHorizontalPadding = 5.dp,
            // 角标垂直内边距。
            badgeVerticalPadding = 1.dp,
            // 角标高度。
            badgeHeight = 16.dp,
            // 主按钮无图标时的占位尺寸。
            primaryWithoutIconSize = 0.dp,
            // 文字按钮无图标时的占位尺寸。
            textWithoutIconSize = 0.dp,
            // 图标按钮水平内边距。
            iconHorizontalPadding = 0.dp,
            // 危险按钮按下背景透明度。
            dangerPressedAlpha = 0.85f,
            // 主按钮阴影透明度。
            primaryShadowAlpha = 0.35f,
            // 禁用按钮整体透明度。
            disabledAlpha = 0.5f,
            // 按钮加载指示器轨道透明度。
            spinnerTrackAlpha = 0.3f,
            // 按钮加载指示器旋转一周时长。
            spinnerRotationDurationMillis = 800,
        ),
        // 菜单尺寸。
        menu = MenuMetrics(
            // 默认菜单宽度。
            defaultWidth = 168.dp,
            // 菜单容器圆角。
            containerRadius = 10.dp,
            // 菜单项高度。
            itemHeight = 36.dp,
            // 菜单项圆角。
            itemRadius = 8.dp,
        ),
        // 表单尺寸。
        form = FormMetrics(
            // 文本框高度。
            textFieldHeight = 38.dp,
            // 文本框圆角。
            textFieldRadius = 8.dp,
            // 数值框高度。
            numberFieldHeight = 38.dp,
            // 数值框圆角。
            numberFieldRadius = 8.dp,
            // 搜索框高度。
            searchFieldHeight = 38.dp,
            // 步进器高度。
            stepperHeight = 36.dp,
            // 表单控件内部尺寸。
            controls = createFormControlMetrics(),
        ),
        // 导航尺寸。
        navigation = NavigationMetrics(
            // 侧边栏项高度。
            sidebarItemHeight = 46.dp,
            // 侧边栏项圆角。
            sidebarItemRadius = 8.dp,
            // 面包屑高度。
            breadcrumbHeight = 40.dp,
            // 面包屑水平内边距。
            breadcrumbHorizontalPadding = 20.dp,
            // 面包屑项间距。
            breadcrumbItemSpacing = 6.dp,
        ),
        // 提示组件尺寸。
        feedback = FeedbackMetrics(
            // 横幅圆角。
            bannerRadius = 10.dp,
            // 小标签圆角。
            smallTagRadius = 5.dp,
            // 中标签圆角。
            mediumTagRadius = 5.dp,
            // 空状态徽章尺寸。
            emptyBadgeSize = 72.dp,
            // 空状态徽章圆角。
            emptyBadgeRadius = 14.dp,
            // 反馈组件内部尺寸。
            controls = createFeedbackControlMetrics(),
        ),
        // 对话框尺寸。
        dialog = DialogMetrics(
            // 对话框圆角。
            containerRadius = 12.dp,
            // 标题图标徽章尺寸。
            iconBadgeSize = 40.dp,
            // 标题图标徽章圆角。
            iconBadgeRadius = 10.dp,
            // Toast 圆角。
            toastRadius = 10.dp,
        ),
        // 文件列表尺寸。
        fileList = FileListMetrics(
            // 重命名弹窗宽度。
            renameDialogWidth = 420.dp,
            // 重命名弹窗圆角。
            renameDialogRadius = 12.dp,
            // 重命名弹窗内边距。
            renameDialogPadding = 24.dp,
            // 重命名弹窗内容间距。
            renameDialogContentSpacing = 16.dp,
            // 重命名弹窗操作间距。
            renameDialogActionSpacing = 8.dp,
            // 移动弹窗宽度。
            moveDialogWidth = 460.dp,
            // 移动弹窗圆角。
            moveDialogRadius = 12.dp,
            // 移动弹窗内边距。
            moveDialogPadding = 24.dp,
            // 移动弹窗内容间距。
            moveDialogContentSpacing = 16.dp,
            // 目标目录列表高度。
            moveDialogFolderListHeight = 260.dp,
            // 目标目录项圆角。
            moveDialogFolderRadius = 8.dp,
            // 目标目录项内边距。
            moveDialogFolderPadding = 12.dp,
            // 目标目录项内容间距。
            moveDialogFolderContentSpacing = 12.dp,
            // 目标目录图标尺寸。
            moveDialogFolderIconSize = 18.dp,
            // 移动弹窗操作间距。
            moveDialogActionSpacing = 8.dp,
            // 文件列表内部尺寸。
            controls = createFileListControlMetrics(),
        ),
        // 基础品牌组件尺寸。
        basic = createBasicMetrics(),
        // 图标尺寸。
        icon = createIconMetrics(),
        // 分隔线尺寸。
        divider = createDividerMetrics(),
        // 组件目录尺寸。
        catalog = createCatalogMetrics(),
        // 同步状态栏尺寸。
        statusBar = createStatusBarMetrics(),
        // 同步设置横幅尺寸。
        syncSetup = createSyncSetupMetrics(),
        // 登录页尺寸。
        login = createLoginMetrics(),
        // 日志查看器尺寸。
        logViewer = createLogViewerMetrics(),
        // 主页面尺寸。
        mainPage = createMainPageMetrics(),
        // 浮层组件尺寸。
        overlay = createOverlayMetrics(),
        // 设置页尺寸。
        settings = createSettingsMetrics(),
        // 更新弹窗尺寸。
        updateDialog = createUpdateDialogMetrics(),
        // 侧边栏尺寸。
        sidebar = createSidebarMetrics(),
        // 传输弹窗尺寸。
        transferPopover = createTransferPopoverMetrics(),
    )
}
