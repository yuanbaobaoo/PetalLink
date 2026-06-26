/**
 * Toast 模块，showToast() 自由调用，单条语义。
 */
import { reactive } from "vue";

export type ToastVariant = "default" | "success" | "warning" | "error";

export interface ToastItem {
  id: number;
  message: string;
  variant: ToastVariant;
}

export const toasts = reactive<ToastItem[]>([]);

// 默认 Toast 显示时长（毫秒）
const DEFAULT_TOAST_DURATION_MS = 2000;

// 自增 ID 计数器
let _seq = 0;

/**
 * 移除指定 ID 的 Toast
 *
 * @param id - Toast ID
 */
function dismiss(id: number): void {
  const i = toasts.findIndex((t) => t.id === id);
  if (i >= 0) toasts.splice(i, 1);
}

/** 显示一条 Toast（默认 2s 自动消失；新 Toast 出现会顶替旧的） */
export function showToast(
  message: string,
  opts?: { variant?: ToastVariant; duration?: number },
): void {
  const id = ++_seq;
  // 单条语义：清空旧的
  toasts.splice(0, toasts.length);
  toasts.push({ id, message, variant: opts?.variant ?? "default" });
  window.setTimeout(() => dismiss(id), opts?.duration ?? DEFAULT_TOAST_DURATION_MS);
}
