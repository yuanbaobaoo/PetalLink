//! 同步执行器 —— 并发池 + 传输队列 + 稳定性检查 + 配额校验。
//!
//! 对齐 `legacy/lib/sync/sync_executor.dart`。
//!
//! 并发数默认 6（可配置 1-20），使用 tokio Semaphore 限流。
//! 传输队列（TransferQueue 表）记录进度，修剪历史（保留最近 100 条已结束任务）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Semaphore;

use crate::data::repository::{
    self, sync_status,
    transfer_direction, transfer_state, SyncItem, TransferTask,
};
use crate::drive::{
    download_api::{DownloadApi, ProgressFn as DownloadProgressFn},
    files_api::FilesApi,
    upload_api::{ProgressFn as UploadProgressFn, UploadApi},
};
use crate::mount::manager::MountManager;
use crate::sync::conflict::ConflictResolver;
use crate::sync::stability::{StabilityChecker, StabilityResult};
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};

/// 进度更新节流间隔（毫秒）。下载流式每个 chunk 都回调，过频写 DB+emit 会卡顿。
const PROGRESS_THROTTLE_MS: i64 = 500;

/// 同步执行器 —— 持有全部外部依赖。
pub struct SyncExecutor {
    concurrency: u32,
    files_api: Arc<FilesApi>,
    download_api: Arc<DownloadApi>,
    upload_api: Arc<UploadApi>,
    mount: Option<Arc<MountManager>>,
    conflict: Option<Arc<std::sync::Mutex<ConflictResolver>>>,
    stability: Option<Arc<tokio::sync::Mutex<StabilityChecker>>>,
    db: Option<Arc<parking_lot::Mutex<rusqlite::Connection>>>,
    /// 传输更新通知发送端（每次传输结算时触发前端刷新）
    transfer_update_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

// ===== 进度更新（节流） =====

/// 节流写传输进度到 DB 并通知前端刷新。
///
/// - `db` / `tx`：传输队列 DB 连接 + 传输更新广播发送端
/// - `local_path`：用于定位 transfer_queue 行（WHERE local_path AND state=RUNNING）
/// - `transferred`：已传输字节数
/// - `last_emit_ms`：上次 emit 的 epoch 毫秒（AtomicI64，0=从未 emit）；按 500ms 节流
///
/// 节流状态由每个传输任务的闭包独立持有，无共享状态、无锁、无竞态。
/// AtomicI64 是 Send+Sync，满足 ProgressFn 的线程安全要求。
fn emit_throttled_progress(
    db: &Option<Arc<parking_lot::Mutex<rusqlite::Connection>>>,
    tx: &Option<tokio::sync::broadcast::Sender<()>>,
    local_path: &str,
    transferred: i64,
    last_emit_ms: &AtomicI64,
) {
    // 节流：距上次 emit 不足 PROGRESS_THROTTLE_MS 则跳过
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let last = last_emit_ms.load(Ordering::Relaxed);
    if last != 0 && now - last < PROGRESS_THROTTLE_MS {
        return;
    }
    last_emit_ms.store(now, Ordering::Relaxed);

    // 更新 DB 进度（仅 RUNNING 行）
    if let Some(db) = db {
        let conn = db.lock();
        let _ = conn.execute(
            "UPDATE transfer_queue SET transferred=?1 WHERE local_path=?2 AND state=?3",
            rusqlite::params![transferred, local_path, transfer_state::RUNNING],
        );
    }
    // 通知前端 + 托盘菜单刷新
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
}

impl SyncExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        concurrency: u32,
        files_api: Arc<FilesApi>,
        download_api: Arc<DownloadApi>,
        upload_api: Arc<UploadApi>,
    ) -> Self {
        Self {
            concurrency,
            files_api,
            download_api,
            upload_api,
            mount: None,
            conflict: None,
            stability: None,
            db: None,
            transfer_update_tx: None,
        }
    }

    /// 设置 mount manager（延迟注入，避免循环依赖）。
    pub fn set_mount(&mut self, mount: Arc<MountManager>) {
        self.mount = Some(mount);
    }

    /// 设置冲突解决器。
    pub fn set_conflict(&mut self, conflict: Arc<std::sync::Mutex<ConflictResolver>>) {
        self.conflict = Some(conflict);
    }

    /// 设置稳定性检查器。
    pub fn set_stability(&mut self, s: Arc<tokio::sync::Mutex<StabilityChecker>>) {
        self.stability = Some(s);
    }

    /// 设置 DB 连接。
    pub fn set_db(&mut self, db: Arc<parking_lot::Mutex<rusqlite::Connection>>) {
        self.db = Some(db);
    }

    /// 设置传输更新通知通道（每次结算时触发前端刷新）。
    pub fn set_transfer_update_tx(&mut self, tx: tokio::sync::broadcast::Sender<()>) {
        self.transfer_update_tx = Some(tx);
    }

    /// 获取 UploadApi 引用（供引擎断点续传等外部调用）。
    pub fn upload_api(&self) -> &Arc<UploadApi> {
        &self.upload_api
    }

    /// 并发执行全部动作。
    /// 对齐 dart `executor.executeAll`。
    pub async fn execute_all(&self, actions: &[SyncAction]) -> Vec<ActionResult> {
        // 修剪传输历史（保留最近 100 条）
        self.prune_transfer_history();

        let semaphore = Arc::new(Semaphore::new(self.concurrency as usize));
        let mut handles = Vec::new();

        for action in actions {
            let sem = semaphore.clone();
            let action = action.clone();
            let self_clone = self.clone_executor(); // cheap clone of Arc'd fields

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                self_clone.execute_one(&action).await
            }));
        }

        let mut results = Vec::new();
        for h in handles {
            if let Ok(r) = h.await {
                results.push(r);
            }
        }
        results
    }

    /// 克隆执行器（只保留 Arc 字段的引用，轻量）。
    pub fn clone_executor(&self) -> Self {
        Self {
            concurrency: self.concurrency,
            files_api: self.files_api.clone(),
            download_api: self.download_api.clone(),
            upload_api: self.upload_api.clone(),
            mount: self.mount.clone(),
            conflict: self.conflict.clone(),
            stability: self.stability.clone(),
            db: self.db.clone(),
            transfer_update_tx: self.transfer_update_tx.clone(),
        }
    }

    /// 执行单个动作。
    async fn execute_one(&self, action: &SyncAction) -> ActionResult {
        // 入队传输（仅 upload/download/conflict 可见于传输面板）
        self.enqueue_transfer(action);

        tracing::debug!(rel = action.relative_path.as_deref(), action_type = ?action.action_type, "executor: 开始执行");

        let result = match action.action_type {
            SyncActionType::Upload => self.do_upload(action).await,
            SyncActionType::Download => self.do_download(action).await,
            SyncActionType::CreatePlaceholder => self.do_create_placeholder(action).await,
            SyncActionType::CreateFolder => self.do_create_folder(action).await,
            SyncActionType::DeleteFromCloud => self.do_delete_from_cloud(action).await,
            SyncActionType::DeleteFromLocal => self.do_delete_from_local(action).await,
            SyncActionType::CreateConflictCopy => self.do_conflict(action).await,
            SyncActionType::BackupBeforeCloudDelete => self.do_backup_before_cloud_delete(action).await,
            SyncActionType::Skip => ActionResult {
                success: true,
                error_message: Some(action.reason.clone().unwrap_or_default()),
                deferred: false,
                cloud_file: None,
            },
        };

        // 传输状态清理
        self.settle_transfer(action, &result);
        result
    }

    /// 入队传输记录（仅 upload/download/conflict 可见）。返回 transfer id。
    /// 公开供 sync_folder_recursive_impl 等直接调用下载/上传 API 的路径手动入队。
    pub fn enqueue_transfer(&self, action: &SyncAction) -> Option<i64> {
        let direction = match action.action_type {
            SyncActionType::Upload | SyncActionType::CreateConflictCopy => transfer_direction::UPLOAD,
            SyncActionType::Download => {
                // 区分「更新」与「下载」：planner 的 Download 动作仅在本地已有真实内容、
                // 云端较新时产生（local_has_content = !placeholder）。此处用「本地已存在且 size>0」
                // 判定 → 标记为 DOWNLOAD_UPDATE（UI 显示「更新」）；本地不存在/为占位符 → DOWNLOAD。
                let local_has_content = action.local_path.as_ref()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len() > 0)
                    .unwrap_or(false);
                if local_has_content { transfer_direction::DOWNLOAD_UPDATE } else { transfer_direction::DOWNLOAD }
            }
            _ => return None, // 建目录/占位符/删除不入队
        };
        if let Some(db) = &self.db {
            let total_size = match direction {
                d if d == transfer_direction::UPLOAD => {
                    action.local_path.as_ref()
                        .and_then(|p| std::fs::metadata(p).ok())
                        .map(|m| m.len() as i64)
                        .unwrap_or(0)
                }
                _ => {
                    action.cloud_file.as_ref()
                        .map(|f| f.size)
                        .unwrap_or(0)
                }
            };
            let conn = db.lock();
            let task = TransferTask {
                id: 0,
                direction,
                file_id: action.file_id.clone(),
                local_path: action.local_path.clone(),
                name: action.relative_path.as_deref().unwrap_or("unknown").to_string(),
                total_size,
                transferred: 0,
                state: transfer_state::RUNNING,
                error_message: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
            };
            let id = repository::insert_transfer(&conn, &task).ok();
            // 立即通知前端显示新传输项
            if let Some(ref tx) = self.transfer_update_tx {
                let _ = tx.send(());
            }
            id
        } else {
            None
        }
    }

    /// 传输结果清理：更新状态 + 完成后 transferred=total_size。
    fn settle_transfer(&self, action: &SyncAction, result: &ActionResult) {
        if let Some(db) = &self.db {
            let conn = db.lock();
            let state = if result.deferred {
                transfer_state::PENDING
            } else if result.success {
                transfer_state::COMPLETED
            } else {
                transfer_state::FAILED
            };
            let is_delete = action.action_type == SyncActionType::DeleteFromCloud
                || action.action_type == SyncActionType::DeleteFromLocal;
            if is_delete {
                let name = action.relative_path.as_deref().unwrap_or("?");
                let _ = conn.execute(
                    "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, transferred=total_size WHERE name=?4 AND direction=?5 AND state=?6",
                    rusqlite::params![state, result.error_message.as_deref(), chrono::Utc::now().timestamp_millis(), name, transfer_direction::DELETE, transfer_state::RUNNING],
                );
            } else if let Some(ref lp) = action.local_path {
                let size = if result.success { std::fs::metadata(lp).ok().map(|m| m.len() as i64).unwrap_or(0) } else { 0 };
                // 结算传输：transferred 设为实际文件大小；若 total_size 为 0（入队时 cloud_file 缺失），
                // 同步修正为实际大小，确保前端进度百分比正确显示（避免 0/0 → 0%）
                let _ = conn.execute(
                    "UPDATE transfer_queue SET state=?1, error_message=?2, finished_at=?3, transferred=?4, total_size=CASE WHEN total_size=0 AND ?5!=0 THEN ?5 ELSE total_size END WHERE local_path=?6 AND state=?7",
                    rusqlite::params![state, result.error_message.as_deref(), chrono::Utc::now().timestamp_millis(), size, size, lp.as_str(), transfer_state::RUNNING],
                );
            }
            // 触发前端传输面板刷新
            if let Some(ref tx) = self.transfer_update_tx {
                let _ = tx.send(());
            }
        }
    }

    /// 入队删除传输记录（供 do_delete_from_cloud / do_delete_from_local 用）。
    fn enqueue_delete_transfer(&self, action: &SyncAction) {
        if let Some(db) = &self.db {
            let conn = db.lock();
            let task = TransferTask {
                id: 0,
                direction: transfer_direction::DELETE,
                file_id: action.file_id.clone(),
                local_path: action.local_path.clone(),
                name: action.relative_path.as_deref().unwrap_or("unknown").to_string(),
                total_size: 0,
                transferred: 0,
                state: transfer_state::RUNNING,
                error_message: None,
                created_at: chrono::Utc::now().timestamp_millis(),
                finished_at: None,
                server_id: None,
                upload_id: None,
                resume_offset: 0,
            };
            let _ = repository::insert_transfer(&conn, &task);
        }
    }

    /// 修剪传输历史（保留最近 100 条已结束任务）。
    fn prune_transfer_history(&self) {
        if let Some(db) = &self.db {
            { let conn = db.lock();
                let _ = repository::prune_transfer_history(&conn, 100);
            }
        }
    }

    // ===== 各动作实现 =====

    async fn do_upload(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => return ActionResult { success: false, error_message: Some("上载缺少本地路径".into()), deferred: false, cloud_file: None },
        };
        if !path.exists() {
            return ActionResult { success: false, error_message: Some("本地文件不存在".into()), deferred: false, cloud_file: None };
        }

        // 稳定性检查（4 次重试，退避 [0s, 2s, 3s, 5s]，对齐 dart _checkStabilityWithRetry）
        let backoffs = [0u64, 2, 3, 5];
        let mut stable = false;
        if let Some(stab) = &self.stability {
            for &delay in &backoffs {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                let mut checker = stab.lock().await;
                match checker.check(&path).await {
                    StabilityResult::Editing => return ActionResult {
                        success: false, error_message: Some("用户正在编辑，暂停自动同步".into()), deferred: true, cloud_file: None,
                    },
                    StabilityResult::Stable => { stable = true; break; }
                    StabilityResult::Unstable => {} // 继续重试
                }
            }
            if !stable {
                return ActionResult {
                    success: false, error_message: Some("文件尚不稳定，延迟到下次周期".into()), deferred: true, cloud_file: None,
                };
            }
        }

        let parent_id = action.parent_file_id.as_deref();
        let has_cloud_id = action.file_id.is_some();
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        let is_large = size > 20 * 1024 * 1024;
        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 断点续传回调：持久化 serverId/uploadId/offset
        let db_resume = self.db.clone();
        let task_name = action.relative_path.clone().unwrap_or_default();
        let on_resume: crate::drive::upload_api::ResumeProgressFn = Box::new(move |sid, uid, offset| {
            if let Some(ref db) = db_resume {
                let conn = db.lock();
                let _ = conn.execute(
                    "UPDATE transfer_queue SET server_id=?1, upload_id=?2, resume_offset=?3, transferred=?3 WHERE name=?4 AND state=?5",
                    rusqlite::params![sid, uid, offset as i64, task_name, transfer_state::RUNNING],
                );
            }
        });
        let resume_cb: Option<&crate::drive::upload_api::ResumeProgressFn> = if is_large { Some(&on_resume) } else { None };

        // 进度回调：节流写 transferred 并通知前端刷新（500ms 一次）
        // throttle 节流状态由闭包 move 捕获，跨多次回调持久；AtomicI64 内部可变，Fn 闭包可用 &。
        let db_prog = self.db.clone();
        let tx_prog = self.transfer_update_tx.clone();
        let lp_prog = action.local_path.clone().unwrap_or_default();
        let total = size;
        let throttle = AtomicI64::new(0);
        let on_progress: UploadProgressFn = Box::new(move |ratio: f64| {
            let transferred = (ratio * total as f64) as i64;
            emit_throttled_progress(&db_prog, &tx_prog, &lp_prog, transferred, &throttle);
        });
        let result = if has_cloud_id && !is_large {
            match self.upload_api.upload_update(action.file_id.as_ref().unwrap(), &path, parent_id, Some(&on_progress)).await {
                Ok(f) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: Some(f) },
                Err(_e) => {
                    match self.upload_api.upload(&path, parent_id, Some(&on_progress), resume_cb).await {
                        Ok(f) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: Some(f) },
                        Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
                    }
                }
            }
        } else {
            match self.upload_api.upload(&path, parent_id, Some(&on_progress), resume_cb).await {
                Ok(f) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: Some(f) },
                Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
            }
        };
        if result.success {
            tracing::info!(rel, size, "上传成功");
        } else if !result.deferred {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "上传失败");
        }
        result
    }

    async fn do_download(&self, action: &SyncAction) -> ActionResult {
        let local_path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => return ActionResult { success: false, error_message: Some("下载缺少本地路径".into()), deferred: false, cloud_file: None },
        };
        let file_id = match &action.file_id {
            Some(id) => id.clone(),
            None => return ActionResult { success: false, error_message: Some("下载缺少 fileId".into()), deferred: false, cloud_file: None },
        };

        // 确保父目录存在（对齐 dart: parent.create(recursive: true)）
        if let Some(parent) = local_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ActionResult { success: false, error_message: Some(format!("创建父目录失败：{e}")), deferred: false, cloud_file: None };
            }
        }

        // 备份被修改的占位符（改名保留，对齐 dart backupModifiedPlaceholderIfNeeded）
        if let Some(m) = &self.mount {
            let _ = m.backup_modified_placeholder_if_needed(&local_path).await;
        }

        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 进度回调：节流写 transferred 并通知前端刷新（500ms 一次）
        let db_prog = self.db.clone();
        let tx_prog = self.transfer_update_tx.clone();
        let lp_prog = local_path.to_string_lossy().to_string();
        let throttle = AtomicI64::new(0);
        let on_progress: DownloadProgressFn = Box::new(move |received: u64, _total: u64| {
            emit_throttled_progress(&db_prog, &tx_prog, &lp_prog, received as i64, &throttle);
        });
        let result = match self.download_api.download(&file_id, &local_path, Some(&on_progress)).await {
            Ok(()) => {
                if let Some(m) = &self.mount {
                    let _ = m.mark_downloaded(&local_path).await;
                }
                ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
            }
            Err(e) => {
                // 清理 .tmp 残留（对齐 dart: tmpPath.delete()）
                let tmp = std::path::PathBuf::from(format!("{}.tmp", local_path.display()));
                let _ = std::fs::remove_file(&tmp);
                ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None }
            }
        };
        if result.success {
            tracing::info!(rel, "下载成功");
        } else {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "下载失败");
        }
        result
    }

    async fn do_create_placeholder(&self, action: &SyncAction) -> ActionResult {
        let cloud = match &action.cloud_file {
            Some(c) => c,
            None => return ActionResult { success: false, error_message: Some("缺少云端文件元数据".into()), deferred: false, cloud_file: None },
        };
        let rel_path = match &action.relative_path {
            Some(p) => p,
            None => return ActionResult { success: false, error_message: Some("缺少相对路径".into()), deferred: false, cloud_file: None },
        };
        if let Some(m) = &self.mount {
            match m.create_placeholder_if_needed(rel_path, &cloud.id, cloud.size).await {
                Ok(()) => {
                    // 对齐 dart：写 DB 记录（status=cloudOnly），防止孤儿占位符
                    if let (Some(db), Some(local_path)) = (&self.db, &action.local_path) {
                        { let conn = db.lock();
                            let _ = repository::upsert(&conn, &SyncItem {
                                file_id: cloud.id.clone(), local_path: local_path.clone(),
                                parent_folder_id: cloud.parent_folder.as_ref().and_then(|v| v.first().cloned()),
                                name: cloud.name.clone(), is_folder: false, size: cloud.size,
                                local_size: None, sha256: None, local_mtime: None,
                                cloud_edited_time: cloud.edited_time.map(|t| t.timestamp_millis()),
                                last_sync_time: None, status: sync_status::CLOUD_ONLY, error_message: None,
                            });
                        }
                    }
                    ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
                }
                Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
            }
        } else {
            ActionResult { success: false, error_message: Some("mount manager 未初始化".into()), deferred: false, cloud_file: None }
        }
    }

    async fn do_create_folder(&self, action: &SyncAction) -> ActionResult {
        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 本地新文件夹（无云端文件）→ 调 createFolder API
        let result = if let Some(cloud_file) = &action.cloud_file {
            // 云端已有文件夹 → 本地 ensure
            let _cloud = cloud_file;
            if let Some(m) = &self.mount {
                match m.ensure_folder(rel) {
                    Ok(_) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: None },
                    Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
                }
            } else {
                ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
            }
        } else {
            // FIX:fileName 取相对路径的**最后一段**，而非整条 relative_path。
            // 之前 name = "学习/程序设计"（含 /）撞华为文件名校验 → 400 21004002
            // fileName can not contain '<>|:"*?/\'。这与 engine.rs 写 DB 时
            // 取 rel.rsplit('/').next() 保持一致。
            let full = action.relative_path.as_deref().unwrap_or("新建文件夹");
            let name = full.rsplit('/').next().unwrap_or(full);

            // ★ 创建前先检查云端是否已存在同名目录。
            // 场景：目录被删除（cloud_tree 已清）→ 用户从回收站恢复 → watcher 先于
            // 云端刷新触发 → planner 生成 CreateFolder → 若不检查，华为 API 会创建
            // "name(1)" 后缀副本，而非复用已有目录。
            // parent_file_id 为 None 表示根目录，同样需要检查。
            {
                let pid = action.parent_file_id.as_deref();
                if let Ok(list) = self.files_api.list_all(pid).await {
                    if let Some(existing) = list.iter().find(|f| f.is_folder() && f.name == name) {
                        tracing::info!(
                            rel,
                            existing_id = %existing.id,
                            parent = pid.unwrap_or("root"),
                            "CreateFolder 跳过：云端已存在同名文件夹，复用已有 ID"
                        );
                        return ActionResult {
                            success: true,
                            error_message: None,
                            deferred: false,
                            cloud_file: Some(existing.clone()),
                        };
                    }
                }
            }

            match self.files_api.create_folder(name, action.parent_file_id.as_deref()).await {
                Ok(f) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: Some(f) },
                // 对齐 dart：400/409 时查同名已存在文件夹，存在则视为成功
                // （同样用末段 name 匹配，与云端真名一致才能命中）
                Err(ref e) if e.to_string().contains("400") || e.to_string().contains("409") => {
                    if let Some(pid) = action.parent_file_id.as_deref() {
                        if let Ok(list) = self.files_api.list_all(Some(pid)).await {
                            if let Some(existing) = list.iter().find(|f| f.is_folder() && f.name == name) {
                                ActionResult { success: true, error_message: None, deferred: false, cloud_file: Some(existing.clone()) }
                            } else {
                                ActionResult { success: false, error_message: Some(format!("{e}")), deferred: false, cloud_file: None }
                            }
                        } else {
                            ActionResult { success: false, error_message: Some(format!("{e}")), deferred: false, cloud_file: None }
                        }
                    } else {
                        ActionResult { success: false, error_message: Some(format!("{e}")), deferred: false, cloud_file: None }
                    }
                }
                Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
            }
        };
        if result.success {
            tracing::info!(rel, "创建目录成功");
        } else {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "创建目录失败");
        }
        result
    }

    async fn do_delete_from_cloud(&self, action: &SyncAction) -> ActionResult {
        let file_id = match &action.file_id {
            Some(id) => id.clone(),
            None => return ActionResult { success: false, error_message: Some("缺少 fileId".into()), deferred: false, cloud_file: None },
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 入队传输记录
        self.enqueue_delete_transfer(action);
        let result = match self.files_api.delete(&file_id).await {
            Ok(()) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: None },
            Err(e) => {
                let msg = e.to_string();
                // 404 表示云端已不存在（可能已被前序操作删除），视为成功
                if msg.contains("404") {
                    tracing::info!(rel, file_id, "云端文件已不存在（404），视为删除成功");
                    ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
                } else {
                    ActionResult { success: false, error_message: Some(msg), deferred: false, cloud_file: None }
                }
            }
        };
        if result.success {
            tracing::info!(rel, "删除云端文件成功");
        } else {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "删除云端文件失败");
        }
        result
    }

    async fn do_delete_from_local(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => return ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }, // DB 清理场景
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        let result = if let Some(m) = &self.mount {
            match m.delete_local(&path).await {
                Ok(()) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: None },
                Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
            }
        } else {
            ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
        };
        if result.success {
            tracing::info!(rel, "删除本地文件成功");
        } else {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "删除本地文件失败");
        }
        result
    }

    async fn do_conflict(&self, action: &SyncAction) -> ActionResult {
        let local_path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => return ActionResult { success: false, error_message: Some("冲突处理缺少本地路径".into()), deferred: false, cloud_file: None },
        };
        let cloud_file = match &action.cloud_file {
            Some(c) => c,
            None => return ActionResult { success: false, error_message: Some("冲突处理缺少云端文件元数据".into()), deferred: false, cloud_file: None },
        };

        // 获取本地 mtime
        let local_mtime = tokio::fs::metadata(&local_path).await
            .ok().and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()).unwrap_or(chrono::Utc::now()))
            .unwrap_or(chrono::Utc::now());

        // 解析冲突
        let resolution = if let Some(conflict) = &self.conflict {
            if let Ok(mut resolver) = conflict.lock() {
                resolver.resolve(&local_path, cloud_file, &local_mtime)
            } else {
                return ActionResult { success: false, error_message: Some("冲突解决器获取失败".into()), deferred: false, cloud_file: None };
            }
        } else {
            return ActionResult { success: false, error_message: Some("冲突解决器未初始化".into()), deferred: false, cloud_file: None };
        };

        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 对齐 dart：cloud-wins → 本地副本保存到 copyPath，云端下载到 localPath
        // local-wins → 云端副本下载到 copyPath，本地覆盖上传到云端
        let result = match resolution.winner {
            crate::sync::conflict::ConflictSide::Cloud => {
                // 云端获胜：移动本地 → copyPath，下载云端 → localPath
                // 改名失败绝不能继续下载——否则本地修改被覆盖且无副本，数据丢失。
                // 返回失败保住本地原文件，下轮重试。
                if let Err(e) = tokio::fs::rename(&local_path, &resolution.copy_path).await {
                    ActionResult {
                        success: false,
                        error_message: Some(format!("冲突备份改名失败，跳过下载以保本地修改：{e}")),
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    match self.download_api.download(&cloud_file.id, &local_path, None).await {
                        Ok(()) => {
                            if let Some(m) = &self.mount { let _ = m.mark_downloaded(&local_path).await; }
                            if let Some(m) = &self.mount { let _ = m.clear_placeholder_xattr(&resolution.copy_path).await; }
                            ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
                        }
                        Err(e) => {
                            let _ = tokio::fs::rename(&resolution.copy_path, &local_path).await;
                            ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None }
                        }
                    }
                }
            }
            crate::sync::conflict::ConflictSide::Local => {
                // 本地获胜：下载云端旧版 → copyPath（败方副本），上传本地覆盖云端。
                if let Err(e) = self.download_api.download(&cloud_file.id, &resolution.copy_path, None).await {
                    ActionResult {
                        success: false,
                        error_message: Some(format!("冲突副本（云端旧版）下载失败，跳过覆盖以保云端旧版：{e}")),
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    if let Some(m) = &self.mount { let _ = m.clear_placeholder_xattr(&resolution.copy_path).await; }
                    let parent_id = cloud_file.parent_folder.as_ref().and_then(|v| v.first().map(|s| s.as_str()));
                    match self.upload_api.upload_update(&cloud_file.id, &local_path, parent_id, None).await {
                        Ok(_) => ActionResult { success: true, error_message: None, deferred: false, cloud_file: None },
                        Err(e) => ActionResult { success: false, error_message: Some(e.to_string()), deferred: false, cloud_file: None },
                    }
                }
            }
        };
        if result.success {
            tracing::info!(rel, "冲突处理完成");
        } else {
            tracing::warn!(rel, error = result.error_message.as_deref().unwrap_or("?"), "冲突处理失败");
        }
        result
    }

    /// 云端已删除但本地有未上传修改：改名备份副本（保内容），原路径腾空即满足云端删除。
    /// 副本清掉占位 xattr，下轮作为全新本地文件上传（救援用户改动）。
    async fn do_backup_before_cloud_delete(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => return ActionResult { success: true, error_message: None, deferred: false, cloud_file: None },
        };
        if !path.exists() {
            return ActionResult { success: true, error_message: None, deferred: false, cloud_file: None };
        }
        // 本地 mtime 作为副本时间戳（败方=本地的修改时间）
        let local_mtime = tokio::fs::metadata(&path).await
            .ok().and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()).unwrap_or(chrono::Utc::now()))
            .unwrap_or_else(chrono::Utc::now);
        let copy_path = crate::sync::conflict::dedupe_copy_path(&path, "本地副本", &local_mtime);
        match tokio::fs::rename(&path, &copy_path).await {
            Ok(()) => {
                if let Some(m) = &self.mount { let _ = m.clear_placeholder_xattr(&copy_path).await; }
                tracing::info!(
                    src = %path.display(),
                    backup = %copy_path.display(),
                    "云端删除但本地有未上传修改，已备份副本"
                );
                ActionResult { success: true, error_message: None, deferred: false, cloud_file: None }
            }
            Err(e) => ActionResult {
                success: false,
                error_message: Some(format!("备份副本失败：{e}")),
                deferred: false,
                cloud_file: None,
            },
        }
    }
}
