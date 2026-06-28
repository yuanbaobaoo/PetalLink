<!-- 日志查看页，级别筛选 + 清空 + 导出。inline 模式下不显示 AppBar，直接嵌入父容器。 -->
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { MateButton, MateTag, MateEmpty, MateCircularProgress } from "@/components/mate";
import { showToast } from "@/components/mate";
import { useAsyncAction } from "@/composables/useAsyncAction";
import * as logsApi from "@/api/logs";
import type { LogRecord } from "@/api/logs";

const props = withDefaults(defineProps<{
  /** 内嵌模式：不渲染顶部 AppBar，由父组件提供导航 */
  inline?: boolean;
}>(), {
  inline: false,
});

type Level = "ALL" | "INFO" | "WARN" | "ERROR";
// 日志级别筛选
const filter = ref<Level>("ALL");
// 原始日志记录
const records = ref<LogRecord[]>([]);
// 加载状态（仅首次加载显示 spinner）
const loading = ref(true);
// 定时轮询句柄
let pollTimer: ReturnType<typeof setInterval> | null = null;
// 导出操作 loading + 防重复
const { loading: exportLoading, run: runExport } = useAsyncAction();
// 清空操作 loading + 防重复
const { loading: clearLoading, run: runClear } = useAsyncAction();

// 按级别精确筛选后的日志
const filtered = computed(() => {
  if (filter.value === "ALL") return records.value;
  return records.value.filter((r) => r.level.toUpperCase() === filter.value);
});

const emit = defineEmits<{ (e: "back"): void }>();

/**
 * 挂载后首次加载日志列表，并启动 2 秒轮询保持实时更新
 */
onMounted(() => {
  load();
  pollTimer = setInterval(load, 2000);
});

/**
 * 卸载时清除轮询定时器
 */
onUnmounted(() => {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
});

async function load(): Promise<void> {
  try {
    const data = await logsApi.listLogs();
    records.value = data;
  } catch {
    // 轮询失败静默保留旧数据，不覆盖为空
    if (records.value.length === 0) records.value = [];
  } finally {
    loading.value = false;
  }
}

/**
 * 日志级别映射为标签主题色
 *
 * @param level - 日志级别字符串
 */
function tagTheme(level: string): "error" | "warning" | "primary" | "default" {
  const l = level.toUpperCase();
  if (l === "ERROR") return "error";
  if (l === "WARN" || l === "WARNING") return "warning";
  if (l === "INFO") return "primary";
  return "default";
}

/**
 * 格式化毫秒时间戳
 *
 * @param ms - 毫秒时间戳
 */
function fmtTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

async function handleClearLogs(): Promise<void> {
  await runClear(async () => {
    try {
      await logsApi.clearLogs();
      records.value = [];
      showToast("已清空日志缓冲");
    } catch {
      showToast("清空失败");
    }
  });
}

async function handleExportLogs(): Promise<void> {
  await runExport(async () => {
    const stamp = new Date().toISOString().slice(0, 10);
    const path = await save({
      defaultPath: `PetalLink-logs-${stamp}.txt`,
      filters: [{ name: "Text", extensions: ["txt", "log"] }],
    });
    if (!path) return;
    try {
      await logsApi.exportLogs(path);
      showToast("日志已导出");
    } catch (e) {
      showToast("导出失败：" + String(e));
    }
  });
}

</script>

<template>
  <div :class="['log-page', { 'log-page--inline': inline }]">
    <!-- AppBar（独立页面模式） -->
    <div v-if="!inline" class="log-appbar">
      <MateButton variant="icon" icon="arrow" tooltip="返回" class="log-appbar__back" @click="emit('back')" />
      <span class="log-appbar__title">同步日志</span>
    </div>

    <!-- 工具栏 -->
    <div class="log-toolbar">
      <div class="log-filters">
        <MateTag v-for="lv in (['ALL','INFO','WARN','ERROR'] as Level[])" :key="lv"
          :label="lv"
          :theme="filter === lv ? (lv === 'ERROR' ? 'error' : lv === 'WARN' ? 'warning' : lv === 'INFO' ? 'primary' : 'default') : 'default'"
          @click="filter = lv"
        />
      </div>
      <MateButton variant="icon" icon="download" tooltip="导出" :loading="exportLoading" :disabled="exportLoading || clearLoading" @click="handleExportLogs" />
      <MateButton variant="icon" icon="trash" tooltip="清空" :loading="clearLoading" :disabled="exportLoading || clearLoading" @click="handleClearLogs" />
    </div>

    <!-- 列表 -->
    <div class="log-body">
      <div v-if="loading" class="log-loading"><MateCircularProgress :size="24" /></div>
      <MateEmpty v-else-if="filtered.length === 0" icon="list" title="暂无日志" />
      <div v-else>
        <div v-for="(r, i) in filtered" :key="i" class="log-item">
          <MateTag :label="r.level" :theme="tagTheme(r.level)" size="small" />
          <div class="log-item__content">
            <div class="log-item__msg">{{ r.message }}</div>
            <div class="log-item__meta">{{ fmtTime(r.time_ms) }} · {{ r.logger_name }}</div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.log-page { display: flex; flex-direction: column; width: 100%; height: 100%; background: var(--bg-page); }
.log-page--inline { height: 100%; background: transparent; }
.log-appbar { height: var(--appbar-height); display: flex; align-items: center; gap: var(--space-sm); padding: 0 var(--space-lg); border-bottom: 0.5px solid var(--border); background: var(--bg-container); flex-shrink: 0; }
.log-appbar__back { transform: rotate(180deg); }
.log-appbar__title { font-size: var(--font-title-sm); font-weight: var(--fw-semibold); }
.log-toolbar { display: flex; align-items: center; gap: var(--space-sm); padding: var(--space-md) var(--space-lg); flex-shrink: 0; }
.log-filters { display: flex; gap: var(--space-sm); }
.log-filters :deep(.mate-tag) { cursor: pointer; user-select: none; }
.log-body { flex: 1; overflow-y: auto; padding: var(--space-md); }
.log-loading { display: flex; justify-content: center; padding: var(--space-xl); }
.log-item { display: flex; gap: var(--space-md); padding: var(--space-sm) var(--space-md); border-bottom: 0.5px solid var(--border); }
.log-item__content { flex: 1; min-width: 0; }
.log-item__msg { font-size: var(--font-body); color: var(--text-primary); }
.log-item__meta { font-size: var(--font-caption); color: var(--text-secondary); font-family: var(--font-mono); margin-top: 2px; }
</style>
