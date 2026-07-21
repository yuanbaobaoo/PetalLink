import 'package:flutter/material.dart';

// =============================================================================
// Layer 1: 排版 token —— 按 UI 职责拆分的独立文字样式。
// 逐值对齐 CMP DesignTokens.kt 的 TYPOGRAPHY（9–22sp，系统字体）。
// 相同数值的不同职责保留独立字段，修改一处不会意外影响另一处。
// 样式不携带颜色，颜色由使用处按语义色填充（与 CMP 一致）。
// =============================================================================

/// 构造默认字体 token；[lineHeight] 为 CMP 的绝对行高（sp），
/// 转换为 Flutter 的 height 倍率（lineHeight / size）。
TextStyle _ts(
  double size, {
  FontWeight weight = FontWeight.normal,
  double? lineHeight,
  double? letterSpacing,
}) {
  return TextStyle(
    fontSize: size,
    fontWeight: weight,
    height: lineHeight != null ? lineHeight / size : null,
    letterSpacing: letterSpacing,
  );
}

/// 品牌标识文字样式。
class BrandTypography {
  /// 紧凑 Logo 文字。
  final TextStyle compactLogoLabel;

  /// 完整 Logo 文字。
  final TextStyle fullLogoLabel;

  const BrandTypography({
    required this.compactLogoLabel,
    required this.fullLogoLabel,
  });
}

/// 按钮各变体的独立文字样式。
class ButtonTypography {
  /// 主按钮文字。
  final TextStyle primaryLabel;

  /// 软色按钮文字。
  final TextStyle softLabel;

  /// 文字按钮文字。
  final TextStyle textLabel;

  /// 图标文字按钮文字。
  final TextStyle iconTextLabel;

  /// 按钮角标文字。
  final TextStyle badgeLabel;

  const ButtonTypography({
    required this.primaryLabel,
    required this.softLabel,
    required this.textLabel,
    required this.iconTextLabel,
    required this.badgeLabel,
  });
}

/// 弹出菜单文字样式。
class MenuTypography {
  /// 菜单项文字。
  final TextStyle itemLabel;

  const MenuTypography({required this.itemLabel});
}

/// 表单控件文字样式。
class FormTypography {
  /// 文本框输入文字。
  final TextStyle textFieldInput;

  /// 文本框占位文字。
  final TextStyle textFieldPlaceholder;

  /// 数值框输入文字。
  final TextStyle numberFieldInput;

  /// 数值框单位文字。
  final TextStyle numberFieldSuffix;

  /// 步进器数值文字。
  final TextStyle stepperValue;

  /// 步进器操作文字。
  final TextStyle stepperAction;

  const FormTypography({
    required this.textFieldInput,
    required this.textFieldPlaceholder,
    required this.numberFieldInput,
    required this.numberFieldSuffix,
    required this.stepperValue,
    required this.stepperAction,
  });
}

/// 提示和导航文字样式。
class FeedbackTypography {
  /// 横幅标题。
  final TextStyle bannerTitle;

  /// 横幅正文。
  final TextStyle bannerMessage;

  /// 小标签文字。
  final TextStyle smallTagLabel;

  /// 中标签文字。
  final TextStyle mediumTagLabel;

  /// 空状态标题。
  final TextStyle emptyStateTitle;

  /// 空状态说明。
  final TextStyle emptyStateDescription;

  /// 统计数量。
  final TextStyle statChipCount;

  /// 统计标签。
  final TextStyle statChipLabel;

  /// 区块标题。
  final TextStyle sectionHeader;

  /// 导航项文字。
  final TextStyle navigationItem;

  /// 活动导航项文字。
  final TextStyle activeNavigationItem;

  /// 导航分组文字。
  final TextStyle navigationGroupLabel;

  const FeedbackTypography({
    required this.bannerTitle,
    required this.bannerMessage,
    required this.smallTagLabel,
    required this.mediumTagLabel,
    required this.emptyStateTitle,
    required this.emptyStateDescription,
    required this.statChipCount,
    required this.statChipLabel,
    required this.sectionHeader,
    required this.navigationItem,
    required this.activeNavigationItem,
    required this.navigationGroupLabel,
  });
}

/// 对话框和 Toast 文字样式。
class DialogTypography {
  /// 对话框标题。
  final TextStyle title;

  /// 对话框正文。
  final TextStyle body;

  /// Toast 消息。
  final TextStyle toastMessage;

  const DialogTypography({
    required this.title,
    required this.body,
    required this.toastMessage,
  });
}

