// =============================================================================
// Layer 1: 尺寸/度量 token —— 按 UI 职责拆分的独立度量值。
// 逐值对齐 CMP DesignTokens.kt 的 METRICS 及各 create*Metrics()。
// 相同数值的不同职责保留独立字段，单位为逻辑像素（dp）。
// =============================================================================

/// 按钮各变体的独立尺寸。
class ButtonMetrics {
  /// 主按钮高度。
  final double primaryHeight;

  /// 软色按钮高度。
  final double softHeight;

  /// 文字按钮高度。
  final double textHeight;

  /// 图标文字按钮高度。
  final double iconTextHeight;

  /// 图标按钮尺寸。
  final double iconButtonSize;

  /// 主按钮圆角。
  final double primaryRadius;

  /// 软色按钮圆角。
  final double softRadius;

  /// 文字按钮圆角。
  final double textRadius;

  /// 图标文字按钮圆角。
  final double iconTextRadius;

  /// 图标按钮图标尺寸。
  final double iconVariantIconSize;

  /// 图标文字按钮图标尺寸。
  final double iconTextVariantIconSize;

  /// 软色按钮图标尺寸。
  final double softVariantIconSize;

  /// 主按钮图标尺寸。
  final double primaryVariantIconSize;

  /// 文字按钮图标尺寸。
  final double textVariantIconSize;

  /// 图标文字按钮水平内边距。
  final double iconTextHorizontalPadding;

  /// 文字按钮水平内边距。
  final double textHorizontalPadding;

  /// 软色按钮水平内边距。
  final double softHorizontalPadding;

  /// 主按钮水平内边距。
  final double primaryHorizontalPadding;

  /// 主按钮阴影高度。
  final double primaryShadowElevation;

  /// 加载指示器尺寸。
  final double loadingSpinnerSize;

  /// 加载指示器与文字间距。
  final double loadingLabelSpacing;

  /// 图标与文字间距。
  final double iconLabelSpacing;

  /// 角标起始内边距。
  final double badgeStartPadding;

  /// 角标水平内边距。
  final double badgeHorizontalPadding;

  /// 角标垂直内边距。
  final double badgeVerticalPadding;

  /// 角标高度。
  final double badgeHeight;

  /// 主按钮无图标时的占位尺寸。
  final double primaryWithoutIconSize;

  /// 文字按钮无图标时的占位尺寸。
  final double textWithoutIconSize;

  /// 图标按钮水平内边距。
  final double iconHorizontalPadding;

  /// 危险按钮按下背景透明度。
  final double dangerPressedAlpha;

  /// 主按钮阴影透明度。
  final double primaryShadowAlpha;

  /// 禁用按钮整体透明度。
  final double disabledAlpha;

  /// 按钮加载指示器轨道透明度。
  final double spinnerTrackAlpha;

  /// 按钮加载指示器旋转一周时长（毫秒）。
  final int spinnerRotationDurationMillis;

  const ButtonMetrics({
    required this.primaryHeight,
    required this.softHeight,
    required this.textHeight,
    required this.iconTextHeight,
    required this.iconButtonSize,
    required this.primaryRadius,
    required this.softRadius,
    required this.textRadius,
    required this.iconTextRadius,
    required this.iconVariantIconSize,
    required this.iconTextVariantIconSize,
    required this.softVariantIconSize,
    required this.primaryVariantIconSize,
    required this.textVariantIconSize,
    required this.iconTextHorizontalPadding,
    required this.textHorizontalPadding,
    required this.softHorizontalPadding,
    required this.primaryHorizontalPadding,
    required this.primaryShadowElevation,
    required this.loadingSpinnerSize,
    required this.loadingLabelSpacing,
    required this.iconLabelSpacing,
    required this.badgeStartPadding,
    required this.badgeHorizontalPadding,
    required this.badgeVerticalPadding,
    required this.badgeHeight,
    required this.primaryWithoutIconSize,
    required this.textWithoutIconSize,
    required this.iconHorizontalPadding,
    required this.dangerPressedAlpha,
    required this.primaryShadowAlpha,
    required this.disabledAlpha,
    required this.spinnerTrackAlpha,
    required this.spinnerRotationDurationMillis,
  });
}

/// 弹出菜单尺寸。
class MenuMetrics {
  /// 默认菜单宽度。
  final double defaultWidth;

  /// 菜单容器圆角。
  final double containerRadius;

  /// 菜单项高度。
  final double itemHeight;

  /// 菜单项圆角。
  final double itemRadius;

  const MenuMetrics({
    required this.defaultWidth,
    required this.containerRadius,
    required this.itemHeight,
    required this.itemRadius,
  });
}

/// 表单控件内部尺寸；字段只影响名称对应的具体职责。
class FormControlMetrics {
  /// 文本输入框边框宽度。
  final double textFieldBorderWidth;

  /// 文本输入框水平内边距。
  final double textFieldHorizontalPadding;

  /// 文本输入框内容间距。
  final double textFieldContentSpacing;

  /// 文本输入框前缀图标尺寸。
  final double textFieldPrefixIconSize;

  /// 数字输入框边框宽度。
  final double numberFieldBorderWidth;

  /// 数字输入框水平内边距。
  final double numberFieldHorizontalPadding;

  /// 数字输入框内容间距。
  final double numberFieldContentSpacing;

  /// 数字输入区域宽度。
  final double numberFieldInputWidth;

  /// 步进器容器圆角。
  final double stepperRadius;

  /// 步进器容器内边距。
  final double stepperPadding;

  /// 步进器减号图标尺寸。
  final double stepperMinusIconSize;

  /// 步进器数值区域宽度。
  final double stepperValueWidth;

  /// 步进器按钮尺寸。
  final double stepperButtonSize;

  /// 步进器按钮按下阴影高度。
  final double stepperButtonShadowElevation;

  /// 步进器按钮静止阴影高度。
  final double stepperButtonIdleElevation;

  /// 步进器按钮圆角。
  final double stepperButtonRadius;

  /// 搜索框不限制宽度时的占位宽度。
  final double searchUnboundedWidth;

  /// 开关选中时滑块偏移。
  final double switchCheckedKnobOffset;

  /// 开关未选中时滑块偏移。
  final double switchUncheckedKnobOffset;

  /// 开关宽度。
  final double switchWidth;

  /// 开关高度。
  final double switchHeight;

  /// 开关轨道圆角。
  final double switchRadius;

  /// 开关滑块尺寸。
  final double switchKnobSize;

  /// 开关滑块阴影高度。
  final double switchKnobShadowElevation;

  /// 复选框默认尺寸。
  final double checkboxDefaultSize;

  /// 复选框圆角。
  final double checkboxRadius;

  /// 复选框边框宽度。
  final double checkboxBorderWidth;

  /// 复选框勾选标记内缩量。
  final double checkboxCheckInset;

  /// 复选框不确定标记内缩量。
  final double checkboxIndeterminateInset;

  /// 复选框不确定标记高度。
  final double checkboxIndeterminateHeight;

  /// 复选框不确定标记圆角。
  final double checkboxIndeterminateRadius;

  /// 单选框默认尺寸。
  final double radioDefaultSize;

  /// 单选框边框宽度。
  final double radioBorderWidth;

  /// 禁用文本输入框透明度。
  final double textFieldDisabledAlpha;

  /// 禁用步进器按钮透明度。
  final double stepperDisabledAlpha;

  /// 禁用开关透明度。
  final double switchDisabledAlpha;

  /// 禁用复选框透明度。
  final double checkboxDisabledAlpha;

  /// 禁用单选框透明度。
  final double radioDisabledAlpha;

  const FormControlMetrics({
    required this.textFieldBorderWidth,
    required this.textFieldHorizontalPadding,
    required this.textFieldContentSpacing,
    required this.textFieldPrefixIconSize,
    required this.numberFieldBorderWidth,
    required this.numberFieldHorizontalPadding,
    required this.numberFieldContentSpacing,
    required this.numberFieldInputWidth,
    required this.stepperRadius,
    required this.stepperPadding,
    required this.stepperMinusIconSize,
    required this.stepperValueWidth,
    required this.stepperButtonSize,
    required this.stepperButtonShadowElevation,
    required this.stepperButtonIdleElevation,
    required this.stepperButtonRadius,
    required this.searchUnboundedWidth,
    required this.switchCheckedKnobOffset,
    required this.switchUncheckedKnobOffset,
    required this.switchWidth,
    required this.switchHeight,
    required this.switchRadius,
    required this.switchKnobSize,
    required this.switchKnobShadowElevation,
    required this.checkboxDefaultSize,
    required this.checkboxRadius,
    required this.checkboxBorderWidth,
    required this.checkboxCheckInset,
    required this.checkboxIndeterminateInset,
    required this.checkboxIndeterminateHeight,
    required this.checkboxIndeterminateRadius,
    required this.radioDefaultSize,
    required this.radioBorderWidth,
    required this.textFieldDisabledAlpha,
    required this.stepperDisabledAlpha,
    required this.switchDisabledAlpha,
    required this.checkboxDisabledAlpha,
    required this.radioDisabledAlpha,
  });
}

/// 表单控件尺寸。
class FormMetrics {
  /// 文本框高度。
  final double textFieldHeight;

  /// 文本框圆角。
  final double textFieldRadius;

  /// 数值框高度。
  final double numberFieldHeight;

  /// 数值框圆角。
  final double numberFieldRadius;

  /// 搜索框高度。
  final double searchFieldHeight;

  /// 步进器高度。
  final double stepperHeight;

  /// 表单控件内部尺寸。
  final FormControlMetrics controls;

  const FormMetrics({
    required this.textFieldHeight,
    required this.textFieldRadius,
    required this.numberFieldHeight,
    required this.numberFieldRadius,
    required this.searchFieldHeight,
    required this.stepperHeight,
    required this.controls,
  });
}

/// 导航控件尺寸。
class NavigationMetrics {
  /// 侧边栏项高度。
  final double sidebarItemHeight;

  /// 侧边栏项圆角。
  final double sidebarItemRadius;

