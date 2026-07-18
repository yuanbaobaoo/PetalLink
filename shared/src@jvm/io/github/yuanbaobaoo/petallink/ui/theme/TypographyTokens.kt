package io.github.yuanbaobaoo.petallink.ui.theme

import androidx.compose.ui.text.TextStyle

/**
 * 按 UI 区域组织的字体 token 集合。
 */
data class PetalTypography(
    /**
     * 品牌标识字体。
     */
    val brand: BrandTypography,

    /**
     * 按钮字体。
     */
    val button: ButtonTypography,

    /**
     * 菜单字体。
     */
    val menu: MenuTypography,

    /**
     * 表单字体。
     */
    val form: FormTypography,

    /**
     * 提示和导航字体。
     */
    val feedback: FeedbackTypography,

    /**
     * 对话框字体。
     */
    val dialog: DialogTypography,

    /**
     * 侧边栏字体。
     */
    val sidebar: SidebarTypography,

    /**
     * 面包屑字体。
     */
    val breadcrumb: BreadcrumbTypography,

    /**
     * 状态栏字体。
     */
    val statusBar: StatusBarTypography,

    /**
     * 文件列表字体。
     */
    val fileList: FileListTypography,

    /**
     * 设置页字体。
     */
    val settings: SettingsTypography,

    /**
     * 登录页字体。
     */
    val login: LoginTypography,

    /**
     * 传输面板字体。
     */
    val transfer: TransferTypography,

    /**
     * 更新弹窗字体。
     */
    val update: UpdateTypography,

    /**
     * 日志查看器字体。
     */
    val logViewer: LogViewerTypography,

    /**
     * 主页面字体。
     */
    val main: MainTypography,

    /**
     * 图标目录字体。
     */
    val catalog: CatalogTypography,
)

/**
 * 品牌标识文字样式。
 */
data class BrandTypography(
    /**
     * 紧凑 Logo 文字。
     */
    val compactLogoLabel: TextStyle,

    /**
     * 完整 Logo 文字。
     */
    val fullLogoLabel: TextStyle,
)

/**
 * 按钮各变体的独立文字样式。
 */
data class ButtonTypography(
    /**
     * 主按钮文字。
     */
    val primaryLabel: TextStyle,

    /**
     * 软色按钮文字。
     */
    val softLabel: TextStyle,

    /**
     * 文字按钮文字。
     */
    val textLabel: TextStyle,

    /**
     * 图标文字按钮文字。
     */
    val iconTextLabel: TextStyle,

    /**
     * 按钮角标文字。
     */
    val badgeLabel: TextStyle,
)

/**
 * 弹出菜单文字样式。
 */
data class MenuTypography(
    /**
     * 菜单项文字。
     */
    val itemLabel: TextStyle,
)

/**
 * 表单控件文字样式。
 */
data class FormTypography(
    /**
     * 文本框输入文字。
     */
    val textFieldInput: TextStyle,

    /**
     * 文本框占位文字。
     */
    val textFieldPlaceholder: TextStyle,

    /**
     * 数值框输入文字。
     */
    val numberFieldInput: TextStyle,

    /**
     * 数值框单位文字。
     */
    val numberFieldSuffix: TextStyle,

    /**
     * 步进器数值文字。
     */
    val stepperValue: TextStyle,

    /**
     * 步进器操作文字。
     */
    val stepperAction: TextStyle,
)

/**
 * 提示、标签和导航反馈文字样式。
 */
data class FeedbackTypography(
    /**
     * 横幅标题。
     */
    val bannerTitle: TextStyle,

    /**
     * 横幅正文。
     */
    val bannerMessage: TextStyle,

    /**
     * 小标签文字。
     */
    val smallTagLabel: TextStyle,

    /**
     * 中标签文字。
     */
    val mediumTagLabel: TextStyle,

    /**
     * 空状态标题。
     */
    val emptyStateTitle: TextStyle,

    /**
     * 空状态说明。
     */
    val emptyStateDescription: TextStyle,

    /**
     * 统计数量。
     */
    val statChipCount: TextStyle,

    /**
     * 统计标签。
     */
    val statChipLabel: TextStyle,

    /**
     * 区块标题。
     */
    val sectionHeader: TextStyle,

    /**
     * 导航项文字。
     */
    val navigationItem: TextStyle,

    /**
     * 活动导航项文字。
     */
    val activeNavigationItem: TextStyle,

    /**
     * 导航分组文字。
     */
    val navigationGroupLabel: TextStyle,
)

