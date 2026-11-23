//! 目录同步作业队列：入队、排队、遍历投递。
//!
//! 一次「双端对齐」被拆成两段：
//!
//! 1. **作业（本模块）**：遍历云端子树、补建目录、计算差异，然后把每个文件
//!    **投递**成 `transfer_queue` 任务；这一段是秒级的控制面工作。
//! 2. **传输（TaskRunner）**：真正的上传/下载在传输队列里按并发槽执行。
//!
//! 这样做的直接收益：引擎忙（`syncing`/`folder_syncing`）时用户点击不再报错，
//! 作业排队等待；投递完成后作业立即结束，不会因为一个大文件上传而占着队列。

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::data::repository::folder_sync_jobs::{self, FolderSyncJob};
use crate::data::repository::{transfer_direction, TransferTask};
use crate::error::{AppError, AppResult};
use crate::sync::task_runner::TaskDisposition;
use crate::sync::transfer_state::TransferOperation;

use super::SyncEngine;

/// worker 空闲时的最长等待；有新作业入队时会被立即唤醒。
const WORKER_IDLE_WAIT_SECS: u64 = 2;
/// 引擎正忙时重新尝试获取目录同步许可的间隔。
const GUARD_RETRY_INTERVAL_MS: u64 = 500;
/// 终态作业保留条数，避免历史无限增长。
const TERMINAL_JOB_RETENTION: i64 = 20;
/// 父目录子项快照的复用时长（秒）。
///
/// 同一批补建目录常常落在同一个父目录下；复用一次遍历结果可以把 N 次全量翻页
/// 压到 1 次（D15）。窗口刻意取小：快照越旧，「同名目录已存在」的漏判窗口越大，
/// 而漏判会在云端留下重复目录。
const PARENT_CHILDREN_TTL_SECS: u64 = 5;
/// 查询作业列表时的返回上限（活跃 + 最近终态）。
const JOB_LIST_LIMIT: i64 = 40;
/// 投递进度写入步长：每累计这么多条写一次作业行并通知前端。
///
/// 逐条写库/发事件会让上千条作业产生上千次 DB 写入与 IPC；步长取 10
/// 既能给出可见的「已投递 N/M」推进，又把写入控制在可接受量级。
const ENQUEUED_PROGRESS_STEP: i64 = 10;

impl SyncEngine {
    /// 入队一次双端对齐作业。
    ///
    /// **永远不因引擎忙而报错**：作业只落库排队，由 worker 在合适时机执行。
    /// 同一目录重复点击会复用活跃作业，不会排队第二次。
    pub fn enqueue_folder_sync(&self, folder_id: &str, rel_path: &str) -> AppResult<FolderSyncJob> {
        if folder_id.trim().is_empty() {
            return Err(AppError::generic("目录同步缺少云端目录 ID"));
        }
        crate::core::paths::validate_relative_path(rel_path, true)?;
        self.ensure_cycle_active()?;
        let now = chrono::Utc::now().timestamp_millis();
        let job = {
            let conn = self.db.lock();
            folder_sync_jobs::enqueue(&conn, folder_id, rel_path, now)?
        };
        tracing::info!(job_id = job.id, folder_id, rel = %rel_path, "目录同步作业已入队");
        self.notify_folder_queue_changed();
        Ok(job)
    }

    /// 列出目录同步作业：活跃作业在前，其后是最近的终态历史。
    pub fn list_folder_sync_jobs(&self) -> AppResult<Vec<FolderSyncJob>> {
        let conn = self.db.lock();
        folder_sync_jobs::list_recent(&conn, JOB_LIST_LIMIT)
    }

    /// 取消一个排队中或规划中的目录同步作业。
    pub fn cancel_folder_sync_job(&self, job_id: i64) -> AppResult<bool> {
        let now = chrono::Utc::now().timestamp_millis();
        let cancelled = {
            let conn = self.db.lock();
            folder_sync_jobs::cancel(&conn, job_id, now)?
        };
        if cancelled {
            tracing::info!(job_id, "目录同步作业已取消");
            self.notify_folder_queue_changed();
        }
        Ok(cancelled)
    }