  /// 面包屑高度。
  final double breadcrumbHeight;

  /// 面包屑水平内边距。
  final double breadcrumbHorizontalPadding;

  /// 面包屑项间距。
  final double breadcrumbItemSpacing;

  const NavigationMetrics({
    required this.sidebarItemHeight,
    required this.sidebarItemRadius,
    required this.breadcrumbHeight,
    required this.breadcrumbHorizontalPadding,
    required this.breadcrumbItemSpacing,
  });
}

/// 反馈组件内部尺寸；每个字段只控制名称对应的视觉职责。
class FeedbackControlMetrics {
  /// 线性进度条默认高度。
  final double linearProgressHeight;

  /// 环形进度条默认尺寸。
  final double circularProgressSize;

  /// 环形进度条线宽。
  final double circularProgressStrokeWidth;

  /// 横幅水平内边距。
  final double bannerHorizontalPadding;

  /// 横幅垂直内边距。
  final double bannerVerticalPadding;

  /// 横幅内容间距。
  final double bannerContentSpacing;

  /// 横幅状态图标尺寸。
  final double bannerIconSize;

  /// 横幅关闭图标尺寸。
  final double bannerCloseIconSize;

  /// 小标签水平内边距。
  final double smallTagHorizontalPadding;

  /// 中标签水平内边距。
  final double mediumTagHorizontalPadding;

  /// 小标签垂直内边距。
  final double smallTagVerticalPadding;

  /// 中标签垂直内边距。
  final double mediumTagVerticalPadding;

  /// 小标签图标尺寸。
  final double smallTagIconSize;

  /// 中标签图标尺寸。
  final double mediumTagIconSize;

  /// 标签图标与文字间距。
  final double tagContentSpacing;

  /// 空状态整体内边距。
  final double emptyStatePadding;

  /// 空状态图标尺寸。
  final double emptyStateIconSize;

  /// 空状态图标与标题间距。
  final double emptyStateTitleSpacing;

  /// 空状态标题与说明间距。
  final double emptyStateDescriptionSpacing;

  /// 空状态说明与操作间距。
  final double emptyStateActionSpacing;

  /// 统计标签圆角。
  final double statChipRadius;

  /// 统计标签水平内边距。
  final double statChipHorizontalPadding;

  /// 统计标签垂直内边距。
  final double statChipVerticalPadding;

  /// 统计标签内容间距。
  final double statChipContentSpacing;

  /// 统计标签图标尺寸。
  final double statChipIconSize;

  /// 区块标题底部内边距。
  final double sectionHeaderBottomPadding;

  /// 区块标题内容间距。
  final double sectionHeaderContentSpacing;

  /// 区块标题图标尺寸。
  final double sectionHeaderIconSize;

  /// 导航项高度。
  final double navigationItemHeight;

  /// 导航项圆角。
  final double navigationItemRadius;

  /// 导航项水平内边距。
  final double navigationItemHorizontalPadding;

  /// 导航项每层缩进量。
  final double navigationItemIndentPerLevel;

  /// 导航项内容间距。
  final double navigationItemContentSpacing;

  /// 导航项图标尺寸。
  final double navigationItemIconSize;

  /// 导航分组标签起始内边距。
  final double navigationGroupStartPadding;

  /// 导航分组标签顶部内边距。
  final double navigationGroupTopPadding;

  /// 导航分组标签底部内边距。
  final double navigationGroupBottomPadding;

  /// 标签图标透明度。
  final double tagIconAlpha;

  /// 环形进度指示器旋转一周时长（毫秒）。
  final int circularProgressRotationDurationMillis;

  const FeedbackControlMetrics({
    required this.linearProgressHeight,
    required this.circularProgressSize,
    required this.circularProgressStrokeWidth,
    required this.bannerHorizontalPadding,
    required this.bannerVerticalPadding,
    required this.bannerContentSpacing,
    required this.bannerIconSize,
    required this.bannerCloseIconSize,
    required this.smallTagHorizontalPadding,
    required this.mediumTagHorizontalPadding,
    required this.smallTagVerticalPadding,
    required this.mediumTagVerticalPadding,
    required this.smallTagIconSize,
    required this.mediumTagIconSize,
    required this.tagContentSpacing,
    required this.emptyStatePadding,
    required this.emptyStateIconSize,
    required this.emptyStateTitleSpacing,
    required this.emptyStateDescriptionSpacing,
    required this.emptyStateActionSpacing,
    required this.statChipRadius,
    required this.statChipHorizontalPadding,
    required this.statChipVerticalPadding,
    required this.statChipContentSpacing,
    required this.statChipIconSize,
    required this.sectionHeaderBottomPadding,
    required this.sectionHeaderContentSpacing,
    required this.sectionHeaderIconSize,
    required this.navigationItemHeight,
    required this.navigationItemRadius,
    required this.navigationItemHorizontalPadding,
    required this.navigationItemIndentPerLevel,
    required this.navigationItemContentSpacing,
    required this.navigationItemIconSize,
    required this.navigationGroupStartPadding,
    required this.navigationGroupTopPadding,
    required this.navigationGroupBottomPadding,
    required this.tagIconAlpha,
    required this.circularProgressRotationDurationMillis,
  });
}

/// 提示类组件尺寸。
class FeedbackMetrics {
  /// 横幅圆角。
  final double bannerRadius;

  /// 小标签圆角。
  final double smallTagRadius;

  /// 中标签圆角。
  final double mediumTagRadius;

  /// 空状态徽章尺寸。
  final double emptyBadgeSize;

  /// 空状态徽章圆角。
  final double emptyBadgeRadius;

  /// 反馈组件内部尺寸。
  final FeedbackControlMetrics controls;

  const FeedbackMetrics({
    required this.bannerRadius,
    required this.smallTagRadius,
    required this.mediumTagRadius,
    required this.emptyBadgeSize,
    required this.emptyBadgeRadius,
    required this.controls,
  });
}

/// 对话框和 Toast 尺寸。
class DialogMetrics {
  /// 对话框圆角。
  final double containerRadius;

  /// 标题图标徽章尺寸。
  final double iconBadgeSize;

  /// 标题图标徽章圆角。
  final double iconBadgeRadius;

  /// Toast 圆角。
  final double toastRadius;

  const DialogMetrics({
    required this.containerRadius,
    required this.iconBadgeSize,
    required this.iconBadgeRadius,
    required this.toastRadius,
  });
}

/// 文件列表内部尺寸。
class FileListControlMetrics {
  /// 文件大小列初始宽度。
  final double sizeColumnInitialWidth;

  /// 修改时间列初始宽度。
  final double timeColumnInitialWidth;

  /// 可调整列的最小宽度。
  final double resizableColumnMinimumWidth;

  /// 可调整列的最大宽度。
  final double resizableColumnMaximumWidth;

  /// 批量操作栏水平外边距。
  final double bulkBarHorizontalMargin;

  /// 批量操作栏顶部外边距。
  final double bulkBarTopMargin;

  /// 批量操作栏高度。
  final double bulkBarHeight;

  /// 批量操作栏圆角。
  final double bulkBarRadius;

  /// 批量操作栏起始内边距。
  final double bulkBarStartPadding;

  /// 批量操作栏结束内边距。
  final double bulkBarEndPadding;

  /// 批量操作栏内容间距。
  final double bulkBarContentSpacing;

  /// 表格水平内边距。
  final double tableHorizontalPadding;

  /// 表头高度。
  final double headerHeight;

  /// 表头单元格水平内边距。
  final double headerHorizontalPadding;

  /// 复选框列宽度。
  final double checkboxColumnWidth;

  /// 状态列宽度。
  final double statusColumnWidth;

  /// 操作列宽度。
  final double actionColumnWidth;

  /// 表格底栏高度。
  final double footerHeight;

  /// 批量操作按钮高度。
  final double bulkActionHeight;

  /// 批量操作按钮圆角。
  final double bulkActionRadius;

  /// 批量操作按钮水平内边距。
  final double bulkActionHorizontalPadding;

  /// 批量操作按钮内容间距。
  final double bulkActionContentSpacing;

  /// 批量操作按钮图标尺寸。
  final double bulkActionIconSize;

  /// 批量操作栏关闭按钮尺寸。
  final double bulkCloseSize;

  /// 批量操作栏关闭图标尺寸。
  final double bulkCloseIconSize;

  /// 表头文字与排序图标间距。
  final double headerSortSpacing;

  /// 表头排序图标尺寸。
  final double headerSortIconSize;

  /// 列宽拖动手柄宽度。
  final double resizeHandleWidth;

  /// 文件行高度。
  final double rowHeight;

  /// 文件行圆角。
  final double rowRadius;

  /// 文件行水平内边距。
  final double rowHorizontalPadding;

  /// 文件图标与名称间距。
  final double rowNameContentSpacing;

  /// 文件状态图标尺寸。
  final double rowStatusIconSize;

  /// 右键菜单宽度。
  final double contextMenuWidth;

  /// 右键菜单阴影高度。
  final double contextMenuShadowElevation;

  /// 右键菜单圆角。
  final double contextMenuRadius;

  /// 右键菜单边框宽度。
  final double contextMenuBorderWidth;

  /// 右键菜单内边距。
  final double contextMenuPadding;

  /// 文件缩略图尺寸。
  final double thumbnailSize;

  /// 文件缩略图圆角。
  final double thumbnailRadius;

  /// 文件类型图标尺寸。
  final double fileTypeIconSize;

  /// 右键菜单操作项高度。
  final double contextActionHeight;

  /// 右键菜单操作项圆角。
  final double contextActionRadius;

  /// 右键菜单操作项水平内边距。
  final double contextActionHorizontalPadding;

  /// 右键菜单操作项内容间距。
  final double contextActionContentSpacing;

  /// 右键菜单操作项图标尺寸。
  final double contextActionIconSize;

  /// 右键菜单分隔线水平内边距。
  final double contextDividerHorizontalPadding;

  /// 右键菜单分隔线垂直内边距。
  final double contextDividerVerticalPadding;

