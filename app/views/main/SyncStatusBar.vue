<!-- 同步状态条，全局进度 + 失败数点击查看详情 -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useSyncStore } from "@/stores/sync";
import { MateIcon, MateDialog, MateButton } from "@/components/mate";
import { pad2 } from "@/utils/format";

const sync = useSyncStore();

// 状态文案：根据 sync_phase 精确显示当前操作场景
const statusText = computed(() => {
  switch (sync.syncPhase) {
    case "indexing-startup": return "正在读取云端索引（首次）…";
    case "indexing-manual": return "正在读取云端索引…";
    case "indexing-auto-full": return "正在读取云端索引（全量纠偏）…";
    case "querying-changes": return "正在查询云端变更…";
    case "syncing-auto-incremental": return "正在同步云端变更…";
    case "syncing-local": return "正在同步本地变更…";
    case "syncing-manual": return "正在同步…";
    case "syncing-retry": return "正在重试失败项…";
    case "syncing-startup": return "正在同步（启动恢复）…";
    default:
      // 有传输进行中但无 sync cycle（如手动下载）
      if (sync.hasActiveTransfer) return "同步中";
      return "同步完成";
  }
});

const lastSyncFormatted = computed(() => {
  if (!sync.lastSyncTime) return "";
  const d = new Date(sync.lastSyncTime);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
});

const isIdle = computed(() => !sync.hasActiveTransfer && !sync.isIndexing && !sync.isRunning);

const showFailedDialog = ref(false);
function handleShowFailed(): void { showFailedDialog.value = true; }
</script>

<template>
  <div class="sync-bar" v-if="sync.mountConfigured">
    <div class="sync-bar__left">
      <MateIcon
        :name="isIdle ? 'check' : 'sync'"
        :size="16"
        :spin="!isIdle"
        :class="{ 'is-success': isIdle, 'is-active': !isIdle }"
      />
      <span class="sync-bar__text">{{ statusText }}</span>
      <span v-if="lastSyncFormatted && isIdle" class="sync-bar__time">· 上次同步 {{ lastSyncFormatted }}</span>
    </div>

    <div class="sync-bar__tags">
      <span v-if="sync.uploading" class="tag tag--primary">上传 {{ sync.uploading }}</span>
      <span v-if="sync.downloading" class="tag tag--primary">下载 {{ sync.downloading }}</span>
      <span v-if="sync.editing" class="tag tag--warning">编辑中 {{ sync.editing }}</span>
      <span v-if="sync.conflict" class="tag tag--warning">冲突 {{ sync.conflict }}</span>
      <span v-if="sync.failed" class="tag tag--error" @click="handleShowFailed">失败 {{ sync.failed }}</span>
    </div>

    <!-- 失败项弹窗 -->
    <MateDialog
      :open="showFailedDialog"
      title-icon="alert"
      danger
      :title="`同步失败项 (${sync.failedItems.length})`"
      @update:open="(v) => (showFailedDialog = v)"
    >
      <div v-if="sync.failedItems.length === 0" class="failed-empty">暂无失败项详情</div>
      <div v-else class="failed-list">
        <div v-for="(item, i) in sync.failedItems" :key="i" class="failed-item">
          <div class="failed-item__path">{{ item.relative_path }}</div>
          <div v-if="item.error_message" class="failed-item__err">{{ item.error_message }}</div>
        </div>
      </div>
      <template #footer>
        <MateButton variant="primary" @click="showFailedDialog = false">关闭</MateButton>
      </template>
    </MateDialog>
  </div>
</template>

<style scoped>
.sync-bar {
  height: var(--sync-bar-height); display: flex; align-items: center; padding: 0 var(--space-lg);
  gap: var(--space-sm); border-bottom: 0.5px solid var(--border); background-color: var(--bg-container);
  font-size: var(--font-body-sm); flex-shrink: 0;
}
.sync-bar__left { display: flex; align-items: center; gap: var(--space-sm); flex: 3; min-width: 0; }
.sync-bar__left :deep(.is-success) { color: var(--color-success); }
.sync-bar__left :deep(.is-active) { color: var(--color-brand); }
.sync-bar__text { color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sync-bar__time { color: var(--text-secondary); }
.sync-bar__tags { display: flex; gap: var(--space-xs); flex: 4; justify-content: flex-end; flex-wrap: wrap; }
.tag { display: inline-flex; align-items: center; padding: 1px 6px; border-radius: var(--radius-sm); font-size: var(--font-caption); font-weight: var(--fw-medium); line-height: 18px; }
.tag--primary { background-color: var(--color-brand-lighter); color: var(--color-brand); }
.tag--warning { background-color: var(--color-warning-bg); color: var(--color-warning); }
.tag--error { background-color: var(--color-error-bg); color: var(--color-error); cursor: pointer; }

.failed-empty { color: var(--text-secondary); }
.failed-list { display: flex; flex-direction: column; gap: var(--space-sm); max-height: 320px; overflow-y: auto; }
.failed-item__path { font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--text-primary); }
.failed-item__err { font-size: var(--font-caption); color: var(--color-error); margin-top: 2px; }
</style>
