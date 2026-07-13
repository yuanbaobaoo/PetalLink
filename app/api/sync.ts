/**
 * Sync API —— 同步引擎操作。
 */
import { invoke } from "./tauri";

/** 同步全局状态 */
export interface SyncGlobalState {
  total: number;
  completed: number;
  uploading: number;
  downloading: number;
  /** 因网络不可用而等待恢复的当前任务数（后端 JSON 为 camelCase）。 */
  waitingNetwork: number;
  failed: number;
  /** 传输队列中的永久失败历史数，不等同于当前同步项 failed。 */
  transferFailed: number;
  failed_items: FailedItem[];
  conflict: number;
  editing: number;
  is_running: boolean;
  last_sync_time: number | null;
  is_indexing: boolean;
  indexing_scanned_folders: number;
  indexing_discovered_items: number;
  content_changed: boolean;
  // 当前同步阶段（供状态条精确显示场景）；null/undefined = 空闲
  sync_phase?: string | null;
}

/** 失败项详情（供 SyncStatusBar 失败项弹窗） */
export interface FailedItem {
  relative_path: string;
  error_message?: string;
}

/** 释放空间安全校验结果 */
export type FreeUpResult = "safe" | "not_in_cloud" | "not_synced";

/** 文件本地同步状态（供删除确认用） */
export type FileLocalStatus = "folder" | "synced" | "placeholder" | "not_synced";

/** 批量文件状态映射（fileId → 同步状态字符串） */
export type BatchFileStatusMap = Record<string, string>;

/**
 * 手动刷新（全量 BFS + 同步周期）
 */
export function manualRefresh(): Promise<void> {
  return invoke<void>("sync_manual_refresh");
}

/**
 * 安全校验释放空间
 *
 * @param relPath - 文件相对路径
 * @param fileId - 文件 ID
 */
export function checkSafeFreeUp(relPath: string, fileId: string): Promise<string> {
  return invoke<string>("sync_check_safe_free_up", { relPath, fileId });
}

/**
 * 查询文件本地同步状态（供删除确认）
 *
 * @param fileId - 文件 ID
 */
export function checkFileLocalStatus(fileId: string): Promise<string> {
  return invoke<string>("sync_check_file_local_status", { fileId });
}

/**
 * 批量查询文件同步状态（供文件列表状态列展示）
 *
 * @param fileIds - 文件 ID 列表
 */
export function getBatchFileStatus(fileIds: string[]): Promise<BatchFileStatusMap> {
  return invoke<BatchFileStatusMap>("sync_batch_file_status", { fileIds });
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
  return invoke<void>("sync_free_up_space", { fileId, relPath, localPath, name, size });
}

/**
 * 按需下载单个文件到本地
 *
 * @param fileId - 文件 ID
 * @param destPath - 目标本地路径
 */
export function downloadOnDemand(fileId: string, destPath: string): Promise<boolean> {
  return invoke<boolean>("sync_download_on_demand", { fileId, destPath });
}

/**
 * 递归同步云端目录子树（下载缺失 + 上传本地独有 + 建目录），返回处理数。
 * 进度经 "folder_sync_progress" 事件推送 {done, total}。
 *
 * @param folderId - 云端目录 ID
 * @param relPath - 目录相对路径
 */
export function syncFolderRecursive(folderId: string, relPath: string): Promise<number> {
  return invoke<number>("sync_folder_recursive", { folderId, relPath });
}

/**
 * 重试失败项
 */
export function retryFailed(): Promise<void> {
  return invoke<void>("sync_retry_failed");
}

/**
 * 获取当前同步全局状态
 */
export function getSyncState(): Promise<SyncGlobalState> {
  return invoke<SyncGlobalState>("sync_state");
}