/**
 * 对话框和 Toast 文字样式。
 */
data class DialogTypography(
    /**
     * 对话框标题。
     */
    val title: TextStyle,

    /**
     * 对话框正文。
     */
    val body: TextStyle,

    /**
     * Toast 消息。
     */
    val toastMessage: TextStyle,
)

/**
 * 侧边栏各区域文字样式。
 */
data class SidebarTypography(
    /**
     * 区块标签。
     */
    val sectionLabel: TextStyle,

    /**
     * 账号头像文字。
     */
    val accountAvatar: TextStyle,

    /**
     * 账号名称。
     */
    val accountName: TextStyle,

    /**
     * 账号邮箱。
     */
    val accountEmail: TextStyle,

    /**
     * 配额说明。
     */
    val quotaDescription: TextStyle,

    /**
     * 更新下载提示。
     */
    val downloadUpdateLabel: TextStyle,

    /**
     * 更新下载进度。
     */
    val downloadUpdateProgress: TextStyle,

    /**
     * 可用更新提示。
     */
    val availableUpdateLabel: TextStyle,

    /**
     * 关闭更新操作。
     */
    val dismissUpdateAction: TextStyle,

    /**
     * 安装更新操作。
     */
    val installUpdateAction: TextStyle,

    /**
     * 目录树节点。
     */
    val treeNodeLabel: TextStyle,

    /**
     * 选中目录树节点。
     */
    val selectedTreeNodeLabel: TextStyle,
)

/**
 * 面包屑导航文字样式。
 */
data class BreadcrumbTypography(
    /**
     * 分隔符。
     */
    val separator: TextStyle,

    /**
     * 普通路径项。
     */
    val item: TextStyle,

    /**
     * 当前路径项。
     */
    val currentItem: TextStyle,
)

/**
 * 同步状态栏文字样式。
 */
data class StatusBarTypography(
    /**
     * 当前同步状态。
     */
    val currentStatus: TextStyle,

    /**
     * 最近同步时间。
     */
    val lastSyncTime: TextStyle,
)

/**
 * 文件列表及其操作弹窗文字样式。
 */
data class FileListTypography(
    /**
     * 选中项汇总。
     */
    val selectionSummary: TextStyle,

    /**
     * 状态列标题。
     */
    val statusColumnHeader: TextStyle,

    /**
     * 操作列标题。
     */
    val actionColumnHeader: TextStyle,

    /**
     * 已加载项汇总。
     */
    val loadedSummary: TextStyle,

    /**
     * 工具栏操作。
     */
    val toolbarAction: TextStyle,

    /**
     * 通用列标题。
     */
    val genericColumnHeader: TextStyle,

    /**
     * 文件名称。
     */
    val rowFileName: TextStyle,

    /**
     * 文件大小。
     */
    val rowFileSize: TextStyle,

    /**
     * 修改时间。
     */
    val rowModifiedTime: TextStyle,

    /**
     * 次要操作。
     */
    val secondaryAction: TextStyle,

    /**
     * 重命名弹窗标题。
     */
    val renameDialogTitle: TextStyle,

    /**
     * 移动弹窗标题。
     */
    val moveDialogTitle: TextStyle,

    /**
     * 移动弹窗说明。
     */
    val moveDialogDescription: TextStyle,

    /**
     * 移动目标目录。
     */
    val moveDialogFolder: TextStyle,
)

/**
 * 设置页各区域文字样式。
 */
