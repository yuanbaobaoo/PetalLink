<!-- 同步状态条，全局进度 + 失败数点击查看详情 -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useSyncStore } from "@/stores/sync";
import { useTransferStore } from "@/stores/transfer";
import { isNameConflictTask } from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import { MateDialog, MateButton, confirmDialog, showToast } from "@/components/mate";
import { pad2 } from "@/utils/format";
import { commands } from "@/api/generated";
import type { InvalidPlaceholder } from "@/api/generated";
import { extractErrorMessage, formatUserMessage } from "@/utils/error";
import type { FailedItem } from "@/api/generated";

// 同步 store
const sync = useSyncStore();
// 传输 store
const transfer = useTransferStore();

// 后端同步阶段对应的精确用户文案。
const SYNC_PHASE_TEXT: Readonly<Record<string, string>> = {
  "indexing-startup": "正在首次读取云端文件…",
  "indexing-manual": "正在刷新云端文件…",
  "indexing-auto-full": "正在重新检查全部云端文件…",
  "querying-changes": "正在检查云端更新…",
  "syncing-auto-incremental": "正在同步云端变更…",
  "syncing-local": "正在同步本地变更…",
  "syncing-manual": "正在同步…",
  "syncing-retry": "正在重试失败项…",
  "syncing-startup": "正在继续上次未完成的同步…",
  "planning-startup": "正在分析首次同步内容…",
  "planning-sync": "正在分析同步内容…",
  "materializing-local": "正在创建本地目录和占位文件…",
  "executing-actions": "正在执行同步操作…",
};

// 状态文案：根据 sync_phase 精确显示当前操作场景
const statusText = computed(() => {
  // 已知后端阶段优先于聚合队列状态。
  const phaseText = sync.syncPhase ? SYNC_PHASE_TEXT[sync.syncPhase] : undefined;
  if (phaseText) return phaseText;

  // 无精确 phase 时仍按持久化传输队列区分活动态。
  if (transfer.verifyingRemote) return "正在确认同步结果…";
  if (sync.uploading || sync.downloading || transfer.running) return "同步中";
  if (sync.waitingNetwork || transfer.waitingNetwork) return "等待网络恢复…";
  if (transfer.backingOff) return "等待下次重试…";
  if (transfer.restartRequired) return "有文件需要重新检查…";
  if (transfer.pending) return "等待传输…";
  // 运行态字段是最终防线，漏 phase 时也不得误报完成。
  if (sync.isRunning || sync.isIndexing) return "正在同步…";
  if (sync.failed) return "同步存在失败项";
  return "同步完成";
});

