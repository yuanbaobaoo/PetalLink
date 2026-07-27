/**
 * Transfer API —— 传输队列相关常量。
 */
import {
  commands,
  TRANSFER_DIR,
  TRANSFER_ERROR_KIND,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
} from "./generated";
import { call, discard } from "./tauri";
import type { TransferTask as GeneratedTransferTask } from "./generated";
export {
  TRANSFER_DIR,
  TRANSFER_ERROR_KIND,
  TRANSFER_OPERATION,
  TRANSFER_STATE,
} from "./generated";

/**
 * 传输方向常量
 */
export type TransferDirection = (typeof TRANSFER_DIR)[keyof typeof TRANSFER_DIR];

// 传输方向标签
export const DIR_LABEL: Record<number, string> = {
  [TRANSFER_DIR.UPLOAD]: "上传",
  [TRANSFER_DIR.DOWNLOAD]: "下载",
  [TRANSFER_DIR.DELETE]: "删除",
  [TRANSFER_DIR.DOWNLOAD_UPDATE]: "更新",
};

export type TransferState = (typeof TRANSFER_STATE)[keyof typeof TRANSFER_STATE];

/**
 * 持久化传输操作，与 Rust TransferOperation discriminant 一致。
 */
export type TransferOperation = (typeof TRANSFER_OPERATION)[keyof typeof TRANSFER_OPERATION];

/**
 * 持久化错误分类，与 Rust TransferErrorKind discriminant 一致。
 */
export type TransferErrorKind = (typeof TRANSFER_ERROR_KIND)[keyof typeof TRANSFER_ERROR_KIND];

/**
 * SQLite v5 传输任务合同；字段来自 Rust，数值状态在前端收窄为常量联合。
 */
export type TransferTask = Omit<
  GeneratedTransferTask,
  "direction" | "state" | "operation" | "error_kind"
> & {
  direction: TransferDirection;
  state: TransferState;
  operation: TransferOperation | null;
  error_kind: TransferErrorKind | null;
};

/**
 * 仅暴露统一 TaskRunner 确实能处理的重试入口。
 * RestartRequired 由引擎接管并触发重新规划，Failed 则按原 task ID 重新执行。
 */
export function canRetryTransferTask(task: TransferTask): boolean {
  if (
    task.state !== TRANSFER_STATE.FAILED
    && task.state !== TRANSFER_STATE.RESTART_REQUIRED
  ) return false;

  // 任务是否为前端支持的上传操作。
  const supportedUpload = task.direction === TRANSFER_DIR.UPLOAD
    && (task.operation === TRANSFER_OPERATION.CREATE
      || task.operation === TRANSFER_OPERATION.UPDATE);
  // 任务是否为前端支持的下载操作。
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
  return call(commands.transferListAll()) as Promise<TransferTask[]>;
}

/**
 * 清除已完成
 */
export function clearCompleted(): Promise<void> {
  return discard(commands.transferClearCompleted());
}

/**
 * 清除失败项
 */
export function clearFailed(): Promise<void> {
  return discard(commands.transferClearFailed());
}

/**
 * 清除已完成+失败
 */
export function clearFinished(): Promise<void> {
  return discard(commands.transferClearFinished());
}

/**
 * 重试单个传输任务；Failed 重放，RestartRequired 请求重新规划。
 *
 * @param taskId - 传输任务 ID
 */
export function retryTransfer(taskId: number): Promise<void> {
  return discard(commands.transferRetry(taskId));
}
