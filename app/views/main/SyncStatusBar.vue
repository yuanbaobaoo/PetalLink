<!-- 同步状态条，全局进度 + 失败数点击查看详情 -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useSyncStore } from "@/stores/sync";
import { useTransferStore } from "@/stores/transfer";
import { MateIcon, MateDialog, MateButton } from "@/components/mate";
import { pad2 } from "@/utils/format";

// 同步 store
const sync = useSyncStore();
// 传输 store
const transfer = useTransferStore();

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
      // 无 sync cycle 时仍按持久化传输队列区分活动态，不能把等待/退避/核验显示为完成。
      if (transfer.verifyingRemote) return "正在核验远端状态…";
      if (sync.uploading || sync.downloading || transfer.running) return "同步中";
      if (sync.waitingNetwork || transfer.waitingNetwork) return "等待网络恢复…";
      if (transfer.backingOff) return "等待下次重试…";
      if (transfer.restartRequired) return "等待重新规划…";
      if (transfer.pending) return "等待传输…";
      if (sync.failed) return "同步存在失败项";
      return "同步完成";
  }
});

// 上次同步时间（格式化 HH:MM）
const lastSyncFormatted = computed(() => {
  if (!sync.lastSyncTime) return "";
  const d = new Date(sync.lastSyncTime);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
});

// 是否处于空闲态（无任何活动同步/传输）
const isIdle = computed(
  () => !sync.hasActiveTransfer
    && !transfer.hasActiveTasks
    && !sync.isIndexing
    && !sync.isRunning,
);
// 状态图标（同步中/失败/完成）
const statusIcon = computed(() => {
  if (!isIdle.value) return "sync";
  return sync.failed ? "alert" : "check";
});

// 失败项弹窗是否可见
const showFailedDialog = ref(false);

/**
 * 首次进入主页也读取持久化队列，避免在下一次事件到来前把
 * BackingOff/VerifyingRemote 误报为空闲。
 */
onMounted(async () => {
  try {
    await transfer.loadAll();
  } catch {
    // IPC/引擎瞬时失败：保留默认状态，等待下一次事件纠正
  }
});

/** 打开失败项弹窗 */
function handleShowFailed(): void {
  showFailedDialog.value = true;
}
</script>

<template>
  <div class="sync-bar" v-if="sync.mountConfigured">
    <div class="sync-bar__left">
      <MateIcon
        :name="statusIcon"
        :size="16"
        :spin="!isIdle"
        :class="{
          'is-success': isIdle && !sync.failed,
          'is-error': isIdle && !!sync.failed,
          'is-active': !isIdle,
        }"
      />
      <span class="sync-bar__text">{{ statusText }}</span>
      <span v-if="lastSyncFormatted && isIdle" class="sync-bar__time">· 上次同步 {{ lastSyncFormatted }}</span>
    </div>

    <div class="sync-bar__tags">
      <span v-if="sync.uploading" class="tag tag--primary">上传 {{ sync.uploading }}</span>
      <span v-if="sync.downloading" class="tag tag--primary">下载 {{ sync.downloading }}</span>
      <span v-if="sync.waitingNetwork" class="tag tag--warning">等待网络 {{ sync.waitingNetwork }}</span>
      <span v-if="sync.editing" class="tag tag--warning">编辑中 {{ sync.editing }}</span>
      <span v-if="sync.conflict" class="tag tag--warning">冲突 {{ sync.conflict }}</span>
      <span v-if="sync.failed" class="tag tag--error" @click="handleShowFailed">同步失败 {{ sync.failed }}</span>
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
.sync-bar__left :deep(.is-error) { color: var(--color-error); }
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
