/**
 * 对话框模块，confirmDialog / openDialog 自由调用。
 */
import { reactive } from "vue";

export interface DialogOptions {
  title?: string;
  /** 标题图标 icon-name */
  titleIcon?: string;
  danger?: boolean;
  /** 正文（纯文本） */
  content?: string;
  /** 关闭遮罩可关闭（默认 true） */
  closeOnOverlay?: boolean;
  width?: number;
}

export interface ConfirmOptions extends DialogOptions {
  cancelText?: string;
  confirmText?: string;
}

interface DialogState extends Required<Omit<ConfirmOptions, "content">> {
  open: boolean;
  content: string;
  /** Promise resolver（confirm 流程用） */
  resolver: ((v: boolean) => void) | null;
  /** 是否处于 confirm 流程（控制 host 是否渲染取消/确认按钮） */
  isConfirm: boolean;
}

// 全局对话框状态
export const dialogState = reactive<DialogState>({
  open: false,
  title: "",
  titleIcon: "",
  danger: false,
  content: "",
  closeOnOverlay: true,
  width: 420,
  cancelText: "取消",
  confirmText: "确定",
  resolver: null,
  isConfirm: false,
});

/** 打开自定义对话框（不返回值，由调用方通过 closeDialog 关闭） */
export function openDialog(opts: DialogOptions): void {
  Object.assign(dialogState, {
    open: true,
    title: opts.title ?? "",
    titleIcon: opts.titleIcon ?? "",
    danger: opts.danger ?? false,
    content: opts.content ?? "",
    closeOnOverlay: opts.closeOnOverlay ?? true,
    width: opts.width ?? 420,
    isConfirm: false,
    resolver: null,
  });
}

/** 确认对话框，返回用户是否点击确认。 */
export function confirmDialog(opts: ConfirmOptions): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    Object.assign(dialogState, {
      open: true,
      title: opts.title ?? "",
      titleIcon: opts.titleIcon ?? "",
      danger: opts.danger ?? false,
      content: opts.content ?? "",
      closeOnOverlay: opts.closeOnOverlay ?? true,
      width: opts.width ?? 420,
      cancelText: opts.cancelText ?? "取消",
      confirmText: opts.confirmText ?? "确定",
      isConfirm: true,
      resolver: resolve,
    });
  });
}

/** 结束当前对话框（confirm 流程会 resolve 给定的值） */
export function closeDialog(value = false): void {
  const resolver = dialogState.resolver;
  dialogState.open = false;
  dialogState.resolver = null;
  dialogState.isConfirm = false;
  if (resolver) resolver(value);
}
