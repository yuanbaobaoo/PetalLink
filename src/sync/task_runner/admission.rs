//! 提供任务入队、重试准入与 Running 仲裁。

use std::sync::Arc;

use super::contracts::{
    EnqueuedTask, EnqueuedTaskOutcome, RetryAllPlan, RetryEnqueued, TaskActivityGate,
    TaskDisposition, TaskExecutionOutcome,
};
use super::persistence::{transition_error, update_compatibility_sync_status};
use super::preflight::PreflightFailure;
use super::TaskRunner;
use crate::data::repository::{self, ColumnPatch, TransferPatch, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferErrorKind, TransferOperation, TransferState};

/// 入队时对既有意图、重新规划与新增任务的仲裁结果。
pub(super) enum ExistingOrInsertedTask {
    Existing(Box<TransferTask>),
    Replanned(Box<TransferTask>),
    Blocked(i64),
    Inserted(i64),
}

/// 准入仲裁结果：可直接执行的任务行，或被同路径活动意图阻塞的任务 ID。
pub(super) enum Admitted {
    /// 已落库且可直接执行的任务行。
    Runnable(Box<TransferTask>),
    /// 被同路径活动意图阻塞；只返回已有任务 ID，不重复执行。
    Blocked(i64),
}

/// 任务尝试进入 Running 时的门禁结果。
pub(super) enum RunningGateOutcome {
    Running(Box<TransferTask>),
    Blocked,
}

/// 判断持久状态是否会阻止同路径新意图。
/// 与 `TransferState::occupies_path` 刻意不同：RestartRequired 的滞留行由
/// 重试/清除流程处理，不阻塞新意图的准入判定。
pub(super) fn is_path_blocking_state(state: TransferState) -> bool {
    matches!(
        state,
        TransferState::Pending
            | TransferState::Running
            | TransferState::WaitingForNetwork
            | TransferState::BackingOff
            | TransferState::VerifyingRemote
    )
}

/// 按操作、路径、远程身份与源快照判断两任务是否同一意图。
pub(super) fn same_transfer_intent(left: &TransferTask, right: &TransferTask) -> bool {
    if left.relative_path != right.relative_path
        || left.local_path != right.local_path
        || left.name != right.name
        || left.direction != right.direction
        || left.operation != right.operation
        || left.file_id != right.file_id
        || left.total_size != right.total_size
    {
        return false;
    }
    match left.operation_kind().ok().flatten() {
        Some(TransferOperation::Create | TransferOperation::Update) => {
            left.parent_file_id == right.parent_file_id
                && left.source_mtime == right.source_mtime
                && left.source_size == right.source_size
                && (left.operation_kind().ok().flatten() != Some(TransferOperation::Update)
                    || left.expected_cloud_edited_time == right.expected_cloud_edited_time)
        }
        Some(TransferOperation::Download | TransferOperation::DownloadUpdate) => {
            left.parent_file_id == right.parent_file_id
                && left.expected_cloud_edited_time == right.expected_cloud_edited_time
        }
        _ => false,
    }
}

/// 判断任务是否属于可重放的传输操作。
///
/// 与前端 `canRetryTransferTask` 保持同一集合：只有上传（Create/Update）与
/// 下载（Download/DownloadUpdate）有可重放的传输后端，结构动作没有。
pub(super) fn is_retryable_transfer_operation(task: &TransferTask) -> bool {
    matches!(
        task.operation_kind().ok().flatten(),
        Some(
            TransferOperation::Create
                | TransferOperation::Update
                | TransferOperation::Download
                | TransferOperation::DownloadUpdate
        )
    )
}

/// 判断写任务是否已持久可能提交的远程结果。
pub(super) fn has_ambiguous_remote_write_result(task: &TransferTask) -> bool {
    matches!(
        task.operation_kind().ok().flatten(),
        Some(TransferOperation::Create | TransferOperation::Update)
    ) && has_persisted_remote_result(task)
}

/// 判断任务是否保存了非空远程结果 ID。
pub(super) fn has_persisted_remote_result(task: &TransferTask) -> bool {
    task.remote_result_file_id
        .as_deref()
        .is_some_and(|file_id| !file_id.trim().is_empty())
}

