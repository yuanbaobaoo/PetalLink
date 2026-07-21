import 'dart:ui';

import 'mate_tokens.dart';

// =============================================================================
// Layer 2: 语义颜色解析 —— 将原始 token 按明暗主题组装为语义别名。
// 字段与值逐一对齐 CMP Theme.kt 的 ThemeSemanticColors / LIGHT / DARK。
// 两套静态实例 .light 和 .dark 由 MateSkin 或 MateLinkTheme 自动选用。
// =============================================================================

/// 当前明暗主题下的语义颜色别名集合。
///
/// 同一语义字段在不同主题下映射到不同的原始 token，
/// 使用者只需引用语义名，无需关心当前是明色还是暗色。
class MateSemanticColors {
  // ------ 品牌 ------
  /// 品牌主色。
  final Color brand;

  /// 品牌悬停色。
  final Color brandHover;

  /// 品牌按下色。
  final Color brandActive;

  /// 品牌浅色。
  final Color brandLight;

  /// 品牌 100 色阶。
  final Color brand100;

  /// 品牌最浅色。
  final Color brandLighter;

  /// 品牌渐变（brandHover → brand）。
  final List<Color> brandGradient;

  /// 品牌浅色渐变（brandLighter → brand100）。
  final List<Color> brandGradientSoft;

  // ------ 功能色 ------
  /// 成功色。
  final Color success;

  /// 成功背景色。
  final Color successBg;

  /// 警告色。
  final Color warning;

  /// 警告背景色。
  final Color warningBg;

  /// 错误色。
  final Color error;

  /// 错误背景色。
  final Color errorBg;

  /// 信息色。
  final Color info;

  /// 信息背景色。
  final Color infoBg;

  // ------ 文件类型色 ------
  /// 文件夹色。
  final Color folder;

  /// 文件夹背景色。
  final Color folderBg;

  /// 文档色。
  final Color document;

  /// 文档背景色。
  final Color documentBg;

  /// 图片色。
  final Color image;

  /// 图片背景色。
  final Color imageBg;

  /// 视频色。
  final Color video;

  /// 视频背景色。
  final Color videoBg;

  /// 表格色。
  final Color sheet;

  /// 表格背景色。
  final Color sheetBg;

  // ------ 控件色 ------
  /// 关闭状态开关轨道色。
  final Color switchOffTrack;

  /// 开关滑块色。
  final Color switchKnob;

  /// 复选框标记色。
  final Color checkboxMark;

  // ------ 页面/容器背景 ------
  /// 页面背景色。
  final Color bgPage;

  /// 容器背景色。
  final Color bgContainer;

  /// 填充背景色。
  final Color bgFill;

  /// 悬停背景色。
  final Color bgHover;

  /// 激活背景色。
  final Color bgActive;

  // ------ 边框 ------
  /// 默认边框色。
  final Color border;

  /// 悬停边框色。
  final Color borderHover;

  // ------ 文字 ------
  /// 主要文字色。
  final Color textPrimary;

  /// 次要文字色。
  final Color textSecondary;

  /// 占位文字色。
  final Color textPlaceholder;

  // ------ 固定前景色 ------
  /// 紧凑 Logo 文字色。
  final Color appLogoCompactText;

  /// 完整 Logo 文字色。
  final Color appLogoFullText;

  /// 紧凑 Logo 图标色。
  final Color appLogoCompactIcon;

  /// 图标默认颜色。
  final Color defaultIconTint;

  // ------ 文件列表批量操作 ------
  /// 批量操作栏背景色。
  final Color fileListBulkBackground;

  /// 批量危险操作文字色。
  final Color fileListBulkDangerText;

  /// 批量危险操作图标色。
  final Color fileListBulkDangerIcon;

  /// 批量危险操作悬停背景色。
  final Color fileListBulkDangerHoverBackground;

  /// 批量选择摘要文字色。
  final Color fileListBulkSelectionText;

  /// 批量普通操作文字色。
  final Color fileListBulkActionText;

  /// 批量普通操作图标色。
  final Color fileListBulkActionIcon;

  /// 批量普通操作悬停背景色。
  final Color fileListBulkActionHoverBackground;

  /// 批量关闭图标色。
  final Color fileListBulkCloseIcon;

  /// 批量关闭悬停图标色。
  final Color fileListBulkCloseHoverIcon;

  /// 批量关闭悬停背景色。
  final Color fileListBulkCloseHoverBackground;

  // ------ Toast ------
  /// 成功 Toast 图标色。
  final Color toastSuccessIcon;

  /// 错误 Toast 图标色。
  final Color toastErrorIcon;

  /// Toast 背景色。
  final Color toastBackground;

  /// 默认 Toast 图标色。
  final Color toastDefaultIcon;