data class SettingsTypography(
    /**
     * 页面标题。
     */
    val pageTitle: TextStyle,

    /**
     * 数值范围提示。
     */
    val numberRangeHint: TextStyle,

    /**
     * 日志保留说明。
     */
    val logRetentionDescription: TextStyle,

    /**
     * 校验错误。
     */
    val validationError: TextStyle,

    /**
     * 保存成功提示。
     */
    val saveSuccess: TextStyle,

    /**
     * 设置分组标题。
     */
    val groupHeader: TextStyle,

    /**
     * 设置项标题。
     */
    val optionTitle: TextStyle,

    /**
     * 设置项说明。
     */
    val optionDescription: TextStyle,

    /**
     * 未挂载标题。
     */
    val emptyMountTitle: TextStyle,

    /**
     * 未挂载说明。
     */
    val emptyMountDescription: TextStyle,

    /**
     * 当前挂载标题。
     */
    val currentMountTitle: TextStyle,

    /**
     * 当前挂载路径。
     */
    val currentMountPath: TextStyle,

    /**
     * 账号头像文字。
     */
    val accountAvatar: TextStyle,

    /**
     * 账号名称。
     */
    val accountName: TextStyle,

    /**
     * 详情标签。
     */
    val detailLabel: TextStyle,

    /**
     * 详情内容。
     */
    val detailValue: TextStyle,

    /**
     * 版本号。
     */
    val version: TextStyle,

    /**
     * 更新状态。
     */
    val updateStatus: TextStyle,

    /**
     * 关于说明。
     */
    val aboutDescription: TextStyle,

    /**
     * 外部链接。
     */
    val externalLink: TextStyle,
)

/**
 * 登录页文字样式。
 */
data class LoginTypography(
    /**
     * 登录标题。
     */
    val title: TextStyle,

    /**
     * 授权状态提示。
     */
    val authorizingMessage: TextStyle,

    /**
     * 页脚提示。
     */
    val footerHint: TextStyle,
)

/**
 * 传输面板文字样式。
 */
data class TransferTypography(
    /**
     * 面板标题。
     */
    val panelTitle: TextStyle,

    /**
     * 汇总数值。
     */
    val summaryValue: TextStyle,

    /**
     * 汇总标签。
     */
    val summaryLabel: TextStyle,

    /**
     * 任务名称。
     */
    val taskName: TextStyle,

    /**
     * 任务说明。
     */
    val taskDescription: TextStyle,

    /**
     * 任务进度。
     */
    val taskProgress: TextStyle,

    /**
     * 删除操作提示。
     */
    val deleteOperation: TextStyle,

    /**
     * 任务状态。
     */
    val taskState: TextStyle,
)

/**
 * 更新弹窗文字样式。
 */
data class UpdateTypography(
    /**
     * 弹窗标题。
     */
    val dialogTitle: TextStyle,

    /**
     * 版本号。
     */
    val version: TextStyle,

    /**
     * 失败提示。
     */
    val failureMessage: TextStyle,

    /**
     * 更新说明标签。
     */
    val releaseNotesLabel: TextStyle,

    /**
     * 更新说明正文。
     */
    val releaseNotesBody: TextStyle,

    /**
     * 无更新说明提示。
     */
    val noReleaseNotesMessage: TextStyle,

    /**
     * 下载进度。
     */
    val progress: TextStyle,

    /**
     * 等待提示。
     */
    val waitingMessage: TextStyle,
)

/**
 * 日志查看器文字样式。
 */
data class LogViewerTypography(
    /**
     * 页面标题。
     */
    val pageTitle: TextStyle,

    /**
     * 日志消息。
     */
    val recordMessage: TextStyle,

    /**
     * 日志元数据。
     */
    val recordMetadata: TextStyle,
)

/**
 * 主页面搜索区域文字样式。
 */
data class MainTypography(
    /**
     * 搜索区域标题。
     */
    val searchHeader: TextStyle,

    /**
     * 搜索结果名称。
     */
    val searchResultName: TextStyle,

    /**
     * 搜索结果说明。
     */
    val searchResultDescription: TextStyle,
)

/**
 * 图标目录文字样式。
 */
data class CatalogTypography(
    /**
     * 图标名称。
     */
    val iconName: TextStyle,
)
