import 'dart:ui';

// =============================================================================
// Layer 1: 原始设计 token —— 包内私有，不得被外部直接引用。
// 所有色值逐值对齐 CMP shared/src@jvm/.../ui/theme/DesignTokens.kt。
// =============================================================================

/// 原始色彩 token（包内私有，仅限 theme 目录内引用）。
class MateColors {
  MateColors._();

  // ------ 品牌色 ------
  /// 品牌主色。
  static const Color brand = Color(0xFF0053DB);

  /// 品牌控件悬停色。
  static const Color brandHover = Color(0xFF4A8BF0);

  /// 品牌控件按下色。
  static const Color brandActive = Color(0xFF0047B8);

  /// 品牌浅色强调。
  static const Color brandLight = Color(0xFFB7D0F7);

  /// 品牌 100 色阶背景。
  static const Color brand100 = Color(0xFFDCE8FC);

  /// 品牌最浅背景。
  static const Color brandLighter = Color(0xFFEFF4FE);

  // ------ 功能色 ------
  /// 成功状态前景色。
  static const Color success = Color(0xFF0CA678);

  /// 浅色主题成功状态背景色。
  static const Color successBackground = Color(0xFFE3F5EE);

  /// 警告状态前景色。
  static const Color warning = Color(0xFFF08C00);

  /// 浅色主题警告状态背景色。
  static const Color warningBackground = Color(0xFFFFF3DE);

  /// 错误状态前景色。
  static const Color error = Color(0xFFE5484D);

  /// 浅色主题错误状态背景色。
  static const Color errorBackground = Color(0xFFFDECEC);

  /// 信息状态前景色。
  static const Color info = Color(0xFF3B82F6);

  /// 浅色主题信息状态背景色。
  static const Color infoBackground = Color(0xFFE8F0FE);

  // ------ 文件类型色 ------
  /// 文件夹图标前景色。
  static const Color folder = Color(0xFFF0A63C);

  /// 浅色主题文件夹图标背景色。
  static const Color folderBackground = Color(0xFFFFF4DE);

  /// 文档图标前景色。
  static const Color document = Color(0xFF6366F1);

  /// 浅色主题文档图标背景色。
  static const Color documentBackground = Color(0xFFEEF2FF);

  /// 图片图标前景色。
  static const Color image = Color(0xFFEC4899);

  /// 浅色主题图片图标背景色。
  static const Color imageBackground = Color(0xFFFDE7F3);

  /// 视频图标前景色。
  static const Color video = Color(0xFF8B5CF6);

  /// 浅色主题视频图标背景色。
  static const Color videoBackground = Color(0xFFF3E8FF);

  /// 表格图标前景色。
  static const Color sheet = Color(0xFF10B981);

  /// 浅色主题表格图标背景色。
  static const Color sheetBackground = Color(0xFFE6F7EE);

  // ------ 控件色 ------
  /// 浅色主题关闭状态开关轨道色。
  static const Color lightSwitchOffTrack = Color(0xFFE3E3E6);

  // ------ 浅色主题 ------
  /// 浅色主题页面背景色。
  static const Color lightBgPage = Color(0xFFF5F5F7);

  /// 浅色主题容器背景色。
  static const Color lightBgContainer = Color(0xFFFFFFFF);

  /// 浅色主题填充背景色。
  static const Color lightBgFill = Color(0xFFF1F1F3);

  /// 浅色主题悬停背景色。
  static const Color lightBgHover = Color(0xFFF7F7F9);

  /// 浅色主题强悬停背景色（黑 0.04）——侧边栏树节点/设置导航项，
  /// 对齐 Tauri rgba(0,0,0,0.04)（bgHover 在灰底上仅差 2/255 不可见）。
  static const Color lightHoverStrong = Color(0x0A000000);

  /// 浅色主题激活背景色。
  static const Color lightBgActive = Color(0xFFECECEF);

  /// 浅色主题默认边框色。
  static const Color lightBorder = Color(0x0F000000);

  /// 浅色主题悬停边框色。
  static const Color lightBorderHover = Color(0x1A000000);

  /// 浅色主题主要文字色。
  static const Color lightTextPrimary = Color(0xE6000000);

  /// 浅色主题次要文字色。
  static const Color lightTextSecondary = Color(0x99000000);

  /// 浅色主题占位文字色。
  static const Color lightTextPlaceholder = Color(0x59000000);

  // ------ 固定前景色 ------
  /// Material 主色上的固定内容色。
  static const Color onPrimary = Color(0xFFFFFFFF);

  /// 紧凑 Logo 的固定文字色。
  static const Color appLogoCompactText = Color(0xFF1C1C1E);

  /// 完整 Logo 的固定文字色。
  static const Color appLogoFullText = Color(0xFF181818);

  /// 图标未指定颜色时的默认色。
  static const Color defaultIconTint = Color(0xFF181818);

  /// 文件列表批量操作栏背景色。
  static const Color fileListBulkBackground = Color(0xF01C1C1E);

