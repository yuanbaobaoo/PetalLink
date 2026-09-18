/**
 * 传输队列 Store —— 
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { commands } from "@/api/generated";
import * as transferApi from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import { TRANSFER_DIR, TRANSFER_STATE } from "@/api/transfer";

// 全局传输队列 Store。
export const useTransferStore = defineStore("transfer", () => {
  // 全部传输任务
  const tasks = ref<TransferTask[]>([]);
  // loadAll 请求序号（递增），用于丢弃乱序响应
  let nextLoadRequest = 0;
  // 已应用的最大 loadAll 请求序号，防止旧响应覆盖新状态
  let lastAppliedLoadRequest = 0;

  // 上传任务
  const uploads = computed(() => tasks.value.filter((t) => t.direction === TRANSFER_DIR.UPLOAD));
  // 下载任务（含「更新」——云端新版本覆盖本地，本质也是下载方向）
  const downloads = computed(() => tasks.value.filter(
    (t) => t.direction === TRANSFER_DIR.DOWNLOAD
      || t.direction === TRANSFER_DIR.DOWNLOAD_UPDATE,
  ));
  // 各状态计数一次遍历聚合；下方同名 computed 仅做读取转发。
  const stateCounts = computed(() => {
    // 按传输状态分类的任务计数。
    const counts = {
      running: 0,
      pending: 0,
      waitingNetwork: 0,
      backingOff: 0,
      verifyingRemote: 0,
      restartRequired: 0,
      completed: 0,
      failed: 0,
      canceled: 0,
    };
    for (const task of tasks.value) {
      switch (task.state) {
        case TRANSFER_STATE.RUNNING: counts.running++; break;
        case TRANSFER_STATE.PENDING: counts.pending++; break;
        case TRANSFER_STATE.WAITING_FOR_NETWORK: counts.waitingNetwork++; break;
        case TRANSFER_STATE.BACKING_OFF: counts.backingOff++; break;
        case TRANSFER_STATE.VERIFYING_REMOTE: counts.verifyingRemote++; break;
        case TRANSFER_STATE.RESTART_REQUIRED: counts.restartRequired++; break;
        case TRANSFER_STATE.COMPLETED: counts.completed++; break;
        case TRANSFER_STATE.FAILED: counts.failed++; break;
        case TRANSFER_STATE.CANCELED: counts.canceled++; break;
      }
    }
    return counts;
  });
  // 进行中
  const running = computed(() => stateCounts.value.running);
  // 等待调度
  const pending = computed(() => stateCounts.value.pending);
  // 等待网络恢复
  const waitingNetwork = computed(() => stateCounts.value.waitingNetwork);
  // 等待退避截止时间
  const backingOff = computed(() => stateCounts.value.backingOff);
  // 正在核验有歧义的远端结果
  const verifyingRemote = computed(() => stateCounts.value.verifyingRemote);
  // 原任务不能原样重试，等待同步引擎重新规划
  const restartRequired = computed(() => stateCounts.value.restartRequired);
  // 已完成
  const completed = computed(() => stateCounts.value.completed);
  // 永久失败历史
  const failed = computed(() => stateCounts.value.failed);
  // 已取消
  const canceled = computed(() => stateCounts.value.canceled);
  // 真正执行中的状态（传输或远端核验）
  const processing = computed(() => running.value + verifyingRemote.value);
  // 尚未执行完成、但当前在等待条件的状态
  const waiting = computed(() => pending.value + waitingNetwork.value + backingOff.value + restartRequired.value);
  // 所有非终态任务；不能把等待/退避/核验/重新规划误判成完成
  const active = computed(() => processing.value + waiting.value);
  // 是否存在活跃传输任务。
  const hasActiveTasks = computed(() => active.value > 0);

  /**
   * 加载全部传输任务
   *
   * @returns 是否成功应用（乱序/IPC 失败返回 false，保留旧快照）
   */
  async function loadAll(): Promise<boolean> {
    // 本次加载请求序号。
    const requestId = ++nextLoadRequest;
    try {
      // 后端返回的完整传输列表。
      const loaded = await transferApi.listAllTransfers();
      if (requestId < lastAppliedLoadRequest) return false;

      // 即使两个 invoke 的响应乱序，也不能让同一 task 的旧 state_revision 回写。
      const currentRevisions = new Map(
        tasks.value.map((task) => [task.id, task.state_revision]),
      );
      if (loaded.some((task) => {
        // 本地已知的任务修订号。
        const currentRevision = currentRevisions.get(task.id);
        return currentRevision !== undefined && task.state_revision < currentRevision;
      })) return false;

      tasks.value = loaded;
      lastAppliedLoadRequest = requestId;
      return true;
    } catch {
      // IPC/引擎瞬时失败不等于队列为空；保留最后一份成功快照。
      return false;
    }
  }

  /**
   * 执行后端命令后重载队列（队列靠 transfer_update 重载，主页靠 sync_state 更新）。
   */
  async function runAndReload(action: () => Promise<unknown>): Promise<void> {
    await action();
    await loadAll();
  }

  /**
   * 清除已完成
   */
  async function clearCompleted(): Promise<void> {
    await runAndReload(commands.transferClearCompleted);
  }

  /**
   * 清除失败项
   */
  async function clearFailed(): Promise<void> {
    await runAndReload(commands.transferClearFailed);
  }

  /**
   * 清除已完成+失败
   */
  async function clearFinished(): Promise<void> {
    await runAndReload(commands.transferClearFinished);
  }

  /**
   * 后端非阻塞接受重试；队列靠 transfer_update 重载，主页靠 sync_state 更新。
   *
   * @param taskId - 传输任务 ID
   */
  async function retry(taskId: number): Promise<void> {
    await runAndReload(() => commands.transferRetry(taskId));
  }

  /**
   * 同名冲突决策：覆盖远端（远端旧版自动保留为云端副本，不丢数据）。
   *
   * @param taskId - 传输任务 ID
   */
  async function overwriteRemote(taskId: number): Promise<void> {
    await runAndReload(() => commands.transferOverwriteRemote(taskId));
  }

  /**
   * 同名冲突决策：保留两者（本地文件改名后作为新文件上传，云端原文件不动）。
   *
   * @param taskId - 传输任务 ID
   */
  async function keepBoth(taskId: number): Promise<void> {
    await runAndReload(() => commands.transferKeepBoth(taskId));
  }

  /**
   * 取消「需要重新检查」或失败的任务。
   *
   * @param taskId - 传输任务 ID
   */
  async function cancel(taskId: number): Promise<void> {
    await runAndReload(() => commands.transferCancel(taskId));
  }

  return {
    tasks, uploads, downloads,
    running, pending, waitingNetwork, backingOff, verifyingRemote, restartRequired,
    completed, failed, canceled, processing, waiting, active, hasActiveTasks,
    loadAll, clearCompleted, clearFailed, clearFinished, retry,
    overwriteRemote, keepBoth, cancel,
  };
});
