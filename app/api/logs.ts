/**
 * Logs API —— 日志查看与导出。
 */
import { commands } from "./generated";
import { call, discard } from "./tauri";
export type { LogRecord } from "./generated";
import type { LogRecord } from "./generated";

/**
 * 读取最近日志（newest-first）
 */
export function listLogs(): Promise<LogRecord[]> {
  return call(commands.logsList());
}

/**
 * 导出完整日志到指定路径（拼接滚动日志文件，oldest-first）
 */
export function exportLogs(path: string): Promise<void> {
  return discard(commands.logsExport(path));
}

/**
 * 清空后端日志环形缓冲
 */
export function clearLogs(): Promise<void> {
  return discard(commands.logsClear());
}
