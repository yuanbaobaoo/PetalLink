<!-- 传输队列弹窗（v2：440×580，stat-pill 统计条 + 方向 tile 任务行），单列表含上传/下载/删除 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useTransferStore } from "@/stores/transfer";
import { useFolderSyncStore } from "@/stores/folderSync";
import { FOLDER_SYNC_JOB_STATE } from "@/api/folderSync";
import type { FolderSyncJob } from "@/api/folderSync";
import {
  TRANSFER_DIR,
  TRANSFER_ERROR_KIND,
  TRANSFER_STATE,
  DIR_LABEL,
  canRetryTransferTask,
  canCancelTransferTask,
  isNameConflictTask,
  isNewlyQueued,
  capTransferHistory,
} from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import {
  MateIcon,
  MateButton,
  MateInfoBanner,
  MateLinearProgress,
  MateEmpty,
  MatePopupMenu,
  confirmDialog,
  showToast,
} from "@/components/mate";
import type { PopupItem } from "@/components/mate";
import { formatFileSize } from "@/utils/format";
import { extractErrorMessage, formatUserMessage } from "@/utils/error";

// 传输 store
const transfer = useTransferStore();
// 目录同步作业 store
const folderSync = useFolderSyncStore();

// 当前传输队列的渲染范围：非终态全量 + 终态历史限量（大批量同步时全量渲染会卡死 UI）
const cappedItems = computed(() => capTransferHistory(transfer.tasks));
// 队列筛选：全部 / 进行中 / 需要处理 / 已结束
type TransferFilter = "all" | "active" | "failed" | "done";
// 当前筛选条件
const transferFilter = ref<TransferFilter>("all");
// 筛选条选项
const FILTER_OPTIONS: { value: TransferFilter; label: string; match: (state: number) => boolean }[] = [
  { value: "all", label: "全部", match: () => true },
  {
    value: "active",
    label: "进行中",
    match: (state) => state === TRANSFER_STATE.RUNNING
      || state === TRANSFER_STATE.PENDING
      || state === TRANSFER_STATE.WAITING_FOR_NETWORK
      || state === TRANSFER_STATE.BACKING_OFF
      || state === TRANSFER_STATE.VERIFYING_REMOTE,
  },
  {
    value: "failed",
    label: "需要处理",
    match: (state) => state === TRANSFER_STATE.FAILED
      || state === TRANSFER_STATE.RESTART_REQUIRED,
  },
  {
    value: "done",
    label: "已结束",
    match: (state) => state === TRANSFER_STATE.COMPLETED
      || state === TRANSFER_STATE.CANCELED,
  },
];
// 实际渲染的任务行（先按筛选条件过滤，再应用终态折叠）
const allItems = computed(() => {
  // 当前筛选条件的匹配函数。
  const option = FILTER_OPTIONS.find((item) => item.value === transferFilter.value);
  if (!option || option.value === "all") return cappedItems.value.items;
  return cappedItems.value.items.filter((task) => option.match(task.state));
});

// 每个筛选条件下的可见条数（用于筛选条上的计数）
const filterCounts = computed(() => {
  // 各筛选项的数量快照。
  const counts: Record<TransferFilter, number> = { all: 0, active: 0, failed: 0, done: 0 };
  for (const task of cappedItems.value.items) {
    counts.all += 1;
    for (const option of FILTER_OPTIONS) {
      if (option.value !== "all" && option.match(task.state)) counts[option.value] += 1;
    }
  }
  return counts;
});

// 上一次进度采样：任务 ID → 已传字节与采样时间
const progressSamples = new Map<number, { transferred: number; at: number }>();
// 当前估算速率：任务 ID → 字节/秒
const speeds = ref<Record<number, number>>({});
// 展开「详情」的任务 ID 集合
const expandedIds = ref<Set<number>>(new Set());
// 被折叠的终态历史条数
const hiddenTerminalCount = computed(() => cappedItems.value.hiddenTerminalCount);

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

