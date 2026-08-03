/**
 * 同步 Store —— 全局同步状态。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { commands } from "@/api/generated";
import * as syncApi from "@/api/sync";
import type { FailedItem } from "@/api/sync";
import * as configApi from "@/api/config";
import { isLinuxPlatform } from "@/utils/platform";

// 重新导出 FailedItem，保持既有导入路径（`from "@/stores/sync"`）可用
export type { FailedItem };

// 全局同步 Store。
export const useSyncStore = defineStore("sync", () => {
  // Linux 发行版只有 FUSE 云盘模式；其他平台保留传统同步目录。
  const isLinux = isLinuxPlatform();
  // 全局同步状态
  const revision = ref(0);
  // 本轮总任务数。
  const total = ref(0);
  // 本轮已完成任务数。
  const completed = ref(0);
  // 正在上传的任务数。
  const uploading = ref(0);
  // 正在下载的任务数。
  const downloading = ref(0);
  // 等待网络的任务数。
  const waitingNetwork = ref(0);
  // 当前同步失败数。
  const failed = ref(0);
  // 传输队列永久失败历史；与 sync_items 的当前失败 failed 分开保存
  const transferFailed = ref(0);
  // 当前失败项明细。
  const failedItems = ref<FailedItem[]>([]);
  // 冲突项数量。
  const conflict = ref(0);
  // 编辑中项目数量。
  const editing = ref(0);
  // 同步引擎是否运行。
  const isRunning = ref(false);
  // 云端索引是否重建中。
  const isIndexing = ref(false);
  // 已扫描目录数。
  const indexingScannedFolders = ref(0);
  // 已发现项目数。
  const indexingDiscoveredItems = ref(0);
  // 当前同步阶段（精确显示：indexing-startup / querying-changes / syncing-local 等）
  const syncPhase = ref<string | null>(null);
  // 最近同步完成时间。
  const lastSyncTime = ref<number | null>(null);
  // 本次快照是否包含目录变化。
  const contentChanged = ref(false);
  // 侧边栏刷新计数器（folder_content_changed 事件每触一次 +1，布尔值无法重复触发 watch）
  const sidebarRefresh = ref(0);
  // 是否已配置同步目录
  const mountConfigured = ref(false);
  // 同步目录路径
  const mountDir = ref("");
  // Linux FUSE 云盘是否启用；Linux 配置页不再提供关闭入口。
  const virtualDriveEnabled = ref(false);
  // Linux FUSE 用户可见挂载目录。
  const virtualMountDir = ref("");
  // FUSE 当前是否真实挂载；不能用配置开关代替。
  const virtualDriveMounted = ref(false);
  // 当前真实挂载目录（成功挂载后由后端返回）。
  const mountedVirtualDir = ref("");
  // 最近一次 FUSE 挂载失败原因。
  const virtualDriveError = ref<string | null>(null);
  // Linux 的产品模式固定为 FUSE；状态查询失败或返回 disabled 也不能退回传统同步。
  const usesVirtualDrive = computed(() => isLinux || virtualDriveEnabled.value);
  // 用户应在文件管理器中访问的目录。Linux 永不回退暴露 hidden backing：
  // FUSE 未真实挂载时返回空字符串，让所有“打开云盘”入口保持禁用。
  const userVisibleRoot = computed(() => {
    if (isLinux) {
      return virtualDriveMounted.value ? mountedVirtualDir.value : "";
    }
    if (virtualDriveEnabled.value) {
      return virtualDriveMounted.value ? mountedVirtualDir.value : "";
    }
    return mountDir.value;
  });
  // 同步阶段
  const setupPhase = ref<"loading" | "needsSetup" | "needsFirstSync" | "active">("loading");

  // 进度
  const progress = computed(() => {
    // 首次同步动作尚未写入 sync_items 时使用不确定态，不能把空表解释成 100%。
    if (total.value === 0) {
      return isRunning.value || isIndexing.value ? null : 1.0;
    }
    return completed.value / total.value;
  });

  // 是否有活跃传输
  const hasActiveTransfer = computed(
    () => uploading.value + downloading.value + waitingNetwork.value > 0,
  );

  /**
   * 应用完整权威快照；缺字段、默认对象和旧 revision 均不改变现有 UI。
   *
   * @param value - 来自 Tauri 事件的状态快照
   * @returns 是否成功应用
   */
  function applyState(value: unknown): boolean {
    // 只接受完整且版本有效的后端权威快照。
    if (!syncApi.isSyncGlobalState(value)) return false;
    // 已应用过的更新不能被乱序旧事件回滚。
    const s = value;
    if (s.revision < revision.value) return false;

    // 仅新 revision 可以触发一次性副作用。
    const isNewRevision = s.revision > revision.value;
    // 同步赋值保持 UI 看到同一 revision 下的一组字段。
    revision.value = s.revision;
    total.value = s.total;
    completed.value = s.completed;
    uploading.value = s.uploading;
    downloading.value = s.downloading;
    waitingNetwork.value = s.waiting_network;
    failed.value = s.failed;
    transferFailed.value = s.transfer_failed;
    failedItems.value = [...s.failed_items];
    conflict.value = s.conflict;
    editing.value = s.editing;
    isRunning.value = s.is_running;
    lastSyncTime.value = s.last_sync_time;
    isIndexing.value = s.is_indexing;
    indexingScannedFolders.value = s.indexing_scanned_folders;
    indexingDiscoveredItems.value = s.indexing_discovered_items;
    syncPhase.value = s.sync_phase ?? null;
    if (s.content_changed) {
      contentChanged.value = true;
      // 同一 revision 重复投递只允许幂等赋值，不能重复触发目录刷新。
      if (isNewRevision) sidebarRefresh.value++;
    } else {
      contentChanged.value = false;
    }
    return true;
  }

  /**
   * 在配置提交成功后立即应用挂载事实，避免瞬时配置重读失败回退为未配置。
   *
   * Linux 的用户选择是 FUSE 可见目录，不能覆盖后端管理的 backing 目录。
   *
   * @param path - 已持久化的用户可见目录
   */
  function applyMountConfiguration(path: string): void {
    mountConfigured.value = true;
    if (isLinux) {
      virtualDriveEnabled.value = true;
      virtualMountDir.value = path;
    } else {
      mountDir.value = path;
    }
    setupPhase.value = "active";
  }

  /**
   * 应用后端报告的真实 FUSE 挂载状态。
   */
  function applyVirtualDriveStatus(value: unknown): boolean {
    if (!value || typeof value !== "object") return false;
    const status = value as Record<string, unknown>;
    if (
      typeof status.enabled !== "boolean"
      || typeof status.mounted !== "boolean"
      || !(status.mount_dir === null || typeof status.mount_dir === "string")
      || !(status.error === null || typeof status.error === "string")
    ) {
      return false;
    }
    virtualDriveEnabled.value = status.enabled;
    virtualDriveMounted.value = status.mounted;
    mountedVirtualDir.value = status.mounted && typeof status.mount_dir === "string"
      ? status.mount_dir
      : "";
    virtualDriveError.value = typeof status.error === "string" ? status.error : null;
    return true;
  }

  /**
   * 主动刷新真实挂载状态，用于覆盖前端初始化早于异步 FUSE 启动的窗口。
   */
  async function refreshVirtualDriveStatus(): Promise<void> {
    try {
      applyVirtualDriveStatus(await commands.virtualDriveStatus());
    } catch {
      // 后端尚未启动或旧版本无此命令时保留当前状态，等待状态事件。
    }
  }

  /**
   * 初始化：加载配置判断阶段；配置就绪时主动拉一次当前同步状态，
   * 避免错过配置完成前已发出的 is_indexing 事件（BFS 可能先于 init 启动）。
   */
  async function init(): Promise<void> {
    try {
      // 配置决定同步视图能否进入 active 阶段。
      const config = await configApi.loadConfig();
      const visibleDirectoryConfigured = !isLinux
        || Boolean((config.virtual_mount_dir ?? "").trim());
      mountConfigured.value = config.mount_configured && visibleDirectoryConfigured;
      mountDir.value = config.mount_dir;
      virtualDriveEnabled.value = isLinux || (config.virtual_drive_enabled ?? false);
      virtualMountDir.value = config.virtual_mount_dir ?? "";
      virtualDriveMounted.value = false;
      mountedVirtualDir.value = "";
      virtualDriveError.value = null;
      await refreshVirtualDriveStatus();
      if (!mountConfigured.value) {
        setupPhase.value = "needsSetup";
      } else {
        setupPhase.value = "active";
        // 主动拉取当前状态：配置刚就绪，引擎 BFS 可能已在跑并广播了 is_indexing=true，
        // 但那时 mountConfigured 还是 false、状态条未渲染 → 该事件被"错过"。
        // 这里同步一次真实状态，确保 UI（状态条"正在读取云端索引…"、刷新按钮转圈）正确。
        try {
          // 当前后端权威状态。
          const state = await commands.syncState();
          applyState(state);
        } catch {
          // 引擎尚未启动（配置目录但引擎启动失败）→ 忽略，保留默认状态
        }
      }
    } catch {
      // 已由成功保存提交的挂载事实不得因瞬时配置读取失败被回退。
      if (!mountConfigured.value) setupPhase.value = "needsSetup";
    }
  }

  /**
   * 触发全量刷新
   */
  async function triggerManualRefresh(): Promise<void> {
    try {
      await commands.syncManualRefresh();
    } catch {
      // handled by event update
    }
  }

  /**
   * 重试失败项
   */
  async function retryFailed(): Promise<void> {
    try {
      await commands.syncRetryFailed();
    } catch {
      // handled by event update
    }
  }

  return {
    revision, total, completed, uploading, downloading, waitingNetwork,
    failed, transferFailed, failedItems, conflict, editing,
    isRunning, isIndexing, indexingScannedFolders, indexingDiscoveredItems,
    syncPhase, lastSyncTime, contentChanged,
    mountConfigured, setupPhase, mountDir,
    virtualDriveEnabled, virtualMountDir, virtualDriveMounted,
    mountedVirtualDir, virtualDriveError, usesVirtualDrive, userVisibleRoot,
    progress, hasActiveTransfer,
    init, applyState, applyMountConfiguration,
    applyVirtualDriveStatus, refreshVirtualDriveStatus,
    triggerManualRefresh, retryFailed,
    sidebarRefresh,
  };
});
