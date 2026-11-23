/**
 * PetalLink 前端入口 —— createApp + Pinia + 全局样式 + 事件监听
 *
 * 启动顺序：
 * 1. 加载全局样式
 * 2. 创建应用 + Pinia
 * 3. 挂载到 #app
 * 4. 注册 Tauri 事件监听（sync_state / folder_content_changed / transfer_update）
 */
import { createApp } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createPinia } from "pinia";

// 全局样式（顺序：reset → tokens → animations）
import "./styles/reset.css";
import "./styles/tokens.css";
import "./styles/animations.css";

import App from "./App.vue";
import { events } from "@/api/generated";

// Vue 应用实例。
const APP = createApp(App);
// 全局 Pinia 实例。
const PINIA = createPinia();
APP.use(PINIA);

APP.mount("#app");

// 仅 release 模式屏蔽 WebView 原生右键菜单（Reload / Inspect Element）；
// dev 模式保留，方便调试。文件列表等自定义右键菜单自行 preventDefault，不受此影响。
if (import.meta.env.PROD) {
  document.addEventListener("contextmenu", (e) => e.preventDefault());
}

// ===== 全局事件监听（挂载后注册） =====
// 延迟导入 stores 避免 Pinia 未就绪
import { useSyncStore } from "@/stores/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useTransferStore } from "@/stores/transfer";
import { useFolderSyncStore } from "@/stores/folderSync";
import { throttleTrailing } from "@/utils/debounce";
import { showToast } from "@/components/mate";

// 上传失败提示（自动同步的上传失败，非用户手动操作）
// 展示文件名 + 具体错误原因（如"空间不足"），5 秒去重避免刷屏
let _lastFailToastTime = 0;
// 最近一次上传失败提示文本。
let _lastFailToastMsg = "";

/**
 * 静默刷新文件列表，事件触发失败不打断其他监听器。
 */
async function refreshBrowser(): Promise<void> {
  // 后端还会继续广播状态，单次刷新失败可由下一次事件自然恢复。
  try {
    // 当前文件浏览器状态。
    const browser = useFileBrowserStore();
    await browser.refresh();
  } catch {}
}

/**
 * 静默重载传输队列，事件触发失败不产生未处理 Promise。
 */
async function reloadTransfers(): Promise<void> {
  try {
    // 当前传输队列状态。
    const transfer = useTransferStore();
    await transfer.loadAll();
  } catch {}
}

/**
 * 把失败数量同步到 Dock 角标。
 *
 * 窗口不在前台时这是唯一能看见「有东西失败了」的系统级提示；
 * 角标只是提醒，真实状态与重试入口仍在传输队列里（U09）。
 */
async function syncDockBadge(failed: number): Promise<void> {
  try {
    await getCurrentWindow().setBadgeCount(failed > 0 ? failed : undefined);
  } catch {
    // 平台不支持角标时静默降级，不影响其余反馈。
  }
}

/**
 * 注册后端事件监听器。
 */
async function registerGlobalListeners(): Promise<void> {
  // 各监听器独立注册，单个事件不可用时不影响其余事件。
  try {
    await events.syncState.listen(({ payload: state }) => {
      // 当前同步状态。
      const sync = useSyncStore();
      // sync_state 只承载完整权威状态；队列变化由独立事件重载。
      sync.applyState(state);
      void syncDockBadge(state.failed);
    });
  } catch {}

  try {
    await events.folderContentChanged.listen(() => {
      void refreshBrowser();
      // 计数器允许连续事件重复触发侧边栏刷新。
      const sync = useSyncStore();
      sync.sidebarRefresh++;
    });
  } catch {}

  try {
    // 传输事件在高峰（大批量上传 + 进度回调）可达每秒十余次；
    // 拖尾节流合并洪峰，每次仍触发全量 loadAll，但每秒最多 ~3 次。
    const scheduleTransferReload = throttleTrailing(() => {
      void reloadTransfers();
    }, 300);
    await events.transferUpdate.listen(() => {
      scheduleTransferReload();
    });
  } catch {}

  try {
    // 传输成功结算后主动刷新当前目录：不必再等云轮询周期（默认 60 秒）。
    // 批量上传时结算事件很密集，用 800ms 拖尾节流合并为一次列表重载。
    const scheduleSettledRefresh = throttleTrailing(() => {
      void refreshBrowser();
    }, 800);
    await events.transferSettled.listen(() => {
      scheduleSettledRefresh();
    });
  } catch {}

  try {
    // 目录同步作业变化（入队/开始规划/完成/失败）都整表重载：
    // 作业数量天然很小，整表重载比增量合并更不容易出错。
    await events.folderSyncJobsChanged.listen(() => {
      void useFolderSyncStore().loadAll();
    });
  } catch {}

  try {
    await events.virtualDriveStatus.listen(({ payload }) => {
      useSyncStore().applyVirtualDriveStatus(payload);
    });
  } catch {}

  try {
    await events.uploadFailed.listen(({ payload }) => {
      // 提示中优先展示后端返回的文件名和错误。
      const name = payload?.name ?? "未知文件";
      // 可读的失败原因。
      const error = payload?.error ?? "未知原因";
      // 完整提示文本同时作为去重键。
      const message = `上传失败：${name}（${error}）`;
      // 本次事件时间。
      const now = Date.now();
      // 同一错误 5 秒内只提示一次，避免重试风暴刷屏。
      if (message === _lastFailToastMsg && now - _lastFailToastTime < 5000) return;
      _lastFailToastTime = now;
      _lastFailToastMsg = message;
      showToast(message, { variant: "error" });
    });
  } catch {}
}

void registerGlobalListeners();
