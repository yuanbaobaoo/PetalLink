/**
 * 更新状态 Store —— 管理更新检查、下载、安装全流程。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as updaterApi from "@/api/updater";
import type { UpdateInfo, DownloadProgress } from "@/api/updater";

export type UpdatePhase =
  | "idle"           // 空闲
  | "checking"       // 检查中
  | "available"      // 有更新可用
  | "upToDate"       // 已是最新（手动检查结果）
  | "downloading"    // 下载中
  | "downloaded"     // 下载完成，等待传输
  | "waitingTransfer" // 等待传输任务完成
  | "ready"          // 准备重启
  | "error";         // 出错

export const useUpdaterStore = defineStore("updater", () => {
  // ---- 状态 ----
  const phase = ref<UpdatePhase>("idle");
  const updateInfo = ref<UpdateInfo | null>(null);
  const downloadProgress = ref(0); // 0-100
  const downloadTotal = ref(0);
  const downloaded = ref(0);
  const errorMessage = ref("");
  /** 用户是否已关闭侧边栏提示（本次启动不再显示） */
  const dismissed = ref(false);
  /** 上次检查时间 */
  const lastCheckTime = ref<number | null>(null);
  /** 对话框是否打开（控制 UpdateDialog 显隐） */
  const dialogOpen = ref(false);

  // ---- 计算 ----
  const updateAvailable = computed(() => phase.value === "available");
  const isChecking = computed(() => phase.value === "checking");
  const isDownloading = computed(() => phase.value === "downloading");
  const newVersion = computed(() => updateInfo.value?.version ?? "");

  // ---- 动作 ----

  /** 静默检查更新（启动时使用），仅更新侧边栏提示，不弹对话框 */
  async function silentCheck(): Promise<void> {
    try {
      const info = await updaterApi.checkForUpdate();
      if (info) {
        updateInfo.value = info;
        phase.value = "available";
        dismissed.value = false;
        dialogOpen.value = false; // 启动时不弹窗
      }
    } catch {
      // 静默失败
    }
    lastCheckTime.value = Date.now();
  }

  /** 手动检查更新（关于页 / 侧边栏点击），有更新时自动弹出对话框 */
  async function manualCheck(): Promise<boolean> {
    phase.value = "checking";
    errorMessage.value = "";
    try {
      const info = await updaterApi.checkForUpdate();
      if (info) {
        updateInfo.value = info;
        phase.value = "available";
        dialogOpen.value = true; // 手动检查 → 弹窗
        lastCheckTime.value = Date.now();
        return true;
      } else {
        phase.value = "upToDate";
        lastCheckTime.value = Date.now();
        return false;
      }
    } catch (e) {
      phase.value = "error";
      errorMessage.value = String(e);
      lastCheckTime.value = Date.now();
      return false;
    }
  }

  /** 打开更新对话框（供侧边栏点击使用） */
  function showDialog(): void {
    if (phase.value === "available") {
      dialogOpen.value = true;
    }
  }

  /** 下载并安装 */
  async function downloadAndInstall(): Promise<void> {
    phase.value = "downloading";
    downloadProgress.value = 0;
    errorMessage.value = "";
    try {
      await updaterApi.downloadAndInstall((p: DownloadProgress) => {
        if (p.stage === "started") {
          downloadTotal.value = p.total ?? 0;
        } else if (p.stage === "progress" && p.total && p.total > 0) {
          downloaded.value += p.downloaded ?? 0;
          downloadProgress.value = Math.min(
            Math.round((downloaded.value / p.total) * 100),
            99,
          );
        } else if (p.stage === "finished") {
          downloadProgress.value = 100;
        }
      });
      // 下载安装完成 → 等待传输
      phase.value = "downloaded";
    } catch (e) {
      phase.value = "error";
      errorMessage.value = String(e);
    }
  }

  /** 检查传输并决定是否可重启。完成后如果对话框已关闭则自动弹出提醒。 */
  async function waitForTransfers(): Promise<boolean> {
    phase.value = "waitingTransfer";
    const maxWaitMs = 5 * 60 * 1000; // 最多等 5 分钟
    const pollIntervalMs = 2000;
    const startTime = Date.now();
    while (Date.now() - startTime < maxWaitMs) {
      try {
        const hasActive = await updaterApi.hasActiveTransfers();
        if (!hasActive) {
          phase.value = "ready";
          dialogOpen.value = true; // 传输完成 → 重新弹出对话框提示重启
          return true;
        }
      } catch {
        // 查询失败也继续等
      }
      await new Promise((r) => setTimeout(r, pollIntervalMs));
    }
    // 超时，但传输仍在进行 → 提示用户
    phase.value = "downloaded";
    dialogOpen.value = true;
    return false;
  }

  /** 关闭侧边栏更新提示（本次启动不再显示） */
  function dismissUpdate(): void {
    dismissed.value = true;
  }

  /** 关闭对话框 */
  function closeDialog(): void {
    dialogOpen.value = false;
    if (phase.value === "available" || phase.value === "upToDate" || phase.value === "error") {
      // 保持 available 状态以便侧边栏继续显示提示
    }
  }

  /** 重置为 idle */
  function reset(): void {
    phase.value = "idle";
    dialogOpen.value = false;
    errorMessage.value = "";
  }

  return {
    phase,
    updateInfo,
    downloadProgress,
    downloadTotal,
    downloaded,
    errorMessage,
    dismissed,
    lastCheckTime,
    dialogOpen,
    // 计算
    updateAvailable,
    isChecking,
    isDownloading,
    newVersion,
    // 动作
    silentCheck,
    manualCheck,
    showDialog,
    downloadAndInstall,
    waitForTransfers,
    dismissUpdate,
    closeDialog,
    reset,
  };
});
