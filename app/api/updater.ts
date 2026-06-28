/**
 * Updater API —— 封装 @tauri-apps/plugin-updater 与后端 transfer_has_active。
 */
import { check } from "@tauri-apps/plugin-updater";
import type { Update } from "@tauri-apps/plugin-updater";
import { invoke } from "./tauri";

/** 更新信息（前端可用） */
export interface UpdateInfo {
  version: string;
  body?: string;
  date?: string;
}

/** 下载进度事件 */
export interface DownloadProgress {
  stage: "started" | "progress" | "finished";
  downloaded?: number;
  total?: number;
}

/**
 * 检查是否有可用更新。
 * 返回 UpdateInfo 表示有新版本，null 表示已是最新。
 */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  let update: Update | null = null;
  try {
    update = await check();
  } catch {
    // 网络错误等静默返回 null
    return null;
  }
  if (!update) return null;
  return {
    version: update.version,
    body: update.body,
    date: update.date,
  };
}

/**
 * 下载并安装更新。传入 onProgress 回调以获取进度。
 * 下载完成后会自动准备安装（需重启生效）。
 */
export async function downloadAndInstall(
  onProgress?: (p: DownloadProgress) => void
): Promise<void> {
  let update: Update | null = null;
  try {
    update = await check();
  } catch (e) {
    throw new Error(`检查更新失败：${String(e)}`);
  }
  if (!update) throw new Error("没有可用更新");

  let total = 0;
  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress?.({ stage: "started", total });
        break;
      case "Progress":
        onProgress?.({
          stage: "progress",
          downloaded: event.data.chunkLength,
          total,
        });
        break;
      case "Finished":
        onProgress?.({ stage: "finished" });
        break;
    }
  });
}

/** 检查是否有进行中的传输任务（PENDING / RUNNING） */
export function hasActiveTransfers(): Promise<boolean> {
  return invoke<boolean>("transfer_has_active");
}
