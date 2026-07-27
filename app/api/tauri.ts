import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { AppError } from "./generated";

export type { AppError } from "./generated";

/**
 * 调用后端命令并将非结构化异常统一转换为 AppError。
 *
 * @param command - Tauri command 名称
 * @param args - command 参数
 */
export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    // 后端 AppError 已满足合同，未知异常才补齐统一字段。
    if (e && typeof e === "object" && "kind" in e) {
      throw e as AppError;
    }
    throw {
      kind: "Generic",
      code: null,
      message: typeof e === "string" ? e : String(e),
      status_code: null,
      error_code: null,
    } satisfies AppError;
  }
}