// 目录同步作业状态的展示元信息（排队 / 规划 / 完成 / 失败）
const folderJobMeta: Record<number, { icon: string; label: string; color: string }> = {
  [FOLDER_SYNC_JOB_STATE.QUEUED]: { icon: "clock", label: "排队中", color: "var(--ink-400)" },
  [FOLDER_SYNC_JOB_STATE.PLANNING]: { icon: "sync", label: "正在规划", color: "var(--brand-500)" },
  [FOLDER_SYNC_JOB_STATE.COMPLETED]: { icon: "check", label: "已投递", color: "var(--ok)" },
  [FOLDER_SYNC_JOB_STATE.FAILED]: { icon: "x", label: "失败", color: "var(--err)" },
  [FOLDER_SYNC_JOB_STATE.CANCELED]: { icon: "x", label: "已取消", color: "var(--ink-400)" },
};

// 清除菜单选项（已完成 / 失败与取消 / 全部已结束）
const clearItems: PopupItem[] = [
  { value: "completed", label: "清除已完成", icon: "check" },
  { value: "failed", label: "清除失败与取消", icon: "x", danger: true },
  { value: "finished", label: "清除全部已结束", icon: "transfer" },
];

// 关闭弹窗事件
const emit = defineEmits<{ (e: "close"): void }>();

// 正在提交重试的任务 id 集合：只影响各自那一行，不阻塞其它行
const retryingIds = ref<Set<number>>(new Set());
// 正在执行冲突决策/取消的任务 id 集合：同样只影响各自那一行
const actingIds = ref<Set<number>>(new Set());
// 一键重试全部是否正在提交（毫秒级，仅用于按钮 loading）
const retryingAll = ref(false);
// 批量取消失败任务是否正在提交（毫秒级，仅用于按钮 loading）
const cancelingAll = ref(false);
// 正在取消的目录同步作业 id 集合
const cancelingJobIds = ref<Set<number>>(new Set());
// 每秒刷新的当前时间，用于「等待重试确认」的倒计时文案
const nowTick = ref(Date.now());
// 倒计时定时器句柄（弹层关闭时释放）
let tickTimer: number | null = null;
// 当前可重试的失败项数量，用于决定是否展示「重试全部」汇总条
const retryableCount = computed(
  () => transfer.tasks.filter((item) => canRetryTransferTask(item) && !isNameConflictTask(item)).length,
);
// 当前可取消的失败项数量（失败 / 需要重新检查）：决定是否展示失败汇总条与「取消全部」
const cancelableCount = computed(
  () => transfer.tasks.filter((item) => canCancelTransferTask(item)).length,
);

onMounted(() => {
  transfer.loadAll();
  folderSync.loadAll();
  // 倒计时只影响文案，1 秒一次足够；弹层关闭即释放。
  tickTimer = window.setInterval(() => {
    nowTick.value = Date.now();
    sampleProgress();
  }, 1000);
});

/**
 * 卸载时释放倒计时定时器。
 */
onUnmounted(() => {
  if (tickTimer !== null) window.clearInterval(tickTimer);
  tickTimer = null;
});

/**
 * 返回传输方向对应的图标名。
 */
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

/**
 * 把传输字节数转换为进度组件值。
 */
function progressValue(t: { total_size: number; transferred: number }): number | null {
  if (t.total_size <= 0) return null;
  return Math.min(1, t.transferred / t.total_size);
}
/**
 * 返回任务状态对应的进度条颜色。
 */
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

/**
 * 把字节数格式化为紧凑的可读文本。
 */
function fmtSize(bytes: number): string {
  return formatFileSize(bytes);
}
/**
 * 计算任务当前的整数百分比。
 */
function pct(t: { total_size: number; transferred: number }): number {
  return t.total_size > 0 ? Math.round((t.transferred / t.total_size) * 100) : 0;
}