impl TaskRunner {
    /// 设置任务活动准入门。
    pub fn set_activity_gate(&self, activity_gate: Arc<dyn TaskActivityGate>) {
        *self.activity_gate.write() = Some(activity_gate);
    }

    /// 获取任务活动许可。
    pub(super) fn begin_activity(&self, task: &TransferTask) -> AppResult<Option<Box<dyn Send>>> {
        self.activity_gate
            .read()
            .clone()
            .map(|gate| gate.begin(task.relative_path.as_deref()))
            .transpose()
    }

    /// 持久化 Pending 意图后执行对应任务行（自动同步周期入口，等待执行结果）。
    pub async fn enqueue_and_run(&self, task: TransferTask) -> AppResult<EnqueuedTaskOutcome> {
        match self.admit(&task)? {
            Admitted::Runnable(existing) => {
                let task_id = existing.id;
                let outcome = self.run_existing_or_observe(*existing).await?;
                Ok(EnqueuedTaskOutcome { task_id, outcome })
            }
            Admitted::Blocked(task_id) => Ok(EnqueuedTaskOutcome {
                task_id,
                outcome: TaskExecutionOutcome {
                    cloud_file: None,
                    disposition: TaskDisposition::BlockedByActiveIntent,
                },
            }),
        }
    }

    /// 只做准入与落库，把执行留给调用方在后台调度。
    ///
    /// 目录同步等批量入口用它把「投递任务」与「等待传输」解耦：投递是毫秒级的
    /// 数据库仲裁，传输进度由传输队列独立呈现。
    pub fn enqueue(&self, task: TransferTask) -> AppResult<EnqueuedTask> {
        match self.admit(&task)? {
            Admitted::Runnable(existing) => {
                let state = existing.state_kind().map_err(transition_error)?;
                Ok(EnqueuedTask {
                    task_id: existing.id,
                    disposition: TaskDisposition::from_active_state(state)
                        .unwrap_or(TaskDisposition::Pending),
                })
            }
            Admitted::Blocked(task_id) => Ok(EnqueuedTask {
                task_id,
                disposition: TaskDisposition::BlockedByActiveIntent,
            }),
        }
    }

    /// 校验新意图并完成同路径仲裁，返回可直接执行的任务行或被阻塞的任务 ID。
    fn admit(&self, task: &TransferTask) -> AppResult<Admitted> {
        // 新意图必须从未持久化的初始状态进入，防止调用方伪造任务修订。
        if task.id != 0
            || task.state_revision != 0
            || task.state_kind().map_err(transition_error)? != TransferState::Pending
        {
            self.notify_rejection();
            return Err(AppError::generic(
                "新传输意图必须是 id=0/revision=0 的 Pending 任务",
            ));
        }
        // 在持久化入队前完成准入，避免独占路径许可释放后立即执行的 Pending 行绕过限制。
        let _enqueue_activity = self.begin_activity(task)?;
        // 同一路径只允许一个阻塞意图，重复请求复用已有任务。
        let existing_or_task_id = {
            let conn = self.db.lock();
            // 同路径任务按索引点查：入队是高频路径，整表扫描会退化成 O(n²)（D30）。
            let path_tasks = match task.relative_path.as_deref() {
                Some(relative_path) => repository::list_by_relative_path(&conn, relative_path)?,
                None => Vec::new(),
            };
            let blocking = path_tasks
                .iter()
                .filter(|candidate| candidate.state_kind().is_ok_and(is_path_blocking_state))
                .collect::<Vec<_>>();
            // 运行中或远端核验中的写入具有最高优先级，不能被重新规划覆盖。
            if let Some(inflight) = blocking.iter().find(|candidate| {
                candidate.state_kind().is_ok_and(|state| {
                    matches!(
                        state,
                        TransferState::Running | TransferState::VerifyingRemote
                    )
                })
            }) {
                if same_transfer_intent(inflight, task) {
                    Ok(ExistingOrInsertedTask::Existing(Box::new(
                        (*inflight).clone(),
                    )))
                } else {
                    Ok(ExistingOrInsertedTask::Blocked(inflight.id))
                }
            } else if let Some(ambiguous_restart) = path_tasks.iter().find(|candidate| {
                candidate.state_kind() == Ok(TransferState::RestartRequired)
                    && has_ambiguous_remote_write_result(candidate)
            }) {
                self.promote_restart_to_verifying(&conn, ambiguous_restart)
                    .map(|task| ExistingOrInsertedTask::Existing(Box::new(task)))
            } else if let Some(existing) = blocking
                .iter()
                .find(|candidate| same_transfer_intent(candidate, task))
            {
                Ok(ExistingOrInsertedTask::Existing(Box::new(
                    (*existing).clone(),
                )))
            // 可恢复任务保留原 ID 并原子替换意图，便于审计历史连续。
            } else if let Some(replannable) = blocking.first() {
                self.replan_task(&conn, replannable, task)
                    .map(|task| ExistingOrInsertedTask::Replanned(Box::new(task)))
            } else if let Some(restart) = path_tasks
                .iter()
                .find(|candidate| candidate.state_kind() == Ok(TransferState::RestartRequired))
            {
                self.replan_task(&conn, restart, task)
                    .map(|task| ExistingOrInsertedTask::Replanned(Box::new(task)))
            } else if let Some(failed) = path_tasks
                .iter()
                .find(|candidate| candidate.state_kind() == Ok(TransferState::Failed))
            {
                // Failed 是自动任务的终态；保留任务行及可见错误作为路径屏障，显式重试复用该 ID。
                Ok(ExistingOrInsertedTask::Blocked(failed.id))
            } else {
                repository::insert_transfer(&conn, task).map(ExistingOrInsertedTask::Inserted)
            }
        };
        let existing_or_task_id = match existing_or_task_id {
            Ok(value) => value,
            Err(error) => {
                self.notify_rejection();
                return Err(error);
            }
        };
        // 先广播持久化结果，再按实际准入结果返回可执行行或阻塞态。
        self.notify_best_effort();
        match existing_or_task_id {
            ExistingOrInsertedTask::Inserted(task_id) => {
                Ok(Admitted::Runnable(Box::new(self.load(task_id)?)))
            }
            ExistingOrInsertedTask::Existing(existing)
            | ExistingOrInsertedTask::Replanned(existing) => Ok(Admitted::Runnable(existing)),
            ExistingOrInsertedTask::Blocked(task_id) => Ok(Admitted::Blocked(task_id)),
        }
    }

