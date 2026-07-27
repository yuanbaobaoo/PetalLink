/**
 * 错误处理工具 —— 统一从 unknown 类型提取可读错误信息。
 *
 * 替代散布在各视图/组件中的 `(e as { message?: string }).message ?? String(e)` 模式。
 */
import { SYNC_USER_MESSAGE_RULES } from "@/api/generated";

/**
 * 将内部同步错误转换为用户能理解的提示。
 *
 * 已保存的历史任务仍可能包含旧术语，因此转换必须在展示层保留。
 *
 * @param message - 后端返回或数据库保存的原始错误
 * @returns 用户侧提示
 */
export function formatUserMessage(message: string): string {
  // Rust 同源规则同时兼容新错误和数据库中的历史技术文案。
  for (const rule of SYNC_USER_MESSAGE_RULES) {
    if (rule.patterns.some((pattern) => message.includes(pattern))) {
      return rule.message;
    }
  }

  // 持久化状态机名称不直接暴露给普通用户。
  if (message.includes("WaitingForNetwork")) {
    return "网络不可用，恢复后会自动继续。";
  }
  if (message.includes("BackingOff")) {
    return "服务暂时不可用，稍后会自动重试。";
  }
  if (message.includes("VerifyingRemote")) {
    return "正在确认上次同步是否成功。";
  }
  if (message.includes("RestartRequired")) {
    return "文件状态已变化，请重新检查并重试。";
  }
  if (message.includes("BlockedByActiveIntent")) {
    return "该文件正在执行其他同步任务，请稍后再试。";
  }
  return message;
}

/**
 * 从 unknown 错误对象中提取人类可读的错误消息。
 *
 * 优先取 `.message`（后端 AppError / JS Error），否则回退到 String(e)，
 * 最后统一替换用户不需要理解的内部术语。
 *
 * @param e - 捕获到的错误（类型未知）
 * @returns 错误消息字符串
 */
export function extractErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    // 后端错误消息
    const msg = (e as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return formatUserMessage(msg);
  }
  if (typeof e === "string") return formatUserMessage(e);
  return formatUserMessage(String(e));
}
