<!-- 传输队列弹窗（v2：440×580，stat-pill 统计条 + 方向 tile 任务行），单列表含上传/下载/删除 -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useTransferStore } from "@/stores/transfer";
import {
  TRANSFER_DIR,
  TRANSFER_STATE,
  DIR_LABEL,
  canRetryTransferTask,
} from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import {
  MateIcon,
  MateButton,
  MateLinearProgress,
  MateEmpty,
  MatePopupMenu,
  showToast,
} from "@/components/mate";
import type { PopupItem } from "@/components/mate";
import { formatFileSize } from "@/utils/format";
import { extractErrorMessage, formatUserMessage } from "@/utils/error";

// 传输 store
const transfer = useTransferStore();

// 当前传输队列（全部任务）
const allItems = computed(() => transfer.tasks);

interface StateMeta { icon: string; label: string; color: string; spin?: boolean; }
// 各传输状态的展示元信息（图标/文案/颜色）
const stateMeta: Record<number, StateMeta> = {
  [TRANSFER_STATE.PENDING]: { icon: "clock", label: "等待开始", color: "var(--ink-400)" },
  [TRANSFER_STATE.RUNNING]: { icon: "sync", label: "传输中", color: "var(--brand-500)", spin: true },
  [TRANSFER_STATE.WAITING_FOR_NETWORK]: { icon: "clock", label: "等待网络", color: "var(--warn)" },
  [TRANSFER_STATE.BACKING_OFF]: { icon: "clock", label: "等待重试", color: "var(--warn)" },
  [TRANSFER_STATE.VERIFYING_REMOTE]: { icon: "sync", label: "正在确认结果", color: "var(--brand-500)", spin: true },
  [TRANSFER_STATE.RESTART_REQUIRED]: { icon: "refresh", label: "需要重新检查", color: "var(--warn)" },
  [TRANSFER_STATE.COMPLETED]: { icon: "check", label: "已完成", color: "var(--ok)" },
  [TRANSFER_STATE.FAILED]: { icon: "x", label: "失败", color: "var(--err)" },
  [TRANSFER_STATE.CANCELED]: { icon: "x", label: "已取消", color: "var(--ink-400)" },
};

// 清除菜单选项（已完成 / 失败历史 / 完成+失败历史）
const clearItems: PopupItem[] = [
  { value: "completed", label: "清除已完成", icon: "check" },
  { value: "failed", label: "清除失败历史", icon: "x", danger: true },
  { value: "finished", label: "清除完成+失败历史", icon: "transfer" },
];

// 关闭弹窗事件
const emit = defineEmits<{ (e: "close"): void }>();

// 正在重试的任务 id（防抖：重试中禁用该按钮，避免连点）
const retryingId = ref<number | null>(null);

onMounted(() => {
  transfer.loadAll();
});

function dirIcon(direction: number): string {
  if (direction === TRANSFER_DIR.DOWNLOAD) return "download";
  if (direction === TRANSFER_DIR.DOWNLOAD_UPDATE) return "refresh";
  if (direction === TRANSFER_DIR.DELETE) return "trash";
  return "transfer";
}

/**
 * 方向 tile 的配色类名（上传蓝 / 下载浅蓝 / 删除灰）
 *
 * @param direction - 传输方向
 */
function dirTileClass(direction: number): string {
  if (direction === TRANSFER_DIR.DOWNLOAD || direction === TRANSFER_DIR.DOWNLOAD_UPDATE) {
    return "tp-item__dir--down";
  }
  if (direction === TRANSFER_DIR.DELETE) return "tp-item__dir--del";
  return "tp-item__dir--up";
}

