/**
 * FolderSync API —— 目录「双端对齐」作业。
 *
 * 一次点击只产生一条作业：后端入队立即返回，遍历与投递在后台 worker 完成，
 * 文件传输本身进入统一的传输队列。
 */
import { commands, FOLDER_SYNC_JOB_STATE } from "./generated";
import type { FolderSyncJob as GeneratedFolderSyncJob } from "./generated";
export { FOLDER_SYNC_JOB_STATE } from "./generated";

/**
 * 作业状态取值，与后端 `job_state` 模块同源。
 */
export type FolderSyncJobState = (typeof FOLDER_SYNC_JOB_STATE)[keyof typeof FOLDER_SYNC_JOB_STATE];

/**
 * 目录同步作业实体。
 */
export type FolderSyncJob = GeneratedFolderSyncJob;

/**
 * 入队一次目录双端对齐；同一目录重复点击会复用活跃作业。
 *
 * @param folderId - 云端目录 ID
 * @param relPath - 挂载根内的相对路径
 */
export async function enqueueFolderSync(folderId: string, relPath: string): Promise<FolderSyncJob> {
  return await commands.syncFolderEnqueue(folderId, relPath);
}

/**
 * 列出最近的目录同步作业（含活跃与终态，按创建时间倒序）。
 */
export async function listFolderSyncJobs(): Promise<FolderSyncJob[]> {
  return await commands.syncFolderJobs();
}

/**
 * 取消一个排队中或规划中的作业。
 *
 * @param jobId - 作业 ID
 */
export async function cancelFolderSyncJob(jobId: number): Promise<boolean> {
  return await commands.syncFolderCancel(jobId);
}

/**
 * 清除已结束的作业，与传输历史的清除共用同一套开关。
 *
 * 作业只是「投递」这一步，清除不会影响已经在传输队列里的任务。
 *
 * @param includeCompleted - 是否清除已完成
 * @param includeFailed - 是否清除失败与已取消
 * @returns 实际删除的作业条数
 */
export async function clearFolderSyncJobs(
  includeCompleted: boolean,
  includeFailed: boolean,
): Promise<number> {
  return await commands.syncFolderClear(includeCompleted, includeFailed);
}

/**
 * 判断作业是否仍在排队或规划中。
 *
 * @param job - 作业实体
 */
export function isActiveFolderSyncJob(job: FolderSyncJob): boolean {
  return job.state === FOLDER_SYNC_JOB_STATE.QUEUED
    || job.state === FOLDER_SYNC_JOB_STATE.PLANNING;
}