/// 侧边栏各区域文字样式。
class SidebarTypography {
  /// 区块标签。
  final TextStyle sectionLabel;

  /// 账号头像文字。
  final TextStyle accountAvatar;

  /// 账号名称。
  final TextStyle accountName;

  /// 账号邮箱。
  final TextStyle accountEmail;

  /// 配额说明。
  final TextStyle quotaDescription;

  /// 更新下载提示。
  final TextStyle downloadUpdateLabel;

  /// 更新下载进度。
  final TextStyle downloadUpdateProgress;

  /// 可用更新提示。
  final TextStyle availableUpdateLabel;

  /// 关闭更新操作。
  final TextStyle dismissUpdateAction;

  /// 安装更新操作。
  final TextStyle installUpdateAction;

  /// 目录树节点。
  final TextStyle treeNodeLabel;

  /// 选中目录树节点。
  final TextStyle selectedTreeNodeLabel;

  const SidebarTypography({
    required this.sectionLabel,
    required this.accountAvatar,
    required this.accountName,
    required this.accountEmail,
    required this.quotaDescription,
    required this.downloadUpdateLabel,
    required this.downloadUpdateProgress,
    required this.availableUpdateLabel,
    required this.dismissUpdateAction,
    required this.installUpdateAction,
    required this.treeNodeLabel,
    required this.selectedTreeNodeLabel,
  });
}

/// 面包屑导航文字样式。
class BreadcrumbTypography {
  /// 分隔符。
  final TextStyle separator;

  /// 普通路径项。
  final TextStyle item;

  /// 当前路径项。
  final TextStyle currentItem;

  const BreadcrumbTypography({
    required this.separator,
    required this.item,
    required this.currentItem,
  });
}

/// 同步状态栏文字样式。
class StatusBarTypography {
  /// 当前同步状态。
  final TextStyle currentStatus;

  /// 最近同步时间。
  final TextStyle lastSyncTime;

  const StatusBarTypography({
    required this.currentStatus,
    required this.lastSyncTime,
  });
}

/// 文件列表及其操作弹窗文字样式。
class FileListTypography {
  /// 选中项汇总。
  final TextStyle selectionSummary;

  /// 状态列标题。
  final TextStyle statusColumnHeader;

  /// 操作列标题。
  final TextStyle actionColumnHeader;

  /// 已加载项汇总。
  final TextStyle loadedSummary;

  /// 工具栏操作。
  final TextStyle toolbarAction;

  /// 通用列标题。
  final TextStyle genericColumnHeader;

  /// 文件名称。
  final TextStyle rowFileName;

  /// 文件大小。
  final TextStyle rowFileSize;

  /// 修改时间。
  final TextStyle rowModifiedTime;

  /// 次要操作。
  final TextStyle secondaryAction;

  /// 重命名弹窗标题。
  final TextStyle renameDialogTitle;

  /// 移动弹窗标题。
  final TextStyle moveDialogTitle;

  /// 移动弹窗说明。
  final TextStyle moveDialogDescription;

  /// 移动目标目录。
  final TextStyle moveDialogFolder;

  const FileListTypography({
    required this.selectionSummary,
    required this.statusColumnHeader,
    required this.actionColumnHeader,
    required this.loadedSummary,
    required this.toolbarAction,
    required this.genericColumnHeader,
    required this.rowFileName,
    required this.rowFileSize,
    required this.rowModifiedTime,
    required this.secondaryAction,
    required this.renameDialogTitle,
    required this.moveDialogTitle,
    required this.moveDialogDescription,
    required this.moveDialogFolder,
  });
}

/// 设置页各区域文字样式。
class SettingsTypography {
  /// 页面标题。
  final TextStyle pageTitle;

  /// 数值范围提示。
  final TextStyle numberRangeHint;

  /// 日志保留说明。
  final TextStyle logRetentionDescription;

  /// 校验错误。
  final TextStyle validationError;

  /// 保存成功提示。
  final TextStyle saveSuccess;

  /// 设置分组标题。
  final TextStyle groupHeader;

  /// 设置项标题。
  final TextStyle optionTitle;

  /// 设置项说明。
  final TextStyle optionDescription;

  /// 未挂载标题。
  final TextStyle emptyMountTitle;

  /// 未挂载说明。
  final TextStyle emptyMountDescription;

  /// 当前挂载标题。
  final TextStyle currentMountTitle;

  /// 当前挂载路径。
  final TextStyle currentMountPath;