  /// 右键菜单分隔线高度。
  final double contextDividerHeight;

  /// 禁用批量操作按钮透明度。
  final double bulkActionDisabledAlpha;

  const FileListControlMetrics({
    required this.sizeColumnInitialWidth,
    required this.timeColumnInitialWidth,
    required this.resizableColumnMinimumWidth,
    required this.resizableColumnMaximumWidth,
    required this.bulkBarHorizontalMargin,
    required this.bulkBarTopMargin,
    required this.bulkBarHeight,
    required this.bulkBarRadius,
    required this.bulkBarStartPadding,
    required this.bulkBarEndPadding,
    required this.bulkBarContentSpacing,
    required this.tableHorizontalPadding,
    required this.headerHeight,
    required this.headerHorizontalPadding,
    required this.checkboxColumnWidth,
    required this.statusColumnWidth,
    required this.actionColumnWidth,
    required this.footerHeight,
    required this.bulkActionHeight,
    required this.bulkActionRadius,
    required this.bulkActionHorizontalPadding,
    required this.bulkActionContentSpacing,
    required this.bulkActionIconSize,
    required this.bulkCloseSize,
    required this.bulkCloseIconSize,
    required this.headerSortSpacing,
    required this.headerSortIconSize,
    required this.resizeHandleWidth,
    required this.rowHeight,
    required this.rowRadius,
    required this.rowHorizontalPadding,
    required this.rowNameContentSpacing,
    required this.rowStatusIconSize,
    required this.contextMenuWidth,
    required this.contextMenuShadowElevation,
    required this.contextMenuRadius,
    required this.contextMenuBorderWidth,
    required this.contextMenuPadding,
    required this.thumbnailSize,
    required this.thumbnailRadius,
    required this.fileTypeIconSize,
    required this.contextActionHeight,
    required this.contextActionRadius,
    required this.contextActionHorizontalPadding,
    required this.contextActionContentSpacing,
    required this.contextActionIconSize,
    required this.contextDividerHorizontalPadding,
    required this.contextDividerVerticalPadding,
    required this.contextDividerHeight,
    required this.bulkActionDisabledAlpha,
  });
}

/// 文件列表专属尺寸。重命名和移动对话框即使当前视觉数值一致，也分别保留变量。
class FileListMetrics {
  /// 重命名弹窗宽度。
  final double renameDialogWidth;

  /// 重命名弹窗圆角。
  final double renameDialogRadius;

  /// 重命名弹窗内边距。
  final double renameDialogPadding;

  /// 重命名弹窗内容间距。
  final double renameDialogContentSpacing;

  /// 重命名弹窗操作间距。
  final double renameDialogActionSpacing;

  /// 移动弹窗宽度。
  final double moveDialogWidth;

  /// 移动弹窗圆角。
  final double moveDialogRadius;

  /// 移动弹窗内边距。
  final double moveDialogPadding;

  /// 移动弹窗内容间距。
  final double moveDialogContentSpacing;

  /// 目标目录列表高度。
  final double moveDialogFolderListHeight;

  /// 目标目录项圆角。
  final double moveDialogFolderRadius;

  /// 目标目录项内边距。
  final double moveDialogFolderPadding;

  /// 目标目录项内容间距。
  final double moveDialogFolderContentSpacing;

  /// 目标目录图标尺寸。
  final double moveDialogFolderIconSize;

  /// 移动弹窗操作间距。
  final double moveDialogActionSpacing;

  /// 文件列表内部尺寸。
  final FileListControlMetrics controls;

  const FileListMetrics({
    required this.renameDialogWidth,
    required this.renameDialogRadius,
    required this.renameDialogPadding,
    required this.renameDialogContentSpacing,
    required this.renameDialogActionSpacing,
    required this.moveDialogWidth,
    required this.moveDialogRadius,
    required this.moveDialogPadding,
    required this.moveDialogContentSpacing,
    required this.moveDialogFolderListHeight,
    required this.moveDialogFolderRadius,
    required this.moveDialogFolderPadding,
    required this.moveDialogFolderContentSpacing,
    required this.moveDialogFolderIconSize,
    required this.moveDialogActionSpacing,
    required this.controls,
  });
}

/// 基础品牌组件尺寸。
class BasicMetrics {
  /// 紧凑 Logo 默认尺寸。
  final double compactLogoSize;

  /// 登录页 Logo 尺寸。
  final double largeLogoSize;

  /// 紧凑 Logo 与文字间距。
  final double compactLogoTextSpacing;

  /// 完整 Logo 默认高度。
  final double fullLogoHeight;

  /// 完整 Logo 与文字间距。
  final double fullLogoTextSpacing;

  /// 竖分隔线默认高度。
  final double verticalSeparatorHeight;

  /// 竖分隔线宽度。
  final double verticalSeparatorWidth;

  /// 底部分隔线厚度。
  final double bottomBorderThickness;

  const BasicMetrics({
    required this.compactLogoSize,
    required this.largeLogoSize,
    required this.compactLogoTextSpacing,
    required this.fullLogoHeight,
    required this.fullLogoTextSpacing,
    required this.verticalSeparatorHeight,
    required this.verticalSeparatorWidth,
    required this.bottomBorderThickness,
  });
}

/// 图标基础尺寸。
class IconMetrics {
  /// 图标默认尺寸。
  final double defaultSize;

  /// 图标旋转一周时长（毫秒）。
  final int spinDurationMillis;

  const IconMetrics({
    required this.defaultSize,
    required this.spinDurationMillis,
  });
}

/// 分隔线尺寸。
class DividerMetrics {
  /// 横分隔线默认厚度。
  final double horizontalThickness;

  /// 竖分隔线默认高度。
  final double verticalHeight;

  /// 竖分隔线宽度。
  final double verticalWidth;

  const DividerMetrics({
    required this.horizontalThickness,
    required this.verticalHeight,
    required this.verticalWidth,
  });
}

/// 组件目录页面尺寸。
class CatalogMetrics {
  /// 页面内边距。
  final double pagePadding;

  /// 区块间距。
  final double sectionSpacing;

  /// 示例项间距。
  final double itemSpacing;

  /// 图标预览尺寸。
  final double iconPreviewSize;

  /// 输入框预览宽度。
  final double fieldPreviewWidth;

  /// 进度条预览宽度。
  final double progressPreviewWidth;

  /// 环形进度预览尺寸。
  final double circularProgressSize;

  /// 纵向示例间距。
  final double verticalGroupSpacing;

  /// 紧凑示例间距。
  final double compactItemSpacing;

  /// 弹窗示例宽度。
  final double dialogPreviewWidth;

  const CatalogMetrics({
    required this.pagePadding,
    required this.sectionSpacing,
    required this.itemSpacing,
    required this.iconPreviewSize,
    required this.fieldPreviewWidth,
    required this.progressPreviewWidth,
    required this.circularProgressSize,
    required this.verticalGroupSpacing,
    required this.compactItemSpacing,
    required this.dialogPreviewWidth,
  });
}

/// 同步状态栏尺寸。
class StatusBarMetrics {
  /// 最小高度。
  final double minimumHeight;

  /// 水平内边距。
  final double horizontalPadding;

  /// 垂直内边距。
  final double verticalPadding;

  /// 状态内容间距。
  final double statusContentSpacing;

  /// 同步图标尺寸。
  final double syncingIconSize;

  /// 空闲指示点尺寸。
  final double idleIndicatorSize;

  /// 操作区水平间距。
  final double actionHorizontalSpacing;

  /// 操作区垂直间距。
  final double actionVerticalSpacing;

  const StatusBarMetrics({
    required this.minimumHeight,
    required this.horizontalPadding,
    required this.verticalPadding,
    required this.statusContentSpacing,
    required this.syncingIconSize,
    required this.idleIndicatorSize,
    required this.actionHorizontalSpacing,
    required this.actionVerticalSpacing,
  });
}

/// 同步设置横幅尺寸。
class SyncSetupMetrics {
  /// 水平内边距。
  final double horizontalPadding;

  /// 垂直内边距。
  final double verticalPadding;

  const SyncSetupMetrics({
    required this.horizontalPadding,
    required this.verticalPadding,
  });
}

/// 登录页尺寸。
class LoginMetrics {
  /// 顶部装饰横向偏移。
  final double topDecorationOffsetX;

  /// 顶部装饰纵向偏移。
  final double topDecorationOffsetY;

  /// 顶部装饰尺寸。
  final double topDecorationSize;

  /// 底部装饰横向偏移。
  final double bottomDecorationOffsetX;

  /// 底部装饰纵向偏移。
  final double bottomDecorationOffsetY;

  /// 底部装饰尺寸。
  final double bottomDecorationSize;

  /// 中部装饰尺寸。
  final double centerDecorationSize;

  /// 登录卡片宽度。
  final double cardWidth;

  /// 登录卡片阴影高度。
  final double cardShadowElevation;

  /// 登录卡片圆角。
  final double cardRadius;

  /// 登录卡片水平内边距。
  final double cardHorizontalPadding;

  /// 登录卡片垂直内边距。
  final double cardVerticalPadding;

  /// Logo 与标题间距。
  final double logoTitleSpacing;

  /// 标题与副标题间距。
  final double subtitleSpacing;

  /// 标题强调线宽度。
  final double accentWidth;

  /// 标题强调线高度。
  final double accentHeight;

  /// 标题强调线圆角。
  final double accentRadius;

  /// 标题强调线底部间距。
  final double accentBottomSpacing;

  /// 登录消息间距。
  final double messageSpacing;

  /// 登录内容底部间距。
  final double contentBottomSpacing;

  /// 授权状态栏高度。
  final double authorizingHeight;

  /// 授权状态栏圆角。
  final double authorizingRadius;

  /// 授权状态栏水平内边距。
  final double authorizingHorizontalPadding;

  /// 授权状态栏内容间距。
  final double authorizingContentSpacing;

  /// 授权旋转图标尺寸。
  final double authorizingSpinnerSize;

  /// 授权旋转图标线宽。
  final double authorizingSpinnerStroke;

