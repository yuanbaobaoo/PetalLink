/**
 * 文件浏览器 Store —— 路径栈 + 文件列表（真实后端数据）。
 * 
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as driveApi from "@/api/drive";
import type { DriveFile } from "@/api/drive";
import { extractErrorMessage } from "@/utils/error";

/**
 * 文件夹位置
 */
export interface FolderLocation {
  id: string;
  name: string;
}

// 根目录
export const ROOT: FolderLocation = { id: "", name: "我的云盘" };

// 全局文件浏览器 Store。
export const useFileBrowserStore = defineStore("fileBrowser", () => {
  // 路径栈
  const pathStack = ref<FolderLocation[]>([ROOT]);
  // 当前文件列表
  const files = ref<DriveFile[]>([]);
  // 加载中
  const loading = ref(false);
  // 错误信息
  const errorMessage = ref<string | null>(null);
  // 目录加载请求代次：只有最新一次请求的结果才能落到 files 上
  let loadRequestSeq = 0;

  // 当前文件夹位置
  const current = computed(() => pathStack.value[pathStack.value.length - 1]);
  // 当前文件夹相对挂载根路径（跳过根节点按名称拼接，根目录为空串）
  const currentRelPath = computed(() =>
    pathStack.value.slice(1).map((location) => location.name).join("/"),
  );

  /**
   * 加载当前目录内容
   *
   * @param silent - 静默刷新：不切换 loading（避免遮罩闪烁），失败时保留旧列表；
   *   目录切换/首次加载传 false 走完整 loading 流程
   */
  async function loadCurrent(silent = false): Promise<void> {
    // 本次请求代次：目录切换后旧响应必须被丢弃，否则会覆盖新目录列表。
    const requestSeq = ++loadRequestSeq;
    if (!silent) loading.value = true;
    errorMessage.value = null;
    try {
      // 目录内容按当前 folderId 拉取。
      const loaded = await driveApi.listFiles(current.value.id || undefined);
      if (requestSeq !== loadRequestSeq) return;
      files.value = loaded;
    } catch (e) {
      if (requestSeq !== loadRequestSeq) return;
      errorMessage.value = extractErrorMessage(e);
      if (!silent) files.value = [];
    } finally {
      // 最新一次请求结束时必须清掉 loading，即使它是静默刷新：
      // 否则「非静默加载被静默刷新超越」会把遮罩永久卡在列表上。
      if (requestSeq === loadRequestSeq) loading.value = false;
    }
  }

  /**
   * 加载根目录
   */
  async function loadRoot(): Promise<void> {
    pathStack.value = [ROOT];
    await loadCurrent();
  }

  /**
   * 进入文件夹
   */
  async function enterFolder(folder: DriveFile): Promise<void> {
    if (!driveApi.isFolder(folder)) return;
    pathStack.value.push({ id: folder.id, name: folder.name });
    await loadCurrent();
  }

  /**
   * 跳转到路径中的第 i 级
   */
  async function jumpTo(index: number): Promise<void> {
    pathStack.value = pathStack.value.slice(0, index + 1);
    await loadCurrent();
  }

  /**
   * 跳转到任意已解析路径（侧边目录树专用：树提供的路径不是当前栈的前缀）。
   */
  async function navigateTo(path: FolderLocation[]): Promise<void> {
    pathStack.value = [...path];
    await loadCurrent();
  }

  /**
   * 返回上级
   */
  async function goUp(): Promise<void> {
    if (pathStack.value.length <= 1) return;
    await jumpTo(pathStack.value.length - 2);
  }

  /**
   * 刷新当前目录（静默：已有列表不闪 loading 遮罩，失败保留旧数据）
   */
  async function refresh(): Promise<void> {
    await loadCurrent(true);
  }

  return {
    pathStack,
    files,
    loading,
    errorMessage,
    current,
    currentRelPath,
    loadRoot,
    loadCurrent,
    enterFolder,
    jumpTo,
    navigateTo,
    goUp,
    refresh,
  };
});