// 上次同步时间（格式化 HH:MM）
const lastSyncFormatted = computed(() => {
  if (!sync.lastSyncTime) return "";
  // 待格式化的日期对象。
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
// 同名冲突弹窗是否可见
const showConflictDialog = ref(false);
// 正在提交冲突决策的任务 ID
const conflictActingId = ref<number | null>(null);
// 正在提交「确认删除」请求
const approvingMassDelete = ref(false);
// 失效占位文件弹窗是否可见
const showInvalidPlaceholders = ref(false);
// 正在清理的失效占位文件路径集合
const cleaningPlaceholders = ref<Set<string>>(new Set());
// 需要用户决策的同名冲突任务
const conflictTasks = computed(() => transfer.tasks.filter(isNameConflictTask));
// 正在提交重试的失败项路径集合（只影响该行按钮，毫秒级）
const retryingFailedPaths = ref<Set<string>>(new Set());
// 是否正在提交「重试全部」
const retryingAllFailed = ref(false);

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

/**
 * 打开失败项弹窗
 */
function handleShowFailed(): void {
  showFailedDialog.value = true;
}

/**
 * 重试单个失败项（只在该行按钮上显示 loading，不阻塞其它行）。
 *
 * @param item - 失败项明细
 */
async function handleRetryFailedItem(item: FailedItem): Promise<void> {
  if (item.task_id === null) {
    showToast("该失败项没有可重试的传输任务，请同步索引后重试", { variant: "warning" });
    return;
  }
  if (retryingFailedPaths.value.has(item.relative_path)) return;
  retryingFailedPaths.value.add(item.relative_path);
  try {
    // 后端只入队并立即返回，执行交给传输队列。
    await transfer.retry(item.task_id);
    showToast("已加入任务队列");
  } catch (e) {
    showToast("重试失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    retryingFailedPaths.value.delete(item.relative_path);
  }
}

/**
 * 打开同名冲突列表。
 */
function handleShowConflict(): void {
  void transfer.loadAll();
  showConflictDialog.value = true;
}

/**
 * 执行同名冲突的三选一决策。
 *
 * @param task - 冲突任务
 * @param decision - overwrite / keep-both / cancel
 */
async function handleConflictDecision(task: TransferTask, decision: "overwrite" | "keep-both" | "cancel"): Promise<void> {
  if (conflictActingId.value !== null) return;
  conflictActingId.value = task.id;
  try {
    if (decision === "overwrite") {
      await transfer.overwriteRemote(task.id);
      showToast("已保留云端旧版为副本，开始覆盖上传");
    } else if (decision === "keep-both") {
      await transfer.keepBoth(task.id);
      showToast("本地文件已改名，将作为新文件上传");
    } else {
      await transfer.cancel(task.id);
      showToast("任务已取消");
    }
  } catch (e) {
    showToast("操作失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    conflictActingId.value = null;
  }
}

/**
 * 清理一个失效占位文件。
 *
 * @param item - 失效占位明细
 */
async function handleCleanupPlaceholder(item: InvalidPlaceholder): Promise<void> {
  if (cleaningPlaceholders.value.has(item.relative_path)) return;
  cleaningPlaceholders.value.add(item.relative_path);
  try {
    const removed = await commands.syncCleanupPlaceholder(item.relative_path);
    showToast(removed ? "已清理本地残留占位文件" : "该文件无需清理");
  } catch (e) {
    showToast("清理失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    cleaningPlaceholders.value.delete(item.relative_path);
  }
}

/**
 * 确认执行被安全闸门拦下的大批量云端删除。
 *
 * 这是唯一能让这批删除继续的入口：只有用户明确知道「本地确实已删除」时才应该点。
 */
async function handleApproveMassDelete(): Promise<void> {
  if (approvingMassDelete.value) return;
  // 二次确认：误点的代价是不可恢复的云端数据丢失。
  const ok = await confirmDialog({
    title: "确认删除云端文件",
    titleIcon: "alert",
    danger: true,
    confirmText: "确认删除",
    content: `本地已看不到 ${sync.blockedCloudDeletes} 项文件，引擎因此阻止了这批云端删除。\n\n如果你确实要删除这些云端文件，请先确认同步目录已正确挂载、文件确实已被删除。点击「确认删除」后本次删除会立即执行且无法撤销。`,
  });
  if (!ok) return;
  approvingMassDelete.value = true;
  try {
    await commands.syncApproveMassDelete();
    showToast("已确认，正在执行这批云端删除");
  } catch (e) {
    showToast("确认失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    approvingMassDelete.value = false;
  }
}

/**
 * 一键重试全部失败项。
 */
async function handleRetryAllFailed(): Promise<void> {
  if (retryingAllFailed.value) return;
  retryingAllFailed.value = true;
  try {
    // 后端遍历失败项入队后立即返回汇总。
    const summary = await transfer.retryAll();
    const parts = [`已加入队列 ${summary.queued} 项`];
    if (summary.already_queued > 0) parts.push(`${summary.already_queued} 项已在队列中`);
    if (summary.skipped > 0) parts.push(`跳过 ${summary.skipped} 项`);
    if (summary.rejected > 0) parts.push(`${summary.rejected} 项校验未通过`);
    showToast(parts.join("，"), { variant: summary.rejected > 0 ? "warning" : "success" });
  } catch (e) {
    showToast("批量重试失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    retryingAllFailed.value = false;
  }
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
      <span v-if="sync.conflict" class="chip chip--warn chip--clickable" @click="handleShowConflict">冲突 {{ sync.conflict }}</span>
      <span v-if="sync.failed" class="chip chip--err chip--clickable" @click="handleShowFailed">同步失败 {{ sync.failed }}</span>
      <span
        v-if="sync.deferredActions"
        class="chip chip--warn"
        title="这些同步动作因同路径仍有传输任务而暂缓；任务结束或取消后会自动重试"
      >
        同步暂缓 {{ sync.deferredActions }}
      </span>
      <span v-if="sync.blockedCloudDeletes" class="chip chip--err chip--clickable" @click="handleApproveMassDelete">
        已阻止 {{ sync.blockedCloudDeletes }} 项云端删除
      </span>
      <span v-if="sync.invalidPlaceholders.length" class="chip chip--warn chip--clickable" @click="showInvalidPlaceholders = true">
        {{ sync.invalidPlaceholders.length }} 个占位文件已失效
      </span>
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
          <div class="failed-item__row">
            <div class="failed-item__path">{{ item.relative_path }}</div>
            <MateButton
              v-if="item.task_id !== null"
              variant="text"
              :loading="retryingFailedPaths.has(item.relative_path)"
              :disabled="retryingFailedPaths.has(item.relative_path)"
              @click="handleRetryFailedItem(item)"
            >
              重试
            </MateButton>
          </div>
          <div v-if="item.error_message" class="failed-item__err">
            {{ formatUserMessage(item.error_message) }}
          </div>
        </div>
      </div>
      <template #footer>
        <MateButton variant="soft" :loading="retryingAllFailed" :disabled="retryingAllFailed" @click="handleRetryAllFailed">
          重试全部
        </MateButton>
        <MateButton variant="primary" @click="showFailedDialog = false">关闭</MateButton>
      </template>
    </MateDialog>

    <!-- 失效占位文件：云端副本已删除，本地那次恢复是无效的 -->
    <MateDialog
      :open="showInvalidPlaceholders"
      title-icon="alert"
      :title="`占位文件已失效 (${sync.invalidPlaceholders.length})`"
      @update:open="(v) => (showInvalidPlaceholders = v)"
    >
      <div class="failed-list">
        <div class="invalid-hint">
          这些文件你本地只剩一个 0 字节占位符，云端副本已经被删除（可能是在本地删除后同步过去的）。
          本地这次恢复是无效的，打开会失败。可以从云端「最近删除」里恢复原文件，或清理本地残留。
        </div>
        <div v-for="item in sync.invalidPlaceholders" :key="item.relative_path" class="failed-item">
          <div class="failed-item__row">
            <div class="failed-item__path">{{ item.relative_path }}</div>
            <MateButton
              variant="text"
              :loading="cleaningPlaceholders.has(item.relative_path)"
              :disabled="cleaningPlaceholders.has(item.relative_path)"
              @click="handleCleanupPlaceholder(item)"
            >
              清理本地残留
            </MateButton>
          </div>
        </div>
      </div>
      <template #footer>
        <MateButton variant="primary" @click="showInvalidPlaceholders = false">关闭</MateButton>
      </template>
    </MateDialog>

    <!-- 同名冲突：直接给出三选一决策，不必再去任务队列里翻 -->
    <MateDialog
      :open="showConflictDialog"
      title-icon="alert"
      :title="`同名冲突 (${conflictTasks.length})`"
      @update:open="(v) => (showConflictDialog = v)"
    >
      <div v-if="conflictTasks.length === 0" class="failed-empty">暂无需处理的同名冲突</div>
      <div v-else class="failed-list">
        <div v-for="task in conflictTasks" :key="task.id" class="failed-item">
          <div class="failed-item__path">{{ task.relative_path ?? task.name }}</div>
          <div class="failed-item__err">
            云端已存在同名文件且内容不同，请选择处理方式
          </div>
          <div class="conflict-actions">
            <MateButton
              variant="soft"
              :loading="conflictActingId === task.id"
              :disabled="conflictActingId !== null"
              @click="handleConflictDecision(task, 'overwrite')"
            >
              覆盖远端
            </MateButton>
            <MateButton
              variant="soft"
              :loading="conflictActingId === task.id"
              :disabled="conflictActingId !== null"
              @click="handleConflictDecision(task, 'keep-both')"
            >
              保留两者
            </MateButton>
            <MateButton
              variant="text"
              :disabled="conflictActingId !== null"
              @click="handleConflictDecision(task, 'cancel')"
            >
              取消任务
            </MateButton>
          </div>
        </div>
      </div>
      <template #footer>
        <MateButton variant="primary" @click="showConflictDialog = false">关闭</MateButton>
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
.failed-item__row {
  display: flex; align-items: center; justify-content: space-between; gap: var(--space-sm);
}
.invalid-hint { font-size: var(--font-caption); color: var(--ink-600); line-height: 1.6; padding: 8px 2px; }
.conflict-actions { display: flex; gap: var(--space-sm); margin-top: 6px; }
.failed-item__path { font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--ink-900); word-break: break-all; }
.failed-item__err { font-size: var(--font-caption); color: var(--err); margin-top: 2px; }
</style>