  /// 错误提示与操作间距。
  final double errorActionSpacing;

  /// 登录按钮高度。
  final double loginButtonHeight;

  /// 页脚顶部间距。
  final double footerSpacing;

  /// 顶部装饰透明度。
  final double topDecorationAlpha;

  /// 底部装饰透明度。
  final double bottomDecorationAlpha;

  /// 中部装饰透明度。
  final double centerDecorationAlpha;

  const LoginMetrics({
    required this.topDecorationOffsetX,
    required this.topDecorationOffsetY,
    required this.topDecorationSize,
    required this.bottomDecorationOffsetX,
    required this.bottomDecorationOffsetY,
    required this.bottomDecorationSize,
    required this.centerDecorationSize,
    required this.cardWidth,
    required this.cardShadowElevation,
    required this.cardRadius,
    required this.cardHorizontalPadding,
    required this.cardVerticalPadding,
    required this.logoTitleSpacing,
    required this.subtitleSpacing,
    required this.accentWidth,
    required this.accentHeight,
    required this.accentRadius,
    required this.accentBottomSpacing,
    required this.messageSpacing,
    required this.contentBottomSpacing,
    required this.authorizingHeight,
    required this.authorizingRadius,
    required this.authorizingHorizontalPadding,
    required this.authorizingContentSpacing,
    required this.authorizingSpinnerSize,
    required this.authorizingSpinnerStroke,
    required this.errorActionSpacing,
    required this.loginButtonHeight,
    required this.footerSpacing,
    required this.topDecorationAlpha,
    required this.bottomDecorationAlpha,
    required this.centerDecorationAlpha,
  });
}

/// 日志查看器尺寸。
class LogViewerMetrics {
  /// 内嵌模式标题栏高度。
  final double inlineHeaderHeight;

  /// 内嵌模式标题栏水平内边距。
  final double inlineHeaderHorizontalPadding;

  /// 内嵌模式标题栏内容间距。
  final double inlineHeaderContentSpacing;

  /// 独立模式标题栏水平内边距。
  final double standaloneHeaderHorizontalPadding;

  /// 独立模式标题栏垂直内边距。
  final double headerVerticalPadding;

  /// 独立模式标题栏内容间距。
  final double headerContentSpacing;

  /// 加载指示器尺寸。
  final double loadingSize;

  /// 内嵌模式内容内边距。
  final double inlineContentPadding;

  /// 独立模式内容内边距。
  final double standaloneContentPadding;

  /// 日志列表圆角。
  final double listRadius;

  /// 日志列表边框宽度。
  final double listBorderWidth;

  /// 日志记录水平内边距。
  final double recordHorizontalPadding;

  /// 日志记录垂直内边距。
  final double recordVerticalPadding;

  /// 日志记录内容间距。
  final double recordContentSpacing;

  /// 日志元信息顶部内边距。
  final double metadataTopPadding;

  const LogViewerMetrics({
    required this.inlineHeaderHeight,
    required this.inlineHeaderHorizontalPadding,
    required this.inlineHeaderContentSpacing,
    required this.standaloneHeaderHorizontalPadding,
    required this.headerVerticalPadding,
    required this.headerContentSpacing,
    required this.loadingSize,
    required this.inlineContentPadding,
    required this.standaloneContentPadding,
    required this.listRadius,
    required this.listBorderWidth,
    required this.recordHorizontalPadding,
    required this.recordVerticalPadding,
    required this.recordContentSpacing,
    required this.metadataTopPadding,
  });
}

/// 主页面尺寸。
class MainPageMetrics {
  /// 顶部应用栏高度。
  final double appBarHeight;

  /// 顶部应用栏水平内边距。
  final double appBarHorizontalPadding;

  /// 搜索框最大宽度。
  final double searchMaximumWidth;

  /// 顶部应用栏操作间距。
  final double appBarActionSpacing;

  /// 页面加载指示器尺寸。
  final double loadingSize;

  /// 搜索结果面板起始内边距。
  final double searchPanelStartPadding;

  /// 搜索结果面板顶部内边距。
  final double searchPanelTopPadding;

  /// 搜索结果面板结束内边距。
  final double searchPanelEndPadding;

  /// 搜索结果面板底部内边距。
  final double searchPanelBottomPadding;

  /// 搜索结果项高度。
  final double searchResultHeight;

  /// 搜索结果项水平内边距。
  final double searchResultHorizontalPadding;

  /// 搜索结果项内容间距。
  final double searchResultContentSpacing;

  /// 搜索结果图标容器尺寸。
  final double searchResultIconContainerSize;

  /// 搜索结果图标容器圆角。
  final double searchResultIconRadius;

  /// 搜索结果图标尺寸。
  final double searchResultIconSize;

  const MainPageMetrics({
    required this.appBarHeight,
    required this.appBarHorizontalPadding,
    required this.searchMaximumWidth,
    required this.appBarActionSpacing,
    required this.loadingSize,
    required this.searchPanelStartPadding,
    required this.searchPanelTopPadding,
    required this.searchPanelEndPadding,
    required this.searchPanelBottomPadding,
    required this.searchResultHeight,
    required this.searchResultHorizontalPadding,
    required this.searchResultContentSpacing,
    required this.searchResultIconContainerSize,
    required this.searchResultIconRadius,
    required this.searchResultIconSize,
  });
}

/// 弹出菜单、对话框和 Toast 的内部尺寸。
class OverlayMetrics {
  /// 弹出菜单边框宽度。
  final double menuBorderWidth;

  /// 弹出菜单内边距。
  final double menuPadding;

  /// 菜单分隔线水平内边距。
  final double menuDividerHorizontalPadding;

  /// 菜单分隔线垂直内边距。
  final double menuDividerVerticalPadding;

  /// 菜单分隔线高度。
  final double menuDividerHeight;

  /// 菜单项水平内边距。
  final double menuItemHorizontalPadding;

  /// 菜单项内容间距。
  final double menuItemContentSpacing;

  /// 菜单项图标尺寸。
  final double menuItemIconSize;

  /// 对话框标题区水平内边距。
  final double dialogHeaderHorizontalPadding;

  /// 对话框标题区顶部内边距。
  final double dialogHeaderTopPadding;

  /// 对话框标题区底部内边距。
  final double dialogHeaderBottomPadding;

  /// 对话框标题区内容间距。
  final double dialogHeaderContentSpacing;

  /// 对话框标题图标尺寸。
  final double dialogTitleIconSize;

  /// 对话框正文水平内边距。
  final double dialogBodyHorizontalPadding;

  /// 对话框正文顶部内边距。
  final double dialogBodyTopPadding;

  /// 对话框正文底部内边距。
  final double dialogBodyBottomPadding;

  /// 对话框操作区水平内边距。
  final double dialogFooterHorizontalPadding;

  /// 对话框操作区底部内边距。
  final double dialogFooterBottomPadding;

  /// 对话框操作按钮间距。
  final double dialogActionSpacing;

  /// Toast 外边距。
  final double toastOuterPadding;

  /// Toast 水平内边距。
  final double toastHorizontalPadding;

  /// Toast 垂直内边距。
  final double toastVerticalPadding;

  /// Toast 内容间距。
  final double toastContentSpacing;

  /// Toast 图标尺寸。
  final double toastIconSize;

  const OverlayMetrics({
    required this.menuBorderWidth,
    required this.menuPadding,
    required this.menuDividerHorizontalPadding,
    required this.menuDividerVerticalPadding,
    required this.menuDividerHeight,
    required this.menuItemHorizontalPadding,
    required this.menuItemContentSpacing,
    required this.menuItemIconSize,
    required this.dialogHeaderHorizontalPadding,
    required this.dialogHeaderTopPadding,
    required this.dialogHeaderBottomPadding,
    required this.dialogHeaderContentSpacing,
    required this.dialogTitleIconSize,
    required this.dialogBodyHorizontalPadding,
    required this.dialogBodyTopPadding,
    required this.dialogBodyBottomPadding,
    required this.dialogFooterHorizontalPadding,
    required this.dialogFooterBottomPadding,
    required this.dialogActionSpacing,
    required this.toastOuterPadding,
    required this.toastHorizontalPadding,
    required this.toastVerticalPadding,
    required this.toastContentSpacing,
    required this.toastIconSize,
  });
}

/// 设置页尺寸。
class SettingsMetrics {
  /// 顶部标题栏高度。
  final double headerHeight;

  /// 顶部标题栏水平内边距。
  final double headerHorizontalPadding;

  /// 顶部标题栏内容间距。
  final double headerContentSpacing;

  /// 左侧导航宽度。
  final double navigationWidth;

  /// 左侧导航水平内边距。
  final double navigationHorizontalPadding;

  /// 左侧导航垂直内边距。
  final double navigationVerticalPadding;

  /// 左侧导航项间距。
  final double navigationItemSpacing;

  /// 导航与内容区分隔线宽度。
  final double navigationBorderWidth;

  /// 设置内容水平内边距。
  final double bodyHorizontalPadding;

  /// 设置内容垂直内边距。
  final double bodyVerticalPadding;

  /// 并发设置控件间距。
  final double concurrencyContentSpacing;

  /// 跳过文件输入框宽度。
  final double skipPatternFieldWidth;

  /// OAuth 提示顶部间距。
  final double oauthBannerTopPadding;

  /// OAuth 提示底部间距。
  final double oauthBannerBottomPadding;

  /// 日志面板内边距。
  final double logPanelPadding;

  /// 日志面板内容间距。
  final double logPanelContentSpacing;

  /// 底部操作栏分隔线宽度。
  final double footerBorderWidth;

  /// 底部操作栏高度。
  final double footerHeight;

  /// 底部操作栏水平内边距。
  final double footerHorizontalPadding;

  /// 底部操作项间距。
  final double footerActionSpacing;

  /// 保存成功内容间距。
  final double savedIndicatorSpacing;

  /// 保存成功指示点尺寸。
  final double savedIndicatorSize;

  /// 设置卡片默认水平内边距。
  final double panelHorizontalPadding;

