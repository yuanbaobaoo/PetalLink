/**
 * 传输队列 Store —— 
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as transferApi from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import { TRANSFER_DIR, TRANSFER_STATE } from "@/api/transfer";

export const useTransferStore = defineStore("transfer", () => {
  // 全部传输任务
  const tasks = ref<TransferTask[]>([]);

  // 上传任务
  const uploads = computed(() => tasks.value.filter((t) => t.direction === TRANSFER_DIR.UPLOAD));
  // 下载任务
  const downloads = computed(() => tasks.value.filter((t) => t.direction === TRANSFER_DIR.DOWNLOAD));
  // 进行中
  const running = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.RUNNING).length);
  // 等待中
  const pending = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.PENDING).length);
  // 已完成
  const completed = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.COMPLETED).length);

  /** 加载全部传输任务 */
  async function loadAll(): Promise<void> {
    try {
      tasks.value = await transferApi.listAllTransfers();
    } catch {
      tasks.value = [];
    }
  }

  /** 清除已完成 */
  async function clearCompleted(): Promise<void> {
    await transferApi.clearCompleted();
    await loadAll();
  }

  /** 清除失败项 */
  async function clearFailed(): Promise<void> {
    await transferApi.clearFailed();
    await loadAll();
  }

  /** 清除已完成+失败 */
  async function clearFinished(): Promise<void> {
    await transferApi.clearFinished();
    await loadAll();
  }

  return {
    tasks, uploads, downloads, running, pending, completed,
    loadAll, clearCompleted, clearFailed, clearFinished,
  };
});