    /// 广播目录同步作业变化，并唤醒 worker。
    pub(super) fn notify_folder_queue_changed(&self) {
        let _ = self.folder_queue_tx.send(());
        self.folder_queue_changed.notify_waiters();
    }

    /// 订阅目录同步作业变化（运行时据此向前端转发事件）。
    pub fn subscribe_folder_queue(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.folder_queue_tx.subscribe()
    }

    /// 启动目录同步 worker：串行消费排队作业，永不因引擎忙而丢弃。
    pub(crate) fn start_folder_sync_worker(self: &Arc<Self>) {
        let engine = self.clone();
        // 关闭信号：引擎 shutdown 后 worker 退出，不再领取新作业。
        let mut shutdown = self.shutdown_tx.subscribe();
        tokio::spawn(async move {
            loop {
                if *shutdown.borrow() {
                    return;
                }
                // 没有排队作业时等待唤醒或超时，不空转查库。
                let has_queued = {
                    let conn = engine.db.lock();
                    folder_sync_jobs::list_active(&conn)
                        .map(|jobs| {
                            jobs.iter()
                                .any(|job| job.state == folder_sync_jobs::job_state::QUEUED)
                        })
                        .unwrap_or(false)
                };
                if !has_queued {
                    tokio::select! {
                        _ = engine.folder_queue_changed.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(WORKER_IDLE_WAIT_SECS)) => {}
                        _ = shutdown.changed() => return,
                    }
                    continue;
                }
                // 引擎正在跑同步周期时只等待，不报错、不丢作业。
                let Some(folder_guard) = engine.try_begin_folder_sync_guard() else {
                    tokio::time::sleep(Duration::from_millis(GUARD_RETRY_INTERVAL_MS)).await;
                    continue;
                };
                let job = {
                    let conn = engine.db.lock();
                    folder_sync_jobs::take_next_queued(&conn, chrono::Utc::now().timestamp_millis())
                        .ok()
                        .flatten()
                };
                let Some(job) = job else {
                    // 已被取消或并发领取：释放许可后重新等待。
                    drop(folder_guard);
                    engine.notify_folder_queue_changed();
                    continue;
                };
                engine.run_folder_sync_job(&job).await;
                drop(folder_guard);
                // 作业结束后补齐待处理的周期请求（传输阶段被让路的周期在这里补跑）。
                engine.notify_folder_queue_changed();
            }
        });
    }

    /// 执行单个作业：遍历投递，成功或失败都写回作业状态。
    async fn run_folder_sync_job(self: &Arc<Self>, job: &FolderSyncJob) {
        tracing::info!(job_id = job.id, rel = %job.rel_path, "目录同步作业开始规划");
        let result = self.plan_folder_sync(job).await;
        let now = chrono::Utc::now().timestamp_millis();
        let outcome = {
            let conn = self.db.lock();
            match &result {
                Ok(enqueued) => folder_sync_jobs::mark_completed(&conn, job.id, *enqueued, now)
                    .map(|()| {
                        tracing::info!(job_id = job.id, enqueued, "目录同步作业完成");
                    }),
                Err(error) => {
                    let message = error.to_string();
                    folder_sync_jobs::mark_failed(&conn, job.id, &message, now)
                        .map(|()| tracing::warn!(job_id = job.id, %error, "目录同步作业失败"))
                }
            }
        };
        if let Err(error) = outcome {
            tracing::warn!(job_id = job.id, %error, "写回目录同步作业状态失败");
        }
        // 修剪终态历史，避免长期运行后作业表无限增长。
        // 尽量不阻塞其它线程：拿不到锁就跳过本次修剪，下次作业再试。
        if let Some(conn) = self.db.try_lock() {
            if let Err(error) = folder_sync_jobs::prune_terminal(&conn, TERMINAL_JOB_RETENTION) {
                tracing::debug!(%error, "修剪目录同步作业历史失败");
            }
        }
        // 目录内容已变化：让前端刷新列表与状态。
        if let Err(error) = self.update_runtime_and_broadcast(|runtime| {
            runtime.content_changed = true;
            runtime.last_sync_time = Some(now);
        }) {
            tracing::warn!(%error, "目录同步完成后重算全局状态失败");
        }
    }

    /// 遍历云端子树并把差异逐条投递成传输任务，返回投递数量。
    async fn plan_folder_sync(self: &Arc<Self>, job: &FolderSyncJob) -> AppResult<i64> {
        let mount = self
            .mount
            .clone()
            .ok_or_else(|| AppError::generic("同步引擎尚未绑定挂载目录"))?;
        let task_runner = self.task_runner()?;
        let dest_dir = crate::core::paths::safe_join_under(mount.mount_dir(), &job.rel_path, true)?;

        // 1. BFS 读取云端子树（跳过规则与全局扫描保持一致）。
        let (cloud_files, cloud_folders, folder_rel_to_id) =
            self.collect_cloud_subtree(&job.folder_id).await?;
        tracing::info!(
            job_id = job.id,
            files = cloud_files.len(),
            folders = cloud_folders.len(),
            "目录同步作业：云端子树读取完成"
        );

        // 2. 本地缺的目录先建出来，保证后续下载有落点。
        tokio::fs::create_dir_all(&dest_dir).await.ok();
        for sub in &cloud_folders {
            let path = crate::core::paths::safe_join_under(&dest_dir, sub, false)?;
            let _ = tokio::fs::create_dir_all(path).await;
        }

        // 3. 扫描本地真实文件（排除临时文件与占位符）。
        let local_files = self.scan_local_files(&dest_dir).await;

        // 4. 计算差异。
        let cloud_map: HashMap<String, crate::drive::models::DriveFile> = cloud_files
            .iter()
            .map(|(relative, file)| (relative.clone(), file.clone()))
            .collect();
        let to_download: Vec<(String, crate::drive::models::DriveFile)> = cloud_files
            .into_iter()
            .filter(|(relative, _)| !local_files.contains_key(relative))
            .collect();
        let to_upload: Vec<(String, PathBuf)> = local_files
            .into_iter()
            .filter(|(relative, _)| !cloud_map.contains_key(relative))
            .collect();
        let total = (to_download.len() + to_upload.len()) as i64;
        {
            let conn = self.db.lock();
            folder_sync_jobs::set_planned_total(&conn, job.id, total)?;
        }
        tracing::info!(
            job_id = job.id,
            download = to_download.len(),
            upload = to_upload.len(),
            "目录同步作业：差异计算完成"
        );

        // 5. 为本地独有文件补建云端父目录链。
        let mut folder_ids = folder_rel_to_id;
        self.ensure_cloud_folders(&job.folder_id, &to_upload, &cloud_folders, &mut folder_ids)
            .await;

        // 6. 逐个投递任务：入队即返回，执行交给后台调度。
        let mut enqueued = 0_i64;
        for (subrel, file) in &to_download {
            if let Some(task) = self.download_task(job, &dest_dir, subrel, file) {
                enqueued += self.dispatch_folder_task(&task_runner, task);
                self.publish_folder_progress(job.id, enqueued);
            }
        }
        for (subrel, local_path) in &to_upload {
            let parent_subrel = parent_subrel_of(subrel);
            let parent_id = if parent_subrel.is_empty() {
                Some(job.folder_id.clone())
            } else {
                folder_ids.get(&parent_subrel).cloned()
            };
            let Some(parent_id) = parent_id else {
                tracing::warn!(
                    job_id = job.id,
                    subrel = %subrel,
                    parent = %parent_subrel,
                    "目录同步作业：云端父目录缺失，文件延期以避免上传到错误层级"
                );
                continue;
            };
            let task = self.upload_task(job, subrel, local_path, &parent_id);
            enqueued += self.dispatch_folder_task(&task_runner, task);
            self.publish_folder_progress(job.id, enqueued);
        }
        Ok(enqueued)
    }

    /// 按步长把「已投递 N 项」写回作业行并通知前端。
    ///
    /// 逐条写库/发事件会让大批量作业产生上千次 DB 写入与 IPC；这里每累计
    /// `ENQUEUED_PROGRESS_STEP` 条落库一次。完成时 `mark_completed` 再写权威总数。
    fn publish_folder_progress(self: &Arc<Self>, job_id: i64, enqueued: i64) {
        if enqueued == 0 || enqueued % ENQUEUED_PROGRESS_STEP != 0 {
            return;
        }
        {
            let conn = self.db.lock();
            if let Err(error) = folder_sync_jobs::set_enqueued(&conn, job_id, enqueued) {
                tracing::debug!(job_id, %error, "目录同步作业进度写入失败");
            }
        }
        self.notify_folder_queue_changed();
    }

    /// 读取云端子树：返回文件表、目录表与目录相对路径到云端 ID 的映射。
    async fn collect_cloud_subtree(
        self: &Arc<Self>,
        folder_id: &str,
    ) -> AppResult<(
        Vec<(String, crate::drive::models::DriveFile)>,
        Vec<String>,
        HashMap<String, String>,
    )> {
        let skip_matcher = self.skip_matcher();
        let mut cloud_files: Vec<(String, crate::drive::models::DriveFile)> = Vec::new();
        let mut cloud_folders: Vec<String> = Vec::new();
        let mut folder_rel_to_id: HashMap<String, String> = HashMap::new();
        folder_rel_to_id.insert(String::new(), folder_id.to_string());
        // 用游标推进的队列替代 Vec::remove(0)，目录很多时避免 O(n²)。
        let mut queue: std::collections::VecDeque<(String, String)> =
            std::collections::VecDeque::new();
        queue.push_back((folder_id.to_string(), String::new()));
        while let Some((id, path)) = queue.pop_front() {
            self.ensure_cycle_active()?;
            let _operation = self.begin_external_activity()?;
            let children = crate::commands::FILES_API
                .list_all(Some(id.as_str()))
                .await?;
            for child in children {
                if skip_matcher.should_skip(&child.name) {
                    continue;
                }
                crate::core::paths::validate_path_segment(&child.name)?;
                let subrel = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}/{}", child.name)
                };
                if child.is_folder() {
                    cloud_folders.push(subrel.clone());
                    folder_rel_to_id.insert(subrel.clone(), child.id.clone());
                    queue.push_back((child.id.clone(), subrel));
                } else {
                    cloud_files.push((subrel, child));
                }
            }
        }
        Ok((cloud_files, cloud_folders, folder_rel_to_id))
    }

    /// 扫描本地真实文件（相对路径 → 绝对路径）。
    async fn scan_local_files(&self, dest_dir: &Path) -> HashMap<String, PathBuf> {
        let skip_matcher = self.skip_matcher();
        let base = dest_dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut out = HashMap::new();
            let _ = scan_dir_for_real_files(&base, &base, &skip_matcher, &mut out);
            out
        })
        .await
        .unwrap_or_default()
    }

    /// 为本地独有文件补建缺失的云端父目录链。
    async fn ensure_cloud_folders(
        self: &Arc<Self>,
        root_folder_id: &str,
        to_upload: &[(String, PathBuf)],
        cloud_folders: &[String],
        folder_ids: &mut HashMap<String, String>,
    ) {
        // 收集所有需要上传的文件所缺的祖先目录。
        let mut missing_dirs: BTreeSet<String> = BTreeSet::new();
        for (subrel, _) in to_upload {
            let parts: Vec<&str> = subrel.split('/').collect();
            // 最后一段是文件名，之前的是目录层级。
            for index in 1..parts.len() {
                let ancestor = parts[..index].join("/");
                if !cloud_folders.contains(&ancestor) && !folder_ids.contains_key(&ancestor) {
                    missing_dirs.insert(ancestor);
                }
            }
        }
        // 按深度升序创建：父目录先建，子目录才能找到父 ID。
        let mut sorted_dirs: Vec<String> = missing_dirs.into_iter().collect();
        sorted_dirs.sort_by_key(|path| path.matches('/').count());
        // 父目录子项快照：同一批创建复用同一次遍历（D15）。
        let mut children_cache: HashMap<String, (Vec<crate::drive::models::DriveFile>, Instant)> =
            HashMap::new();
        for dir_rel in &sorted_dirs {
            let dir_name = dir_rel.rsplit('/').next().unwrap_or(dir_rel);
            let parent_rel = parent_subrel_of(dir_rel);
            // 克隆为自有字符串：后续要写回 folder_ids，不能同时持有它的借用。
            let parent_id = if parent_rel.is_empty() {
                Some(root_folder_id.to_string())
            } else {
                folder_ids.get(&parent_rel).cloned()
            };
            let Some(parent_id) = parent_id else {
                tracing::warn!(
                    dir = %dir_rel,
                    parent = %parent_rel,
                    "目录同步作业：父目录尚无云端 ID，跳过创建以避免错误平铺"
                );
                continue;
            };
            if let Err(error) = self.ensure_cycle_active() {
                tracing::warn!(%error, "目录同步作业：引擎已停止，中止补建目录");
                return;
            }
            // 先用（可能复用的）父目录快照做同名判定，命中就直接复用，不必 POST。
            let cached_existing = match self
                .find_cached_child_folder(&mut children_cache, &parent_id, dir_name)
                .await
            {
                Ok(existing) => existing,
                Err(error) => {
                    // 读不到父目录就不能断言「不存在」，宁可延期也不冒重复创建的风险。
                    tracing::warn!(
                        dir = %dir_rel,
                        error = %error,
                        "目录同步作业：父目录遍历失败，跳过该目录以避免重复创建"
                    );
                    continue;
                }
            };
            if let Some(existing) = cached_existing {
                folder_ids.insert(dir_rel.clone(), existing.id.clone());
                tracing::info!(dir = %dir_rel, cloud_id = %existing.id, "目录同步作业：同名目录已存在，复用云端身份");
                continue;
            }
            let create_result = {
                let _operation = self.begin_external_activity().ok();
                // 快照已证明同名目录不存在，这里只发一次 POST；失败仍走 409 容错复核。
                crate::commands::FILES_API
                    .create_folder_once(dir_name, Some(&parent_id), &parent_id)
                    .await
            };
            match create_result {
                Ok(folder) => {
                    folder_ids.insert(dir_rel.clone(), folder.id.clone());
                    // 新目录立刻并入快照，后续同名前缀判断不必再翻页。
                    if let Some(entry) = children_cache.get_mut(&parent_id) {
                        entry.0.push(folder.clone());
                    }
                    tracing::info!(dir = %dir_rel, cloud_id = %folder.id, "目录同步作业：已补建云端父目录");
                }
                Err(error) if matches!(error.drive_status(), Some(400 | 409)) => {
                    // 已存在同名目录时按 409 容错，避免整棵子树延期。
                    if let Ok(list) = crate::commands::FILES_API.list_all(Some(&parent_id)).await {
                        if let Some(existing) = list
                            .iter()
                            .find(|child| child.is_folder() && child.name == dir_name)
                        {
                            folder_ids.insert(dir_rel.clone(), existing.id.clone());
                            tracing::info!(dir = %dir_rel, cloud_id = %existing.id, "目录同步作业：父目录已存在（409 容错）");
                            continue;
                        }
                    }
                    tracing::warn!(dir = %dir_rel, error = %error, "目录同步作业：补建云端父目录失败");
                }
                Err(error) => {
                    tracing::warn!(dir = %dir_rel, error = %error, "目录同步作业：补建云端父目录失败，其内文件将延期");
                }
            }
        }
    }

    /// 在父目录子项快照里查找同名目录，快照过期或缺失时重新遍历一次。
    ///
    /// 返回 Ok(Some) 表示云端已存在同名目录；Ok(None) 表示快照明确显示不存在。
    ///
    /// 遍历失败必须返回 Err：调用方据此跳过创建。把「读不到」当成「不存在」
    /// 会在云端留下重复目录，这正是本函数要避免的。
    async fn find_cached_child_folder(
        &self,
        cache: &mut HashMap<String, (Vec<crate::drive::models::DriveFile>, Instant)>,
        parent_id: &str,
        dir_name: &str,
    ) -> AppResult<Option<crate::drive::models::DriveFile>> {
        // 快照是否仍新鲜。
        let fresh = cache
            .get(parent_id)
            .is_some_and(|(_, at)| at.elapsed().as_secs() < PARENT_CHILDREN_TTL_SECS);
        if !fresh {
            let listed = crate::commands::FILES_API.list_all(Some(parent_id)).await?;
            cache.insert(parent_id.to_string(), (listed, Instant::now()));
        }
        Ok(cache.get(parent_id).and_then(|(children, _)| {
            children
                .iter()
                .find(|child| child.is_folder() && child.name == dir_name)
                .cloned()
        }))
    }

    /// 构造云端独有文件的下载任务。
    fn download_task(
        &self,
        job: &FolderSyncJob,
        dest_dir: &Path,
        subrel: &str,
        file: &crate::drive::models::DriveFile,
    ) -> Option<TransferTask> {
        let dest = crate::core::paths::safe_join_under(dest_dir, subrel, false).ok()?;
        // 本地已有内容时按「更新」处理，保留版本对比语义。
        let is_update = std::fs::metadata(&dest)
            .ok()
            .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
        Some(TransferTask {
            direction: if is_update {
                transfer_direction::DOWNLOAD_UPDATE
            } else {
                transfer_direction::DOWNLOAD
            },
            file_id: Some(file.id.clone()),
            local_path: Some(dest.to_string_lossy().into_owned()),
            name: file.name.clone(),
            total_size: file.size,
            created_at: chrono::Utc::now().timestamp_millis(),
            relative_path: Some(join_relative(&job.rel_path, subrel)),
            parent_file_id: file
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned()),
            operation: Some(i32::from(if is_update {
                TransferOperation::DownloadUpdate
            } else {
                TransferOperation::Download
            })),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: file.edited_time.map(|time| time.timestamp_millis()),
            ..TransferTask::fresh_intent()
        })
    }

    /// 构造本地独有文件的上传任务。
    fn upload_task(
        &self,
        job: &FolderSyncJob,
        subrel: &str,
        local_path: &Path,
        parent_id: &str,
    ) -> TransferTask {
        // 任务快照必须固定源大小与 mtime，执行前会再次校验。
        let metadata = std::fs::metadata(local_path).ok();
        let file_size = metadata
            .as_ref()
            .map(|metadata| metadata.len() as i64)
            .unwrap_or(0);
        let source_mtime = metadata
            .as_ref()
            .and_then(crate::core::fs_meta::metadata_mtime_ms);
        TransferTask {
            direction: transfer_direction::UPLOAD,
            file_id: None,
            local_path: Some(local_path.to_string_lossy().into_owned()),
            name: subrel.rsplit('/').next().unwrap_or(subrel).to_string(),
            total_size: file_size,
            created_at: chrono::Utc::now().timestamp_millis(),
            relative_path: Some(join_relative(&job.rel_path, subrel)),
            parent_file_id: Some(parent_id.to_string()),
            operation: Some(i32::from(TransferOperation::Create)),
            source_mtime,
            source_size: Some(file_size),
            expected_cloud_edited_time: None,
            ..TransferTask::fresh_intent()
        }
    }

    /// 投递一个任务：入队成功即启动后台执行，绝不在此等待传输完成。
    fn dispatch_folder_task(
        self: &Arc<Self>,
        task_runner: &Arc<crate::sync::task_runner::TaskRunner>,
        task: TransferTask,
    ) -> i64 {
        match task_runner.enqueue(task) {
            Ok(enqueued) => {
                // 被同路径活动意图阻塞时不重复调度，避免自相踩踏。
                if enqueued.disposition != TaskDisposition::BlockedByActiveIntent {
                    self.spawn_background_retry(enqueued.task_id);
                }
                1
            }
            Err(error) => {
                tracing::warn!(%error, "目录同步作业：投递传输任务失败");
                0
            }
        }
    }
}

