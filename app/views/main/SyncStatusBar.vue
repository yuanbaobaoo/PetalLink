<!-- 同步状态条，全局进度 + 失败数点击查看详情 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useSyncStore } from "@/stores/sync";
import { useTransferStore } from "@/stores/transfer";
import { MateDialog, MateButton } from "@/components/mate";
import { pad2 } from "@/utils/format";
import { formatUserMessage } from "@/utils/error";

// 同步 store
const sync = useSyncStore();
// 传输 store
const transfer = useTransferStore();

// 状态文案：根据 sync_phase 精确显示当前操作场景
const statusText = computed(() => {
  switch (sync.syncPhase) {
    case "indexing-startup": return "正在首次读取云端文件…";
    case "indexing-manual": return "正在刷新云端文件…";
    case "indexing-auto-full": return "正在重新检查全部云端文件…";
    case "querying-changes": return "正在检查云端更新…";
    case "syncing-auto-incremental": return "正在同步云端变更…";
    case "syncing-local": return "正在同步本地变更…";
    case "syncing-manual": return "正在同步…";
    case "syncing-retry": return "正在重试失败项…";
    case "syncing-startup": return "正在继续上次未完成的同步…";
    default:
      // 无 sync cycle 时仍按持久化传输队列区分活动态，不能把等待/退避/核验显示为完成。
      if (transfer.verifyingRemote) return "正在确认同步结果…";
      if (sync.uploading || sync.downloading || transfer.running) return "同步中";
      if (sync.waitingNetwork || transfer.waitingNetwork) return "等待网络恢复…";
      if (transfer.backingOff) return "等待下次重试…";
      if (transfer.restartRequired) return "有文件需要重新检查…";
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

/**
 * 自动自愈完成后关闭失败详情，避免保留“失败项 (0)”的陈旧弹窗。
 */
watch(() => sync.failed, (failed) => {
  if (failed === 0) showFailedDialog.value = false;
});

/** 打开失败项弹窗 */
function handleShowFailed(): void {
  showFailedDialog.value = true;
}
</script>

<template>
  <div class="sync-bar" v-if="sync.mountConfigured">
    <div class="sync-bar__left">
      <!-- 状态点：同步中品牌蓝脉冲 / 失败红 / 完成绿 -->
      <span
        class="dot"
        :class="{
          'dot--brand pulse': !isIdle,
          'dot--err': isIdle && !!sync.failed,
          'dot--ok': isIdle && !sync.failed,
        }"
      />
      <span class="sync-bar__text">{{ statusText }}</span>
      <span v-if="lastSyncFormatted && isIdle" class="sync-bar__time">上次同步 {{ lastSyncFormatted }}</span>
    </div>

    <div class="sync-bar__tags">
      <span v-if="sync.uploading" class="chip chip--brand">上传 {{ sync.uploading }}</span>
      <span v-if="sync.downloading" class="chip chip--brand">下载 {{ sync.downloading }}</span>
      <span v-if="sync.waitingNetwork" class="chip chip--warn">等待网络 {{ sync.waitingNetwork }}</span>
      <span v-if="sync.editing" class="chip chip--warn">编辑中 {{ sync.editing }}</span>
      <span v-if="sync.conflict" class="chip chip--warn">冲突 {{ sync.conflict }}</span>
      <span v-if="sync.failed" class="chip chip--err chip--clickable" @click="handleShowFailed">同步失败 {{ sync.failed }}</span>
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
          <div v-if="item.error_message" class="failed-item__err">
            {{ formatUserMessage(item.error_message) }}
          </div>
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
  min-height: var(--sync-bar-height);
  display: flex;
  align-items: center;
  padding: 6px 20px;
  gap: 10px;
  border-top: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
  background-color: var(--bg-card);
  font-size: var(--font-body-sm);
  flex-shrink: 0;
}
.sync-bar__left { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
.sync-bar__text { color: var(--ink-900); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sync-bar__time { color: var(--ink-400); font-size: 12.5px; flex-shrink: 0; }
.sync-bar__tags { display: flex; align-items: center; gap: 6px; justify-content: flex-end; flex-wrap: wrap; }

/* 状态点（v2 dot，同步中脉冲） */
.dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
.dot--ok { background: var(--ok); }
.dot--err { background: var(--err); }
.dot--brand { background: var(--brand-500); }

/* 状态 chip（v2） */
.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 24px;
  padding: 0 10px;
  border-radius: var(--radius-sm);
  font-size: var(--font-caption);
  font-weight: var(--fw-medium);
  white-space: nowrap;
}
.chip--brand { background: var(--brand-50); color: var(--brand-500); }
.chip--warn { background: var(--warn-bg); color: var(--warn); }
.chip--err { background: var(--err-bg); color: var(--err); }
.chip--clickable { cursor: pointer; }
.chip--clickable:hover { filter: brightness(0.96); }

.failed-empty { color: var(--ink-400); }
.failed-list { display: flex; flex-direction: column; gap: var(--space-sm); max-height: 320px; overflow-y: auto; }
.failed-item__path { font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--ink-900); word-break: break-all; }
.failed-item__err { font-size: var(--font-caption); color: var(--err); margin-top: 2px; }
</style>
