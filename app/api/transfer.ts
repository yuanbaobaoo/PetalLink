/**
 * Transfer API —— 传输队列相关常量。
 */
import { invoke } from "./tauri";

/** 传输任务 */
export interface TransferTask {
  id: number;
  direction: number; // 0=upload, 1=download
  file_id?: string;
  local_path?: string;
  name: string;
  total_size: number;
  transferred: number;
  state: number; // 0=pending,1=running,2=paused,3=completed,4=failed,5=canceled
  error_message?: string;
  created_at: number;
  finished_at?: number;
}

/** 传输方向常量 */
export const TRANSFER_DIR = { UPLOAD: 0, DOWNLOAD: 1 } as const;

/** 传输状态常量 */
export const TRANSFER_STATE = {
  PENDING: 0, RUNNING: 1, PAUSED: 2, COMPLETED: 3, FAILED: 4, CANCELED: 5,
} as const;

/** 列举全部传输任务 */
export function listAllTransfers(): Promise<TransferTask[]> {
  return invoke<TransferTask[]>("transfer_list_all");
}

/** 清除已完成 */
export function clearCompleted(): Promise<void> {
  return invoke<void>("transfer_clear_completed");
}

/** 清除失败项 */
export function clearFailed(): Promise<void> {
  return invoke<void>("transfer_clear_failed");
}

/** 清除已完成+失败 */
export function clearFinished(): Promise<void> {
  return invoke<void>("transfer_clear_finished");
}
