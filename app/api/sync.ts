/**
 * Sync API —— 同步引擎操作。
 */
export type {
  FailedItem,
  FreeableItem,
  FreeUpBatchResult,
  SyncGlobalState,
} from "./generated";
import type {
  FileLocalStatus,
  FreeUpCheckResult,
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
