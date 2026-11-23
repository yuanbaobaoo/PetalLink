<!-- 主页面，左栏 220px 侧边栏 + 右侧文件区 -->
<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch } from "vue";
import { MateButton, MateIcon, MateSearchField, MateCircularProgress, MateEmpty, MateInfoBanner, showToast } from "@/components/mate";
import Sidebar from "./Sidebar.vue";
import Breadcrumb from "./Breadcrumb.vue";
import FileListView from "./FileListView.vue";
import SyncStatusBar from "./SyncStatusBar.vue";
import SyncSetupBanner from "./SyncSetupBanner.vue";
import TransferPopover from "./TransferPopover.vue";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import { commands } from "@/api/generated";
import * as driveApi from "@/api/drive";
import { useAsyncAction } from "@/composables/useAsyncAction";
import { extractErrorMessage } from "@/utils/error";

// 当前文件浏览器状态。
const browser = useFileBrowserStore();
// 当前同步状态。
const sync = useSyncStore();
// 搜索关键词
const searchKeyword = ref("");
// 已提交（回车触发）的关键词：输入过程中不切换结果视图，避免未搜索就显示「无匹配结果」
const submittedKeyword = ref("");
// 搜索结果列表
const searchResults = ref<driveApi.DriveFile[]>([]);
// 是否正在执行云端搜索。
const isSearching = ref(false);
// 搜索失败原因：与「无匹配结果」严格区分，避免把故障伪装成空结果（D23）。
const searchError = ref<string | null>(null);
// 搜索范围：true=仅当前目录，false=整盘（U13）
const searchInCurrentFolder = ref(true);
// 搜索框容器：Cmd+F 聚焦用
const searchFieldRef = ref<HTMLElement | null>(null);
// 同步目录是否已配置
const mountConfigured = computed(() => sync.mountConfigured);
// 传输队列 Popover
const showTransferPopover = ref(false);
// 异步按钮 loading + 防重复点击
const { loading: refreshLoading, run: runRefresh } = useAsyncAction();
// 系统文件管理器操作的互斥执行状态。
const { loading: fileManagerLoading, run: runFileManager } = useAsyncAction();

// 定义事件
const emit = defineEmits<{ (e: "open-settings"): void }>();

/**
 * 挂载后加载根目录文件列表
 */
onMounted(async () => {
  await browser.loadRoot();
  window.addEventListener("keydown", handleGlobalKeydown);
});

/**
 * 卸载时移除全局快捷键监听。
 */
onUnmounted(() => {
  window.removeEventListener("keydown", handleGlobalKeydown);
});

/**
 * 关键词被清空时退出搜索视图（否则结果会一直停在旧关键词上）。
 */
watch(searchKeyword, (value) => {
  if (!value.trim()) handleClearSearch();
});

/**
 * 全局快捷键：Cmd/Ctrl+F 聚焦搜索框（U13）。
 *
 * @param event - 键盘事件
 */
function handleGlobalKeydown(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
    // 搜索框内的原生输入元素。
    const input = searchFieldRef.value?.querySelector("input");
    if (!input) return;
    event.preventDefault();
    input.focus();
    input.select();
  }
}

/**
 * 通知根组件打开设置页。
 */
function handleOpenSettings(): void {
  emit("open-settings");
}

/**
 * 按当前关键词查询并展示云端文件。
 */
async function handleSearch(): Promise<void> {
  // 去除首尾空白后的搜索关键词。
  const kw = searchKeyword.value.trim();
  if (!kw || isSearching.value) return; // 防重复
  isSearching.value = true;
  searchError.value = null;
  // 提交后才切换到结果视图；旧结果先清空，避免把上一次关键词的结果当成本次结果。
  submittedKeyword.value = kw;
  searchResults.value = [];
  try {
    // 当前目录搜索只在该目录内匹配；整盘搜索不限定父目录。
    const scope = searchInCurrentFolder.value ? browser.current.id || undefined : undefined;
    searchResults.value = await driveApi.searchFiles(kw, scope);
  } catch (e) {
    // 搜索失败要如实呈现，不能显示成「无匹配结果」让用户以为文件丢了。
    searchResults.value = [];
    searchError.value = extractErrorMessage(e);
  } finally {
    isSearching.value = false;
  }
}

/**
 * 清空关键词和当前搜索结果。
 */
function handleClearSearch(): void {
  searchKeyword.value = "";
  submittedKeyword.value = "";
  searchResults.value = [];
  searchError.value = null;
}

/**
 * 在系统文件管理器中打开用户可见的云盘/同步目录。
 */
async function handleOpenInFileManager(): Promise<void> {
  await runFileManager(async () => {
    try {
      await commands.openInFinder();
    } catch {
      // 会话可能在前端上次状态快照后断开；立即刷新真实 mountinfo 状态，
      // 让按钮禁用并显示后端记录的可操作挂载错误。
      await sync.refreshVirtualDriveStatus();
    }
  });
}

/**
 * 触发全量刷新并等待文件列表收敛。
 */
async function handleRefreshAll(): Promise<void> {
  await runRefresh(async () => {
    try {
      await sync.triggerManualRefresh();
    } catch (e) {
      // 触发都没成功就不必刷新列表，同时把真实原因告诉用户（D22）。
      showToast("刷新失败：" + extractErrorMessage(e), { variant: "error" });
      return;
    }
    await browser.refresh();
  });
}
</script>

