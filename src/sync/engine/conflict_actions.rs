//! 同名冲突的用户决策操作：覆盖远端（旧版保留为云端副本）、保留两者、取消任务。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::SyncEngine;
use crate::error::{AppError, AppResult};
use crate::sync::conflict::{dedupe_cloud_copy_name, dedupe_copy_path};
use crate::sync::executor::verify_source_snapshot;

impl SyncEngine {
    /// 覆盖远端：把目标目录下的同名远端文件改名为云端副本（旧版不丢失），
    /// 再重放上传任务。改名成功后任务迁移回 Pending 并立即在后台执行。
    pub async fn overwrite_remote_conflict(self: &Arc<Self>, task_id: i64) -> AppResult<()> {
        let activity = self.begin_external_activity()?;
        let task_runner = self.task_runner()?;
        let task = task_runner.load_name_conflict_task(task_id)?;
        let local_path = task
            .local_path
            .as_deref()
            .ok_or_else(|| AppError::generic("任务缺少本地路径"))?;
        // 用户承诺上传的是任务快照内容；源已变化时禁止覆盖，交回重新检查。
        verify_source_snapshot(&task, &PathBuf::from(local_path))?;

        let siblings = self
            .files_api
            .list_all(task.parent_file_id.as_deref())
            .await?;
        let existing_names: HashSet<String> =
            siblings.iter().map(|file| file.name.clone()).collect();
        let stamp = chrono::Utc::now();
        // 同名文件可能不止一个（华为允许重名），全部改名为云端副本让位。
        let conflicts: Vec<_> = siblings
            .into_iter()
            .filter(|file| file.name == task.name)
            .collect();
        for file in &conflicts {
            let copy_name = dedupe_cloud_copy_name(&file.name, &existing_names, &stamp);
            self.files_api.rename_file(&file.id, &copy_name).await?;
            tracing::info!(
                task_id,
                file_id = %file.id,
                copy_name = %copy_name,
                "同名远端文件已改名为云端副本，准备覆盖上传"
            );
        }

        let pending = task_runner.requeue_name_conflict_task(&task)?;
        self.run_prepared_in_background(activity, task_runner, pending.id);
        Ok(())
    }

    /// 保留两者：本地文件改名为本地副本（云端原文件不动），取消当前任务；
    /// 改名后的本地副本会被下一轮扫描当作新文件自然上传。
    pub async fn keep_both_conflict(self: &Arc<Self>, task_id: i64) -> AppResult<()> {
        let task_runner = self.task_runner()?;
        let task = task_runner.load_name_conflict_task(task_id)?;
        let local_path = PathBuf::from(
            task.local_path
                .as_deref()
                .ok_or_else(|| AppError::generic("任务缺少本地路径"))?,
        );
        // 改名会转移用户文件，必须确认当前内容就是任务快照内容。
        verify_source_snapshot(&task, &local_path)?;

        let copy_path = dedupe_copy_path(&local_path, "本地副本", &chrono::Utc::now());
        std::fs::rename(&local_path, &copy_path)
            .map_err(|error| AppError::generic(format!("本地文件改名失败：{error}")))?;
        // 残留的旧 fileId xattr 会让副本被误判成原远端文件的配对，必须清除。
        if crate::platform::xattr::get(&copy_path, crate::mount::manager::XATTR_FILE_ID)
            .ok()
            .flatten()
            .is_some()
        {
            if let Err(error) =
                crate::platform::xattr::remove(&copy_path, crate::mount::manager::XATTR_FILE_ID)
            {
                tracing::warn!(task_id, %error, "清除本地副本的旧 fileId xattr 失败");
            }
        }
        let copy_name = copy_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "本地副本".to_string());
        tracing::info!(task_id, copy_name = %copy_name, "本地文件已改名保留，原任务取消");
        task_runner.cancel_user_task(
            task_id,
            &format!(
                "已保留两者：本地文件改名为「{copy_name}」后将作为新文件上传，云端原文件未改动"
            ),
        )?;
        // 触发重扫描让改名后的副本尽快入队上传。
        self.request_cycle_background("keep-both-rescan");
        Ok(())
    }

    /// 用户显式取消任务（仅「需要重新检查」/失败状态）。
    pub async fn cancel_transfer_task(&self, task_id: i64) -> AppResult<()> {
        let task_runner = self.task_runner()?;
        task_runner.cancel_user_task(task_id, "用户已取消该任务")?;
        Ok(())
    }
}
