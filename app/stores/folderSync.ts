/**
 * 目录同步作业 Store —— 「双端对齐」的排队与进度。
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as folderSyncApi from "@/api/folderSync";
import type { FolderSyncJob } from "@/api/folderSync";
import { FOLDER_SYNC_JOB_STATE, isActiveFolderSyncJob } from "@/api/folderSync";

// 最近终态作业在弹层里的展示条数（失败优先，其次最近完成）。
const TERMINAL_JOB_DISPLAY_LIMIT = 3;

// 目录同步作业 Store。
export const useFolderSyncStore = defineStore("folderSync", () => {
  // 全部作业（后端已按创建时间倒序返回）
  const jobs = ref<FolderSyncJob[]>([]);

  // 排队中 / 规划中的作业
  const activeJobs = computed(() => jobs.value.filter(isActiveFolderSyncJob));
  // 需要用户注意的终态作业：失败优先，其次最近完成的
  const recentTerminalJobs = computed(() => {
    // 失败作业排在前面，便于用户第一眼看到问题。
    const failed = jobs.value.filter((job) => job.state === FOLDER_SYNC_JOB_STATE.FAILED);
    // 最近完成的作业。
    const completed = jobs.value.filter((job) => job.state === FOLDER_SYNC_JOB_STATE.COMPLETED);
    return [...failed, ...completed].slice(0, TERMINAL_JOB_DISPLAY_LIMIT);
  });
  // 弹层需要展示的作业行：活跃作业全量 + 有限条终态
  const visibleJobs = computed(() => [...activeJobs.value, ...recentTerminalJobs.value]);
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
   * 取消一个排队中或规划中的作业
   *
   * @param jobId - 作业 ID
   * @returns 后端是否真的取消了作业
   */
  async function cancel(jobId: number): Promise<boolean> {
    const cancelled = await folderSyncApi.cancelFolderSyncJob(jobId);
    await loadAll();
    return cancelled;
  }

  return { jobs, activeJobs, visibleJobs, hasActiveJobs, loadAll, enqueue, cancel };
});