    /// 校验并接受失败/待重启任务的手动重试。
    ///
    /// RestartRequired 也直接接受并重跑：它的「等重规划」不能依赖 planner 重新产生意图——
    /// 同名碰撞场景下 planner 看到「本地有+云端有+无基线」只会 Skip，任务会永久滞留。
    /// 直接重跑让 execute 的同名碰撞检查基于远端实时状态收敛（采纳或进入 NameConflict）。
    ///
    /// 本入口保留同步后端前置校验，供自动恢复周期使用；用户点击走 `enqueue_retry`。
    pub async fn prepare_retry(&self, task_id: i64) -> AppResult<TransferTask> {
        let current = self.load(task_id)?;
        // 重试校验可能持久化拒绝结果，因此关闭准入必须先于校验并持续到 Pending 迁移完成。
        let _activity = self.begin_activity(&current)?;
        if !matches!(
            current.state_kind().map_err(transition_error)?,
            TransferState::Failed | TransferState::RestartRequired
        ) {
            self.notify_rejection();
            return Err(AppError::generic("任务不存在或当前状态不可重试"));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        if let Err(failure) = self.operations.preflight(&current).await {
            let failure = PreflightFailure::from(failure);
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        self.accept_retry_after_preflight(task_id, current.state_revision)
    }

    /// 把失败任务推进到 Pending（幂等），供用户重试入口使用。
    ///
    /// 只做静态校验与行级 CAS，**不做后端前置校验**：稳定性窗口（最长十余秒）
    /// 必须在后台执行链里跑，否则用户连续点击会互相阻塞。任务是否真的在跑，
    /// 一律以传输队列的权威状态为准。
    pub async fn enqueue_retry(&self, task_id: i64) -> AppResult<RetryEnqueued> {
        let current = self.load(task_id)?;
        let state = current.state_kind().map_err(transition_error)?;
        match state {
            // 已经在队列里：幂等返回，前端不必区分，也不重复执行。
            TransferState::Pending
            | TransferState::Running
            | TransferState::VerifyingRemote
            | TransferState::WaitingForNetwork
            | TransferState::BackingOff => {
                return Ok(RetryEnqueued::AlreadyQueued { task_id });
            }
            TransferState::Failed | TransferState::RestartRequired => {}
            _ => {
                self.notify_rejection();
                return Err(AppError::generic("任务已结束，不可重试"));
            }
        }
        // 静态校验只读本地元数据，微秒级；失败要持久化原因以便 UI 显示真实错误。
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        // 人工重试开启新一轮预算：accept_retry_after_preflight 内部会复校静态合同，
        // 并把 attempt_count / verify_attempt_count 归零。
        let pending = self.accept_retry_after_preflight(task_id, current.state_revision)?;
        Ok(RetryEnqueued::Queued {
            task_id: pending.id,
        })
    }

    /// 把全部失败项加入队列并立即返回汇总，不等待任何任务执行。
    pub async fn enqueue_all_failed(&self) -> AppResult<RetryAllPlan> {
        let mut plan = RetryAllPlan::default();
        let mut tasks =
            self.list_states(&[TransferState::Failed, TransferState::RestartRequired])?;
        // 按创建时间升序入队，保证队列顺序与用户认知的历史顺序一致。
        tasks.sort_by_key(|task| task.created_at);
        for task in tasks {
            // 同名冲突必须由用户选择处理方式（覆盖/保留两者），不能自动重放。
            let is_name_conflict = task
                .error_kind_typed()
                .ok()
                .flatten()
                .is_some_and(|kind| kind == TransferErrorKind::NameConflict);
            // 删除/改名/建目录等结构动作没有可重放的传输后端，跳过。
            if is_name_conflict || !is_retryable_transfer_operation(&task) {
                plan.summary.skipped += 1;
                continue;
            }
            match self.enqueue_retry(task.id).await {
                Ok(RetryEnqueued::Queued { task_id }) => {
                    plan.summary.queued += 1;
                    plan.queued_task_ids.push(task_id);
                }
                Ok(RetryEnqueued::AlreadyQueued { .. }) => plan.summary.already_queued += 1,
                Err(error) => {
                    tracing::warn!(task_id = task.id, %error, "一键重试时单个任务未能入队");
                    plan.summary.rejected += 1;
                }
            }
        }
        Ok(plan)
    }

    /// 接受前置校验通过的重试并迁移为 Pending。
    fn accept_retry_after_preflight(
        &self,
        task_id: i64,
        expected_revision: i64,
    ) -> AppResult<TransferTask> {
        let current = self.load(task_id)?;
        if current.state_revision != expected_revision
            || !matches!(
                current.state_kind().map_err(transition_error)?,
                TransferState::Failed | TransferState::RestartRequired
            )
        {
            self.notify_rejection();
            return Err(AppError::generic("传输任务状态已变化，请刷新后重试"));
        }
        if let Err(failure) = self.validate_static(&current) {
            self.persist_preflight_rejection(&current, failure.clone())?;
            return Err(AppError::generic(failure.message));
        }
        let pending = {
            let conn = self.db.lock();
            let transaction = conn
                .unchecked_transaction()
                .map_err(|error| AppError::generic(format!("开始重试接受事务失败：{error}")))?;
            let pending = repository::transition_transfer_in_transaction(
                &transaction,
                current.id,
                current.state_revision,
                TransferState::Pending,
                TransferPatch {
                    error_kind: ColumnPatch::Clear,
                    error_message: ColumnPatch::Clear,
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Clear,
                    // 人工重试是显式开启的新一轮预算周期：计数器归零，
                    // 否则预算耗尽而 Failed 的任务重试后第一次歧义就会再次终态失败。
                    attempt_count: Some(0),
                    verify_attempt_count: Some(0),
                    ..Default::default()
                },
            )
            .map_err(transition_error)?;
            // 任务已被接受重跑，无条件把兼容状态拨回 SYNCING；
            // RestartRequired 来源的行未必处于 FAILED，按旧状态门控会漏拨。
            update_compatibility_sync_status(
                &transaction,
                &pending,
                repository::sync_status::SYNCING,
                None,
                None,
            )?;
            transaction
                .commit()
                .map_err(|error| AppError::generic(format!("提交重试接受事务失败：{error}")))?;
            pending
        };
        self.notify_best_effort();
        Ok(pending)
    }

    /// 使用新意图重规划现有任务。
    fn replan_task(
        &self,
        conn: &rusqlite::Connection,
        current: &TransferTask,
        replacement: &TransferTask,
    ) -> AppResult<TransferTask> {
        // 重规划是自动行为，不构成「人工重试」：两个计数器都只能前进不能归零，
        // 否则熔断会被每一轮重规划反复重置（D9）。
        let preserved_attempt_count = current.attempt_count.max(replacement.attempt_count);
        let preserved_verify_count = current
            .verify_attempt_count
            .max(replacement.verify_attempt_count);
        // 旧任务失效、转回 Pending 和替换意图字段必须在同一事务完成。
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始任务重规划事务失败：{error}")))?;
        let current_state = current.state_kind().map_err(transition_error)?;
        // 已在重启态的任务直接复用，否则先留下旧意图被取代的状态记录。
        let restart = if current_state == TransferState::RestartRequired {
            current.clone()
        } else {
            repository::transition_transfer_in_transaction(
                &transaction,
                current.id,
                current.state_revision,
                TransferState::RestartRequired,
                TransferPatch {
                    error_kind: ColumnPatch::Set(TransferErrorKind::LocalChanged),
                    error_message: ColumnPatch::Set(
                        "新的 planner intent 已取代尚未执行的旧任务".to_string(),
                    ),
                    next_retry_at: ColumnPatch::Clear,
                    finished_at: ColumnPatch::Clear,
                    ..Default::default()
                },
            )
            .map_err(transition_error)?
        };
        // 清理旧错误，沿用替代意图提供的续传字段。
        // remote_result_file_id 必须保留：它是歧义远端写入的唯一身份证据，
        // 清空会让 promote_ambiguous_restarts 无法把任务升回核验态，同名碰撞检查又拒绝重复创建，
        // 任务就会陷入「重试 → 重规划 → 撞同名 → 再重启」的死循环。
        let pending = repository::transition_transfer_in_transaction(
            &transaction,
            restart.id,
            restart.state_revision,
            TransferState::Pending,
            TransferPatch {
                error_kind: ColumnPatch::Clear,
                error_message: ColumnPatch::Clear,
                next_retry_at: ColumnPatch::Clear,
                finished_at: ColumnPatch::Clear,
                remote_result_file_id: ColumnPatch::Keep,
                session_url: replacement
                    .session_url
                    .clone()
                    .map(ColumnPatch::Set)
                    .unwrap_or(ColumnPatch::Clear),
                transferred: Some(replacement.transferred),
                resume_offset: Some(replacement.resume_offset),
                attempt_count: Some(preserved_attempt_count),
                verify_attempt_count: Some(preserved_verify_count),
            },
        )
        .map_err(transition_error)?;
        // 状态迁移后再替换所有规划字段，revision 条件阻止并发覆盖。
        let changed = transaction
            .execute(
                "UPDATE transfer_queue SET
                    direction=?1,
                    file_id=?2,
                    local_path=?3,
                    name=?4,
                    total_size=?5,
                    transferred=?6,
                    created_at=?7,
                    server_id=?8,
                    upload_id=?9,
                    resume_offset=?10,
                    session_url=?11,
                    relative_path=?12,
                    parent_file_id=?13,
                    operation=?14,
                    source_mtime=?15,
                    source_size=?16,
                    expected_cloud_edited_time=?17,
                    attempt_count=?18
                 WHERE id=?19 AND state=?20 AND state_revision=?21",
                rusqlite::params![
                    replacement.direction,
                    replacement.file_id.as_deref(),
                    replacement.local_path.as_deref(),
                    replacement.name,
                    replacement.total_size,
                    replacement.transferred,
                    replacement.created_at,
                    replacement.server_id.as_deref(),
                    replacement.upload_id.as_deref(),
                    replacement.resume_offset,
                    replacement.session_url.as_deref(),
                    replacement.relative_path.as_deref(),
                    replacement.parent_file_id.as_deref(),
                    replacement.operation,
                    replacement.source_mtime,
                    replacement.source_size,
                    replacement.expected_cloud_edited_time,
                    // 与上面的状态迁移保持一致：重规划不重置重试预算。
                    preserved_attempt_count,
                    pending.id,
                    i32::from(TransferState::Pending),
                    pending.state_revision,
                ],
            )
            .map_err(|error| AppError::generic(format!("更新任务重规划意图失败：{error}")))?;
        if changed != 1 {
            return Err(AppError::generic(
                "任务重规划期间状态已变化，请等待下次同步",
            ));
        }
        // 回读完整任务后同步兼容状态，再统一提交。
        let replanned = transaction
            .query_row(
                "SELECT * FROM transfer_queue WHERE id=?1",
                [pending.id],
                TransferTask::from_row,
            )
            .map_err(|error| AppError::generic(format!("读取重规划任务失败：{error}")))?;
        update_compatibility_sync_status(
            &transaction,
            &replanned,
            repository::sync_status::SYNCING,
            None,
            None,
        )?;
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交任务重规划事务失败：{error}")))?;
        Ok(replanned)
    }

    /// 将含歧义远端结果的重启任务提升为待核验。
    pub(super) fn promote_restart_to_verifying(
        &self,
        conn: &rusqlite::Connection,
        restart: &TransferTask,
    ) -> AppResult<TransferTask> {
        repository::transition_transfer(
            conn,
            restart.id,
            restart.state_revision,
            TransferState::VerifyingRemote,
            TransferPatch {
                error_kind: ColumnPatch::Set(TransferErrorKind::RemoteAmbiguous),
                error_message: ColumnPatch::Set(
                    "远端写入已返回资源 ID，禁止重放并等待核验".to_string(),
                ),
                next_retry_at: ColumnPatch::Clear,
                finished_at: ColumnPatch::Clear,
                ..Default::default()
            },
        )
        .map_err(transition_error)
    }

    /// 仲裁任务进入 Running 或被活动意图阻塞。
    pub(super) fn transition_to_running_or_block(
        &self,
        current: &TransferTask,
    ) -> AppResult<RunningGateOutcome> {
        let outcome = {
            let conn = self.db.lock();
            let transaction = conn.unchecked_transaction().map_err(|error| {
                AppError::generic(format!("开始 Running 仲裁事务失败：{error}"))
            })?;
            let relative_path = current
                .relative_path
                .as_deref()
                .ok_or_else(|| AppError::generic("Running 仲裁缺少 relative_path"))?;
            let mut blocked = false;
            // 同路径点查，避免每次进入 Running 都整表扫描（D30）。
            for candidate in repository::list_by_relative_path(&transaction, relative_path)?
                .into_iter()
                .filter(|candidate| candidate.id != current.id)
            {
                match candidate.state_kind().map_err(transition_error)? {
                    TransferState::Running | TransferState::VerifyingRemote => {
                        blocked = true;
                    }
                    TransferState::RestartRequired if has_persisted_remote_result(&candidate) => {
                        repository::transition_transfer_in_transaction(
                            &transaction,
                            candidate.id,
                            candidate.state_revision,
                            TransferState::VerifyingRemote,
                            TransferPatch {
                                error_kind: ColumnPatch::Set(TransferErrorKind::RemoteAmbiguous),
                                error_message: ColumnPatch::Set(
                                    "远端结果 ID 已存在；Running 仲裁禁止重放并等待核验"
                                        .to_string(),
                                ),
                                next_retry_at: ColumnPatch::Clear,
                                finished_at: ColumnPatch::Clear,
                                ..Default::default()
                            },
                        )
                        .map_err(transition_error)?;
                        blocked = true;
                    }
                    _ => {}
                }
            }
            let outcome = if blocked {
                RunningGateOutcome::Blocked
            } else {
                let running = repository::transition_transfer_in_transaction(
                    &transaction,
                    current.id,
                    current.state_revision,
                    TransferState::Running,
                    TransferPatch {
                        error_kind: ColumnPatch::Clear,
                        error_message: ColumnPatch::Clear,
                        next_retry_at: ColumnPatch::Clear,
                        finished_at: ColumnPatch::Clear,
                        ..Default::default()
                    },
                )
                .map_err(transition_error)?;
                RunningGateOutcome::Running(Box::new(running))
            };
            transaction.commit().map_err(|error| {
                AppError::generic(format!("提交 Running 仲裁事务失败：{error}"))
            })?;
            outcome
        };
        self.notify_best_effort();
        Ok(outcome)
    }
}
