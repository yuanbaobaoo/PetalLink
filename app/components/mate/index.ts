/**
 * Mate 组件库导出。
 */

// ===== 图标 =====
export { default as MateIcon } from "./MateIcon.vue";

// ===== 品牌 =====
export { default as MateAppLogo } from "./MateAppLogo.vue";
export { default as MateLogoWithText } from "./MateLogoWithText.vue";

// ===== 按钮 =====
export { default as MateButton } from "./MateButton.vue";
export type { ButtonVariant } from "./MateButton.vue";

// ===== 输入 =====
export { default as MateTextField } from "./MateTextField.vue";
export { default as MateNumberField } from "./MateNumberField.vue";
export { default as MateStepper } from "./MateStepper.vue";
export { default as MateSearchField } from "./MateSearchField.vue";

// ===== 选择 =====
export { default as MateSwitch } from "./MateSwitch.vue";
export { default as MateCheckbox } from "./MateCheckbox.vue";
export { default as MateRadio } from "./MateRadio.vue";
export { default as MateRadioGroup } from "./MateRadioGroup.vue";

// ===== 进度 =====
export { default as MateLinearProgress } from "./MateLinearProgress.vue";
export { default as MateCircularProgress } from "./MateCircularProgress.vue";

// ===== 反馈 =====
export { default as MateInfoBanner } from "./MateInfoBanner.vue";
export { default as MateDialog } from "./MateDialog.vue";
export { default as MateDialogHost } from "./MateDialogHost.vue";
export { default as MateToastHost } from "./MateToastHost.vue";
export { openDialog, confirmDialog, closeDialog, dialogState } from "./useDialog";
export type { DialogOptions, ConfirmOptions } from "./useDialog";
export { showToast, toasts } from "./useToast";
export type { ToastVariant, ToastItem } from "./useToast";

// ===== 菜单 =====
export { default as MatePopupMenu } from "./MatePopupMenu.vue";
export type { PopupItem } from "./MatePopupMenu.vue";

// ===== 展示 =====
export { default as MateEmpty } from "./MateEmpty.vue";
export { default as MateTag } from "./MateTag.vue";
export type { TagTheme, TagSize } from "./MateTag.vue";
export { default as MateNavItem } from "./MateNavItem.vue";
export { default as MateSectionHeader } from "./MateSectionHeader.vue";
export { default as MateStatChip } from "./MateStatChip.vue";
export { default as MateSpinningIcon } from "./MateSpinningIcon.vue";

// ===== 基础设施 =====
export { default as MateHover } from "./MateHover.vue";
export { default as MateVerticalSeparator } from "./MateVerticalSeparator.vue";
export { default as MateBottomDivider } from "./MateBottomDivider.vue";
export { default as MateScaffold } from "./MateScaffold.vue";
