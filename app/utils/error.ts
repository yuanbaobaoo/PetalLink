/**
 * 错误处理工具 —— 统一从 unknown 类型提取可读错误信息。
 *
 * 替代散布在各视图/组件中的 `(e as { message?: string }).message ?? String(e)` 模式。
 */

/**
 * 从 unknown 错误对象中提取人类可读的错误消息。
 *
 * 优先取 `.message`（后端 AppError / JS Error），否则回退到 String(e)。
 *
 * @param e - 捕获到的错误（类型未知）
 * @returns 错误消息字符串
 */
export function extractErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    const msg = (e as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return msg;
  }
  if (typeof e === "string") return e;
  return String(e);
}