  /// Toast 文字色。
  final Color toastText;

  // ------ 遮罩 ------
  /// 主页面加载遮罩色。
  final Color mainLoadingScrim;

  /// 更新弹窗遮罩色。
  final Color updateDialogScrim;

  /// 通用对话框遮罩色。
  final Color overlayDialogScrim;

  // ------ 侧边栏 ------
  /// 侧边栏账号头像文字色。
  final Color sidebarAccountAvatarText;

  /// 侧边栏更新卡文字色。
  final Color sidebarUpdateText;

  /// 侧边栏更新卡进度条色。
  final Color sidebarUpdateProgress;

  /// 侧边栏更新卡关闭按钮背景色。
  final Color sidebarDismissBackground;

  /// 侧边栏更新卡关闭按钮文字色。
  final Color sidebarDismissText;

  /// 侧边栏立即更新按钮背景色。
  final Color sidebarInstallBackground;

  // ------ 设置页 ------
  /// 设置页账号头像文字色。
  final Color settingsAccountAvatarText;

  // ------ 按钮 ------
  /// 主按钮文字色。
  final Color buttonPrimaryText;

  /// 禁用主按钮文字色。
  final Color buttonDisabledPrimaryText;

  /// 危险按钮文字色。
  final Color buttonDangerText;

  /// 按钮角标文字色。
  final Color buttonBadgeText;

  // ------ Flutter 平台补充（CMP 无对应项） ------
  /// 主色上的内容色。
  final Color onPrimary;

  /// 滚动条滑块色。
  final Color scrollbarThumb;

  /// 分隔线色。
  final Color divider;

  /// 通用遮罩/覆盖层色（同 overlayDialogScrim）。
  final Color overlay;

  const MateSemanticColors({
    required this.brand,
    required this.brandHover,
    required this.brandActive,
    required this.brandLight,
    required this.brand100,
    required this.brandLighter,
    required this.brandGradient,
    required this.brandGradientSoft,
    required this.success,
    required this.successBg,
    required this.warning,
    required this.warningBg,
    required this.error,
    required this.errorBg,
    required this.info,
    required this.infoBg,
    required this.folder,
    required this.folderBg,
    required this.document,
    required this.documentBg,
    required this.image,
    required this.imageBg,
    required this.video,
    required this.videoBg,
    required this.sheet,
    required this.sheetBg,
    required this.switchOffTrack,
    required this.switchKnob,
    required this.checkboxMark,
    required this.bgPage,
    required this.bgContainer,
    required this.bgFill,
    required this.bgHover,
    required this.bgActive,
    required this.border,
    required this.borderHover,
    required this.textPrimary,
    required this.textSecondary,
    required this.textPlaceholder,
    required this.appLogoCompactText,
    required this.appLogoFullText,
    required this.appLogoCompactIcon,
    required this.defaultIconTint,
    required this.fileListBulkBackground,
    required this.fileListBulkDangerText,
    required this.fileListBulkDangerIcon,
    required this.fileListBulkDangerHoverBackground,
    required this.fileListBulkSelectionText,
    required this.fileListBulkActionText,
    required this.fileListBulkActionIcon,
    required this.fileListBulkActionHoverBackground,
    required this.fileListBulkCloseIcon,
    required this.fileListBulkCloseHoverIcon,
    required this.fileListBulkCloseHoverBackground,
    required this.toastSuccessIcon,
    required this.toastErrorIcon,
    required this.toastBackground,
    required this.toastDefaultIcon,
    required this.toastText,
    required this.mainLoadingScrim,
    required this.updateDialogScrim,
    required this.overlayDialogScrim,
    required this.sidebarAccountAvatarText,
    required this.sidebarUpdateText,
    required this.sidebarUpdateProgress,
    required this.sidebarDismissBackground,
    required this.sidebarDismissText,
    required this.sidebarInstallBackground,
    required this.settingsAccountAvatarText,
    required this.buttonPrimaryText,
    required this.buttonDisabledPrimaryText,
    required this.buttonDangerText,
    required this.buttonBadgeText,
    required this.onPrimary,
    required this.scrollbarThumb,
    required this.divider,
    required this.overlay,
  });

