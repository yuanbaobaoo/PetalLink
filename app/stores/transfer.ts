/**
 * 传输队列 Store —— 
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as transferApi from "@/api/transfer";
import type { TransferTask } from "@/api/transfer";
import { TRANSFER_DIR, TRANSFER_STATE } from "@/api/transfer";

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
  // 进行中
  const running = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.RUNNING).length);
  // 等待调度
  const pending = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.PENDING).length);
  // 等待网络恢复
  const waitingNetwork = computed(
    () => tasks.value.filter((t) => t.state === TRANSFER_STATE.WAITING_FOR_NETWORK).length,
  );
  // 等待退避截止时间
  const backingOff = computed(
    () => tasks.value.filter((t) => t.state === TRANSFER_STATE.BACKING_OFF).length,
  );
  // 正在核验有歧义的远端结果
  const verifyingRemote = computed(
    () => tasks.value.filter((t) => t.state === TRANSFER_STATE.VERIFYING_REMOTE).length,
  );
  // 原任务不能原样重试，等待同步引擎重新规划
  const restartRequired = computed(
    () => tasks.value.filter((t) => t.state === TRANSFER_STATE.RESTART_REQUIRED).length,
  );
  // 已完成
  const completed = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.COMPLETED).length);
  // 永久失败历史
  const failed = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.FAILED).length);
  // 已取消
  const canceled = computed(() => tasks.value.filter((t) => t.state === TRANSFER_STATE.CANCELED).length);
  // 真正执行中的状态（传输或远端核验）
  const processing = computed(() => running.value + verifyingRemote.value);
  // 尚未执行完成、但当前在等待条件的状态
  const waiting = computed(() => pending.value + waitingNetwork.value + backingOff.value + restartRequired.value);
  // 所有非终态任务；不能把等待/退避/核验/重新规划误判成完成
  const active = computed(() => processing.value + waiting.value);
  const hasActiveTasks = computed(() => active.value > 0);

  /**
   * 加载全部传输任务
   *
   * @returns 是否成功应用（乱序/IPC 失败返回 false，保留旧快照）
   */
  async function loadAll(): Promise<boolean> {
    const requestId = ++nextLoadRequest;
    try {
      const loaded = await transferApi.listAllTransfers();
      if (requestId < lastAppliedLoadRequest) return false;

      // 即使两个 invoke 的响应乱序，也不能让同一 task 的旧 state_revision 回写。
      const currentRevisions = new Map(
        tasks.value.map((task) => [task.id, task.state_revision]),
      );
      if (loaded.some((task) => {
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
   * 清除已完成
   */
  async function clearCompleted(): Promise<void> {
    await transferApi.clearCompleted();
    await loadAll();
  }

  /**
   * 清除失败项
   */
  async function clearFailed(): Promise<void> {
    await transferApi.clearFailed();
    await loadAll();
  }

  /**
   * 清除已完成+失败
   */
  async function clearFinished(): Promise<void> {
    await transferApi.clearFinished();
    await loadAll();
  }

  /**
   * 后端非阻塞接受重试；队列靠 transfer_update 重载，主页靠 sync_state 更新。
   *
   * @param taskId - 传输任务 ID
   */
  async function retry(taskId: number): Promise<void> {
    await transferApi.retryTransfer(taskId);
    await loadAll();
  }

  return {
    tasks, uploads, downloads,
    running, pending, waitingNetwork, backingOff, verifyingRemote, restartRequired,
    completed, failed, canceled, processing, waiting, active, hasActiveTasks,
    loadAll, clearCompleted, clearFailed, clearFinished, retry,
  };
});