/**
 * 采样一次运行中任务的进度，估算每秒速率。
 *
 * 速率只做展示，不参与任何调度判断；只对 RUNNING 任务采样，
 * 其余状态清掉样本，避免恢复后算出跨越暂停的虚假速率。
 */
function sampleProgress(): void {
  // 本次采样到的速率快照。
  const next: Record<number, number> = {};
  for (const task of transfer.tasks) {
    if (task.state !== TRANSFER_STATE.RUNNING) {
      progressSamples.delete(task.id);
      continue;
    }
    // 上一次采样点。
    const previous = progressSamples.get(task.id);
    if (previous) {
      // 两次采样之间新增的字节数与时间差。
      const deltaBytes = task.transferred - previous.transferred;
      const deltaMs = nowTick.value - previous.at;
      if (deltaMs >= 500 && deltaBytes > 0) {
        next[task.id] = Math.round((deltaBytes * 1000) / deltaMs);
      }
    }
    progressSamples.set(task.id, { transferred: task.transferred, at: nowTick.value });
  }
  speeds.value = next;
}

/**
 * 返回任务当前的估算速率文案。
 *
 * @param task - 传输任务
 */
function speedText(task: TransferTask): string {
  // 当前任务的估算速率（字节/秒）。
  const bytesPerSecond = speeds.value[task.id];
  if (!bytesPerSecond || bytesPerSecond <= 0) return "";
  return `${fmtSize(bytesPerSecond)}/s`;
}

/**
 * 返回任务剩余时间的估算文案。
 *
 * @param task - 传输任务
 */