/// 拼接根相对路径与子树相对路径。
fn join_relative(root: &str, subrel: &str) -> String {
    if root.is_empty() {
        subrel.to_string()
    } else {
        format!("{root}/{subrel}")
    }
}

/// 取 subrel 的父 subrel（最后一个 `/` 之前；无 `/` 则空串）。
fn parent_subrel_of(subrel: &str) -> String {
    match subrel.rfind('/') {
        Some(index) => subrel[..index].to_string(),
        None => String::new(),
    }
}

/// 递归扫描目录，收集"真实内容文件"（相对路径 → 绝对路径）。
///
/// 跳过 `.tmp`、`.hwcloud_` 前缀与占位符（需 xattr 验证）；0 字节的用户文件
/// （如空配置）不是占位符，必须保留。
fn scan_dir_for_real_files(
    base: &Path,
    current: &Path,
    skip_matcher: &crate::mount::skip::SkipMatcher,
    out: &mut HashMap<String, PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_string();
        if skip_matcher.should_skip(&name) {
            continue;
        }
        if file_type.is_dir() {
            scan_dir_for_real_files(base, &path, skip_matcher, out)?;
        } else if file_type.is_file() {
            let metadata = entry.metadata()?;
            if metadata.len() == 0 && crate::mount::manager::is_placeholder_file(&path) {
                continue;
            }
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            out.insert(relative, path);
        }
    }
    Ok(())
}