  /// 设置卡片默认垂直内边距。
  final double panelVerticalPadding;

  /// 设置卡片默认内容间距。
  final double panelDefaultContentSpacing;

  /// 设置卡片圆角。
  final double panelRadius;

  /// 设置卡片边框宽度。
  final double panelBorderWidth;

  /// 首个分组标题顶部间距。
  final double firstGroupTopPadding;

  /// 后续分组标题顶部间距。
  final double groupTopPadding;

  /// 分组标题底部间距。
  final double groupBottomPadding;

  /// 设置行垂直内边距。
  final double settingRowVerticalPadding;

  /// 设置行内容间距。
  final double settingRowContentSpacing;

  /// 设置说明顶部间距。
  final double settingDescriptionTopPadding;

  /// 设置行分隔线宽度。
  final double settingRowDividerWidth;

  /// 同步目录卡片圆角。
  final double mountPanelRadius;

  /// 已配置目录边框宽度。
  final double configuredMountBorderWidth;

  /// 未配置目录边框宽度。
  final double emptyMountBorderWidth;

  /// 同步目录卡片水平内边距。
  final double mountPanelHorizontalPadding;

  /// 已配置目录卡片垂直内边距。
  final double configuredMountVerticalPadding;

  /// 未配置目录卡片垂直内边距。
  final double emptyMountVerticalPadding;

  /// 同步目录卡片内容间距。
  final double mountPanelContentSpacing;

  /// 未配置目录徽章尺寸。
  final double emptyMountBadgeSize;

  /// 未配置目录徽章圆角。
  final double emptyMountBadgeRadius;

  /// 未配置目录图标尺寸。
  final double emptyMountIconSize;

  /// 已配置目录徽章尺寸。
  final double configuredMountBadgeSize;

  /// 已配置目录徽章圆角。
  final double configuredMountBadgeRadius;

  /// 已配置目录图标尺寸。
  final double configuredMountIconSize;

  /// 同步目录路径背景圆角。
  final double mountPathRadius;

  /// 同步目录路径水平内边距。
  final double mountPathHorizontalPadding;

  /// 同步目录路径垂直内边距。
  final double mountPathVerticalPadding;

  /// 同步目录提示间距。
  final double mountBannerSpacing;

  /// 账号卡片水平内边距。
  final double accountPanelHorizontalPadding;

  /// 账号卡片垂直内边距。
  final double accountPanelVerticalPadding;

  /// 账号头像与名称间距。
  final double accountContentSpacing;

  /// 账号头像尺寸。
  final double accountAvatarSize;

  /// 账号区块间距。
  final double accountSectionSpacing;

  /// 详情行垂直内边距。
  final double detailRowVerticalPadding;

  /// 详情标签宽度。
  final double detailLabelWidth;

  /// 详情内容底部间距。
  final double detailContentSpacing;

  /// 详情分隔线宽度。
  final double detailDividerWidth;

  /// 关于卡片内边距。
  final double aboutPanelPadding;

  /// 关于卡片内容间距。
  final double aboutPanelContentSpacing;

  /// 关于区域 Logo 高度。
  final double aboutLogoHeight;

  /// 版本信息间距。
  final double versionContentSpacing;

  /// 外链之间的间距。
  final double externalLinksSpacing;

  /// 外链垂直内边距。
  final double externalLinkVerticalPadding;

  /// 外链图标文字间距。
  final double externalLinkContentSpacing;

  /// 外链图标尺寸。
  final double externalLinkIconSize;

  const SettingsMetrics({
    required this.headerHeight,
    required this.headerHorizontalPadding,
    required this.headerContentSpacing,
    required this.navigationWidth,
    required this.navigationHorizontalPadding,
    required this.navigationVerticalPadding,
    required this.navigationItemSpacing,
    required this.navigationBorderWidth,
    required this.bodyHorizontalPadding,
    required this.bodyVerticalPadding,
    required this.concurrencyContentSpacing,
    required this.skipPatternFieldWidth,
    required this.oauthBannerTopPadding,
    required this.oauthBannerBottomPadding,
    required this.logPanelPadding,
    required this.logPanelContentSpacing,
    required this.footerBorderWidth,
    required this.footerHeight,
    required this.footerHorizontalPadding,
    required this.footerActionSpacing,
    required this.savedIndicatorSpacing,
    required this.savedIndicatorSize,
    required this.panelHorizontalPadding,
    required this.panelVerticalPadding,
    required this.panelDefaultContentSpacing,
    required this.panelRadius,
    required this.panelBorderWidth,
    required this.firstGroupTopPadding,
    required this.groupTopPadding,
    required this.groupBottomPadding,
    required this.settingRowVerticalPadding,
    required this.settingRowContentSpacing,
    required this.settingDescriptionTopPadding,
    required this.settingRowDividerWidth,
    required this.mountPanelRadius,
    required this.configuredMountBorderWidth,
    required this.emptyMountBorderWidth,
    required this.mountPanelHorizontalPadding,
    required this.configuredMountVerticalPadding,
    required this.emptyMountVerticalPadding,
    required this.mountPanelContentSpacing,
    required this.emptyMountBadgeSize,
    required this.emptyMountBadgeRadius,
    required this.emptyMountIconSize,
    required this.configuredMountBadgeSize,
    required this.configuredMountBadgeRadius,
    required this.configuredMountIconSize,
    required this.mountPathRadius,
    required this.mountPathHorizontalPadding,
    required this.mountPathVerticalPadding,
    required this.mountBannerSpacing,
    required this.accountPanelHorizontalPadding,
    required this.accountPanelVerticalPadding,
    required this.accountContentSpacing,
    required this.accountAvatarSize,
    required this.accountSectionSpacing,
    required this.detailRowVerticalPadding,
    required this.detailLabelWidth,
    required this.detailContentSpacing,
    required this.detailDividerWidth,
    required this.aboutPanelPadding,
    required this.aboutPanelContentSpacing,
    required this.aboutLogoHeight,
    required this.versionContentSpacing,
    required this.externalLinksSpacing,
    required this.externalLinkVerticalPadding,
    required this.externalLinkContentSpacing,
    required this.externalLinkIconSize,
  });
}

/// 更新弹窗尺寸。
class UpdateDialogMetrics {
  /// 弹窗宽度。
  final double dialogWidth;

  /// 弹窗阴影高度。
  final double dialogShadowElevation;

  /// 弹窗圆角。
  final double dialogRadius;

  /// 头部水平内边距。
  final double headerHorizontalPadding;

  /// 头部顶部内边距。
  final double headerTopPadding;

  /// 头部底部内边距。
  final double headerBottomPadding;

  /// 头部内容间距。
  final double headerContentSpacing;

  /// 头部徽章尺寸。
  final double headerBadgeSize;

  /// 头部徽章圆角。
  final double headerBadgeRadius;

  /// 头部图标尺寸。
  final double headerIconSize;

  /// 版本号水平内边距。
  final double versionHorizontalPadding;

  /// 正文水平内边距。
  final double bodyHorizontalPadding;

  /// 正文顶部内边距。
  final double bodyTopPadding;

  /// 正文底部内边距。
  final double bodyBottomPadding;

  /// 底部水平内边距。
  final double footerHorizontalPadding;

  /// 底部顶部内边距。
  final double footerTopPadding;

  /// 底部底部内边距。
  final double footerBottomPadding;

  /// 更新说明标签间距。
  final double releaseNotesLabelSpacing;

  /// 更新说明最大高度。
  final double releaseNotesMaximumHeight;

  /// 更新说明圆角。
  final double releaseNotesRadius;

  /// 更新说明内边距。
  final double releaseNotesPadding;

  /// 进度内容间距。
  final double progressContentSpacing;

  /// 进度轨道高度。
  final double progressTrackHeight;

  /// 进度轨道圆角。
  final double progressTrackRadius;

  /// 进度填充高度。
  final double progressFillHeight;

  /// 进度填充圆角。
  final double progressFillRadius;

  /// 等待内容间距。
  final double waitingContentSpacing;

  /// 加载容器尺寸。
  final double spinnerContainerSize;

  /// 加载顶部内边距。
  final double spinnerTopPadding;

  /// 加载圆环尺寸。
  final double spinnerRingSize;

  /// 加载圆环线宽。
  final double spinnerRingStrokeWidth;

  /// 底部操作间距。
  final double footerActionSpacing;

  /// 加载旋转一周时长（毫秒）。
  final int spinnerRotationDurationMillis;

  const UpdateDialogMetrics({
    required this.dialogWidth,
    required this.dialogShadowElevation,
    required this.dialogRadius,
    required this.headerHorizontalPadding,
    required this.headerTopPadding,
    required this.headerBottomPadding,
    required this.headerContentSpacing,
    required this.headerBadgeSize,
    required this.headerBadgeRadius,
    required this.headerIconSize,
    required this.versionHorizontalPadding,
    required this.bodyHorizontalPadding,
    required this.bodyTopPadding,
    required this.bodyBottomPadding,
    required this.footerHorizontalPadding,
    required this.footerTopPadding,
    required this.footerBottomPadding,
    required this.releaseNotesLabelSpacing,
    required this.releaseNotesMaximumHeight,
    required this.releaseNotesRadius,
    required this.releaseNotesPadding,
    required this.progressContentSpacing,
    required this.progressTrackHeight,
    required this.progressTrackRadius,
    required this.progressFillHeight,
    required this.progressFillRadius,
    required this.waitingContentSpacing,
    required this.spinnerContainerSize,
    required this.spinnerTopPadding,
    required this.spinnerRingSize,
    required this.spinnerRingStrokeWidth,
    required this.footerActionSpacing,
    required this.spinnerRotationDurationMillis,
  });
}

/// 侧边栏尺寸。
class SidebarMetrics {
  /// 侧边栏宽度。
  final double width;

  /// Logo 头部高度。
  final double logoHeaderHeight;

  /// Logo 头部水平内边距。
  final double logoHeaderHorizontalPadding;

  /// Logo 尺寸。
  final double logoSize;