function progressValue(t: { total_size: number; transferred: number }): number | null {
  if (t.total_size <= 0) return null;
  return Math.min(1, t.transferred / t.total_size);
}
function progressColor(state: number): string {
  if (state === TRANSFER_STATE.COMPLETED) return "var(--ok)";
  if (state === TRANSFER_STATE.FAILED) return "var(--err)";
  if (
    state === TRANSFER_STATE.PENDING
    || state === TRANSFER_STATE.WAITING_FOR_NETWORK
    || state === TRANSFER_STATE.BACKING_OFF
    || state === TRANSFER_STATE.RESTART_REQUIRED
  ) return "var(--ink-300)";
  return "var(--grad-brand)";
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

/**
 * 重试失败任务
 *
 * @param item - 传输任务
 */
async function onRetry(item: TransferTask): Promise<void> {
  if (retryingId.value !== null) return; // 防抖：已有重试进行中
  retryingId.value = item.id;
  try {
    await transfer.retry(item.id);
    showToast(
      item.state === TRANSFER_STATE.RESTART_REQUIRED
        ? "已开始重新检查"
        : "已重新加入传输队列",
      { variant: "success" },
    );
  } catch (e) {
    // 用户侧错误提示
    const msg = extractErrorMessage(e);
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

    <!-- 统计条（stat-pill） -->
    <div class="tp-stats">
      <div class="stat-pill">
        <span class="stat-pill__num">{{ transfer.processing }}</span>
        <span class="stat-pill__label">处理中</span>
      </div>
      <div class="stat-pill">
        <span class="stat-pill__num">{{ transfer.waiting }}</span>
        <span class="stat-pill__label">等待中</span>
      </div>
      <div class="stat-pill">
        <span class="stat-pill__num">{{ transfer.completed }}</span>
        <span class="stat-pill__label">已完成</span>
      </div>
      <div class="stat-pill" :class="{ 'stat-pill--err': transfer.failed > 0 }">
        <span class="stat-pill__num">{{ transfer.failed }}</span>
        <span class="stat-pill__label">历史失败</span>
      </div>
      <MatePopupMenu class="tp-stats__clear" :items="clearItems" @select="onClear">
        <MateButton variant="icon" icon="trash" tooltip="清除" />
      </MatePopupMenu>
    </div>

    <!-- 单列表 -->
    <div class="tp-body">
      <MateEmpty v-if="allItems.length === 0" icon="cloud" title="暂无传输任务" />
      <div v-for="item in allItems" :key="item.id" class="tp-item">
        <span class="tp-item__dir" :class="dirTileClass(item.direction)">
          <MateIcon :name="dirIcon(item.direction)" :size="18" />
        </span>
        <div class="tp-item__info">
          <div class="tp-item__namerow">
            <span class="tp-item__tag">{{ DIR_LABEL[item.direction] ?? "—" }}</span>
            <span class="tp-item__name">{{ item.name }}</span>
          </div>
          <!-- 失败时显示错误原因，否则显示进度条 -->
          <div
            v-if="
              (item.state === TRANSFER_STATE.FAILED
                || item.state === TRANSFER_STATE.RESTART_REQUIRED)
              && item.error_message
            "
            class="tp-item__error"
          >
            {{ formatUserMessage(item.error_message) }}
          </div>
          <div class="tp-item__progress" v-else-if="item.direction !== TRANSFER_DIR.DELETE">
            <MateLinearProgress
              class="tp-item__bar"
              :value="progressValue(item)"
              :color="progressColor(item.state)"
            />
            <span class="tp-item__pct">
              {{ pct(item) }}% · {{ fmtSize(item.transferred) }}/{{ fmtSize(item.total_size) }}
            </span>
          </div>
          <div v-else class="tp-item__progress tp-item__progress--delete">删除操作</div>
        </div>
        <div class="tp-item__state" :style="{ color: stateMeta[item.state]?.color }">
          <MateIcon
            :name="stateMeta[item.state]?.icon ?? 'clock'"
            :size="12"
            :spin="stateMeta[item.state]?.spin"
          />
          {{ stateMeta[item.state]?.label ?? "" }}
        </div>
        <!-- 只展示后端真正支持的上传、下载重试或重新检查入口。 -->
        <MateButton
          v-if="canRetryTransferTask(item)"
          variant="icon"
          icon="refresh"
          :tooltip="item.state === TRANSFER_STATE.RESTART_REQUIRED ? '重新检查并重试' : '重试'"
          :loading="retryingId === item.id"
          :disabled="retryingId !== null"
          class="tp-item__retry"
          @click="onRetry(item)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.transfer-popover {
  width: var(--transfer-popover-width);
  height: var(--transfer-popover-height);
  background-color: var(--bg-card);
  border-radius: var(--radius-xl);
  box-shadow: var(--sh-pop), 0 0 0 0.5px rgba(0, 0, 0, 0.05);
  display: flex; flex-direction: column; overflow: hidden;
}

.tp-header {
  height: 60px; display: flex; align-items: center; padding: 0 12px 0 20px; gap: 10px;
  flex-shrink: 0;
}
.tp-header__icon { color: var(--brand-500); }
.tp-header__title { font-size: 17px; font-weight: var(--fw-semibold); flex: 1; color: var(--ink-900); }

/* 统计条（v2 stat-pill） */
.tp-stats {
  display: flex; align-items: center; gap: var(--space-sm);
  padding: 0 20px 14px; flex-shrink: 0;
}
.stat-pill {
  flex: 1;
  background: var(--bg-fill);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex; flex-direction: column; gap: 2px;
}
.stat-pill__num { font-size: 16px; font-weight: 700; font-variant-numeric: tabular-nums; color: var(--ink-900); }
.stat-pill__label { font-size: 11px; color: var(--ink-400); }
.stat-pill--err { background: var(--err-bg); }
.stat-pill--err .stat-pill__num { color: var(--err); }
.tp-stats__clear { flex-shrink: 0; }

.tp-body { flex: 1; overflow-y: auto; border-top: 1px solid var(--line); }

/* 任务行（v2：方向 tile + 迷你 chip + 进度条/错误文案） */
.tp-item {
  min-height: 68px; display: flex; align-items: center; padding: 10px 20px; gap: 12px;
  border-bottom: 1px solid var(--line);
}
.tp-item__dir {
  width: 36px; height: 36px; border-radius: var(--radius-md); flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
}
.tp-item__dir--up { background: var(--brand-50); color: var(--brand-500); }
.tp-item__dir--down { background: var(--info-bg); color: var(--info); }
.tp-item__dir--del { background: var(--bg-fill); color: var(--ink-400); }
.tp-item__info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
.tp-item__namerow { display: flex; align-items: center; gap: 6px; min-width: 0; }
.tp-item__tag {
  font-size: 11px; color: var(--ink-600); padding: 0 6px; height: 20px;
  display: inline-flex; align-items: center;
  background: var(--bg-fill); border-radius: var(--radius-sm); flex-shrink: 0;
}
.tp-item__name {
  font-size: 13.5px; font-weight: var(--fw-medium); color: var(--ink-900);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.tp-item__progress { display: flex; align-items: center; gap: 10px; }
.tp-item__progress--delete { font-size: var(--font-caption); color: var(--ink-400); }
.tp-item__error { font-size: var(--font-caption); color: var(--err); line-height: 1.45; word-break: break-all; }
.tp-item__bar { flex: 1; }
.tp-item__pct {
  font-size: 11.5px; color: var(--ink-400); white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.tp-item__state {
  font-size: var(--font-caption); font-weight: var(--fw-medium); white-space: nowrap;
  flex-shrink: 0; display: inline-flex; align-items: center; gap: 5px;
}
.tp-item__retry { flex-shrink: 0; }
</style>
