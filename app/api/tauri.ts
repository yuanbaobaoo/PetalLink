/**
 * 所有后端命令调用的错误归一化封装。
 */

import type { AppError } from "./generated";

export type { AppError } from "./generated";

/**
 * 调用后端命令，返回 Promise<T>。
 * 失败时抛出 AppError（后端序列化的结构），调用方用 try/catch 捕获。
 *
 * @param operation - generated bindings 返回的命令 Promise
 */
export async function call<T>(operation: Promise<T>): Promise<T> {
  try {
    return await operation;
  } catch (e) {
    // 后端返回的 AppError 已是对象结构；若不是则包装为 Generic
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

/**
 * 将返回 null 的 Rust command 转换为 Promise<void>。
 *
 * @param operation - generated bindings 返回的命令 Promise
 */
export async function discard(operation: Promise<null>): Promise<void> {
  await call(operation);
}