  /// 区块标签起始内边距。
  final double sectionLabelStartPadding;

  /// 区块标签顶部内边距。
  final double sectionLabelTopPadding;

  /// 区块标签底部内边距。
  final double sectionLabelBottomPadding;

  /// 目录树水平内边距。
  final double treeHorizontalPadding;

  /// 目录树垂直内边距。
  final double treeVerticalPadding;

  /// 账号卡片外边距。
  final double accountOuterPadding;

  /// 账号卡片圆角。
  final double accountRadius;

  /// 账号卡片边框宽度。
  final double accountBorderWidth;

  /// 账号卡片内边距。
  final double accountInnerPadding;

  /// 账号内容间距。
  final double accountContentSpacing;

  /// 账号头像尺寸。
  final double accountAvatarSize;

  /// 配额顶部内边距。
  final double accountQuotaTopPadding;

  /// 配额进度条间距。
  final double accountQuotaProgressSpacing;

  /// 配额进度条高度。
  final double accountQuotaProgressHeight;

  /// 更新卡水平外边距。
  final double updateCardHorizontalMargin;

  /// 更新卡底部外边距。
  final double updateCardBottomMargin;

  /// 更新卡圆角。
  final double updateCardRadius;

  /// 更新卡内边距。
  final double updateCardPadding;

  /// 下载进度间距。
  final double downloadProgressSpacing;

  /// 关闭按钮尺寸。
  final double dismissButtonSize;

  /// 可用更新操作间距。
  final double availableActionSpacing;

  /// 安装按钮高度。
  final double installButtonHeight;

  /// 安装按钮圆角。
  final double installButtonRadius;

  /// 目录树节点高度。
  final double treeNodeHeight;

  /// 目录树每层缩进。
  final double treeDepthIndent;

  /// 目录树节点起始内边距。
  final double treeNodeStartPadding;

  /// 目录树节点结束内边距。
  final double treeNodeEndPadding;

  /// 目录树节点圆角。
  final double treeNodeRadius;

  /// 目录树节点内容间距。
  final double treeNodeContentSpacing;

  /// 目录树展开器尺寸。
  final double treeExpanderSize;

  /// 目录树箭头图标尺寸。
  final double treeArrowIconSize;

  /// 目录树文件夹图标尺寸。
  final double treeFolderIconSize;

  const SidebarMetrics({
    required this.width,
    required this.logoHeaderHeight,
    required this.logoHeaderHorizontalPadding,
    required this.logoSize,
    required this.sectionLabelStartPadding,
    required this.sectionLabelTopPadding,
    required this.sectionLabelBottomPadding,
    required this.treeHorizontalPadding,
    required this.treeVerticalPadding,
    required this.accountOuterPadding,
    required this.accountRadius,
    required this.accountBorderWidth,
    required this.accountInnerPadding,
    required this.accountContentSpacing,
    required this.accountAvatarSize,
    required this.accountQuotaTopPadding,
    required this.accountQuotaProgressSpacing,
    required this.accountQuotaProgressHeight,
    required this.updateCardHorizontalMargin,
    required this.updateCardBottomMargin,
    required this.updateCardRadius,
    required this.updateCardPadding,
    required this.downloadProgressSpacing,
    required this.dismissButtonSize,
    required this.availableActionSpacing,
    required this.installButtonHeight,
    required this.installButtonRadius,
    required this.treeNodeHeight,
    required this.treeDepthIndent,
    required this.treeNodeStartPadding,
    required this.treeNodeEndPadding,
    required this.treeNodeRadius,
    required this.treeNodeContentSpacing,
    required this.treeExpanderSize,
    required this.treeArrowIconSize,
    required this.treeFolderIconSize,
  });
}

/// 传输弹窗尺寸。
class TransferPopoverMetrics {
  /// 面板宽度。
  final double panelWidth;

  /// 面板高度。
  final double panelHeight;

  /// 面板顶部偏移。
  final double panelTopOffset;

  /// 面板右侧偏移。
  final double panelEndOffset;

  /// 面板阴影高度。
  final double panelShadowElevation;

  /// 面板圆角。
  final double panelRadius;

  /// 面板边框宽度。
  final double panelBorderWidth;

  /// 头部高度。
  final double headerHeight;

  /// 头部起始内边距。
  final double headerStartPadding;

  /// 头部结束内边距。
  final double headerEndPadding;

  /// 头部内容间距。
  final double headerContentSpacing;

  /// 头部图标尺寸。
  final double headerIconSize;

  /// 汇总水平内边距。
  final double summaryHorizontalPadding;

  /// 汇总底部内边距。
  final double summaryBottomPadding;

  /// 汇总项间距。
  final double summaryItemSpacing;

  /// 汇总圆角。
  final double summaryRadius;

  /// 汇总水平内容内边距。
  final double summaryHorizontalContentPadding;

  /// 汇总垂直内容内边距。
  final double summaryVerticalContentPadding;

  /// 汇总文字间距。
  final double summaryTextSpacing;

  /// 任务最小高度。
  final double taskMinimumHeight;

  /// 任务水平内边距。
  final double taskHorizontalPadding;

  /// 任务垂直内边距。
  final double taskVerticalPadding;

  /// 任务内容间距。
  final double taskContentSpacing;

  /// 方向徽章尺寸。
  final double directionBadgeSize;

  /// 方向徽章圆角。
  final double directionBadgeRadius;

  /// 方向图标尺寸。
  final double directionIconSize;

  /// 任务信息间距。
  final double taskInfoSpacing;

  /// 任务名称间距。
  final double taskNameSpacing;

  /// 任务进度间距。
  final double taskProgressSpacing;

  /// 任务状态宽度。
  final double taskStateWidth;

  /// 任务状态间距。
  final double taskStateSpacing;

  /// 任务状态图标尺寸。
  final double taskStateIconSize;

  const TransferPopoverMetrics({
    required this.panelWidth,
    required this.panelHeight,
    required this.panelTopOffset,
    required this.panelEndOffset,
    required this.panelShadowElevation,
    required this.panelRadius,
    required this.panelBorderWidth,
    required this.headerHeight,
    required this.headerStartPadding,
    required this.headerEndPadding,
    required this.headerContentSpacing,
    required this.headerIconSize,
    required this.summaryHorizontalPadding,
    required this.summaryBottomPadding,
    required this.summaryItemSpacing,
    required this.summaryRadius,
    required this.summaryHorizontalContentPadding,
    required this.summaryVerticalContentPadding,
    required this.summaryTextSpacing,
    required this.taskMinimumHeight,
    required this.taskHorizontalPadding,
    required this.taskVerticalPadding,
    required this.taskContentSpacing,
    required this.directionBadgeSize,
    required this.directionBadgeRadius,
    required this.directionIconSize,
    required this.taskInfoSpacing,
    required this.taskNameSpacing,
    required this.taskProgressSpacing,
    required this.taskStateWidth,
    required this.taskStateSpacing,
    required this.taskStateIconSize,
  });
}

// =============================================================================
// 按 UI 区域组织的度量 token 集合。
// =============================================================================

/// PetalLink 默认度量 token。
class MateMetrics {
  /// 按钮尺寸。
  final ButtonMetrics button;

  /// 菜单尺寸。
  final MenuMetrics menu;

  /// 表单尺寸。
  final FormMetrics form;

  /// 导航尺寸。
  final NavigationMetrics navigation;

  /// 提示组件尺寸。
  final FeedbackMetrics feedback;

  /// 对话框尺寸。
  final DialogMetrics dialog;

  /// 文件列表尺寸。
  final FileListMetrics fileList;

  /// 基础品牌组件尺寸。
  final BasicMetrics basic;

  /// 图标尺寸。
  final IconMetrics icon;

  /// 分隔线尺寸。
  final DividerMetrics divider;

  /// 组件目录尺寸。
  final CatalogMetrics catalog;

  /// 同步状态栏尺寸。
  final StatusBarMetrics statusBar;

  /// 同步设置横幅尺寸。
  final SyncSetupMetrics syncSetup;

  /// 登录页尺寸。
  final LoginMetrics login;

  /// 日志查看器尺寸。
  final LogViewerMetrics logViewer;

  /// 主页面尺寸。
  final MainPageMetrics mainPage;

  /// 浮层组件尺寸。
  final OverlayMetrics overlay;

  /// 设置页尺寸。
  final SettingsMetrics settings;

  /// 更新弹窗尺寸。
  final UpdateDialogMetrics updateDialog;

  /// 侧边栏尺寸。
  final SidebarMetrics sidebar;

  /// 传输弹窗尺寸。
  final TransferPopoverMetrics transferPopover;

  const MateMetrics({
    required this.button,
    required this.menu,
    required this.form,
    required this.navigation,
    required this.feedback,
    required this.dialog,
    required this.fileList,
    required this.basic,
    required this.icon,
    required this.divider,
    required this.catalog,
    required this.statusBar,
    required this.syncSetup,
    required this.login,
    required this.logViewer,
    required this.mainPage,
    required this.overlay,
    required this.settings,
    required this.updateDialog,
    required this.sidebar,
    required this.transferPopover,
  });

