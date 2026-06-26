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
  failed: number;
  failed_items: FailedItem[];
  conflict: number;
  editing: number;
  is_running: boolean;
  last_sync_time: number | null;
  is_indexing: boolean;
  indexing_scanned_folders: number;
  indexing_discovered_items: number;
  content_changed: boolean;
}

/** 失败项详情（供 SyncStatusBar 失败项弹窗） */
export interface FailedItem {
  relative_path: string;
  error_message?: string;
}

/** 释放空间安全校验结果 */
export type FreeUpResult = "safe" | "not_in_cloud" | "not_synced";

/** 手动刷新（全量 BFS + 同步周期） */
export function manualRefresh(): Promise<void> {
  return invoke<void>("sync_manual_refresh");
}

/** 安全校验释放空间 */
export function checkSafeFreeUp(relPath: string, fileId: string): Promise<string> {
  return invoke<string>("sync_check_safe_free_up", { relPath, fileId });
}

/** 执行释放空间（删本地 + 建占位符 + 更新 DB） */
export function freeUpSpace(fileId: string, relPath: string, localPath: string, name: string, size: number): Promise<void> {
  return invoke<void>("sync_free_up_space", { fileId, relPath, localPath, name, size });
}

/** 按需下载单个文件 */
export function downloadOnDemand(fileId: string, destPath: string): Promise<boolean> {
  return invoke<boolean>("sync_download_on_demand", { fileId, destPath });
}

/** 递归同步云端目录子树（下载缺失 + 上传本地独有 + 建目录），返回处理数。
 *  进度经 "folder_sync_progress" 事件推送 {done, total}。 */
export function syncFolderRecursive(folderId: string, relPath: string): Promise<number> {
  return invoke<number>("sync_folder_recursive", { folderId, relPath });
}

/** 重试失败项 */
export function retryFailed(): Promise<void> {
  return invoke<void>("sync_retry_failed");
}

/** 获取当前同步状态 */
export function getSyncState(): Promise<SyncGlobalState> {
  return invoke<SyncGlobalState>("sync_state");
}