<template>
  <div class="main-page">
    <Sidebar />
    <div class="main-content">
      <!-- 工具栏 64px -->
      <div class="app-bar">
        <div ref="searchFieldRef" class="app-bar__left">
          <MateSearchField v-model="searchKeyword" :max-width="420" @submit="handleSearch" />
          <MateButton v-if="searchKeyword" variant="icon" icon="x" tooltip="清除搜索" @click="handleClearSearch" />
          <MateButton
            variant="text"
            :tooltip="searchInCurrentFolder ? '只搜索当前目录，切换到整盘搜索' : '搜索整个云盘，切换回当前目录'"
            @click="searchInCurrentFolder = !searchInCurrentFolder"
          >
            {{ searchInCurrentFolder ? "当前目录" : "整个云盘" }}
          </MateButton>
        </div>
        <div class="app-bar__tools">
          <MateButton v-if="mountConfigured" variant="primary" icon="refresh" tooltip="读取云端文件并同步到本地" :loading="refreshLoading || sync.isIndexing" :disabled="refreshLoading || sync.isIndexing" @click="handleRefreshAll">同步索引</MateButton>
          <MateButton variant="soft" icon="transfer" tooltip="传输队列" @click="showTransferPopover = !showTransferPopover">传输队列</MateButton>
          <MateButton
            v-if="mountConfigured"
            variant="icon-text"
            icon="folder-open"
            :tooltip="sync.usesVirtualDrive ? '在系统文件管理器中打开云盘目录' : '在系统文件管理器中打开同步目录'"
            :loading="fileManagerLoading"
            :disabled="fileManagerLoading || !sync.userVisibleRoot"
            @click="handleOpenInFileManager"
          >{{ sync.usesVirtualDrive ? "云盘目录" : "同步目录" }}</MateButton>
        </div>
        <MateButton variant="icon" icon="settings" tooltip="设置" @click="handleOpenSettings" />
      </div>
      <!-- 信息/错误提示区（面包屑上方） -->
      <div class="info-area">
        <SyncSetupBanner
          v-if="!mountConfigured
            || sync.setupPhase === 'needsFirstSync'
            || (sync.usesVirtualDrive && !sync.virtualDriveMounted)"
        />
        <SyncStatusBar v-if="mountConfigured" />
        <div v-if="browser.errorMessage" class="info-area__error">
          <MateInfoBanner variant="error">{{ browser.errorMessage }}</MateInfoBanner>
        </div>
      </div>
      <Breadcrumb />
      <div class="main-content__files">
        <template v-if="submittedKeyword">
          <div class="search-results">
            <div class="search-header">
              {{ isSearching ? "搜索中…" : `搜索：${submittedKeyword}` }}
              <span class="search-scope">{{ searchInCurrentFolder ? "（当前目录内）" : "（整个云盘）" }}</span>
            </div>
            <!-- 搜索失败：错误横幅 + 重试，而不是「无匹配结果」 -->
            <MateInfoBanner v-if="searchError" variant="error">
              搜索失败：{{ searchError }}
              <template #action>
                <MateButton variant="text" @click="handleSearch">重试</MateButton>
              </template>
            </MateInfoBanner>
            <MateEmpty v-else-if="searchResults.length === 0 && !isSearching" icon="search" title="无匹配结果" description="试试其他关键词" />
            <div v-for="f in searchResults" :key="f.id" class="search-row" @click="driveApi.isFolder(f) ? (browser.enterFolder(f), handleClearSearch()) : undefined">
              <MateIcon :name="driveApi.fileTypeIcon(f)" :size="20" :class="{ 'is-folder': driveApi.isFolder(f) }" />
              <div>
                <div class="search-row__name">{{ f.name }}</div>
                <div class="search-row__sub">{{ driveApi.isFolder(f) ? "文件夹" : f.size + " 字节" }}</div>
              </div>
            </div>
          </div>
        </template>
        <FileListView v-else />
        <div v-if="browser.loading" class="loading-overlay"><MateCircularProgress :size="24" /></div>
      </div>

      <!-- 传输队列 Popover（点击外部关闭） -->
      <div v-if="showTransferPopover" class="tp-capture" @click="showTransferPopover = false" />
      <TransferPopover v-if="showTransferPopover" class="transfer-popover-anchor" @close="showTransferPopover = false" />
    </div>

  </div>
</template>

<style scoped>
.main-page { display: flex; width: 100%; height: 100%; }
.main-content { flex: 1; display: flex; flex-direction: column; min-width: 0; background: var(--bg-card); position: relative; }
.app-bar { height: var(--appbar-height); display: flex; align-items: center; padding: 0 20px; gap: var(--space-sm); flex-shrink: 0; }
.app-bar__left { display: flex; align-items: center; gap: var(--space-sm); flex: 1; min-width: 0; }
.app-bar__tools { display: flex; align-items: center; gap: var(--space-sm); }
.info-area { flex-shrink: 0; }
.info-area__error { padding: var(--space-xs) 20px; }
.main-content__files { flex: 1; min-height: 0; overflow: hidden; display: flex; flex-direction: column; position: relative; }
.loading-overlay { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(255, 255, 255, 0.6); }
.search-results { flex: 1; overflow-y: auto; }
.search-header { padding: var(--space-md) 20px; font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--ink-400); border-bottom: 1px solid var(--line); }
.search-scope { color: var(--ink-300); font-weight: normal; }
.search-row { display: flex; align-items: center; gap: var(--space-md); padding: 10px 20px; cursor: pointer; border-bottom: 1px solid var(--line); }
.search-row:hover { background: var(--bg-hover); }
.search-row__name { font-size: var(--font-body); color: var(--ink-900); }
.search-row__sub { font-size: var(--font-caption); color: var(--ink-400); font-variant-numeric: tabular-nums; }
.search-row :deep(.is-folder) { color: var(--brand-500); }

/* 传输队列 popover 定位：贴工具栏下方右侧 */
.tp-capture { position: fixed; inset: 0; z-index: 90; }
.transfer-popover-anchor { position: absolute; top: calc(var(--appbar-height) + 12px); right: 20px; z-index: 100; }
</style>