  /// 浅色语义配色（对齐 CMP LIGHT_SEMANTIC_COLORS）。
  static const MateSemanticColors light = MateSemanticColors(
    // 品牌。
    brand: MateColors.brand,
    brandHover: MateColors.brandHover,
    brandActive: MateColors.brandActive,
    brandLight: MateColors.brandLight,
    brand100: MateColors.brand100,
    brandLighter: MateColors.brandLighter,
    brandGradient: [MateColors.brandHover, MateColors.brand],
    brandGradientSoft: [MateColors.brandLighter, MateColors.brand100],
    // 功能色。
    success: MateColors.success,
    successBg: MateColors.successBackground,
    warning: MateColors.warning,
    warningBg: MateColors.warningBackground,
    error: MateColors.error,
    errorBg: MateColors.errorBackground,
    info: MateColors.info,
    infoBg: MateColors.infoBackground,
    // 文件类型色。
    folder: MateColors.folder,
    folderBg: MateColors.folderBackground,
    document: MateColors.document,
    documentBg: MateColors.documentBackground,
    image: MateColors.image,
    imageBg: MateColors.imageBackground,
    video: MateColors.video,
    videoBg: MateColors.videoBackground,
    sheet: MateColors.sheet,
    sheetBg: MateColors.sheetBackground,
    // 控件色。
    switchOffTrack: MateColors.lightSwitchOffTrack,
    switchKnob: MateColors.switchKnob,
    checkboxMark: MateColors.checkboxMark,
    // 页面/容器背景。
    bgPage: MateColors.lightBgPage,
    bgContainer: MateColors.lightBgContainer,
    bgFill: MateColors.lightBgFill,
    bgHover: MateColors.lightBgHover,
    bgActive: MateColors.lightBgActive,
    // 边框。
    border: MateColors.lightBorder,
    borderHover: MateColors.lightBorderHover,
    // 文字。
    textPrimary: MateColors.lightTextPrimary,
    textSecondary: MateColors.lightTextSecondary,
    textPlaceholder: MateColors.lightTextPlaceholder,
    // 固定前景色。
    appLogoCompactText: MateColors.appLogoCompactText,
    appLogoFullText: MateColors.appLogoFullText,
    appLogoCompactIcon: MateColors.appLogoCompactIcon,
    defaultIconTint: MateColors.defaultIconTint,
    // 文件列表批量操作。
    fileListBulkBackground: MateColors.fileListBulkBackground,
    fileListBulkDangerText: MateColors.fileListBulkDangerText,
    fileListBulkDangerIcon: MateColors.fileListBulkDangerIcon,
    fileListBulkDangerHoverBackground:
        MateColors.fileListBulkDangerHoverBackground,
    fileListBulkSelectionText: MateColors.fileListBulkSelectionText,
    fileListBulkActionText: MateColors.fileListBulkActionText,
    fileListBulkActionIcon: MateColors.fileListBulkActionIcon,
    fileListBulkActionHoverBackground:
        MateColors.fileListBulkActionHoverBackground,
    fileListBulkCloseIcon: MateColors.fileListBulkCloseIcon,
    fileListBulkCloseHoverIcon: MateColors.fileListBulkCloseHoverIcon,
    fileListBulkCloseHoverBackground:
        MateColors.fileListBulkCloseHoverBackground,
    // Toast。
    toastSuccessIcon: MateColors.toastSuccessIcon,
    toastErrorIcon: MateColors.toastErrorIcon,
    toastBackground: MateColors.toastBackground,
    toastDefaultIcon: MateColors.toastDefaultIcon,
    toastText: MateColors.toastText,
    // 遮罩。
    mainLoadingScrim: MateColors.mainLoadingScrim,
    updateDialogScrim: MateColors.updateDialogScrim,
    overlayDialogScrim: MateColors.overlayDialogScrim,
    // 侧边栏。
    sidebarAccountAvatarText: MateColors.sidebarAccountAvatarText,
    sidebarUpdateText: MateColors.sidebarUpdateText,
    sidebarUpdateProgress: MateColors.sidebarUpdateProgress,
    sidebarDismissBackground: MateColors.sidebarDismissBackground,
    sidebarDismissText: MateColors.sidebarDismissText,
    sidebarInstallBackground: MateColors.sidebarInstallBackground,
    // 设置页。
    settingsAccountAvatarText: MateColors.settingsAccountAvatarText,
    // 按钮。
    buttonPrimaryText: MateColors.buttonPrimaryText,
    buttonDisabledPrimaryText: MateColors.buttonDisabledPrimaryText,
    buttonDangerText: MateColors.buttonDangerText,
    buttonBadgeText: MateColors.buttonBadgeText,
    // Flutter 平台补充。
    onPrimary: MateColors.onPrimary,
    scrollbarThumb: MateColors.scrollbarThumb,
    divider: MateColors.divider,
    overlay: MateColors.overlayDialogScrim,
  );

