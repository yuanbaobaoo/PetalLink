<!-- 传输队列弹窗，420×560，单列表含上传/下载/删除 -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useTransferStore } from "@/stores/transfer";
import { TRANSFER_DIR, TRANSFER_STATE, DIR_LABEL } from "@/api/transfer";
import { MateIcon, MateButton, MateLinearProgress, MateEmpty, MatePopupMenu, showToast } from "@/components/mate";
import type { PopupItem } from "@/components/mate";
import { formatFileSize } from "@/utils/format";

const transfer = useTransferStore();

const allItems = computed(() => transfer.tasks);

interface StateMeta { icon: string; label: string; color: string; spin?: boolean; }
const stateMeta: Record<number, StateMeta> = {
  [TRANSFER_STATE.PENDING]: { icon: "clock", label: "等待中", color: "var(--text-secondary)" },
  [TRANSFER_STATE.RUNNING]: { icon: "sync", label: "进行中", color: "var(--color-brand)", spin: true },
  [TRANSFER_STATE.PAUSED]: { icon: "pause", label: "已暂停", color: "var(--text-secondary)" },
  [TRANSFER_STATE.COMPLETED]: { icon: "check", label: "已完成", color: "var(--color-success)" },
  [TRANSFER_STATE.FAILED]: { icon: "x", label: "失败", color: "var(--color-error)" },
  [TRANSFER_STATE.CANCELED]: { icon: "x", label: "已取消", color: "var(--text-secondary)" },
};

const clearItems: PopupItem[] = [
  { value: "completed", label: "清除已完成", icon: "check" },
  { value: "failed", label: "清除失败项", icon: "x", danger: true },
  { value: "finished", label: "清除已完成+失败", icon: "transfer" },
];

const emit = defineEmits<{ (e: "close"): void }>();

// 正在重试的任务 id（防抖：重试中禁用该按钮，避免连点）
const retryingId = ref<number | null>(null);

onMounted(() => { transfer.loadAll(); });

function dirIcon(direction: number): string {
  if (direction === TRANSFER_DIR.DOWNLOAD) return "download";
  if (direction === TRANSFER_DIR.DOWNLOAD_UPDATE) return "refresh";
  if (direction === TRANSFER_DIR.DELETE) return "trash";
  return "transfer";
}

function progressValue(t: { total_size: number; transferred: number }): number | null {
  if (t.total_size <= 0) return null;
  return Math.min(1, t.transferred / t.total_size);
}
function progressColor(state: number): string {
  if (state === TRANSFER_STATE.COMPLETED) return "var(--color-success)";
  if (state === TRANSFER_STATE.FAILED) return "var(--color-error)";
  if (state === TRANSFER_STATE.PAUSED || state === TRANSFER_STATE.PENDING) return "var(--border-hover)";
  return "var(--color-brand)";
}

function fmtSize(bytes: number): string {
  return formatFileSize(bytes);
}
function pct(t: { total_size: number; transferred: number }): number {
  return t.total_size > 0 ? Math.round((t.transferred / t.total_size) * 100) : 0;
}

async function onClear(value: string | number): Promise<void> {
  if (value === "completed") await transfer.clearCompleted();
  else if (value === "failed") await transfer.clearFailed();
  else if (value === "finished") await transfer.clearFinished();
}

/** 重试失败任务 */
async function onRetry(id: number): Promise<void> {
  if (retryingId.value !== null) return; // 防抖：已有重试进行中
  retryingId.value = id;
  try {
    await transfer.retry(id);
    showToast("已重新加入传输队列", { variant: "success" });
  } catch (e) {
    // AppError 经 Tauri 序列化后是对象，取 message 字段；兜底 String
    const msg = (e as { message?: string })?.message ?? String(e);
    showToast("重试失败：" + msg, { variant: "error" });
  } finally {
    retryingId.value = null;
  }
}
</script>