  /// 默认度量配置（逐值对齐 CMP DesignTokens.METRICS）。
  factory MateMetrics.standard() {
    // 按钮尺寸。
    const button = ButtonMetrics(
      primaryHeight: 36,
      softHeight: 36,
      textHeight: 36,
      iconTextHeight: 36,
      iconButtonSize: 32,
      primaryRadius: 8,
      softRadius: 8,
      textRadius: 5,
      iconTextRadius: 8,
      iconVariantIconSize: 18,
      iconTextVariantIconSize: 16,
      softVariantIconSize: 16,
      primaryVariantIconSize: 14,
      textVariantIconSize: 14,
      iconTextHorizontalPadding: 14,
      textHorizontalPadding: 8,
      softHorizontalPadding: 16,
      primaryHorizontalPadding: 18,
      primaryShadowElevation: 6,
      loadingSpinnerSize: 16,
      loadingLabelSpacing: 8,
      iconLabelSpacing: 6,
      badgeStartPadding: 2,
      badgeHorizontalPadding: 5,
      badgeVerticalPadding: 1,
      badgeHeight: 16,
      primaryWithoutIconSize: 0,
      textWithoutIconSize: 0,
      iconHorizontalPadding: 0,
      dangerPressedAlpha: 0.85,
      primaryShadowAlpha: 0.35,
      disabledAlpha: 0.5,
      spinnerTrackAlpha: 0.3,
      spinnerRotationDurationMillis: 800,
    );

    // 菜单尺寸。
    const menu = MenuMetrics(
      defaultWidth: 168,
      containerRadius: 10,
      itemHeight: 36,
      itemRadius: 8,
    );

    // 表单尺寸。
    const form = FormMetrics(
      textFieldHeight: 38,
      textFieldRadius: 8,
      numberFieldHeight: 38,
      numberFieldRadius: 8,
      searchFieldHeight: 38,
      stepperHeight: 36,
      controls: FormControlMetrics(
        textFieldBorderWidth: 2,
        textFieldHorizontalPadding: 12,
        textFieldContentSpacing: 8,
        textFieldPrefixIconSize: 16,
        numberFieldBorderWidth: 2,
        numberFieldHorizontalPadding: 12,
        numberFieldContentSpacing: 8,
        numberFieldInputWidth: 120,
        stepperRadius: 8,
        stepperPadding: 3,
        stepperMinusIconSize: 14,
        stepperValueWidth: 44,
        stepperButtonSize: 30,
        stepperButtonShadowElevation: 1,
        stepperButtonIdleElevation: 0,
        stepperButtonRadius: 5,
        searchUnboundedWidth: 0,
        switchCheckedKnobOffset: 21,
        switchUncheckedKnobOffset: 3,
        switchWidth: 46,
        switchHeight: 28,
        switchRadius: 14,
        switchKnobSize: 22,
        switchKnobShadowElevation: 2,
        checkboxDefaultSize: 18,
        checkboxRadius: 5,
        checkboxBorderWidth: 1.5,
        checkboxCheckInset: 5,
        checkboxIndeterminateInset: 9,
        checkboxIndeterminateHeight: 1.5,
        checkboxIndeterminateRadius: 1,
        radioDefaultSize: 16,
        radioBorderWidth: 1,
        textFieldDisabledAlpha: 0.6,
        stepperDisabledAlpha: 0.4,
        switchDisabledAlpha: 0.5,
        checkboxDisabledAlpha: 0.5,
        radioDisabledAlpha: 0.5,
      ),
    );

    // 导航尺寸。
    const navigation = NavigationMetrics(
      sidebarItemHeight: 46,
      sidebarItemRadius: 8,
      breadcrumbHeight: 40,
      breadcrumbHorizontalPadding: 20,
      breadcrumbItemSpacing: 6,
    );

    // 提示组件尺寸。
    const feedback = FeedbackMetrics(
      bannerRadius: 10,
      smallTagRadius: 5,
      mediumTagRadius: 5,
      emptyBadgeSize: 72,
      emptyBadgeRadius: 14,
      controls: FeedbackControlMetrics(
        linearProgressHeight: 6,
        circularProgressSize: 24,
        circularProgressStrokeWidth: 2.5,
        bannerHorizontalPadding: 14,
        bannerVerticalPadding: 12,
        bannerContentSpacing: 10,
        bannerIconSize: 18,
        bannerCloseIconSize: 14,
        smallTagHorizontalPadding: 6,
        mediumTagHorizontalPadding: 10,
        smallTagVerticalPadding: 2,
        mediumTagVerticalPadding: 3,
        smallTagIconSize: 12,
        mediumTagIconSize: 14,
        tagContentSpacing: 4,
        emptyStatePadding: 32,
        emptyStateIconSize: 36,
        emptyStateTitleSpacing: 16,
        emptyStateDescriptionSpacing: 6,
        emptyStateActionSpacing: 24,
        statChipRadius: 8,
        statChipHorizontalPadding: 8,
        statChipVerticalPadding: 4,
        statChipContentSpacing: 4,
        statChipIconSize: 12,
        sectionHeaderBottomPadding: 12,
        sectionHeaderContentSpacing: 8,
        sectionHeaderIconSize: 18,
        navigationItemHeight: 46,
        navigationItemRadius: 8,
        navigationItemHorizontalPadding: 14,
        navigationItemIndentPerLevel: 1,
        navigationItemContentSpacing: 12,
        navigationItemIconSize: 18,
        navigationGroupStartPadding: 14,
        navigationGroupTopPadding: 20,
        navigationGroupBottomPadding: 6,
        tagIconAlpha: 0.7,
        circularProgressRotationDurationMillis: 1200,
      ),
    );

    // 对话框尺寸。
    const dialog = DialogMetrics(
      containerRadius: 12,
      iconBadgeSize: 40,
      iconBadgeRadius: 10,
      toastRadius: 10,
    );

    // 文件列表尺寸。
    const fileList = FileListMetrics(
      renameDialogWidth: 420,
      renameDialogRadius: 12,
      renameDialogPadding: 24,
      renameDialogContentSpacing: 16,
      renameDialogActionSpacing: 8,
      moveDialogWidth: 460,
      moveDialogRadius: 12,
      moveDialogPadding: 24,
      moveDialogContentSpacing: 16,
      moveDialogFolderListHeight: 260,
      moveDialogFolderRadius: 8,
      moveDialogFolderPadding: 12,
      moveDialogFolderContentSpacing: 12,
      moveDialogFolderIconSize: 18,
      moveDialogActionSpacing: 8,
      controls: FileListControlMetrics(
        sizeColumnInitialWidth: 110,
        timeColumnInitialWidth: 160,
        resizableColumnMinimumWidth: 64,
        resizableColumnMaximumWidth: 400,
        bulkBarHorizontalMargin: 24,
        bulkBarTopMargin: 10,
        bulkBarHeight: 44,
        bulkBarRadius: 10,
        bulkBarStartPadding: 16,
        bulkBarEndPadding: 8,
        bulkBarContentSpacing: 10,
        tableHorizontalPadding: 12,
        headerHeight: 38,
        headerHorizontalPadding: 12,
        checkboxColumnWidth: 40,
        statusColumnWidth: 72,
        actionColumnWidth: 44,
        footerHeight: 36,
        bulkActionHeight: 32,
        bulkActionRadius: 8,
        bulkActionHorizontalPadding: 14,
        bulkActionContentSpacing: 6,
        bulkActionIconSize: 16,
        bulkCloseSize: 32,
        bulkCloseIconSize: 16,
        headerSortSpacing: 4,
        headerSortIconSize: 12,
        resizeHandleWidth: 6,
        rowHeight: 56,
        rowRadius: 8,
        rowHorizontalPadding: 12,
        rowNameContentSpacing: 12,
        rowStatusIconSize: 16,
        contextMenuWidth: 200,
        contextMenuShadowElevation: 16,
        contextMenuRadius: 10,
        contextMenuBorderWidth: 0.5,
        contextMenuPadding: 6,
        thumbnailSize: 32,
        thumbnailRadius: 6,
        fileTypeIconSize: 18,
        contextActionHeight: 36,
        contextActionRadius: 8,
        contextActionHorizontalPadding: 12,
        contextActionContentSpacing: 10,
        contextActionIconSize: 16,
        contextDividerHorizontalPadding: 8,
        contextDividerVerticalPadding: 4,
        contextDividerHeight: 0.5,
        bulkActionDisabledAlpha: 0.4,
      ),
    );

    // 基础品牌组件尺寸。
    const basic = BasicMetrics(
      compactLogoSize: 26,
      largeLogoSize: 64,
      compactLogoTextSpacing: 8,
      fullLogoHeight: 32,
      fullLogoTextSpacing: 6,
      verticalSeparatorHeight: 20,
      verticalSeparatorWidth: 1,
      bottomBorderThickness: 0.5,
    );

    // 图标尺寸。
    const icon = IconMetrics(
      defaultSize: 16,
      spinDurationMillis: 1000,
    );

    // 分隔线尺寸。
    const divider = DividerMetrics(
      horizontalThickness: 0.5,
      verticalHeight: 24,
      verticalWidth: 1,
    );

    // 组件目录尺寸。
    const catalog = CatalogMetrics(
      pagePadding: 24,
      sectionSpacing: 24,
      itemSpacing: 12,
      iconPreviewSize: 24,
      fieldPreviewWidth: 200,
      progressPreviewWidth: 200,
      circularProgressSize: 32,
      verticalGroupSpacing: 8,
      compactItemSpacing: 8,
      dialogPreviewWidth: 200,
    );

    // 同步状态栏尺寸。
    const statusBar = StatusBarMetrics(
      minimumHeight: 44,
      horizontalPadding: 20,
      verticalPadding: 6,
      statusContentSpacing: 10,
      syncingIconSize: 16,
      idleIndicatorSize: 8,
      actionHorizontalSpacing: 6,
      actionVerticalSpacing: 6,
    );

    // 同步设置横幅尺寸。
    const syncSetup = SyncSetupMetrics(
      horizontalPadding: 20,
      verticalPadding: 8,
    );

    // 登录页尺寸。
    const login = LoginMetrics(
      topDecorationOffsetX: 80,
      topDecorationOffsetY: -100,
      topDecorationSize: 400,
      bottomDecorationOffsetX: -80,
      bottomDecorationOffsetY: 60,
      bottomDecorationSize: 300,
      centerDecorationSize: 200,
      cardWidth: 480,
      cardShadowElevation: 24,
      cardRadius: 12,
      cardHorizontalPadding: 24,
      cardVerticalPadding: 32,
      logoTitleSpacing: 12,
      subtitleSpacing: 4,
      accentWidth: 40,
      accentHeight: 2,
      accentRadius: 1,
      accentBottomSpacing: 12,
      messageSpacing: 12,
      contentBottomSpacing: 24,
      authorizingHeight: 40,
      authorizingRadius: 8,
      authorizingHorizontalPadding: 16,
      authorizingContentSpacing: 8,
      authorizingSpinnerSize: 16,
      authorizingSpinnerStroke: 2,
      errorActionSpacing: 8,
      loginButtonHeight: 46,
      footerSpacing: 12,
      topDecorationAlpha: 0.06,
      bottomDecorationAlpha: 0.06,
      centerDecorationAlpha: 0.05,
    );

    // 日志查看器尺寸。
    const logViewer = LogViewerMetrics(
      inlineHeaderHeight: 56,
      inlineHeaderHorizontalPadding: 16,
      inlineHeaderContentSpacing: 8,
      standaloneHeaderHorizontalPadding: 20,
      headerVerticalPadding: 14,
      headerContentSpacing: 8,
      loadingSize: 24,
      inlineContentPadding: 0,
      standaloneContentPadding: 20,
      listRadius: 10,
      listBorderWidth: 0.5,
      recordHorizontalPadding: 16,
      recordVerticalPadding: 12,
      recordContentSpacing: 12,
      metadataTopPadding: 3,
    );

    // 主页面尺寸。
    const mainPage = MainPageMetrics(
      appBarHeight: 64,
      appBarHorizontalPadding: 20,
      searchMaximumWidth: 420,
      appBarActionSpacing: 8,
      loadingSize: 24,
      searchPanelStartPadding: 12,
      searchPanelTopPadding: 14,
      searchPanelEndPadding: 12,
      searchPanelBottomPadding: 10,
      searchResultHeight: 56,
      searchResultHorizontalPadding: 12,
      searchResultContentSpacing: 12,
      searchResultIconContainerSize: 32,
      searchResultIconRadius: 6,
      searchResultIconSize: 18,
    );

    // 浮层组件尺寸。
    const overlay = OverlayMetrics(
      menuBorderWidth: 0.5,
      menuPadding: 6,
      menuDividerHorizontalPadding: 8,
      menuDividerVerticalPadding: 4,
      menuDividerHeight: 0.5,
      menuItemHorizontalPadding: 12,
      menuItemContentSpacing: 10,
      menuItemIconSize: 16,
      dialogHeaderHorizontalPadding: 24,
      dialogHeaderTopPadding: 24,
      dialogHeaderBottomPadding: 8,
      dialogHeaderContentSpacing: 12,
      dialogTitleIconSize: 20,
      dialogBodyHorizontalPadding: 24,
      dialogBodyTopPadding: 8,
      dialogBodyBottomPadding: 20,
      dialogFooterHorizontalPadding: 24,
      dialogFooterBottomPadding: 20,
      dialogActionSpacing: 10,
      toastOuterPadding: 48,
      toastHorizontalPadding: 18,
      toastVerticalPadding: 10,
      toastContentSpacing: 8,
      toastIconSize: 16,
    );

    // 设置页尺寸。
    const settings = SettingsMetrics(
      headerHeight: 56,
      headerHorizontalPadding: 16,
      headerContentSpacing: 8,
      navigationWidth: 240,
      navigationHorizontalPadding: 12,
      navigationVerticalPadding: 20,
      navigationItemSpacing: 6,
      navigationBorderWidth: 0.5,
      bodyHorizontalPadding: 32,
      bodyVerticalPadding: 28,
      concurrencyContentSpacing: 8,
      skipPatternFieldWidth: 280,
      oauthBannerTopPadding: 4,
      oauthBannerBottomPadding: 8,
      logPanelPadding: 24,
      logPanelContentSpacing: 14,
      footerBorderWidth: 0.5,
      footerHeight: 64,
      footerHorizontalPadding: 32,
      footerActionSpacing: 10,
      savedIndicatorSpacing: 4,
      savedIndicatorSize: 6,
      panelHorizontalPadding: 24,
      panelVerticalPadding: 4,
      panelDefaultContentSpacing: 0,
      panelRadius: 10,
      panelBorderWidth: 0.5,
      firstGroupTopPadding: 12,
      groupTopPadding: 20,
      groupBottomPadding: 8,
      settingRowVerticalPadding: 16,
      settingRowContentSpacing: 24,
      settingDescriptionTopPadding: 3,
      settingRowDividerWidth: 0.5,
      mountPanelRadius: 10,
      configuredMountBorderWidth: 1,
      emptyMountBorderWidth: 0.5,
      mountPanelHorizontalPadding: 24,
      configuredMountVerticalPadding: 32,
      emptyMountVerticalPadding: 40,
      mountPanelContentSpacing: 12,
      emptyMountBadgeSize: 72,
      emptyMountBadgeRadius: 14,
      emptyMountIconSize: 48,
      configuredMountBadgeSize: 40,
      configuredMountBadgeRadius: 10,
      configuredMountIconSize: 20,
      mountPathRadius: 12,
      mountPathHorizontalPadding: 12,
      mountPathVerticalPadding: 4,
      mountBannerSpacing: 16,
      accountPanelHorizontalPadding: 24,
      accountPanelVerticalPadding: 16,
      accountContentSpacing: 16,
      accountAvatarSize: 56,
      accountSectionSpacing: 16,
      detailRowVerticalPadding: 12,
      detailLabelWidth: 96,
      detailContentSpacing: 12,
      detailDividerWidth: 0.5,
      aboutPanelPadding: 24,
      aboutPanelContentSpacing: 14,
      aboutLogoHeight: 30,
      versionContentSpacing: 10,
      externalLinksSpacing: 16,
      externalLinkVerticalPadding: 4,
      externalLinkContentSpacing: 4,
      externalLinkIconSize: 16,
    );

    // 更新弹窗尺寸。
    const updateDialog = UpdateDialogMetrics(
      dialogWidth: 440,
      dialogShadowElevation: 24,
      dialogRadius: 12,
      headerHorizontalPadding: 32,
      headerTopPadding: 32,
      headerBottomPadding: 4,
      headerContentSpacing: 8,
      headerBadgeSize: 40,
      headerBadgeRadius: 10,
      headerIconSize: 20,
      versionHorizontalPadding: 32,
      bodyHorizontalPadding: 32,
      bodyTopPadding: 12,
      bodyBottomPadding: 32,
      footerHorizontalPadding: 16,
      footerTopPadding: 8,
      footerBottomPadding: 16,
      releaseNotesLabelSpacing: 4,
      releaseNotesMaximumHeight: 180,
      releaseNotesRadius: 8,
      releaseNotesPadding: 12,
      progressContentSpacing: 12,
      progressTrackHeight: 8,
      progressTrackRadius: 4,
      progressFillHeight: 8,
      progressFillRadius: 4,
      waitingContentSpacing: 12,
      spinnerContainerSize: 20,
      spinnerTopPadding: 2,
      spinnerRingSize: 20,
      spinnerRingStrokeWidth: 2.5,
      footerActionSpacing: 8,
      spinnerRotationDurationMillis: 800,
    );

    // 侧边栏尺寸。
    const sidebar = SidebarMetrics(
      width: 248,
      logoHeaderHeight: 60,
      logoHeaderHorizontalPadding: 18,
      logoSize: 26,
      sectionLabelStartPadding: 18,
      sectionLabelTopPadding: 12,
      sectionLabelBottomPadding: 6,
      treeHorizontalPadding: 8,
      treeVerticalPadding: 4,
      accountOuterPadding: 10,
      accountRadius: 10,
      accountBorderWidth: 0.5,
      accountInnerPadding: 12,
      accountContentSpacing: 10,
      accountAvatarSize: 32,
      accountQuotaTopPadding: 1,
      accountQuotaProgressSpacing: 6,
      accountQuotaProgressHeight: 4,
      updateCardHorizontalMargin: 10,
      updateCardBottomMargin: 10,
      updateCardRadius: 10,
      updateCardPadding: 12,
      downloadProgressSpacing: 8,
      dismissButtonSize: 20,
      availableActionSpacing: 8,
      installButtonHeight: 28,
      installButtonRadius: 5,
      treeNodeHeight: 32,
      treeDepthIndent: 14,
      treeNodeStartPadding: 8,
      treeNodeEndPadding: 8,
      treeNodeRadius: 6,
      treeNodeContentSpacing: 8,
      treeExpanderSize: 16,
      treeArrowIconSize: 12,
      treeFolderIconSize: 16,
    );

    // 传输弹窗尺寸。
    const transferPopover = TransferPopoverMetrics(
      panelWidth: 440,
      panelHeight: 580,
      panelTopOffset: 64,
      panelEndOffset: 20,
      panelShadowElevation: 16,
      panelRadius: 12,
      panelBorderWidth: 0.5,
      headerHeight: 60,
      headerStartPadding: 20,
      headerEndPadding: 12,
      headerContentSpacing: 10,
      headerIconSize: 18,
      summaryHorizontalPadding: 20,
      summaryBottomPadding: 14,
      summaryItemSpacing: 8,
      summaryRadius: 8,
      summaryHorizontalContentPadding: 10,
      summaryVerticalContentPadding: 8,
      summaryTextSpacing: 2,
      taskMinimumHeight: 68,
      taskHorizontalPadding: 20,
      taskVerticalPadding: 10,
      taskContentSpacing: 12,
      directionBadgeSize: 36,
      directionBadgeRadius: 8,
      directionIconSize: 18,
      taskInfoSpacing: 5,
      taskNameSpacing: 6,
      taskProgressSpacing: 10,
      taskStateWidth: 80,
      taskStateSpacing: 3,
      taskStateIconSize: 12,
    );

    return const MateMetrics(
      button: button,
      menu: menu,
      form: form,
      navigation: navigation,
      feedback: feedback,
      dialog: dialog,
      fileList: fileList,
      basic: basic,
      icon: icon,
      divider: divider,
      catalog: catalog,
      statusBar: statusBar,
      syncSetup: syncSetup,
      login: login,
      logViewer: logViewer,
      mainPage: mainPage,
      overlay: overlay,
      settings: settings,
      updateDialog: updateDialog,
      sidebar: sidebar,
      transferPopover: transferPopover,
    );
  }
}