  /// 深色语义配色（对齐 CMP DARK_SEMANTIC_COLORS）。
  static const MateSemanticColors dark = MateSemanticColors(
    // 品牌（明暗互换：主色用 hover 色）。
    brand: MateColors.brandHover,
    brandHover: MateColors.brand,
    brandActive: MateColors.brandActive,
    brandLight: MateColors.darkBrandLight,
    brand100: MateColors.darkBrand100,
    brandLighter: MateColors.darkBrandLighter,
    brandGradient: [MateColors.brandHover, MateColors.brand],
    brandGradientSoft: [MateColors.darkBrandLighter, MateColors.darkBrand100],
    // 功能色。
    success: MateColors.success,
    successBg: MateColors.darkSuccessBackground,
    warning: MateColors.warning,
    warningBg: MateColors.darkWarningBackground,
    error: MateColors.error,
    errorBg: MateColors.darkErrorBackground,
    info: MateColors.info,
    infoBg: MateColors.darkInfoBackground,
    // 文件类型色。
    folder: MateColors.folder,
    folderBg: MateColors.darkFolderBackground,
    document: MateColors.document,
    documentBg: MateColors.darkDocumentBackground,
    image: MateColors.image,
    imageBg: MateColors.darkImageBackground,
    video: MateColors.video,
    videoBg: MateColors.darkVideoBackground,
    sheet: MateColors.sheet,
    sheetBg: MateColors.darkSheetBackground,
    // 控件色。
    switchOffTrack: MateColors.darkSwitchOffTrack,
    switchKnob: MateColors.switchKnob,
    checkboxMark: MateColors.checkboxMark,
    // 页面/容器背景。
    bgPage: MateColors.darkBgPage,
    bgContainer: MateColors.darkBgContainer,
    bgFill: MateColors.darkBgFill,
    bgHover: MateColors.darkBgHover,
    bgActive: MateColors.darkBgActive,
    // 边框。
    border: MateColors.darkBorder,
    borderHover: MateColors.darkBorderHover,
    // 文字。
    textPrimary: MateColors.darkTextPrimary,
    textSecondary: MateColors.darkTextSecondary,
    textPlaceholder: MateColors.darkTextPlaceholder,
    // 固定前景色（与浅色主题一致）。
    appLogoCompactText: MateColors.appLogoCompactText,
    appLogoFullText: MateColors.appLogoFullText,
    appLogoCompactIcon: MateColors.appLogoCompactIcon,
    defaultIconTint: MateColors.defaultIconTint,
    // 文件列表批量操作。
    fileListBulkBackground: MateColors.fileListBulkBackground,
    fileListBulkDangerText: MateColors.fileListBulkDangerText,
    fileListBulkDangerIcon: MateColors.fileListBulkDangerIcon,
    fileListBulkDangerHoverBackground:
        MateColors.fileListBulkDangerHoverBackground,
    fileListBulkSelectionText: MateColors.fileListBulkSelectionText,
    fileListBulkActionText: MateColors.fileListBulkActionText,
    fileListBulkActionIcon: MateColors.fileListBulkActionIcon,
    fileListBulkActionHoverBackground:
        MateColors.fileListBulkActionHoverBackground,
    fileListBulkCloseIcon: MateColors.fileListBulkCloseIcon,
    fileListBulkCloseHoverIcon: MateColors.fileListBulkCloseHoverIcon,
    fileListBulkCloseHoverBackground:
        MateColors.fileListBulkCloseHoverBackground,
    // Toast。
    toastSuccessIcon: MateColors.toastSuccessIcon,
    toastErrorIcon: MateColors.toastErrorIcon,
    toastBackground: MateColors.toastBackground,
    toastDefaultIcon: MateColors.toastDefaultIcon,
    toastText: MateColors.toastText,
    // 遮罩。
    mainLoadingScrim: MateColors.mainLoadingScrim,
    updateDialogScrim: MateColors.updateDialogScrim,
    overlayDialogScrim: MateColors.overlayDialogScrim,
    // 侧边栏。
    sidebarAccountAvatarText: MateColors.sidebarAccountAvatarText,
    sidebarUpdateText: MateColors.sidebarUpdateText,
    sidebarUpdateProgress: MateColors.sidebarUpdateProgress,
    sidebarDismissBackground: MateColors.sidebarDismissBackground,
    sidebarDismissText: MateColors.sidebarDismissText,
    sidebarInstallBackground: MateColors.sidebarInstallBackground,
    // 设置页。
    settingsAccountAvatarText: MateColors.settingsAccountAvatarText,
    // 按钮。
    buttonPrimaryText: MateColors.buttonPrimaryText,
    buttonDisabledPrimaryText: MateColors.buttonDisabledPrimaryText,
    buttonDangerText: MateColors.buttonDangerText,
    buttonBadgeText: MateColors.buttonBadgeText,
    // Flutter 平台补充。
    onPrimary: MateColors.onPrimary,
    scrollbarThumb: MateColors.darkScrollbarThumb,
    divider: MateColors.darkDivider,
    overlay: MateColors.overlayDialogScrim,
  );
}
