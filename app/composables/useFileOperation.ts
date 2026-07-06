/**
 * 文件操作封装 —— 统一「守卫 → 执行 → toast → 刷新」流程。
 *
 * 各操作函数（删除 / 重命名 / 释放空间 / 批量下载 等）的差异点通过 options 参数化：
 * - 是否需要索引守卫（assertSyncAllowed）
 * - 是否需要同步目录守卫（mountConfigured）
 * - 成功 / 失败后的 toast 文案
 * - 是否完成后刷新文件列表
 * - 是否完成后清除多选状态
 *
 * 业务逻辑（确认弹窗、循环调用、状态查询等）仍由调用方在 action 闭包内完成，
 * 本 composable 只负责「守卫 + 错误归一 + 统一通知」的外壳。
 */

import { showToast } from "@/components/mate";
import { extractErrorMessage } from "@/utils/error";

/** useFileOperation 所需的外部依赖（由调用方注入，避免硬编码 store 引用）。 */
export interface FileOperationDeps {
  /** 索引中？cloud_tree 正在 BFS 重建，此时操作基于不完整数据 */
  isIndexing: () => boolean;
  /** 是否已配置同步目录 */
  mountConfigured: () => boolean;
  /** 刷新当前文件列表 */
  refresh: () => Promise<void>;
  /** 清除多选状态（批量操作完成后调用） */
  clearSelection?: () => void;
}

/** 单次操作的配置。 */
export interface FileOperationOptions {
  /** 前缀提示（如"删除"），失败时显示"删除失败：xxx" */
  errorPrefix: string;
  /** 成功时显示的提示；不传则不显示成功 toast */
  successToast?: string;
  /** 是否需要索引守卫（默认 true） */
  requireSyncGuard?: boolean;
  /** 是否需要同步目录守卫（默认 false） */
  requireMount?: boolean;
  /** 是否完成后刷新文件列表（默认 true） */
  refreshAfter?: boolean;
  /** 是否完成后清除多选状态（默认 false） */
  clearSelectionAfter?: boolean;
}

/**
 * 创建一组文件操作辅助函数。
 *
 * 用法：
 * ```ts
 * const { guard, runAction } = useFileOperation({ isIndexing: () => sync.isIndexing, ... });
 *
 * async function handleDelete(f) {
 *   if (!guard()) return;
 *   if (!(await confirmDialog(...))) return;
 *   await runAction({ errorPrefix: "删除", successToast: "已删除" }, () => driveApi.deleteFile(f.id));
 * }
 * ```
 */
export function useFileOperation(deps: FileOperationDeps) {
  /**
   * 索引 + 同步目录守卫。
   * 索引中 → toast 警告并返回 false；未配置目录 → toast 警告并返回 false。
   */
  function guard(opts?: { requireMount?: boolean }): boolean {
    if (deps.isIndexing()) {
      showToast("正在读取云端索引，请稍后再试", { variant: "warning" });
      return false;
    }
    if (opts?.requireMount && !deps.mountConfigured()) {
      showToast("请先在设置中配置同步目录", { variant: "warning" });
      return false;
    }
    return true;
  }

  /**
   * 执行一个操作：调用 action → 成功时 toast + 可选刷新/清除选中；失败时 toast 错误。
   *
   * action 内部不应再处理 toast —— 统一由本函数负责。
   */
  async function runAction(
    options: FileOperationOptions,
    action: () => Promise<void>,
  ): Promise<boolean> {
    const {
      errorPrefix,
      successToast,
      refreshAfter = true,
      clearSelectionAfter = false,
    } = options;

    try {
      await action();
      if (successToast) showToast(successToast);
      if (refreshAfter) await deps.refresh();
      if (clearSelectionAfter) deps.clearSelection?.();
      return true;
    } catch (e) {
      showToast(`${errorPrefix}失败：` + extractErrorMessage(e), { variant: "error" });
      return false;
    }
  }

  return { guard, runAction };
}
