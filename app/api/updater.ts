/**
 * Updater API —— 封装 @tauri-apps/plugin-updater 与后端 transfer_has_active。
 */
import { check } from "@tauri-apps/plugin-updater";
import type { Update } from "@tauri-apps/plugin-updater";
import { extractErrorMessage } from "@/utils/error";

/**
 * 更新清单没有当前平台条目。
 *
 * 这是发布产物覆盖范围，不是网络、签名或清单解析故障，调用方应按无更新处理。
 */
export class UpdatePlatformUnavailableError extends Error {
  readonly code = "UPDATE_PLATFORM_UNAVAILABLE";

  constructor(readonly details: string) {
    super(details);
    this.name = "UpdatePlatformUnavailableError";
  }
}

/**
 * 跨测试 mock / JS 边界识别“当前平台无产物”错误。
 */
export function isUpdatePlatformUnavailableError(
  error: unknown,
): error is UpdatePlatformUnavailableError {
  return error instanceof UpdatePlatformUnavailableError || (
    typeof error === "object"
    && error !== null
    && "code" in error
    && (error as { code?: unknown }).code === "UPDATE_PLATFORM_UNAVAILABLE"
  );
}

/**
 * Tauri updater 在清单缺少当前平台键时返回的稳定语义。
 * 保持窄匹配，避免把网络、签名或 JSON 解析错误误判为“暂未提供”。
 */
function isMissingPlatformEntry(message: string): boolean {
  const normalized = message.toLowerCase();
  return normalized.includes("none of the fallback platforms")
    && normalized.includes("were found in the response")
    && normalized.includes("platforms")
    && normalized.includes("object");
}

/**
 * 将 updater.check() 的底层错误转换为前端可区分的错误。
 */
function normalizeCheckError(error: unknown): Error {
  const message = extractErrorMessage(error);
  if (isMissingPlatformEntry(message)) {
    return new UpdatePlatformUnavailableError(message);
  }
  return new Error(`检查更新失败：${message}`);
}

/**
 * 更新信息（前端可用）
 */
export interface UpdateInfo {
  version: string;
  body?: string;
  date?: string;
}

/**
 * 下载进度事件
 */
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
  // 保留插件返回对象，供后续下载流程持有。
  let update: Update | null = null;
  try {
    update = await check();
  } catch (error) {
    // 是否静默由调用方决定；这里必须区分“已是最新”和“检查失败”。
    throw normalizeCheckError(error);
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
  // 安装前重新获取一次更新句柄，避免使用过期元数据。
  let update: Update | null = null;
  try {
    update = await check();
  } catch (error) {
    throw normalizeCheckError(error);
  }
  if (!update) throw new Error("没有可用更新");

  // Started 事件提供总量，后续分片事件复用该值计算进度。
  let total = 0;
  // 下载和安装由插件串行完成，回调只投影为前端阶段事件。
  try {
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
  } catch (error) {
    throw new Error(`下载或安装更新失败：${extractErrorMessage(error)}`);
  }
}
