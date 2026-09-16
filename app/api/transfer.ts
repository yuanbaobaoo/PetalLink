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
 * Failed 与 RestartRequired 都由 TaskRunner 直接重跑
 * （RestartRequired 不能等 planner 重规划：同名碰撞场景 planner 只会 Skip，任务会永久滞留）。
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
 * 任务是否处于「同名冲突待用户决策」：目标目录有同名远端文件且内容不一致。
 * 此类任务不会自动推进，必须等用户在覆盖/保留两者/取消中选择。
 */
export function isNameConflictTask(task: TransferTask): boolean {
  return task.state === TRANSFER_STATE.RESTART_REQUIRED
    && task.error_kind === TRANSFER_ERROR_KIND.NAME_CONFLICT;
}

/**
 * 仅暴露后端真正支持的取消入口（「需要重新检查」/失败任务，见 transfer_cancel）。
 */
export function canCancelTransferTask(task: TransferTask): boolean {
  return task.state === TRANSFER_STATE.RESTART_REQUIRED
    || task.state === TRANSFER_STATE.FAILED;
}

/**
 * 读取并收窄后端传输状态数值。
 */
export async function listAllTransfers(): Promise<TransferTask[]> {
  return await commands.transferListAll() as TransferTask[];
}

// 终态（已完成/失败/取消）历史在列表中的最大渲染条数。
// 大批量同步时队列可达数千行，全量渲染会打满 webview 主线程
// （2026-09-15 拖入两个大目录后传输队列卡死事故）。
export const TRANSFER_TERMINAL_HISTORY_LIMIT = 100;

/**
 * 收窄传输列表的渲染范围：非终态任务全量保留（数量天然小），
 * 终态历史只保留最近的若干条（入参按 created_at DESC 排列，靠前者最新）。
 *
 * @param tasks - 后端返回的全部传输任务
 * @param limit - 终态历史渲染上限
 * @returns 渲染用列表与被折叠的终态条数
 */
export function capTransferHistory(
  tasks: TransferTask[],
  limit: number = TRANSFER_TERMINAL_HISTORY_LIMIT,
): { items: TransferTask[]; hiddenTerminalCount: number } {
  // 终态任务集合。
  const terminalStates: ReadonlySet<number> = new Set([
    TRANSFER_STATE.COMPLETED,
    TRANSFER_STATE.FAILED,
    TRANSFER_STATE.CANCELED,
  ]);
  const items: TransferTask[] = [];
  // 已保留的终态条数。
  let terminalKept = 0;
  // 被折叠的终态条数。
  let hiddenTerminalCount = 0;
  for (const task of tasks) {
    if (!terminalStates.has(task.state)) {
      items.push(task);
      continue;
    }
    if (terminalKept < limit) {
      items.push(task);
      terminalKept += 1;
    } else {
      hiddenTerminalCount += 1;
    }
  }
  return { items, hiddenTerminalCount };
}

