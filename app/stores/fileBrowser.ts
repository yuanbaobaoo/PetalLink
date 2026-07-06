/**
 * 文件浏览器 Store —— 路径栈 + 文件列表（真实后端数据）。
 * 
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as driveApi from "@/api/drive";
import type { DriveFile } from "@/api/drive";
import { extractErrorMessage } from "@/utils/error";

/** 文件夹位置 */
export interface FolderLocation {
  id: string;
  name: string;
}

// 根目录
export const ROOT: FolderLocation = { id: "", name: "我的云盘" };

export const useFileBrowserStore = defineStore("fileBrowser", () => {
  // 路径栈
  const pathStack = ref<FolderLocation[]>([ROOT]);
  // 当前文件列表
  const files = ref<DriveFile[]>([]);
  // 加载中
  const loading = ref(false);
  // 错误信息
  const errorMessage = ref<string | null>(null);

  // 当前文件夹位置
  const current = computed(() => pathStack.value[pathStack.value.length - 1]);

  /** 加载当前目录内容 */
  async function loadCurrent(): Promise<void> {
    loading.value = true;
    errorMessage.value = null;
    try {
      files.value = await driveApi.listFiles(current.value.id || undefined);
    } catch (e) {
      errorMessage.value = extractErrorMessage(e);
      files.value = [];
    } finally {
      loading.value = false;
    }
  }

  /** 加载根目录 */
  async function loadRoot(): Promise<void> {
    pathStack.value = [ROOT];
    await loadCurrent();
  }

  /** 进入文件夹 */
  async function enterFolder(folder: DriveFile): Promise<void> {
    if (!driveApi.isFolder(folder)) return;
    pathStack.value.push({ id: folder.id, name: folder.name });
    await loadCurrent();
  }

  /** 跳转到路径中的第 i 级 */
  async function jumpTo(index: number): Promise<void> {
    pathStack.value = pathStack.value.slice(0, index + 1);
    await loadCurrent();
  }

  /** 返回上级 */
  async function goUp(): Promise<void> {
    if (pathStack.value.length <= 1) return;
    await jumpTo(pathStack.value.length - 2);
  }

  /** 刷新当前目录 */
  async function refresh(): Promise<void> {
    await loadCurrent();
  }

  return {
    pathStack,
    files,
    loading,
    errorMessage,
    current,
    loadRoot,
    loadCurrent,
    enterFolder,
    jumpTo,
    goUp,
    refresh,
  };
});
