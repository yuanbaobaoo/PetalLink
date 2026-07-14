//! 同步周期请求合并、唯一 owner drain 与单周期编排。
//!
//! sticky 请求仅由明确调用 restore 的门控或重排路径保留；路径争议则拒绝执行。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::planner::SyncSnapshot;
use crate::sync::transfer_state::TransferState;

use super::action_filters::{
    add_rescue_folder_recreations, dedupe_directory_deletes, dedupe_local_descendants,
    fill_parent_file_ids, filter_active_transfer_actions, filter_anti_oscillation,
    filter_blocked_path_changes, filter_skipped_paths, preserve_dirs_with_pending_backups,
};
use super::coordination::CycleRequest;
use super::{recoverable_cycle_retry_delay, ResetFlag, SyncEngine};

impl SyncEngine {
    /// 合并后台周期请求并安排 drain。
    pub(super) fn request_cycle_background(self: &Arc<Self>, triggered_by: &'static str) {
        self.cycle
            .request(Self::cycle_request_for_trigger(triggered_by));
        self.schedule_background_drain();
    }

    /// 安排唯一后台 owner drain；可恢复失败按退避重试。
    pub(super) fn schedule_background_drain(self: &Arc<Self>) {
        if self
            .background_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let engine = self.clone();
        tokio::spawn(async move {
            let mut recoverable_failures = 0_u32;
            loop {
                let (failed, retryable_failure) = match engine.drain_cycle_requests_for(None).await
                {
                    Ok(()) => (false, false),
                    Err(error) => {
                        tracing::warn!(%error, "后台协调周期失败");
                        (true, SyncEngine::is_recoverable_cycle_error(&error))
                    }
                };
                if retryable_failure && engine.cycle.has_pending() {
                    recoverable_failures = recoverable_failures.saturating_add(1);
                    let delay = recoverable_cycle_retry_delay(recoverable_failures);
                    let mut shutdown = engine.shutdown_tx.subscribe();
                    if *shutdown.borrow() {
                        engine.background_scheduled.store(false, Ordering::Release);
                        break;
                    }
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                engine.background_scheduled.store(false, Ordering::Release);
                                break;
                            }
                        }
                        _ = tokio::time::sleep(delay) => {}
                    }
                    if engine.is_online() && !*engine.shutdown.lock() {
                        continue;
                    }
                }
                if !failed {
                    recoverable_failures = 0;
                }
                engine.background_scheduled.store(false, Ordering::Release);
                // 失败序列恢复的请求不得热循环；调度位持有期间到达的新序列必须交接一次。
                let newer_request_after_failure = engine.cycle.has_uncompleted_request();
                let can_continue = (!failed || newer_request_after_failure)
                    && engine.started.load(Ordering::Acquire)
                    && engine.is_online()
                    && !*engine.folder_syncing.lock()
                    && !*engine.shutdown.lock()
                    && engine.cycle.has_pending();
                if !can_continue
                    || engine
                        .background_scheduled
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                {
                    break;
                }
            }
        });
    }

    /// 判断周期错误能否等待恢复后重试。
    pub(super) fn is_recoverable_cycle_error(error: &AppError) -> bool {
        match error {
            AppError::DriveApi {
                status_code,
                transport_kind,
                ..
            } => {
                transport_kind.is_some()
                    || status_code.is_some_and(|status| status == 429 || status >= 500)
            }
            _ => false,
        }
    }

    /// 提交同步周期请求，通常等待 owner drain；离线或目录同步门控恢复请求时返回已排队错误。
    pub async fn run_sync_cycle(&self, triggered_by: &str) -> AppResult<()> {
        if triggered_by != "startup-resume" && !self.started.load(Ordering::Acquire) {
            return Err(AppError::generic("同步引擎正在启动，请稍后重试"));
        }
        let sequence = self
            .cycle
            .request(Self::cycle_request_for_trigger(triggered_by));
        if triggered_by == "manual-refresh" {
            (self.cycle_observer)("request-manual");
        }
        self.drain_cycle_requests_for(Some(sequence)).await?;
        self.cycle
            .result_if_completed(sequence)
            .unwrap_or_else(|| Err(AppError::generic("同步请求已排队，等待恢复条件")))
    }

    /// 将触发来源映射为 sticky 周期意图。
    fn cycle_request_for_trigger(triggered_by: &str) -> CycleRequest {
        match triggered_by {
            "manual-refresh" => CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_FULL,
            "auto-cloud-refresh" => CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL,
            "network-recovery" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
            }
            "startup-resume" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
                    | CycleRequest::STARTUP
            }
            "retry-failed" => {
                CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL | CycleRequest::RETRY
            }
            "retry-replan" => {
                CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL | CycleRequest::REPLAN
            }
            "backoff-deadline" => {
                CycleRequest::LOCAL_RESCAN
                    | CycleRequest::CLOUD_INCREMENTAL
                    | CycleRequest::ONLINE_RECOVERY
            }
            _ => CycleRequest::LOCAL_RESCAN,
        }
    }

    /// 提交一组恢复结果；失败时强制下一轮重建本地与云端可信视图。
    fn commit_recovery_checkpoint(
        &self,
        recovered: &[crate::sync::task_runner::RecoveredCloudFile],
    ) -> AppResult<()> {
        if let Err(error) = self.commit_recovered_cloud_files(recovered) {
            self.cycle
                .request(CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_FULL);
            return Err(error);
        }
        Ok(())
    }

    /// 由唯一 owner 合并并排空待处理请求。
    async fn drain_cycle_requests_for(&self, awaited: Option<u64>) -> AppResult<()> {
        let _owner = self.cycle.lock_owner().await;
        if let Some(sequence) = awaited {
            if let Some(result) = self.cycle.result_if_completed(sequence) {
                return result;
            }
        }
        *self.syncing.lock() = true;
        let _syncing_reset = ResetFlag::new(&self.syncing);
        loop {
            let (request, cycle_sequence) = self.cycle.take_pending_with_sequence();
            if request.is_empty() {
                return Ok(());
            }
            if *self.shutdown.lock() {
                return Ok(());
            }
            if *self.folder_syncing.lock() {
                self.cycle.restore(request);
                return Ok(());
            }
            if !request.contains(CycleRequest::STARTUP) && !self.is_online() {
                self.cycle.restore(request);
                tracing::info!("网络离线，保留同步请求等待 level recovery");
                return Ok(());
            }
            if let Err(error) = self.run_coordinated_cycle(request).await {
                self.cycle.complete(cycle_sequence, Some(&error));
                self.restore_idle_runtime_after_error();
                return Err(error);
            }
            self.cycle.complete(cycle_sequence, None);
        }
    }

    /// 按请求意图执行单个协调周期；仅明确 restore 的门控或重排路径保留请求，路径争议则拒绝执行。
    async fn run_coordinated_cycle(&self, request: CycleRequest) -> AppResult<()> {
        let triggered_by = if request.contains(CycleRequest::STARTUP) {
            "startup-resume"
        } else if request.contains(CycleRequest::CLOUD_FULL) {
            "manual-refresh"
        } else if request.contains(CycleRequest::RETRY) {
            "retry-failed"
        } else if request.contains(CycleRequest::REPLAN) {
            "retry-replan"
        } else if request.contains(CycleRequest::ONLINE_RECOVERY) {
            "network-recovery"
        } else if request.contains(CycleRequest::CLOUD_INCREMENTAL) {
            "auto-cloud-refresh"
        } else {
            "local-watcher"
        };
        let result = async {
            let mut startup_needs_incremental = true;
            self.update_runtime_and_broadcast(|runtime| {
                runtime.is_running = true;
                if runtime.sync_phase.is_none() {
                    runtime.sync_phase = match triggered_by {
                        "local-watcher" => Some("syncing-local".to_string()),
                        "manual-refresh" => Some("syncing-manual".to_string()),
                        "retry-failed" | "retry-replan" => {
                            Some("syncing-retry".to_string())
                        }
                        "startup-resume" => Some("syncing-startup".to_string()),
                        _ => None, // 自动刷新阶段由上层设置
                    };
                }
            })?;

            if request.contains(CycleRequest::STARTUP) {
                {
                    let _activity = self.begin_external_activity()?;
                    let conn = self.db.lock();
                    let _ = repository::reset_stale_statuses(&conn);
                }
                self.ensure_cycle_active()?;
            }

            // 启动先安装完整基线，Changes catch-up 完成前保持不可信并禁止恢复传输。
            if request.contains(CycleRequest::STARTUP)
                && request.contains(CycleRequest::CLOUD_INCREMENTAL)
            {
                let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
                match self.load_or_refresh_cloud_tree(&mount_dir).await {
                    Ok(loaded_from_cache) => {
                        startup_needs_incremental = loaded_from_cache;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "启动 owner 无法建立可信云端 checkpoint，禁止进入 planner");
                        self.cycle.restore(request);
                        return Err(error);
                    }
                }
                self.ensure_cycle_active()?;
            }
            self.ensure_cycle_active()?;
            if request.contains(CycleRequest::CLOUD_FULL) {
                (self.cycle_observer)("cloud-refresh");
                if let Err(error) = self.refresh_cloud_full_for_cycle().await {
                    self.cycle.restore(request);
                    return Err(error);
                }
            } else if request.contains(CycleRequest::CLOUD_INCREMENTAL)
                && (!request.contains(CycleRequest::STARTUP) || startup_needs_incremental)
            {
                if !self.is_online() {
                    self.cycle.restore(request);
                    if request.contains(CycleRequest::STARTUP) {
                        return Err(AppError::generic(
                            "启动云端追平等待网络恢复",
                        ));
                    }
                    return Ok(());
                } else {
                    (self.cycle_observer)("cloud-refresh");
                    if let Err(error) = self.refresh_cloud_incremental_for_cycle().await {
                        tracing::warn!(%error, "云端刷新失败，完整保留当前周期意图等待补跑");
                        self.cycle.restore(request);
                        return Err(error);
                    }
                }
            }

            // 过期或失败的 checkpoint 仅供展示；不可信时禁止规划，避免覆盖较新的远端版本。
            if !self.cloud_tree_is_trusted() {
                self.cycle.restore(request);
                tracing::warn!("云端 checkpoint 尚未追平，跳过任务恢复与同步规划");
                if request.contains(CycleRequest::STARTUP) {
                    return Err(AppError::generic(
                        "启动云端 checkpoint 尚未追平，等待恢复",
                    ));
                }
                return Ok(());
            }

            // 保存了远端结果 ID 的 RestartRequired 仍有重复写风险，必须先恢复为核验态。
            if let Some(task_runner) = &self.task_runner {
                let promoted = task_runner.promote_ambiguous_restarts()?;
                if promoted > 0 {
                    tracing::warn!(promoted, "已将含远端结果的重规划任务恢复为核验态");
                    self.notify_backoff_schedule_changed();
                }
            }

            // 本地扫描前先基于可信 fileId/path 收敛已提交的远端改名或移动。
            let path_recovery = {
                let _activity = self.begin_external_activity()?;
                let mount_dir = self
                    .mount_dir
                    .lock()
                    .clone()
                    .ok_or_else(|| AppError::generic("同步挂载尚未初始化，无法恢复路径变更"))?;
                let mount_root = std::path::PathBuf::from(crate::core::paths::expand_tilde(
                    &mount_dir,
                ));
                let cloud = self.cloud_tree.lock().clone();
                let conn = self.db.lock();
                crate::sync::path_recovery::recover_verified_remote_path_changes(
                    &mount_root,
                    &conn,
                    &cloud,
                    |old_path, new_path| {
                        let old_lease = self.begin_exclusive_path_activity(old_path)?;
                        let new_lease = self.begin_exclusive_path_activity(new_path)?;
                        Ok((old_lease, new_lease))
                    },
                )
            };
            let blocked_path_changes = match path_recovery {
                Ok(summary) => {
                    if summary.rekeyed_roots > 0 {
                        tracing::info!(
                            recovered = summary.rekeyed_roots,
                            "已在同步规划前收敛中断的远端路径变更"
                        );
                    }
                    if !summary.blocked_changes.is_empty() {
                        tracing::warn!(
                            blocked = summary.blocked_changes.len(),
                            "活动传输尚未结算，争议路径将在本轮同步中隔离"
                        );
                    }
                    summary.blocked_changes
                }
                Err(error) => {
                    // 只有数据库读取等全局错误会到达这里；保留请求等待外部条件恢复。
                    self.cycle.restore(request);
                    tracing::error!(%error, "远端路径变更恢复发生全局错误，本轮不进入同步规划");
                    return Err(error);
                }
            };
            self.ensure_cycle_active()?;
            self.purge_deleted_tombstones_if_trusted(&blocked_path_changes)?;

            let mut completed_recoveries = 0usize;
            if request.contains(CycleRequest::STARTUP) {
                self.ensure_cycle_active()?;
                let summary = self.recover_interrupted_transfers().await;
                completed_recoveries += summary.completed;
                self.commit_recovery_checkpoint(&summary.recovered_cloud_files)?;
            }

            // 路径恢复先隔离活动任务，再核验远端写入。核验不能提前到这里之前，
            // 否则 Changes 尚未可见时，新基线可能被旧云树反向重键。
            if request.contains(CycleRequest::ONLINE_RECOVERY) {
                if !self.is_online() {
                    self.cycle.restore(request);
                    return Ok(());
                }
                self.ensure_cycle_active()?;
                if let Some(task_runner) = &self.task_runner {
                    (self.cycle_observer)("verify-remote");
                    let verifying = task_runner.resume_verifying().await?;
                    completed_recoveries += verifying.completed;
                    self.commit_recovery_checkpoint(&verifying.recovered_cloud_files)?;
                    self.ensure_cycle_active()?;
                    (self.cycle_observer)("resume-waiting");
                    let waiting = task_runner.resume_waiting().await?;
                    completed_recoveries += waiting.completed;
                    self.commit_recovery_checkpoint(&waiting.recovered_cloud_files)?;
                    self.ensure_cycle_active()?;
                    (self.cycle_observer)("resume-due");
                    let backing_off = task_runner.resume_due_backoff().await?;
                    completed_recoveries += backing_off.completed;
                    self.commit_recovery_checkpoint(&backing_off.recovered_cloud_files)?;
                }
            }
            if completed_recoveries > 0 {
                self.cycle
                    .request(CycleRequest::LOCAL_RESCAN | CycleRequest::CLOUD_INCREMENTAL);
            }

            // 全局重试仅在云端视图与恢复预检完成后接受；REPLAN 不清理无关失败项。
            if request.contains(CycleRequest::RETRY) {
                let _activity = self.begin_external_activity()?;
                if let Some(task_runner) = &self.task_runner {
                    let failed_task_ids = {
                        let conn = self.db.lock();
                        repository::list_all_transfers(&conn)?
                            .into_iter()
                            .filter(|task| task.state_kind() == Ok(TransferState::Failed))
                            .map(|task| task.id)
                            .collect::<Vec<_>>()
                    };
                    for task_id in failed_task_ids {
                        match task_runner.prepare_retry(task_id).await {
                            Ok(prepared) => {
                                if let Err(error) = task_runner.run_prepared(prepared.id).await {
                                    tracing::warn!(task_id, %error, "全局重试执行失败，状态已由任务机保留");
                                }
                            }
                            Err(error) => {
                                tracing::warn!(task_id, %error, "失败任务未通过重试前置校验");
                            }
                        }
                        self.ensure_cycle_active()?;
                    }
                }
                {
                    let conn = self.db.lock();
                    conn.execute(
                        "UPDATE sync_items
                         SET status=?1, error_message=NULL
                         WHERE status=?2
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue AS task
                               WHERE task.relative_path=sync_items.local_path AND task.state=?3
                           )",
                        rusqlite::params![
                            repository::sync_status::SYNCING,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Failed),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("接受失败项重试失败：{error}")))?;
                }
                self.recompute_and_broadcast_state()?;
            }
            self.ensure_cycle_active()?;
            (self.cycle_observer)("local-rescan");
            self.run_sync_cycle_inner(triggered_by, &blocked_path_changes)
                .await
        }
        .await;
        let needs_idle_restore = {
            let state = self.state.lock();
            state.is_running || state.is_indexing || state.sync_phase.is_some()
        };
        if needs_idle_restore {
            self.restore_idle_runtime_after_error();
        }
        result
    }

    /// 在副作用前确认引擎尚未被替换或关闭。
    pub(crate) fn ensure_cycle_active(&self) -> AppResult<()> {
        if *self.shutdown.lock() {
            Err(AppError::generic("同步引擎已停止，拒绝开始新副作用"))
        } else {
            Ok(())
        }
    }

    /// 在可信快照和活动门保护下完成规划、执行与结算。
    async fn run_sync_cycle_inner(
        &self,
        triggered_by: &str,
        blocked_path_changes: &[crate::sync::path_recovery::BlockedPathChange],
    ) -> AppResult<()> {
        let local = self.scan_local().await?;
        (self.cycle_observer)("local-scan-complete");
        self.ensure_cycle_active()?;
        let planning_activity = self.begin_external_activity()?;
        let cloud = self.cloud_tree.lock().clone();
        let mut db = self.load_db_snapshot()?;

        // 统计本地、云端与数据库差异。
        let local_in_cloud_not_db: Vec<&str> = local
            .keys()
            .filter(|k| cloud.contains_key(*k) && !db.contains_key(*k))
            .map(|s| s.as_str())
            .collect();
        let in_cloud_db_not_local: Vec<&str> = cloud
            .keys()
            .filter(|k| db.contains_key(*k) && !local.contains_key(*k))
            .map(|s| s.as_str())
            .collect();
        if !local_in_cloud_not_db.is_empty() {
            tracing::debug!(count = local_in_cloud_not_db.len(), paths = ?local_in_cloud_not_db, "本地+云端有但DB无（reconcile 将补）");
        }
        if !in_cloud_db_not_local.is_empty() {
            tracing::info!(count = in_cloud_db_not_local.len(), paths = ?in_cloud_db_not_local, "云端+DB有但本地无（应生成 DeleteFromCloud）");
        }

        let cloud_tree_trusted = self.cloud_tree_is_trusted();

        // 只有可信云端快照才能制造成功基线。
        if cloud_tree_trusted {
            self.reconcile_db_records(&local, &db, blocked_path_changes)?;
            let reconciliation = self.reconcile_failed_and_purge_stale_records(
                &local,
                &cloud,
                blocked_path_changes,
            )?;
            db = self.load_db_snapshot()?;
            tracing::info!(
                healed = reconciliation.healed,
                purged = reconciliation.purged,
                remaining_failed = reconciliation.remaining_failed,
                blocked = blocked_path_changes.len(),
                "可信同步周期已完成失败状态复核与残余清理"
            );
        } else {
            tracing::warn!("云端树不可信，跳过 DB reconcile、失败复核与残余清理");
        }

        let db_len = db.len();
        let local_len = local.len();
        let cloud_len = cloud.len();
        let snapshot = SyncSnapshot {
            local: local.clone(),
            cloud: cloud.clone(),
            db,
            is_startup_resume: triggered_by == "startup-resume",
            cloud_tree_trusted,
        };
        let mut actions = self.planner.plan(&snapshot);
        filter_skipped_paths(&mut actions, &self.skip_patterns);
        // 用 xattr fileId 识别同目录改名，避免误判为上传加删除。
        if cloud_tree_trusted {
            self.detect_renames(&mut actions)?;
        }
        let transfer_tasks = repository::list_all_transfers(&self.db.lock())?;
        filter_active_transfer_actions(&mut actions, &snapshot.db, &transfer_tasks);
        filter_anti_oscillation(&mut actions, &self.recently_deleted_paths.lock());
        fill_parent_file_ids(&mut actions, &self.path_to_id.lock());
        // 为云端已删但仍需救援内容的路径补建目录链。
        add_rescue_folder_recreations(&mut actions, &snapshot, &self.recently_deleted_paths.lock());
        filter_blocked_path_changes(&mut actions, blocked_path_changes);

        // 实际复核本地路径，避免漏扫导致误删云端。
        self.validate_delete_from_cloud(&mut actions);
        // DeleteFromLocal 由 executor 在 unlink 前复核远端删除事实与本地版本。

        // 目录云端删除会级联子树，仅保留祖先动作以维持回收站层级。
        dedupe_directory_deletes(&mut actions, &self.cloud_tree.lock());

        // 本地目录删除同样仅保留祖先动作，避免并发重复删除。
        dedupe_local_descendants(&mut actions);

        // 子项需要备份时保留目录，确保备份副本有落点。
        preserve_dirs_with_pending_backups(&mut actions);

        // 无动作时清零计数并发布空闲状态。
        if actions.is_empty() {
            self.update_runtime_and_broadcast(|runtime| {
                runtime.editing = 0;
                runtime.content_changed = false;
                runtime.is_running = false;
                // 周期结束必须复位索引态，避免状态条卡住。
                runtime.is_indexing = false;
                runtime.sync_phase = None;
                runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
            })?;
            tracing::info!(
                triggered_by,
                local = local_len,
                cloud = cloud_len,
                db = db_len,
                "sync cycle: 无操作，短路返回"
            );
            return Ok(());
        }

        tracing::info!(
            triggered_by,
            actions = actions.len(),
            "sync cycle: 开始执行动作"
        );

        drop(planning_activity);
        self.ensure_cycle_active()?;
        let results = if let Some(ref exec) = self.executor {
            self.execute_actions_ordered(exec, &mut actions).await?
        } else {
            Vec::new()
        };
        // 已提交远端写可由 TaskRunner 结算；替换后的旧引擎不得继续修改数据库或缓存。
        self.ensure_cycle_active()?;
        let _apply_activity = self.begin_external_activity()?;

        // 远端路径写入必须先落入可信 checkpoint，再提交 DB 路径基线。进程若在两者之间退出，
        // 下次启动会按新 checkpoint 正向恢复 DB，而不会用旧 checkpoint 反向改名。
        let recovered_moves = actions
            .iter()
            .zip(results.iter())
            .filter(|(action, result)| {
                result.success
                    && action.action_type == crate::sync::state::SyncActionType::MoveInCloud
            })
            .filter_map(|(action, result)| {
                Some(crate::sync::task_runner::RecoveredCloudFile {
                    relative_path: action.relative_path.clone()?,
                    file: result.cloud_file.clone()?,
                })
            })
            .collect::<Vec<_>>();
        self.commit_recovery_checkpoint(&recovered_moves)?;

        // 结算数据库并发布云树与路径索引。
        self.apply_results(&actions, &results)?;
        if actions.iter().zip(results.iter()).any(|(action, result)| {
            result.success && action.action_type == crate::sync::state::SyncActionType::MoveInCloud
        }) {
            // 移动只结算结构事实并保留内容基线，立即重扫以版本校验方式上传并发编辑。
            self.cycle.request(CycleRequest::LOCAL_RESCAN);
        }

        // 仅成功的结构性动作标记内容变化。
        let content_changed = actions.iter().zip(results.iter()).any(|(a, r)| {
            r.success
                && matches!(
                    a.action_type,
                    crate::sync::state::SyncActionType::Upload
                        | crate::sync::state::SyncActionType::Download
                        | crate::sync::state::SyncActionType::DeleteFromCloud
                        | crate::sync::state::SyncActionType::DeleteFromLocal
                        | crate::sync::state::SyncActionType::CreateFolder
                        | crate::sync::state::SyncActionType::CreateConflictCopy
                        | crate::sync::state::SyncActionType::CreatePlaceholder
                        | crate::sync::state::SyncActionType::MoveInCloud
                        | crate::sync::state::SyncActionType::BackupBeforeCloudDelete
                )
        });
        // 发布周期状态。
        self.update_and_push_state(content_changed)?;

        tracing::info!(
            triggered_by,
            actions = actions.len(),
            content_changed,
            "sync cycle ok"
        );
        Ok(())
    }

    /// 触发手动全量刷新周期。
    pub async fn trigger_manual_sync(&self) -> AppResult<()> {
        let result = self.run_sync_cycle("manual-refresh").await;
        (self.cycle_observer)("manual-cycle-returned");
        if result.is_ok() {
            self.update_runtime_and_broadcast(|runtime| runtime.content_changed = true)?;
        }
        result
    }
}
