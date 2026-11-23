/**
 * Toast 模块，showToast() 自由调用。
 *
 * 队列语义：最多同时展示 `MAX_VISIBLE_TOASTS` 条，超出时挤掉最早的一条。
 * 这样批量失败（如一次 8 个文件上传失败）不会再「后一条顶掉前一条、只看得到最后一条」。
 */
import { reactive } from "vue";

export type ToastVariant = "default" | "success" | "warning" | "error";

export interface ToastItem {
  id: number;
  message: string;
  variant: ToastVariant;
}

// 当前展示中的 Toast 列表。
export const toasts = reactive<ToastItem[]>([]);

// 默认 Toast 显示时长（毫秒）
const DEFAULT_TOAST_DURATION_MS = 2000;
// 同屏最多展示的 Toast 条数；超出时挤掉最早的一条
const MAX_VISIBLE_TOASTS = 3;

// 自增 ID 计数器
let _seq = 0;

/**
 * 移除指定 ID 的 Toast
 *
 * @param id - Toast ID
 */
function dismiss(id: number): void {
  // 目标 toast 在列表中的下标。
  const i = toasts.findIndex((t) => t.id === id);
  if (i >= 0) toasts.splice(i, 1);
}

/**
 * 显示一条 Toast（默认 2s 自动消失；超过上限时挤掉最早的一条）
 */
export function showToast(
  message: string,
  opts?: { variant?: ToastVariant; duration?: number },
): void {
  // 新提示或任务的唯一标识。
  const id = ++_seq;
  // 队列语义：只挤掉溢出的最早条目，其它提示继续展示。
  const overflow = toasts.length + 1 - MAX_VISIBLE_TOASTS;
  if (overflow > 0) toasts.splice(0, overflow);
  toasts.push({ id, message, variant: opts?.variant ?? "default" });
  window.setTimeout(() => dismiss(id), opts?.duration ?? DEFAULT_TOAST_DURATION_MS);
}
