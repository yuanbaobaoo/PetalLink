/**
 * 同步 Store —— 全局同步状态。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as syncApi from "@/api/sync";
import type { SyncGlobalState, FailedItem } from "@/api/sync";
import * as configApi from "@/api/config";

// 重新导出 FailedItem，保持既有导入路径（`from "@/stores/sync"`）可用
export type { FailedItem };

export const useSyncStore = defineStore("sync", () => {
  // 全局同步状态
  const total = ref(0);
  const completed = ref(0);
  const uploading = ref(0);
  const downloading = ref(0);
  const failed = ref(0);
  const failedItems = ref<FailedItem[]>([]);
  const conflict = ref(0);
  const editing = ref(0);
  const isRunning = ref(false);
  const isIndexing = ref(false);
  // 当前同步阶段（精确显示：indexing-startup / querying-changes / syncing-local 等）
  const syncPhase = ref<string | null>(null);
  const lastSyncTime = ref<number | null>(null);
  const contentChanged = ref(false);
  // 侧边栏刷新计数器（folder_content_changed 事件每触一次 +1，布尔值无法重复触发 watch）
  const sidebarRefresh = ref(0);
  // 是否已配置同步目录
  const mountConfigured = ref(false);
  // 同步目录路径
  const mountDir = ref("");
  // 同步阶段
  const setupPhase = ref<"loading" | "needsSetup" | "needsFirstSync" | "active">("loading");

  // 进度
  const progress = computed(() => {
    if (total.value === 0) return 1.0;
    return completed.value / total.value;
  });

  // 是否有活跃传输
  const hasActiveTransfer = computed(() => uploading.value + downloading.value > 0);

  /** 应用一份同步状态到 store（供事件回调和主动拉取共用）。 */
  function applyState(s: SyncGlobalState): void {
    total.value = s.total ?? 0;
    completed.value = s.completed ?? 0;
    uploading.value = s.uploading ?? 0;
    downloading.value = s.downloading ?? 0;
    failed.value = s.failed ?? 0;
    failedItems.value = Array.isArray(s.failed_items) ? s.failed_items : [];
    conflict.value = s.conflict ?? 0;
    editing.value = s.editing ?? 0;
    isRunning.value = s.is_running ?? false;
    lastSyncTime.value = s.last_sync_time ?? null;
    isIndexing.value = s.is_indexing ?? false;
    syncPhase.value = s.sync_phase ?? null;
    if (s.content_changed) {
      contentChanged.value = true;
      sidebarRefresh.value++;
    } else {
      contentChanged.value = false;
    }
  }

  /** 初始化：加载配置判断阶段；配置就绪时主动拉一次当前同步状态，
   *  避免错过配置完成前已发出的 is_indexing 事件（BFS 可能先于 init 启动）。 */
  async function init(): Promise<void> {
    try {
      const config = await configApi.loadConfig();
      mountConfigured.value = config.mount_configured;
      mountDir.value = config.mount_dir;
      if (!config.mount_configured) {
        setupPhase.value = "needsSetup";
      } else {
        setupPhase.value = "active";
        // 主动拉取当前状态：配置刚就绪，引擎 BFS 可能已在跑并广播了 is_indexing=true，
        // 但那时 mountConfigured 还是 false、状态条未渲染 → 该事件被"错过"。
        // 这里同步一次真实状态，确保 UI（状态条"正在读取云端索引…"、刷新按钮转圈）正确。
        try {
          const state = await syncApi.getSyncState();
          applyState(state);
        } catch {
          // 引擎尚未启动（配置目录但引擎启动失败）→ 忽略，保留默认状态
        }
      }
    } catch {
      setupPhase.value = "needsSetup";
    }
  }

  /** 触发全量刷新 */
  async function triggerManualRefresh(): Promise<void> {
    try {
      await syncApi.manualRefresh();
    } catch {
      // handled by event update
    }
  }

  /** 重试失败项 */
  async function retryFailed(): Promise<void> {
    try {
      await syncApi.retryFailed();
    } catch {
      // handled by event update
    }
  }

  return {
    total, completed, uploading, downloading, failed, failedItems, conflict, editing,
    isRunning, isIndexing, syncPhase, lastSyncTime, contentChanged,
    mountConfigured, setupPhase, mountDir, progress, hasActiveTransfer,
    init, applyState, triggerManualRefresh, retryFailed,
    sidebarRefresh,
  };
});
