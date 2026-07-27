/**
 * Sync API —— 同步引擎操作。
 */
import { commands } from "./generated";
import { call, discard } from "./tauri";
export type {
  FailedItem,
  FreeableItem,
  FreeUpBatchResult,
  SyncGlobalState,
} from "./generated";
import type {
  FileLocalStatus,
  FreeUpCheckResult,
  FreeableItem,
  FreeUpBatchResult,
  SyncGlobalState,
} from "./generated";

/**
 * 判断动态值是否为可安全表示的非负整数。
 */
function isNonNegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

/**
 * 校验来自 Tauri 事件的完整 v5 权威快照。
 *
 * 不接受缺字段或 revision=0 的默认对象，避免把“刷新信号”误当成真实状态。
 */
export function isSyncGlobalState(value: unknown): value is SyncGlobalState {
  // 先拒绝无法安全读取字段的输入。
  if (typeof value !== "object" || value === null) return false;
  // 事件载荷只在本函数内按动态键读取。
  const state = value as Record<string, unknown>;
  // 所有计数器必须是可精确表示的非负整数。
  const counters = [
    "revision",
    "total",
    "completed",
    "uploading",
    "downloading",
    "waiting_network",
    "failed",
    "transfer_failed",
    "conflict",
    "editing",
    "indexing_scanned_folders",
    "indexing_discovered_items",
  ];
  if (!counters.every((key) => isNonNegativeInteger(state[key]))) return false;
  // revision=0 是默认占位对象，不能覆盖已展示状态。
  if ((state.revision as number) === 0) return false;
  if (!Array.isArray(state.failed_items)) return false;
  if (!state.failed_items.every((item) => {
    if (typeof item !== "object" || item === null) return false;
    // 失败项合同只允许字符串路径和可空错误。
    const failedItem = item as Record<string, unknown>;
    return typeof failedItem.relative_path === "string"
      && (failedItem.error_message === null
        || typeof failedItem.error_message === "string");
  })) return false;
  // 布尔状态缺失会改变界面分支，因此不接受部分快照。
  if (
    typeof state.is_running !== "boolean"
    || typeof state.is_indexing !== "boolean"
    || typeof state.content_changed !== "boolean"
  ) return false;
  // 时间戳只允许有限数值或 null。
  if (
    state.last_sync_time !== null
    && (typeof state.last_sync_time !== "number"
      || !Number.isFinite(state.last_sync_time))
  ) return false;
  if (state.sync_phase !== undefined && typeof state.sync_phase !== "string") return false;
  return true;
}

/**
 * 释放空间安全校验结果
 */
export type FreeUpResult = FreeUpCheckResult;

/**
 * 文件本地同步状态（供删除确认用）
 */
export type { FileLocalStatus } from "./generated";

/**
 * 批量文件状态映射（fileId → 同步状态字符串）
 */
export type BatchFileStatusMap = Record<string, FileLocalStatus>;

/**
 * 手动刷新（全量 BFS + 同步周期）
 */
export function manualRefresh(): Promise<void> {
  return discard(commands.syncManualRefresh());
}

/**
 * 安全校验释放空间
 *
 * @param relPath - 文件相对路径
 * @param fileId - 文件 ID
 */
export function checkSafeFreeUp(relPath: string, fileId: string): Promise<FreeUpResult> {
  return call(commands.syncCheckSafeFreeUp(relPath, fileId));
}

/**
 * 查询文件本地同步状态（供删除确认）
 *
 * @param fileId - 文件 ID
 */
export function checkFileLocalStatus(fileId: string): Promise<FileLocalStatus> {
  return call(commands.syncCheckFileLocalStatus(fileId));
}

/**
 * 批量查询文件同步状态（供文件列表状态列展示）
 *
 * @param fileIds - 文件 ID 列表
 */
export function getBatchFileStatus(fileIds: string[]): Promise<BatchFileStatusMap> {
  return call(commands.syncBatchFileStatus(fileIds));
}

/**
 * 执行释放空间（删本地 + 建占位符 + 更新 DB）
 *
 * @param fileId - 文件 ID
 * @param relPath - 文件相对路径
 * @param localPath - 本地绝对路径
 * @param name - 文件名
 * @param size - 文件大小
 */
export function freeUpSpace(
  fileId: string,
  relPath: string,
  localPath: string,
  name: string,
  size: number,
): Promise<void> {
  return discard(commands.syncFreeUpSpace(fileId, relPath, localPath, name, size));
}

/**
 * 枚举目录（含子树）下可释放空间的文件候选项
 *
 * @param folderRelPath - 目录相对路径，传空串表示从根枚举
 */
export function listFreeableInFolder(folderRelPath: string): Promise<FreeableItem[]> {
  return call(commands.syncListFreeableInFolder(folderRelPath));
}

/**
 * 批量释放多个文件的本地空间，逐项独立执行
 *
 * @param items - 经用户确认的可释放候选项清单
 */
export function freeUpBatch(items: FreeableItem[]): Promise<FreeUpBatchResult> {
  return call(commands.syncFreeUpBatch(items));
}

/**
 * 按需下载单个文件到本地
 *
 * @param fileId - 文件 ID
 * @param destPath - 目标本地路径
 */
export function downloadOnDemand(fileId: string, destPath: string): Promise<boolean> {
  return call(commands.syncDownloadOnDemand(fileId, destPath));
}

/**
 * 递归同步云端目录子树（下载缺失 + 上传本地独有 + 建目录），返回处理数。
 * 进度经 "folder_sync_progress" 事件推送 {done, total}。
 *
 * @param folderId - 云端目录 ID
 * @param relPath - 目录相对路径
 */
export function syncFolderRecursive(folderId: string, relPath: string): Promise<number> {
  return call(commands.syncFolderRecursive(folderId, relPath));
}

/**
 * 重试失败项
 */
export function retryFailed(): Promise<void> {
  return discard(commands.syncRetryFailed());
}

/**
 * 获取当前同步全局状态
 */
export function getSyncState(): Promise<SyncGlobalState> {
  return call(commands.syncState());
}
