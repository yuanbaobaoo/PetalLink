/**
 * Transfer API —— 传输队列相关常量。
 */
import { invoke } from "./tauri";

/** 传输方向常量 */
export const TRANSFER_DIR = {
  UPLOAD: 0,
  DOWNLOAD: 1,
  DELETE: 2,
  DOWNLOAD_UPDATE: 3,
} as const;
export type TransferDirection = (typeof TRANSFER_DIR)[keyof typeof TRANSFER_DIR];

/** 传输方向标签 */
export const DIR_LABEL: Record<number, string> = {
  [TRANSFER_DIR.UPLOAD]: "上传",
  [TRANSFER_DIR.DOWNLOAD]: "下载",
  [TRANSFER_DIR.DELETE]: "删除",
  [TRANSFER_DIR.DOWNLOAD_UPDATE]: "更新",
};

/** 传输状态常量 */
export const TRANSFER_STATE = {
  PENDING: 0,
  RUNNING: 1,
  WAITING_FOR_NETWORK: 2,
  BACKING_OFF: 3,
  VERIFYING_REMOTE: 4,
  RESTART_REQUIRED: 5,
  COMPLETED: 6,
  FAILED: 7,
  CANCELED: 8,
} as const;
export type TransferState = (typeof TRANSFER_STATE)[keyof typeof TRANSFER_STATE];

/** 持久化传输操作，与 Rust TransferOperation discriminant 一致。 */
export const TRANSFER_OPERATION = {
  CREATE: 0,
  UPDATE: 1,
  DOWNLOAD: 2,
  DOWNLOAD_UPDATE: 3,
  DELETE: 4,
  MOVE: 5,
  RENAME: 6,
  CREATE_FOLDER: 7,
} as const;
export type TransferOperation = (typeof TRANSFER_OPERATION)[keyof typeof TRANSFER_OPERATION];

/** 持久化错误分类，与 Rust TransferErrorKind discriminant 一致。 */
export const TRANSFER_ERROR_KIND = {
  NETWORK: 0,
  TIMEOUT: 1,
  AUTH: 2,
  RATE_LIMIT: 3,
  SERVER: 4,
  QUOTA: 5,
  PERMISSION: 6,
  VALIDATION: 7,
  SESSION_EXPIRED: 8,
  REMOTE_AMBIGUOUS: 9,
  LOCAL_CHANGED: 10,
  UNKNOWN: 11,
} as const;
export type TransferErrorKind = (typeof TRANSFER_ERROR_KIND)[keyof typeof TRANSFER_ERROR_KIND];

/** SQLite v5 传输任务的完整 Tauri 合同。 */
export interface TransferTask {
  id: number;
  direction: TransferDirection;
  file_id: string | null;
  local_path: string | null;
  name: string;
  total_size: number;
  transferred: number;
  state: TransferState;
  error_message: string | null;
  created_at: number;
  finished_at: number | null;
  server_id: string | null;
  upload_id: string | null;
  resume_offset: number;
  session_url: string | null;
  relative_path: string | null;
  parent_file_id: string | null;
  operation: TransferOperation | null;
  source_mtime: number | null;
  source_size: number | null;
  expected_cloud_edited_time: number | null;
  attempt_count: number;
  next_retry_at: number | null;
  error_kind: TransferErrorKind | null;
  remote_result_file_id: string | null;
  state_revision: number;
}

/**
 * 仅暴露统一 TaskRunner 确实能处理的重试入口。
 * RestartRequired 由引擎接管并触发重新规划，Failed 则按原 task ID 重新执行。
 */
export function canRetryTransferTask(task: TransferTask): boolean {
  if (
    task.state !== TRANSFER_STATE.FAILED
    && task.state !== TRANSFER_STATE.RESTART_REQUIRED
  ) return false;

  const supportedUpload = task.direction === TRANSFER_DIR.UPLOAD
    && (task.operation === TRANSFER_OPERATION.CREATE
      || task.operation === TRANSFER_OPERATION.UPDATE);
  const supportedDownload = (
    task.direction === TRANSFER_DIR.DOWNLOAD
      && task.operation === TRANSFER_OPERATION.DOWNLOAD)
    || (
      task.direction === TRANSFER_DIR.DOWNLOAD_UPDATE
      && task.operation === TRANSFER_OPERATION.DOWNLOAD_UPDATE);
  return supportedUpload || supportedDownload;
}

/**
 * 列举全部传输任务
 */
export function listAllTransfers(): Promise<TransferTask[]> {
  return invoke<TransferTask[]>("transfer_list_all");
}

/**
 * 清除已完成
 */
export function clearCompleted(): Promise<void> {
  return invoke<void>("transfer_clear_completed");
}

/**
 * 清除失败项
 */
export function clearFailed(): Promise<void> {
  return invoke<void>("transfer_clear_failed");
}

/**
 * 清除已完成+失败
 */
export function clearFinished(): Promise<void> {
  return invoke<void>("transfer_clear_finished");
}

/**
 * 重试单个传输任务；Failed 重放，RestartRequired 请求重新规划。
 *
 * @param taskId - 传输任务 ID
 */
export function retryTransfer(taskId: number): Promise<void> {
  return invoke<void>("transfer_retry", { taskId });
}