  /// 账号头像文字。
  final TextStyle accountAvatar;

  /// 账号名称。
  final TextStyle accountName;

  /// 详情标签。
  final TextStyle detailLabel;

  /// 详情内容。
  final TextStyle detailValue;

  /// 版本号。
  final TextStyle version;

  /// 更新状态。
  final TextStyle updateStatus;

  /// 关于说明。
  final TextStyle aboutDescription;

  /// 外部链接。
  final TextStyle externalLink;

  const SettingsTypography({
    required this.pageTitle,
    required this.numberRangeHint,
    required this.logRetentionDescription,
    required this.validationError,
    required this.saveSuccess,
    required this.groupHeader,
    required this.optionTitle,
    required this.optionDescription,
    required this.emptyMountTitle,
    required this.emptyMountDescription,
    required this.currentMountTitle,
    required this.currentMountPath,
    required this.accountAvatar,
    required this.accountName,
    required this.detailLabel,
    required this.detailValue,
    required this.version,
    required this.updateStatus,
    required this.aboutDescription,
    required this.externalLink,
  });
}

/// 登录页文字样式。
class LoginTypography {
  /// 登录标题。
  final TextStyle title;

  /// 授权状态提示。
  final TextStyle authorizingMessage;

  /// 页脚提示。
  final TextStyle footerHint;

  const LoginTypography({
    required this.title,
    required this.authorizingMessage,
    required this.footerHint,
  });
}

/// 传输面板文字样式。
class TransferTypography {
  /// 面板标题。
  final TextStyle panelTitle;

  /// 汇总数值。
  final TextStyle summaryValue;

  /// 汇总标签。
  final TextStyle summaryLabel;

  /// 任务名称。
  final TextStyle taskName;

  /// 任务说明。
  final TextStyle taskDescription;

  /// 任务进度。
  final TextStyle taskProgress;

  /// 删除操作提示。
  final TextStyle deleteOperation;

  /// 任务状态。
  final TextStyle taskState;

  const TransferTypography({
    required this.panelTitle,
    required this.summaryValue,
    required this.summaryLabel,
    required this.taskName,
    required this.taskDescription,
    required this.taskProgress,
    required this.deleteOperation,
    required this.taskState,
  });
}

/// 更新弹窗文字样式。
class UpdateTypography {
  /// 弹窗标题。
  final TextStyle dialogTitle;

  /// 版本号。
  final TextStyle version;

  /// 失败提示。
  final TextStyle failureMessage;

  /// 更新说明标签。
  final TextStyle releaseNotesLabel;

  /// 更新说明正文。
  final TextStyle releaseNotesBody;

  /// 无更新说明提示。
  final TextStyle noReleaseNotesMessage;

  /// 下载进度。
  final TextStyle progress;

  /// 等待提示。
  final TextStyle waitingMessage;

  const UpdateTypography({
    required this.dialogTitle,
    required this.version,
    required this.failureMessage,
    required this.releaseNotesLabel,
    required this.releaseNotesBody,
    required this.noReleaseNotesMessage,
    required this.progress,
    required this.waitingMessage,
  });
}

/// 日志查看器文字样式。
class LogViewerTypography {
  /// 页面标题。
  final TextStyle pageTitle;

  /// 日志消息。
  final TextStyle recordMessage;

  /// 日志元数据。
  final TextStyle recordMetadata;

  const LogViewerTypography({
    required this.pageTitle,
    required this.recordMessage,
    required this.recordMetadata,
  });
}

/// 主页面搜索区域文字样式。
class MainTypography {
  /// 搜索区域标题。
  final TextStyle searchHeader;

  /// 搜索结果名称。
  final TextStyle searchResultName;

  /// 搜索结果说明。
  final TextStyle searchResultDescription;

  const MainTypography({
    required this.searchHeader,
    required this.searchResultName,
    required this.searchResultDescription,
  });
}

/// 图标目录文字样式。
class CatalogTypography {
  /// 图标名称。
  final TextStyle iconName;

  const CatalogTypography({required this.iconName});
}

// =============================================================================
// 按 UI 区域组织的排版 token 集合。
// =============================================================================

/// PetalLink 默认排版 token。
class MateTypography {
  /// 品牌标识字体。
  final BrandTypography brand;

  /// 按钮字体。
  final ButtonTypography button;

  /// 菜单字体。
  final MenuTypography menu;

  /// 表单字体。
  final FormTypography form;

