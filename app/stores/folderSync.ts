/**
 * 目录同步作业 Store —— 「双端对齐」的排队与进度。
 *
 * 作业是队列任务的一种：弹层把它和传输任务放在同一个列表里筛选、计数与清除，
 * 所以这里只维护完整的作业快照和队列操作，不再单独裁剪展示条数。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as folderSyncApi from "@/api/folderSync";
import type { FolderSyncJob } from "@/api/folderSync";
import { FOLDER_SYNC_JOB_STATE, isActiveFolderSyncJob } from "@/api/folderSync";

// 目录同步作业 Store。
export const useFolderSyncStore = defineStore("folderSync", () => {
  // 全部作业（后端已按创建时间倒序返回）
  const jobs = ref<FolderSyncJob[]>([]);

  // 排队中 / 规划中的作业
  const activeJobs = computed(() => jobs.value.filter(isActiveFolderSyncJob));
  // 失败作业：需要用户选择重新入队或取消，弹层据此决定是否展示失败汇总条
  const failedJobs = computed(
    () => jobs.value.filter((job) => job.state === FOLDER_SYNC_JOB_STATE.FAILED),
  );
  // 是否存在活跃作业
  const hasActiveJobs = computed(() => activeJobs.value.length > 0);

  /**
   * 加载作业列表
   *
   * @returns 是否成功应用（IPC 失败返回 false，保留上一次快照）
   */
  async function loadAll(): Promise<boolean> {
    try {
      jobs.value = await folderSyncApi.listFolderSyncJobs();
      return true;
    } catch {
      // 引擎尚未启动或瞬时失败都保留旧快照，不把列表清空成“没有作业”。
      return false;
    }
  }

  /**
   * 入队一次双端对齐
   *
   * @param folderId - 云端目录 ID
   * @param relPath - 挂载根内的相对路径
   * @returns 后端返回的作业快照
   */
  async function enqueue(folderId: string, relPath: string): Promise<FolderSyncJob> {
    // 后端只落库排队并立即返回，不会等待传输完成。
    const job = await folderSyncApi.enqueueFolderSync(folderId, relPath);
    void loadAll();
    return job;
  }

  /**
   * 取消一个作业：排队中 / 规划中立即收敛，失败的作业表示用户放弃这次失败
   *
   * @param jobId - 作业 ID
   * @returns 后端是否真的取消了作业
   */
  async function cancel(jobId: number): Promise<boolean> {
    const cancelled = await folderSyncApi.cancelFolderSyncJob(jobId);
    await loadAll();
    return cancelled;
  }

  /**
   * 清除已结束的作业（与传输历史的清除共用一套开关）
   *
   * @param includeCompleted - 是否清除已完成
   * @param includeFailed - 是否清除失败与已取消
   * @returns 后端实际删除的作业条数
   */
  async function clear(includeCompleted: boolean, includeFailed: boolean): Promise<number> {
    const removed = await folderSyncApi.clearFolderSyncJobs(includeCompleted, includeFailed);
    await loadAll();
    return removed;
  }

  /**
   * 批量重试失败作业：逐条重新入队，结束时只重载一次
   *
   * @returns 成功入队与未能入队的作业条数
   */
  async function retryFailed(): Promise<{ queued: number; failed: number }> {
    // 本次待重试的失败作业快照（重载后列表会变化）。
    const targets = failedJobs.value;
    // 成功重新入队的条数。
    let queued = 0;
    // 仍然失败（例如云端目录已被删除）的条数。
    let failed = 0;
    for (const job of targets) {
      try {
        await folderSyncApi.enqueueFolderSync(job.folder_id, job.rel_path);
        queued += 1;
      } catch {
        failed += 1;
      }
    }
    if (targets.length > 0) await loadAll();
    return { queued, failed };
  }

  /**
   * 批量取消失败作业（保留为已取消历史），结束时只重载一次
   *
   * @returns 实际取消的作业条数
   */
  async function cancelFailed(): Promise<number> {
    // 本次待取消的失败作业快照。
    const targets = failedJobs.value;
    // 取消成功的条数。
    let canceled = 0;
    for (const job of targets) {
      try {
        if (await folderSyncApi.cancelFolderSyncJob(job.id)) canceled += 1;
      } catch {
        // 单条取消失败不影响其它作业；下一次事件刷新会纠正状态。
      }
    }
    if (targets.length > 0) await loadAll();
    return canceled;
  }

  return {
    jobs,
    activeJobs,
    failedJobs,
    hasActiveJobs,
    loadAll,
    enqueue,
    cancel,
    clear,
    retryFailed,
    cancelFailed,
  };
});