  /// 文件列表批量危险操作文字色。
  static const Color fileListBulkDangerText = Color(0xFFFDA4AF);

  /// 文件列表批量危险操作图标色。
  static const Color fileListBulkDangerIcon = Color(0xFFFDA4AF);

  /// 文件列表批量危险操作悬停背景色。
  static const Color fileListBulkDangerHoverBackground = Color(0x2EFDA4AF);

  /// 成功 Toast 图标色。
  static const Color toastSuccessIcon = Color(0xFF4ADE80);

  /// 警告 Toast 图标色（对齐 Tauri #fbbf24）。
  static const Color toastWarningIcon = Color(0xFFFBBF24);

  /// 错误 Toast 图标色。
  static const Color toastErrorIcon = Color(0xFFFB7185);

  /// Toast 固定背景色。
  static const Color toastBackground = Color(0xEB1C1C1E);

  /// 默认 Toast 图标色。
  static const Color toastDefaultIcon = Color(0xFFFFFFFF);

  /// Toast 固定文字色。
  static const Color toastText = Color(0xFFFFFFFF);

  /// 主按钮固定文字色。
  static const Color buttonPrimaryText = Color(0xFFFFFFFF);

  /// 危险按钮固定文字色。
  static const Color buttonDangerText = Color(0xFFFFFFFF);

  /// 禁用主按钮固定文字色。
  static const Color buttonDisabledPrimaryText = Color(0xFFFFFFFF);

  /// 按钮角标固定文字色。
  static const Color buttonBadgeText = Color(0xFFFFFFFF);

  /// 紧凑 Logo 图标色。
  static const Color appLogoCompactIcon = Color(0xFFFFFFFF);

  /// 开关滑块色。
  static const Color switchKnob = Color(0xFFFFFFFF);

  /// 复选框标记色。
  static const Color checkboxMark = Color(0xFFFFFFFF);

  // ------ 文件列表批量操作（固定白色系，对齐 CMP Theme.kt） ------
  /// 批量选择摘要文字色。
  static const Color fileListBulkSelectionText = Color(0xFFFFFFFF);

  /// 批量普通操作文字色（白 0.85）。
  static const Color fileListBulkActionText = Color(0xD9FFFFFF);

  /// 批量普通操作图标色（白 0.7）。
  static const Color fileListBulkActionIcon = Color(0xB3FFFFFF);

  /// 批量普通操作悬停背景色（白 0.12）。
  static const Color fileListBulkActionHoverBackground = Color(0x1FFFFFFF);

  /// 批量关闭图标色（白 0.7）。
  static const Color fileListBulkCloseIcon = Color(0xB3FFFFFF);

  /// 批量关闭悬停图标色。
  static const Color fileListBulkCloseHoverIcon = Color(0xFFFFFFFF);

  /// 批量关闭悬停背景色（白 0.12）。
  static const Color fileListBulkCloseHoverBackground = Color(0x1FFFFFFF);

  // ------ 侧边栏（固定白色系，对齐 CMP Theme.kt） ------
  /// 侧边栏账号头像文字色。
  static const Color sidebarAccountAvatarText = Color(0xFFFFFFFF);

  /// 侧边栏更新卡文字色。
  static const Color sidebarUpdateText = Color(0xFFFFFFFF);

  /// 侧边栏更新卡进度条色。
  static const Color sidebarUpdateProgress = Color(0xFFFFFFFF);

  /// 侧边栏更新卡关闭按钮背景色（白 0.25）。
  static const Color sidebarDismissBackground = Color(0x40FFFFFF);

  /// 侧边栏更新卡关闭按钮文字色。
  static const Color sidebarDismissText = Color(0xFFFFFFFF);

  /// 侧边栏立即更新按钮背景色（白 0.95）。
  static const Color sidebarInstallBackground = Color(0xF2FFFFFF);

  // ------ 设置页 ------
  /// 设置页账号头像文字色。
  static const Color settingsAccountAvatarText = Color(0xFFFFFFFF);

  // ------ 遮罩 ------
  /// 主页面加载遮罩色（白 0.6）。
  static const Color mainLoadingScrim = Color(0x99FFFFFF);

  /// 更新弹窗遮罩色（对齐 Tauri rgba(28,28,30,0.36)）。
  static const Color updateDialogScrim = Color(0x5C1C1C1E);

  /// 通用对话框遮罩色（对齐 Tauri rgba(28,28,30,0.36)）。
  static const Color overlayDialogScrim = Color(0x5C1C1C1E);

  /// 控件阴影色（浅，黑 0.08）——步进按钮等浮起控件。
  static const Color controlShadowSoft = Color(0x14000000);

  /// 控件阴影色（深，黑 0.16）——开关旋钮等强浮起控件。
  static const Color controlShadowStrong = Color(0x29000000);

  // ------ Flutter 平台补充（CMP 无对应项，供 ThemeData/滚动条使用） ------
  /// 滚动条滑块色。
  static const Color scrollbarThumb = Color(0x33000000);

  /// 分隔线色。
  static const Color divider = Color(0x1A000000);
}