<template>
  <div class="transfer-popover popover-in">
    <!-- Header -->
    <div class="tp-header">
      <MateIcon name="transfer" :size="18" class="tp-header__icon" />
      <span class="tp-header__title">传输队列</span>
      <MateButton variant="icon" icon="x" tooltip="关闭" @click="emit('close')" />
    </div>

    <!-- 统计栏 -->
    <div class="tp-stats">
      <span>进行中 {{ transfer.running }}</span>
      <span class="tp-stats__sep" />
      <span>等待中 {{ transfer.pending }}</span>
      <span class="tp-stats__sep" />
      <span>已完成 {{ transfer.completed }}</span>
      <MatePopupMenu class="tp-stats__clear" :items="clearItems" @select="onClear">
        <MateButton variant="icon" icon="transfer" tooltip="清除" />
      </MatePopupMenu>
    </div>

    <!-- 单列表 -->
    <div class="tp-body">
      <MateEmpty v-if="allItems.length === 0" icon="cloud" title="暂无传输任务" />
      <div v-for="item in allItems" :key="item.id" class="tp-item">
        <MateIcon :name="dirIcon(item.direction)" :size="16" class="tp-item__dir" />
        <div class="tp-item__info">
          <div class="tp-item__name">
            <span class="tp-item__tag">{{ DIR_LABEL[item.direction] ?? "—" }}</span>
            {{ item.name }}
          </div>
          <!-- 失败时显示错误原因，否则显示进度条 -->
          <div v-if="item.state === TRANSFER_STATE.FAILED && item.error_message" class="tp-item__error">
            {{ item.error_message }}
          </div>
          <div class="tp-item__progress" v-else-if="item.direction !== TRANSFER_DIR.DELETE">
            <MateLinearProgress
              class="tp-item__bar"
              :value="progressValue(item)"
              :height="4"
              :color="progressColor(item.state)"
            />
            <span class="tp-item__pct">{{ pct(item) }}% · {{ fmtSize(item.transferred) }}/{{ fmtSize(item.total_size) }}</span>
          </div>
          <div v-else class="tp-item__progress tp-item__progress--delete">删除操作</div>
        </div>
        <div class="tp-item__state" :style="{ color: stateMeta[item.state]?.color }">
          <MateIcon :name="stateMeta[item.state]?.icon ?? 'clock'" :size="12" :spin="stateMeta[item.state]?.spin" />
          {{ stateMeta[item.state]?.label ?? "" }}
        </div>
        <!-- 失败项重试按钮（仅上传/下载失败时显示） -->
        <MateButton
          v-if="item.state === TRANSFER_STATE.FAILED && item.direction !== TRANSFER_DIR.DELETE"
          variant="icon"
          icon="refresh"
          tooltip="重试"
          :loading="retryingId === item.id"
          :disabled="retryingId !== null"
          class="tp-item__retry"
          @click="onRetry(item.id)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.transfer-popover {
  width: var(--transfer-popover-width);
  height: var(--transfer-popover-height);
  background-color: var(--bg-container);
  border-radius: var(--radius-md);
  border: 0.5px solid var(--border);
  box-shadow: var(--shadow-modal);
  display: flex; flex-direction: column; overflow: hidden;
}

.tp-header {
  height: 48px; display: flex; align-items: center; padding: 0 var(--space-sm) 0 var(--space-lg); gap: var(--space-sm);
  border-bottom: 0.5px solid var(--border); flex-shrink: 0;
}
.tp-header__icon { color: var(--color-brand); }
.tp-header__title { font-size: var(--font-title-sm); font-weight: var(--fw-semibold); flex: 1; color: var(--text-primary); }

.tp-stats { height: 36px; display: flex; align-items: center; padding: 0 var(--space-lg); gap: var(--space-sm); font-size: var(--font-caption); color: var(--text-secondary); border-bottom: 0.5px solid var(--border); flex-shrink: 0; }
.tp-stats__sep { width: 1px; height: 14px; background-color: var(--border); }
.tp-stats__clear { margin-left: auto; }

.tp-body { flex: 1; overflow-y: auto; }

.tp-item { height: 60px; display: flex; align-items: center; padding: 0 var(--space-lg); gap: var(--space-sm); border-bottom: 0.5px solid var(--border); }
.tp-item__dir { color: var(--text-secondary); flex-shrink: 0; }
.tp-item__info { flex: 1; min-width: 0; }
.tp-item__name { font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tp-item__tag { font-size: var(--font-caption); color: var(--text-secondary); margin-right: 4px; padding: 0 4px; background: var(--bg-hover); border-radius: 3px; }
.tp-item__progress { display: flex; align-items: center; gap: var(--space-sm); margin-top: 4px; }
.tp-item__progress--delete { font-size: var(--font-caption); color: var(--text-secondary); }
.tp-item__error { font-size: var(--font-caption); color: var(--color-error); margin-top: 4px; line-height: 1.4; word-break: break-all; }
.tp-item__bar { flex: 1; }
.tp-item__pct { font-size: var(--font-caption); color: var(--text-secondary); white-space: nowrap; }
.tp-item__state { font-size: var(--font-caption); font-weight: var(--fw-medium); white-space: nowrap; width: 80px; text-align: right; display: inline-flex; align-items: center; justify-content: flex-end; gap: 3px; }
.tp-item__retry { flex-shrink: 0; margin-left: var(--space-xs); }
</style>
