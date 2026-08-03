import { importLocalFiles } from "@/api/drive";
import type { ImportFilesResult } from "@/api/drive";
import { useSyncStore } from "@/stores/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { showToast } from "@/components/mate";
import { extractErrorMessage } from "@/utils/error";

// Toast 通知的语义变体。
type NotifyVariant = "success" | "warning" | "error";

/**
 * 拖拽导入流程所需的外部能力。
 */
export interface DragUploadPort {
  rejectionReason: () => string | null;
  targetRelPath: () => string;
  importFiles: (paths: string[], targetRelPath: string) => Promise<ImportFilesResult>;
  refresh: () => Promise<void>;
  notify: (message: string, variant: NotifyVariant) => void;
}

/**
 * 拖拽导入完成后的明确结果。
 */
export interface DragUploadResult {
  // 成功导入的文件数。
  imported: number;
  // 失败项数（含同名冲突拒绝覆盖）。
  failed: number;
}

/**
 * 把拖入的本地路径导入当前同步文件夹，并按后端汇总结果通知用户。
 *
 * @param paths - 拖入的本地绝对路径数组
 * @param port - 拖拽导入流程依赖
 * @returns 导入结果；被守卫拒绝或整体异常时返回 null
 */
export async function runDragImport(
  paths: string[],
  port: DragUploadPort,
): Promise<DragUploadResult | null> {
  // 空拖放（如拖入文本）不产生任何动作。
  if (paths.length === 0) return null;

  // 守卫拒绝必须发生在任何后端调用之前。
  const rejection = port.rejectionReason();
  if (rejection) {
    port.notify(rejection, "warning");
    return null;
  }

  try {
    // 后端逐源复制并触发后台同步周期；部分失败汇总在 failures。
    const result = await port.importFiles(paths, port.targetRelPath());
    const failed = result.failures.length;
    if (result.imported === 0 && failed > 0) {
      port.notify(`导入失败：${result.failures[0].reason}`, "error");
    } else if (failed > 0) {
      port.notify(`已导入 ${result.imported} 项，${failed} 项未导入：${result.failures[0].reason}`, "warning");
    } else {
      port.notify(`已导入 ${result.imported} 项，正在后台同步到云端`, "success");
    }
    // 复制落盘后列表仍是云端视图，静默刷新等 folder_content_changed 接力。
    await port.refresh();
    return { imported: result.imported, failed };
  } catch (e) {
    port.notify(`导入失败：${extractErrorMessage(e)}`, "error");
    return null;
  }
}

/**
 * 文件列表拖放事件的生产入口，组装真实 store 与后端命令。
 *
 * @param paths - 拖入的本地绝对路径数组
 */
export async function runDragImportFromDrop(paths: string[]): Promise<DragUploadResult | null> {
  // 当前同步状态。
  const sync = useSyncStore();
  // 当前文件浏览器状态。
  const browser = useFileBrowserStore();
  return runDragImport(paths, {
    rejectionReason: () => {
      if (!sync.mountConfigured) return "请先配置同步目录";
      if (sync.isIndexing) return "正在读取云端文件，请稍后再试";
      return null;
    },
    targetRelPath: () => browser.currentRelPath,
    importFiles: importLocalFiles,
    refresh: browser.refresh,
    notify: (message, variant) => showToast(message, { variant }),
  });
}
