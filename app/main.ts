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
import { createPinia } from "pinia";

// 全局样式（顺序：reset → tokens → animations）
import "./styles/reset.css";
import "./styles/tokens.css";
import "./styles/animations.css";

import App from "./App.vue";
import { on } from "@/api/tauri";

// 创建 Vue 应用，注入 Pinia 状态管理
const app = createApp(App);
const pinia = createPinia();
app.use(pinia);

app.mount("#app");

// ===== 全局事件监听（挂载后注册） =====
// 延迟导入 stores 避免 Pinia 未就绪
import { useSyncStore } from "@/stores/sync";
import type { SyncGlobalState } from "@/api/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useTransferStore } from "@/stores/transfer";

// 监听同步状态变化
on("sync_state", (state: unknown) => {
  const sync = useSyncStore();
  sync.applyState(state as SyncGlobalState);
}).catch(() => {});

// 监听目录内容变化 → 刷新文件列表 + 侧边栏
on("folder_content_changed", () => {
  const browser = useFileBrowserStore();
  browser.refresh().catch(() => {});
  // 计数器触发侧边栏刷新（布尔值无法重复触发 watch）
  const sync = useSyncStore();
  sync.sidebarRefresh++;
}).catch(() => {});

// 监听传输队列变化 → 重新加载
on("transfer_update", () => {
  const transfer = useTransferStore();
  transfer.loadAll().catch(() => {});
}).catch(() => {});
