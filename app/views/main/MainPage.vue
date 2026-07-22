<!-- 主页面，左栏 220px 侧边栏 + 右侧文件区 -->
<script setup lang="ts">
import { onMounted, computed, ref } from "vue";
import { MateButton, MateIcon, MateSearchField, MateCircularProgress, MateEmpty, MateInfoBanner } from "@/components/mate";
import Sidebar from "./Sidebar.vue";
import Breadcrumb from "./Breadcrumb.vue";
import FileListView from "./FileListView.vue";
import SyncStatusBar from "./SyncStatusBar.vue";
import SyncSetupBanner from "./SyncSetupBanner.vue";
import TransferPopover from "./TransferPopover.vue";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import * as driveApi from "@/api/drive";
import * as platformApi from "@/api/platform";
import * as configApi from "@/api/config";
import { useAsyncAction } from "@/composables/useAsyncAction";

const browser = useFileBrowserStore();
const sync = useSyncStore();
// 搜索关键词
const searchKeyword = ref("");
// 搜索结果列表
const searchResults = ref<driveApi.DriveFile[]>([]);
const isSearching = ref(false);
// 同步目录是否已配置
const mountConfigured = computed(() => sync.mountConfigured);
// 传输队列 Popover
const showTransferPopover = ref(false);
// 异步按钮 loading + 防重复点击
const { loading: refreshLoading, run: runRefresh } = useAsyncAction();
const { loading: finderLoading, run: runFinder } = useAsyncAction();

// 定义事件
const emit = defineEmits<{ (e: "open-settings"): void }>();

/**
 * 挂载后加载根目录文件列表
 */
onMounted(async () => {
  await browser.loadRoot();
});

function handleOpenSettings(): void { emit("open-settings"); }

async function handleSearch(): Promise<void> {
  const kw = searchKeyword.value.trim();
  if (!kw || isSearching.value) return; // 防重复
  isSearching.value = true;
  try { searchResults.value = await driveApi.searchFiles(kw, browser.current.id || undefined); }
  catch { searchResults.value = []; }
  finally { isSearching.value = false; }
}
function handleClearSearch(): void { searchKeyword.value = ""; searchResults.value = []; }

async function handleOpenInFinder(): Promise<void> {
  await runFinder(async () => {
    try { const c = await configApi.loadConfig(); await platformApi.openInFinder(c.mount_dir); } catch {}
  });
}

async function handleRefreshAll(): Promise<void> {
  await runRefresh(async () => {
    await sync.triggerManualRefresh();
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
        <div class="app-bar__left">
          <MateSearchField v-model="searchKeyword" :max-width="420" @submit="handleSearch" />
          <MateButton v-if="searchKeyword" variant="icon" icon="x" tooltip="清除搜索" @click="handleClearSearch" />
        </div>
        <div class="app-bar__tools">
          <MateButton v-if="mountConfigured" variant="primary" icon="refresh" tooltip="拉取云端索引，创建本地目录与占位文件" :loading="refreshLoading || sync.isIndexing" :disabled="refreshLoading || sync.isIndexing" @click="handleRefreshAll">同步索引</MateButton>
          <MateButton variant="soft" icon="transfer" tooltip="传输队列" @click="showTransferPopover = !showTransferPopover">传输队列</MateButton>
          <MateButton v-if="mountConfigured" variant="icon-text" icon="folder-open" tooltip="在 Finder 中打开同步目录" :loading="finderLoading" :disabled="finderLoading" @click="handleOpenInFinder">Finder</MateButton>
        </div>
        <MateButton variant="icon" icon="settings" tooltip="设置" @click="handleOpenSettings" />
      </div>
      <!-- 信息/错误提示区（面包屑上方） -->
      <div class="info-area">
        <SyncSetupBanner v-if="!mountConfigured || sync.setupPhase === 'needsFirstSync'" />
        <SyncStatusBar v-if="mountConfigured" />
        <div v-if="browser.errorMessage" class="info-area__error">
          <MateInfoBanner variant="error">{{ browser.errorMessage }}</MateInfoBanner>
        </div>
      </div>
      <Breadcrumb />
      <div class="main-content__files">
        <template v-if="searchKeyword">
          <div class="search-results">
            <div class="search-header">{{ isSearching ? "搜索中…" : "搜索：" + searchKeyword }}</div>
            <MateEmpty v-if="searchResults.length === 0 && !isSearching" icon="search" title="无匹配结果" description="试试其他关键词" />
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
.search-row { display: flex; align-items: center; gap: var(--space-md); padding: 10px 20px; cursor: pointer; border-bottom: 1px solid var(--line); }
.search-row:hover { background: var(--bg-hover); }
.search-row__name { font-size: var(--font-body); color: var(--ink-900); }
.search-row__sub { font-size: var(--font-caption); color: var(--ink-400); font-variant-numeric: tabular-nums; }
.search-row :deep(.is-folder) { color: var(--brand-500); }

/* 传输队列 popover 定位：贴工具栏下方右侧 */
.tp-capture { position: fixed; inset: 0; z-index: 90; }
.transfer-popover-anchor { position: absolute; top: calc(var(--appbar-height) + 12px); right: 20px; z-index: 100; }
</style>
