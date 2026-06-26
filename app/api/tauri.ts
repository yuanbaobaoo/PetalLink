/**
 * Tauri invoke + listen 封装。
 * 所有后端命令调用统一经过此处，错误归一化为前端可读的 AppError 结构。
 * 
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * 后端错误结构。
 * kind 标识错误类别，message 为用户可读中文描述。
 */
export interface AppError {
  kind: "Auth" | "Token" | "DriveApi" | "Config" | "QuotaExceeded" | "Generic";
  message: string;
  // DriveApi 特有字段
  status_code?: number;
  error_code?: string;
}

/**
 * 调用后端命令，返回 Promise<T>。
 * 失败时抛出 AppError（后端序列化的结构），调用方用 try/catch 捕获。
 *
 * @param command - 命令名
 * @param args - 参数对象
 */
export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    // 后端返回的 AppError 已是对象结构；若不是则包装为 Generic
    if (e && typeof e === "object" && "kind" in e) {
      throw e as AppError;
    }
    throw {
      kind: "Generic",
      message: typeof e === "string" ? e : String(e),
    } as AppError;
  }
}

/**
 * 监听后端事件，返回取消监听函数。
 *
 * @param event - 事件名
 * @param handler - 事件回调，payload 为泛型 T
 */
export function on<T = unknown>(
  event: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}