  /// 提示和导航字体。
  final FeedbackTypography feedback;

  /// 对话框字体。
  final DialogTypography dialog;

  /// 侧边栏字体。
  final SidebarTypography sidebar;

  /// 面包屑字体。
  final BreadcrumbTypography breadcrumb;

  /// 状态栏字体。
  final StatusBarTypography statusBar;

  /// 文件列表字体。
  final FileListTypography fileList;

  /// 设置页字体。
  final SettingsTypography settings;

  /// 登录页字体。
  final LoginTypography login;

  /// 传输面板字体。
  final TransferTypography transfer;

  /// 更新弹窗字体。
  final UpdateTypography update;

  /// 日志查看器字体。
  final LogViewerTypography logViewer;

  /// 主页面字体。
  final MainTypography main;

  /// 图标目录字体。
  final CatalogTypography catalog;

  const MateTypography({
    required this.brand,
    required this.button,
    required this.menu,
    required this.form,
    required this.feedback,
    required this.dialog,
    required this.sidebar,
    required this.breadcrumb,
    required this.statusBar,
    required this.fileList,
    required this.settings,
    required this.login,
    required this.transfer,
    required this.update,
    required this.logViewer,
    required this.main,
    required this.catalog,
  });

  /// 默认排版配置（逐值对齐 CMP DesignTokens.TYPOGRAPHY；字体族为系统默认）。
  factory MateTypography.standard() {
    // 品牌标识字体。
    final brand = BrandTypography(
      compactLogoLabel: _ts(14, weight: FontWeight.w600),
      fullLogoLabel: _ts(14, weight: FontWeight.w600),
    );

    // 按钮字体。
    final button = ButtonTypography(
      primaryLabel: _ts(12, weight: FontWeight.w500),
      softLabel: _ts(11, weight: FontWeight.w500),
      textLabel: _ts(12, weight: FontWeight.w500),
      iconTextLabel: _ts(12, weight: FontWeight.w500),
      badgeLabel: _ts(10, weight: FontWeight.w600),
    );

    // 菜单字体。
    final menu = MenuTypography(
      itemLabel: _ts(13),
    );

    // 表单字体。
    final form = FormTypography(
      textFieldInput: _ts(13),
      textFieldPlaceholder: _ts(13),
      numberFieldInput: _ts(13),
      numberFieldSuffix: _ts(12),
      stepperValue: _ts(13, weight: FontWeight.w500),
      stepperAction: _ts(16, weight: FontWeight.w500),
    );

    // 提示和导航字体。
    final feedback = FeedbackTypography(
      bannerTitle: _ts(13, weight: FontWeight.w600),
      bannerMessage: _ts(13, lineHeight: 21.7),
      smallTagLabel: _ts(12, weight: FontWeight.w500),
      mediumTagLabel: _ts(13, weight: FontWeight.w500),
      emptyStateTitle: _ts(15, weight: FontWeight.w600),
      emptyStateDescription: _ts(13, lineHeight: 21),
      statChipCount: _ts(12, weight: FontWeight.w500),
      statChipLabel: _ts(12, weight: FontWeight.w500),
      sectionHeader: _ts(18, weight: FontWeight.w600),
      navigationItem: _ts(14),
      activeNavigationItem: _ts(14, weight: FontWeight.w500),
      navigationGroupLabel: _ts(12, weight: FontWeight.w600),
    );

    // 对话框字体。
    final dialog = DialogTypography(
      title: _ts(17, weight: FontWeight.w600),
      body: _ts(14, lineHeight: 24.75),
      toastMessage: _ts(13, weight: FontWeight.w500),
    );

    // 侧边栏字体。
    final sidebar = SidebarTypography(
      sectionLabel: _ts(11, weight: FontWeight.w600, letterSpacing: 0.4),
      accountAvatar: _ts(14, weight: FontWeight.w600),
      accountName: _ts(14, weight: FontWeight.w600),
      accountEmail: _ts(12),
      quotaDescription: _ts(11.5),
      downloadUpdateLabel: _ts(13, weight: FontWeight.w600),
      downloadUpdateProgress: _ts(13, weight: FontWeight.w700),
      availableUpdateLabel: _ts(13, weight: FontWeight.w600),
      dismissUpdateAction: _ts(13),
      installUpdateAction: _ts(12, weight: FontWeight.w600),
      treeNodeLabel: _ts(13),
      selectedTreeNodeLabel: _ts(13, weight: FontWeight.w500),
    );

    // 面包屑字体。
    final breadcrumb = BreadcrumbTypography(
      separator: _ts(12),
      item: _ts(13),
      currentItem: _ts(13, weight: FontWeight.w600),
    );

    // 状态栏字体。
    final statusBar = StatusBarTypography(
      currentStatus: _ts(13, weight: FontWeight.w500),
      lastSyncTime: _ts(12.5),
    );

    // 文件列表字体。
    final fileList = FileListTypography(
      selectionSummary: _ts(13, weight: FontWeight.w600),
      statusColumnHeader: _ts(11.5, weight: FontWeight.w600),
      actionColumnHeader: _ts(11.5, weight: FontWeight.w600),
      loadedSummary: _ts(12),
      toolbarAction: _ts(13, weight: FontWeight.w500),
      genericColumnHeader: _ts(11.5, weight: FontWeight.w600),
      rowFileName: _ts(14),
      rowFileSize: _ts(13),
      rowModifiedTime: _ts(13),
      secondaryAction: _ts(13),
      renameDialogTitle: _ts(16, weight: FontWeight.w600),
      moveDialogTitle: _ts(16),
      moveDialogDescription: _ts(12),
      moveDialogFolder: _ts(13),
    );

    // 设置页字体。
    final settings = SettingsTypography(
      pageTitle: _ts(16, weight: FontWeight.w600),
      numberRangeHint: _ts(13),
      logRetentionDescription: _ts(14, lineHeight: 24),
      validationError: _ts(12),
      saveSuccess: _ts(12),
      groupHeader: _ts(12, weight: FontWeight.w600),
      optionTitle: _ts(14, weight: FontWeight.w500),
      optionDescription: _ts(12),
      emptyMountTitle: _ts(14, weight: FontWeight.w600),
      emptyMountDescription: _ts(13),
      currentMountTitle: _ts(14, weight: FontWeight.w600),
      currentMountPath: _ts(12),
      accountAvatar: _ts(22, weight: FontWeight.w600),
      accountName: _ts(16, weight: FontWeight.w600),
      detailLabel: _ts(13),
      detailValue: _ts(13),
      version: _ts(12),
      updateStatus: _ts(12),
      aboutDescription: _ts(12),
      externalLink: _ts(13),
    );

    // 登录页字体。
    final login = LoginTypography(
      title: _ts(20, weight: FontWeight.w600, letterSpacing: -0.2),
      authorizingMessage: _ts(14, weight: FontWeight.w500),
      footerHint: _ts(12),
    );

    // 传输面板字体。
    final transfer = TransferTypography(
      panelTitle: _ts(17, weight: FontWeight.w600),
      summaryValue: _ts(16, weight: FontWeight.w700),
      summaryLabel: _ts(11),
      taskName: _ts(13.5, weight: FontWeight.w500),
      taskDescription: _ts(12, lineHeight: 18.85),
      taskProgress: _ts(11.5),
      deleteOperation: _ts(12),
      taskState: _ts(12, weight: FontWeight.w500),
    );

    // 更新弹窗字体。
    final update = UpdateTypography(
      dialogTitle: _ts(17, weight: FontWeight.w600),
      version: _ts(15, weight: FontWeight.w700),
      failureMessage: _ts(14),
      releaseNotesLabel: _ts(12, weight: FontWeight.w600),
      releaseNotesBody: _ts(12),
      noReleaseNotesMessage: _ts(14),
      progress: _ts(12, weight: FontWeight.w600),
      waitingMessage: _ts(14),
    );

    // 日志查看器字体。
    final logViewer = LogViewerTypography(
      pageTitle: _ts(16, weight: FontWeight.w600),
      recordMessage: _ts(13.5),
      recordMetadata: _ts(11.5),
    );

    // 主页面字体。
    final main = MainTypography(
      searchHeader: _ts(12.5, weight: FontWeight.w600),
      searchResultName: _ts(14),
      searchResultDescription: _ts(12),
    );

    // 图标目录字体。
    final catalog = CatalogTypography(
      iconName: _ts(9),
    );

    return MateTypography(
      brand: brand,
      button: button,
      menu: menu,
      form: form,
      feedback: feedback,
      dialog: dialog,
      sidebar: sidebar,
      breadcrumb: breadcrumb,
      statusBar: statusBar,
      fileList: fileList,
      settings: settings,
      login: login,
      transfer: transfer,
      update: update,
      logViewer: logViewer,
      main: main,
      catalog: catalog,
    );
  }
}
