//! 同步重试入口。

use std::sync::Arc;

use crate::data::repository;
use crate::error::AppResult;
use crate::sync::task_runner::TaskDisposition;
use crate::sync::transfer_state::TransferState;

use super::SyncEngine;

impl SyncEngine {
    /// 重试全部失败任务。
    pub async fn retry_failed(&self) -> AppResult<()> {
        self.run_sync_cycle("retry-failed").await
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

    /// 通过自动同步与启动恢复共用的 TaskRunner 重试单个持久化传输任务。
    pub async fn retry_transfer(self: &Arc<Self>, task_id: i64) -> AppResult<()> {
        let activity = self.begin_external_activity()?;
        let task_runner = self.task_runner()?;
        let pending = match task_runner.prepare_retry(task_id).await {
            Ok(pending) => pending,
            Err(error) => {
                if self.request_retry_replan_if_restart_required(task_id) {
                    return Ok(());
                }
                return Err(error);
            }
        };
        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let _activity = activity;
            match task_runner.run_prepared(pending.id).await {
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
                            engine.cloud_tree_insert(relative_path.clone(), cloud_file.clone());
                            engine.path_to_id_insert(relative_path, cloud_file.id.clone());
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
        Ok(())
    }
}
