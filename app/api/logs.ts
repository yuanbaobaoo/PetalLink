/**
 * Logs API —— 日志查看与导出。
 */
import { invoke } from "./tauri";

/** 单条日志记录（对齐后端 core::logging::LogRecord） */
export interface LogRecord {
  level: string; // "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE"
  message: string;
  time_ms: number;
  logger_name: string;
}

/** 读取最近日志（newest-first） */
export function listLogs(): Promise<LogRecord[]> {
  return invoke<LogRecord[]>("logs_list");
}

/** 导出完整日志到指定路径（拼接滚动日志文件，oldest-first） */
export function exportLogs(path: string): Promise<void> {
  return invoke<void>("logs_export", { path });
}

/** 清空后端日志环形缓冲 */
export function clearLogs(): Promise<void> {
  return invoke<void>("logs_clear");
}