function etaText(task: TransferTask): string {
  // 当前任务的估算速率（字节/秒）。
  const bytesPerSecond = speeds.value[task.id];
  if (!bytesPerSecond || bytesPerSecond <= 0) return "";
  // 剩余字节数。
  const remaining = Math.max(0, task.total_size - task.transferred);
  if (remaining === 0) return "";
  // 剩余秒数。
  const seconds = Math.ceil(remaining / bytesPerSecond);
  if (seconds >= 3600) return `剩余 ${Math.floor(seconds / 3600)} 小时`;
  if (seconds >= 60) return `剩余 ${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
  return `剩余 ${seconds} 秒`;
}

/**
 * 错误分类的中文标签。
 *
 * @param kind - 持久化错误分类值
 */
function errorKindLabel(kind: number | null): string {
  if (kind === null) return "未知";
  switch (kind) {
    case TRANSFER_ERROR_KIND.NETWORK: return "网络不可用";
    case TRANSFER_ERROR_KIND.TIMEOUT: return "请求超时";
    case TRANSFER_ERROR_KIND.AUTH: return "登录状态失效";
    case TRANSFER_ERROR_KIND.RATE_LIMIT: return "服务端限流";
    case TRANSFER_ERROR_KIND.SERVER: return "服务端异常";
    case TRANSFER_ERROR_KIND.QUOTA: return "空间或配额不足";
    case TRANSFER_ERROR_KIND.PERMISSION: return "权限不足";
    case TRANSFER_ERROR_KIND.VALIDATION: return "请求参数被拒绝";
    case TRANSFER_ERROR_KIND.SESSION_EXPIRED: return "续传会话失效";
    case TRANSFER_ERROR_KIND.REMOTE_AMBIGUOUS: return "云端结果不确定";
    case TRANSFER_ERROR_KIND.LOCAL_CHANGED: return "本地文件已变化";
    case TRANSFER_ERROR_KIND.NAME_CONFLICT: return "同名冲突待处理";
    default: return "未知";
  }
}

/**
 * 返回任务下一次自动重试的时间文案。
 *
 * @param task - 传输任务
 */
function nextRetryText(task: TransferTask): string {
  if (task.next_retry_at === null) return "—";
  // 距离下次重试的秒数。
  const seconds = Math.max(0, Math.ceil((task.next_retry_at - nowTick.value) / 1000));
  return seconds === 0 ? "即将重试" : `${seconds} 秒后`;
}

/**
 * 切换某条任务的详情展开状态。
 *
 * @param taskId - 传输任务 ID
 */
function toggleDetails(taskId: number): void {
  if (expandedIds.value.has(taskId)) expandedIds.value.delete(taskId);
  else expandedIds.value.add(taskId);
}

/**
 * 复制一条任务的诊断信息到剪贴板。
 *
 * @param task - 传输任务
 */
async function copyDiagnostics(task: TransferTask): Promise<void> {
  // 可直接贴进问题反馈的诊断文本。
  const lines = [
    `任务 ID：${task.id}`,
    `名称：${task.name}`,
    `相对路径：${task.relative_path ?? "—"}`,
    `方向：${DIR_LABEL[task.direction] ?? task.direction}`,
    `状态：${stateLabel(task)}`,
    `错误分类：${errorKindLabel(task.error_kind)}`,
    `错误信息：${task.error_message ?? "—"}`,
    `已传/总量：${task.transferred}/${task.total_size}`,
    `尝试次数：${task.attempt_count}，核验次数：${task.verify_attempt_count}`,
    `下次重试：${task.next_retry_at ?? "—"}`,
  ];
  try {
    await navigator.clipboard.writeText(lines.join("\n"));
    showToast("诊断信息已复制");
  } catch {
    showToast("复制失败，请手动选择文本", { variant: "warning" });
  }
}

/**
 * 返回任务状态的展示文案。
 *
 * 「正在确认结果」需要再分成「在跑」与「在等」两种，否则用户无法判断是否卡住：
 * 有计划重试时间时显示倒计时，否则显示已经确认过几轮。
 *
 * @param item - 传输任务
 */
function stateLabel(item: TransferTask): string {
  if (item.state !== TRANSFER_STATE.VERIFYING_REMOTE) {
    return stateMeta[item.state]?.label ?? "";
  }
  // 计划中的下一次核验时间。
  const nextRetryAt = item.next_retry_at;
  if (nextRetryAt !== null && nextRetryAt > nowTick.value) {
    // 距离下次核验的秒数，向上取整避免显示 0s。
    const seconds = Math.ceil((nextRetryAt - nowTick.value) / 1000);
    return `等待重试确认（${seconds}s）`;
  }
  // 已确认轮次：0 表示还没跑过，>0 表示反复确认中。
  if (item.verify_attempt_count > 0) {
    return `正在确认结果（第 ${item.verify_attempt_count + 1} 次）`;
  }
  return "正在确认结果";
}

/**
 * 返回目录同步作业的展示元信息。
 *
 * @param job - 目录同步作业
 */
function folderJobMetaOf(job: FolderSyncJob): { icon: string; label: string; color: string } {
  return folderJobMeta[job.state] ?? { icon: "clock", label: "未知", color: "var(--ink-400)" };
}

/**
 * 目录同步作业的进度文案：规划中显示待处理总数，完成后显示已投递数量。
 *
 * @param job - 目录同步作业
 */
function folderJobDetail(job: FolderSyncJob): string {
  if (job.state === FOLDER_SYNC_JOB_STATE.QUEUED) return "等待开始";
  if (job.state === FOLDER_SYNC_JOB_STATE.PLANNING) {
    return job.planned_total > 0
      ? `已投递 ${job.enqueued}/${job.planned_total} 项`
      : "正在读取云端目录";
  }
  if (job.state === FOLDER_SYNC_JOB_STATE.COMPLETED) return `已投递 ${job.enqueued} 项，在下方传输队列执行`;
  return job.error_message ? formatUserMessage(job.error_message) : "未能完成";
}

/**
 * 取消一个排队中或规划中的目录同步作业。
 *
 * @param job - 目录同步作业
 */
async function onCancelFolderJob(job: FolderSyncJob): Promise<void> {
  if (cancelingJobIds.value.has(job.id)) return;
  cancelingJobIds.value.add(job.id);
  try {
    await folderSync.cancel(job.id);
    showToast("目录同步作业已取消");
  } catch (e) {
    showToast("取消失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    cancelingJobIds.value.delete(job.id);
  }
}

/**
 * 按菜单选项清理对应范围的传输记录。
 */
async function onClear(value: string | number): Promise<void> {
  if (value === "completed") await transfer.clearCompleted();
  else if (value === "failed") await transfer.clearFailed();
  else if (value === "finished") await transfer.clearFinished();
}

/**
 * 重试单条失败任务。
 *
 * 只在这一条请求提交期间显示 loading（毫秒级），任何其它行的按钮保持可用；
 * 任务是否真的在跑，一律以传输队列的权威状态为准。
 *
 * @param item - 传输任务
 */
async function onRetry(item: TransferTask): Promise<void> {
  if (retryingIds.value.has(item.id)) return;
  retryingIds.value.add(item.id);
  try {
    // 后端返回值区分「本次新入队」与「本来就在队列中」，两者都是成功。
    const result = await transfer.retry(item.id);
    if (!isNewlyQueued(result)) {
      showToast("该任务已在队列中");
      return;
    }
    showToast(
      item.state === TRANSFER_STATE.RESTART_REQUIRED
        ? "已开始重新检查"
        : "已加入传输队列",
      { variant: "success" },
    );
  } catch (e) {
    // 用户侧错误提示
    const msg = extractErrorMessage(e);
    showToast("重试失败：" + msg, { variant: "error" });
  } finally {
    retryingIds.value.delete(item.id);
  }
}

/**
 * 一键重试全部失败项。
 *
 * 命令只入队并立即返回汇总，不等待任何任务执行完成。
 */
async function onRetryAll(): Promise<void> {
  if (retryingAll.value) return;
  retryingAll.value = true;
  try {
    // 后端返回入队/跳过/拒绝的数量汇总。
    const summary = await transfer.retryAll();
    const parts = [`已加入队列 ${summary.queued} 项`];
    if (summary.already_queued > 0) parts.push(`${summary.already_queued} 项已在队列中`);
    if (summary.skipped > 0) parts.push(`跳过 ${summary.skipped} 项`);
    if (summary.rejected > 0) parts.push(`${summary.rejected} 项校验未通过`);
    showToast(parts.join("，"), { variant: summary.rejected > 0 ? "warning" : "success" });
  } catch (e) {
    const msg = extractErrorMessage(e);
    showToast("批量重试失败：" + msg, { variant: "error" });
  } finally {
    retryingAll.value = false;
  }
}

/**
 * 批量取消全部失败任务。
 *
 * 保留历史行（可在「清除失败与取消」里删除），但释放路径占用，
 * 让被隔离的同步动作（例如目录删除）在下一轮继续执行。
 */
async function onCancelAll(): Promise<void> {
  if (cancelingAll.value) return;
  if (cancelableCount.value === 0) return;
  // 取消是显式动作：先确认，避免误点把待重试的失败项一起清走。
  const ok = await confirmDialog({
    title: "取消全部失败任务",
    titleIcon: "x",
    danger: true,
    confirmText: "全部取消",
    content: `将取消 ${cancelableCount.value} 项失败任务，保留历史记录但不自动重试。\n\n取消后这些路径不再被任务占用，相关同步动作（如删除云端文件）会重新参与规划。`,
  });
  if (!ok) return;
  cancelingAll.value = true;
  try {
    // 后端逐条 CAS 取消并返回数量。
    const canceled = await transfer.cancelAllFailed();
    showToast(canceled > 0 ? `已取消 ${canceled} 项失败任务` : "没有可取消的失败任务");
  } catch (e) {
    const msg = extractErrorMessage(e);
    showToast("批量取消失败：" + msg, { variant: "error" });
  } finally {
    cancelingAll.value = false;
  }
}

/**
 * 是否显示行级更多操作菜单：同名冲突三项决策；普通失败/需要重新检查任务也提供取消出口。
 *
 * @param item - 传输任务
 */
function showRowMenu(item: TransferTask): boolean {
  // 所有可取消状态都给出「取消」：失败行既是重试锚点，也长期占用路径身份，
  // 用户必须能主动清走，否则目录删除等同步动作会被永久隔离（D12/D4）。
  return canCancelTransferTask(item);
}

/**
 * 任务行的处理菜单项：同名冲突给三个决策出口，其他可取消任务只给取消。
 *
 * @param item - 传输任务
 */
function rowActionItems(item: TransferTask): PopupItem[] {
  if (isNameConflictTask(item)) {
    return [
      { value: "overwrite", label: "覆盖远端（旧版保留副本）", icon: "transfer", danger: true },
      { value: "keep-both", label: "保留两者（本地改名上传）", icon: "file" },
      { value: "cancel", label: "取消任务", icon: "x" },
    ];
  }
  return [{ value: "cancel", label: "取消任务", icon: "x" }];
}

/**
 * 执行任务行决策操作（覆盖远端 / 保留两者 / 取消任务），均先弹确认。
 *
 * @param value - 菜单项值
 * @param item - 传输任务
 */
async function onRowAction(value: string | number, item: TransferTask): Promise<void> {
  if (actingIds.value.has(item.id)) return;
  if (value === "overwrite") {
    const ok = await confirmDialog({
      title: "覆盖远端", titleIcon: "transfer", danger: true, confirmText: "覆盖上传",
      content: `将用本地文件「${item.name}」覆盖云端同名文件。\n\n云端现有版本会自动改名为「${item.name} (云端副本 …)」保留，不会丢失任何数据。`,
    });
    if (!ok) return;
  } else if (value === "keep-both") {
    const ok = await confirmDialog({
      title: "保留两者", titleIcon: "file", confirmText: "改名并上传",
      content: `本地文件「${item.name}」将改名为「… (本地副本 …)」并作为新文件上传；云端同名文件保持不变，两份内容都会保留。`,
    });
    if (!ok) return;
  } else if (value === "cancel") {
    const ok = await confirmDialog({
      title: "取消任务", titleIcon: "x", danger: true, confirmText: "取消任务",
      content: `取消后「${item.name}」不再自动同步，直到本地或云端出现新变化。确定取消吗？`,
    });
    if (!ok) return;
  } else {
    return;
  }

  actingIds.value.add(item.id);
  try {
    if (value === "overwrite") {
      await transfer.overwriteRemote(item.id);
      showToast("已保留云端旧版为副本，开始覆盖上传", { variant: "success" });
    } else if (value === "keep-both") {
      await transfer.keepBoth(item.id);
      showToast("本地文件已改名，将作为新文件上传", { variant: "success" });
    } else {
      await transfer.cancel(item.id);
      showToast("任务已取消", { variant: "success" });
    }
  } catch (e) {
    const msg = extractErrorMessage(e);
    showToast("操作失败：" + msg, { variant: "error" });
  } finally {
    actingIds.value.delete(item.id);
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

    <!-- 队列筛选：上百条历史时按状态收敛视图 -->
    <div class="tp-filters">
      <button
        v-for="option in FILTER_OPTIONS"
        :key="option.value"
        :class="['tp-filter', { 'is-active': transferFilter === option.value }]"
        @click="transferFilter = option.value"
      >
        {{ option.label }}
        <span class="tp-filter__count">{{ filterCounts[option.value] }}</span>
      </button>
    </div>

    <!-- 目录同步作业：用户点击「双端对齐」后先在这里出现，投递完成后进入下方传输队列 -->
    <div v-if="folderSync.visibleJobs.length > 0" class="tp-jobs">
      <div class="tp-jobs__title">目录同步</div>
      <div v-for="job in folderSync.visibleJobs" :key="`job-${job.id}`" class="tp-job">
        <span class="tp-job__dir">
          <MateIcon :name="folderJobMetaOf(job).icon" :size="16" />
        </span>
        <div class="tp-job__info">
          <span class="tp-job__name">{{ job.rel_path || '云端根目录' }}</span>
          <span class="tp-job__detail">{{ folderJobDetail(job) }}</span>
        </div>
        <span class="tp-job__state" :style="{ color: folderJobMetaOf(job).color }">
          {{ folderJobMetaOf(job).label }}
        </span>
        <MateButton
          v-if="job.state === FOLDER_SYNC_JOB_STATE.QUEUED || job.state === FOLDER_SYNC_JOB_STATE.PLANNING"
          variant="icon"
          icon="x"
          tooltip="取消该目录同步"
          :loading="cancelingJobIds.has(job.id)"
          :disabled="cancelingJobIds.has(job.id)"
          @click="onCancelFolderJob(job)"
        />
      </div>
    </div>

    <!-- 失败汇总条：一条提示 + 重试/取消两个出口，避免用户逐条点击 -->
    <MateInfoBanner v-if="cancelableCount > 0" variant="error" class="tp-failed-banner">
      <span>有 {{ cancelableCount }} 项需要处理</span>
      <template #action>
        <MateButton
          v-if="retryableCount > 0"
          variant="text"
          :loading="retryingAll"
          :disabled="retryingAll"
          @click="onRetryAll"
        >
          重试全部
        </MateButton>
        <MateButton
          variant="text"
          :loading="cancelingAll"
          :disabled="cancelingAll"
          @click="onCancelAll"
        >
          取消全部
        </MateButton>
      </template>
    </MateInfoBanner>

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
            <!-- 速率与剩余时间只用于展示，由 1 秒采样的差值推算 -->
            <span v-if="speedText(item)" class="tp-item__rate">
              {{ speedText(item) }}<template v-if="etaText(item)"> · {{ etaText(item) }}</template>
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
          {{ stateLabel(item) }}
        </div>
        <!-- 详情：失败与需要处理的条目给出可复制的诊断信息 -->
        <MateButton
          v-if="item.state === TRANSFER_STATE.FAILED
            || item.state === TRANSFER_STATE.RESTART_REQUIRED
            || item.state === TRANSFER_STATE.VERIFYING_REMOTE"
          variant="icon"
          icon="info"
          :tooltip="expandedIds.has(item.id) ? '收起详情' : '查看详情'"
          class="tp-item__detail-btn"
          @click="toggleDetails(item.id)"
        />
        <!-- 只展示后端真正支持的上传、下载重试或重新检查入口；loading 只影响本行。 -->
        <MateButton
          v-if="canRetryTransferTask(item) && !isNameConflictTask(item)"
          variant="icon"
          icon="refresh"
          :tooltip="item.state === TRANSFER_STATE.RESTART_REQUIRED ? '重新检查并重试' : '重试'"
          :loading="retryingIds.has(item.id)"
          :disabled="retryingIds.has(item.id)"
          class="tp-item__retry"
          @click="onRetry(item)"
        />
        <!-- 同名冲突三项决策 / 普通待重启与不可重试失败任务的取消出口。 -->
        <MatePopupMenu
          v-if="showRowMenu(item)"
          :items="rowActionItems(item)"
          class="tp-item__menu"
          @select="(value) => onRowAction(value, item)"
        >
          <MateButton
            variant="icon"
            icon="list"
            :tooltip="isNameConflictTask(item) ? '处理同名冲突' : '更多操作'"
            :loading="actingIds.has(item.id)"
            :disabled="actingIds.has(item.id)"
          />
        </MatePopupMenu>
        <!-- 展开的诊断信息 -->
        <div v-if="expandedIds.has(item.id)" class="tp-item__details">
          <div class="tp-detail"><span>错误分类</span><b>{{ errorKindLabel(item.error_kind) }}</b></div>
          <div class="tp-detail"><span>尝试次数</span><b>{{ item.attempt_count }}</b></div>
          <div class="tp-detail"><span>核验次数</span><b>{{ item.verify_attempt_count }}</b></div>
          <div class="tp-detail" v-if="item.state === TRANSFER_STATE.VERIFYING_REMOTE || item.state === TRANSFER_STATE.BACKING_OFF">
            <span>下次重试</span><b>{{ nextRetryText(item) }}</b>
          </div>
          <div class="tp-detail tp-detail--wide" v-if="item.error_message">
            <span>原始信息</span><b>{{ item.error_message }}</b>
          </div>
          <MateButton variant="text" @click="copyDiagnostics(item)">复制诊断信息</MateButton>
        </div>
      </div>
      <!-- 终态历史超上限时的折叠提示 -->
      <div v-if="hiddenTerminalCount > 0" class="tp-truncated">
        仅显示最近 100 条历史记录，其余 {{ hiddenTerminalCount }} 条已折叠
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

/* 队列筛选条 */
.tp-filters {
  display: flex; gap: 6px; padding: 0 20px 12px; flex-shrink: 0;
}
.tp-filter {
  display: inline-flex; align-items: center; gap: 5px;
  border: none; cursor: pointer; font-family: var(--font-family);
  font-size: var(--font-caption); font-weight: var(--fw-medium);
  color: var(--ink-600); background: var(--bg-fill);
  padding: 5px 10px; border-radius: var(--radius-sm);
}
.tp-filter.is-active { background: var(--brand-50); color: var(--brand-500); }
.tp-filter__count { font-variant-numeric: tabular-nums; color: var(--ink-400); }
.tp-filter.is-active .tp-filter__count { color: var(--brand-500); }

/* 目录同步作业分区 */
.tp-jobs {
  padding: 0 20px 12px; flex-shrink: 0;
  display: flex; flex-direction: column; gap: 6px;
}
.tp-jobs__title {
  font-size: var(--font-caption); color: var(--ink-400); font-weight: var(--fw-medium);
}
.tp-job {
  display: flex; align-items: center; gap: 10px;
  background: var(--bg-fill); border-radius: var(--radius-md); padding: 8px 10px;
}
.tp-job__dir {
  width: 28px; height: 28px; border-radius: var(--radius-sm); flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  background: var(--brand-50); color: var(--brand-500);
}
.tp-job__info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.tp-job__name {
  font-size: 13px; font-weight: var(--fw-medium); color: var(--ink-900);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.tp-job__detail { font-size: var(--font-caption); color: var(--ink-400); }
.tp-job__state { font-size: var(--font-caption); font-weight: var(--fw-medium); white-space: nowrap; }

/* 失败汇总条：与统计条保持同一水平留白 */
.tp-failed-banner { margin: 0 20px 12px; flex-shrink: 0; }

.tp-body { flex: 1; overflow-y: auto; border-top: 1px solid var(--line); }

/* 任务行（v2：方向 tile + 迷你 chip + 进度条/错误文案） */
.tp-item {
  min-height: 68px; display: flex; align-items: center; flex-wrap: wrap;
  padding: 10px 20px; gap: 12px;
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
.tp-item__rate {
  font-size: 11px; color: var(--ink-400); white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.tp-item__detail-btn { flex-shrink: 0; }

/* 诊断详情面板：占满整行，挂在任务行内部 */
.tp-item__details {
  flex-basis: 100%; margin-top: 8px; padding: 10px 12px;
  background: var(--bg-fill); border-radius: var(--radius-md);
  display: flex; flex-wrap: wrap; gap: 6px 18px;
}
.tp-detail { display: flex; gap: 6px; font-size: var(--font-caption); color: var(--ink-400); }
.tp-detail b { font-weight: var(--fw-medium); color: var(--ink-700); word-break: break-all; }
.tp-detail--wide { flex-basis: 100%; }
.tp-item__menu { flex-shrink: 0; }

/* 终态历史折叠提示 */
.tp-truncated {
  padding: 12px 20px; text-align: center;
  font-size: var(--font-caption); color: var(--ink-400);
}
</style>