/// 覆盖手动同步文件夹所依赖的私有递归扫描合同。
#[cfg(test)]
mod tests {
    use super::*;

    /// 手动目录扫描必须在所有层级排除 skipPatterns，避免上传系统文件和被排除目录。
    #[test]
    fn recursive_scan_excludes_skip_patterns() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nested_dir = temp_dir.path().join("nested");
        let trash_dir = temp_dir.path().join(".Trash");
        std::fs::create_dir_all(&nested_dir).unwrap();
        std::fs::create_dir_all(&trash_dir).unwrap();
        std::fs::write(temp_dir.path().join(".DS_Store"), b"system").unwrap();
        std::fs::write(nested_dir.join(".DS_Store"), b"system").unwrap();
        std::fs::write(nested_dir.join("keep.txt"), b"content").unwrap();
        std::fs::write(trash_dir.join("deleted.txt"), b"content").unwrap();
        let skip_patterns = vec![".DS_Store".to_string(), ".Trash".to_string()];
        let skip_matcher = crate::mount::skip::SkipMatcher::new(&skip_patterns);
        let mut files = HashMap::new();

        scan_dir_for_real_files(temp_dir.path(), temp_dir.path(), &skip_matcher, &mut files)
            .unwrap();

        assert_eq!(files.len(), 1);
        assert!(files.contains_key("nested/keep.txt"));
    }

    /// 父目录相对路径推导覆盖根层与多层两种情况。
    #[test]
    fn parent_relative_path_is_derived_from_last_segment() {
        assert_eq!(parent_subrel_of("a/b/c.txt"), "a/b");
        assert_eq!(parent_subrel_of("c.txt"), "");
    }
}
