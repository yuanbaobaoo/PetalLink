//! 同步重试入口。
//!
//! 用户重试一律「先入队、后执行」：命令路径只做静态校验与行级 CAS，
//! 稳定性窗口等耗时校验放到后台执行链，避免 UI 因为一条任务在跑而互相阻塞。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::data::repository;
use crate::error::AppResult;
use crate::sync::task_runner::{RetryAllSummary, RetryEnqueued, TaskDisposition, TaskRunner};
use crate::sync::transfer_state::TransferState;

use super::SyncEngine;

impl SyncEngine {
    /// 重试全部失败任务：全部入队后立即返回汇总，不等待任何任务执行。
    pub async fn retry_all_failed(self: &Arc<Self>) -> AppResult<RetryAllSummary> {
        let task_runner = self.task_runner()?;
        let plan = task_runner.enqueue_all_failed().await?;
        // 逐条后台执行：每个任务各自持有活动许可，并发由共享执行槽约束。
        for task_id in plan.queued_task_ids {
            self.spawn_background_retry(task_id);
        }
        Ok(plan.summary)
    }

    /// 检查任务是否需要重新规划后再重试。
    fn request_retry_replan_if_restart_required(self: &Arc<Self>, task_id: i64) -> bool {
        let restart_required = match repository::get_transfer_by_id(&self.db.lock(), task_id) {
            Ok(Some(task)) => task.state_kind() == Ok(TransferState::RestartRequired),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(task_id, %error, "检查重试任务是否需要重规划失败");
                false
            }
        };
        if restart_required {
            self.request_cycle_background("retry-replan");
        }
        restart_required
    }

    /// 为单个已入队任务取得活动许可并在后台执行。
    ///
    /// 目录同步投递、按需下载入队与核验重放共用此入口；`task_id` 对应的任务行必须
    /// 已经落库为可执行状态（Pending 等），这里只负责调度。
    pub(crate) fn spawn_background_retry(self: &Arc<Self>, task_id: i64) {
        let activity = match self.begin_external_activity() {
            Ok(activity) => activity,
            Err(error) => {
                // 引擎正在关闭时不再启动新执行；任务保持 Pending，由下次启动恢复接管。
                tracing::warn!(task_id, %error, "同步引擎已停止，暂不启动后台重试");
                return;
            }
        };
        let task_runner = match self.task_runner() {
            Ok(task_runner) => task_runner,
            Err(error) => {
                tracing::warn!(task_id, %error, "任务执行器未就绪，暂不启动后台重试");
                return;
            }
        };
        self.run_prepared_in_background(activity, task_runner, task_id);
    }

    /// 后台执行已迁移为 Pending 的任务，并按结果更新云树缓存或触发重规划。
    /// 手动重试与冲突决策重放共用此执行链。
    pub(super) fn run_prepared_in_background(
        self: &Arc<Self>,
        activity: super::ActivityGuard,
        task_runner: Arc<TaskRunner>,
        task_id: i64,
    ) {
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let _activity = activity;
            // 后端前置校验（稳定性窗口）在后台跑完再进入传输。
            match task_runner.run_prepared(task_id, true).await {
                Ok(outcome) => {
                    if outcome.disposition == TaskDisposition::RestartRequired {
                        engine.request_retry_replan_if_restart_required(task_id);
                    } else if let Some(cloud_file) = outcome.cloud_file {
                        let relative_path = {
                            let conn = engine.db.lock();
                            repository::get_transfer_by_id(&conn, task_id)
                                .ok()
                                .flatten()
                                .and_then(|task| task.relative_path)
                        };
                        if let Some(relative_path) = relative_path {
                            engine.cloud_entry_insert(relative_path.clone(), cloud_file.clone());
                        }
                    }
                }
                Err(error) => {
                    if !engine.request_retry_replan_if_restart_required(task_id) {
                        tracing::warn!(task_id, %error, "后台重试任务失败");
                    }
                }
            }
            engine.notify_backoff_schedule_changed();
        });
    }

    /// 在同一执行入口下启动已入队任务（语义化别名，供 hydrate/目录同步调用）。
    pub(crate) fn spawn_background_transfer(self: &Arc<Self>, task_id: i64) {
        self.spawn_background_retry(task_id);
    }

    /// 用户确认执行被拦下的大批量云端删除。
    ///
    /// 只是一次性令牌：下一次周期消费后即失效；不确认则删除永久停留在被拦下状态。
    pub fn approve_mass_delete(&self) {
        self.mass_delete_approved.store(true, Ordering::Release);
        tracing::warn!("用户已确认执行大批量云端删除");
    }

    /// 幂等入队单个任务：命令在百毫秒内返回，执行交给后台调度。
    pub async fn retry_transfer(self: &Arc<Self>, task_id: i64) -> AppResult<RetryEnqueued> {
        let task_runner = self.task_runner()?;
        let enqueued = task_runner.enqueue_retry(task_id).await?;
        // 只有本次真正推进到 Pending 才需要启动执行；已在队列中的任务不重复调度。
        if enqueued.is_newly_queued() {
            self.spawn_background_retry(enqueued.task_id());
        }
        Ok(enqueued)
    }
}
