//! 同步动作的并发编排与非传输动作实现。

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::{stream, FutureExt, StreamExt};
use tokio::sync::Semaphore;

use crate::data::repository::{self, sync_status, SyncItem};
use crate::drive::download_api::{DownloadExpectation, LocalDestinationSnapshot};
use crate::error::{AppError, AppResult};
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};

use super::SyncExecutor;

/// 读取冲突源文件的严格本地快照；符号链接和不可复核时间戳均拒绝处理。
fn inspect_conflict_source(
    path: &std::path::Path,
) -> AppResult<(chrono::DateTime<chrono::Utc>, LocalDestinationSnapshot)> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| AppError::generic(format!("读取冲突源文件失败：{error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::generic("冲突源路径不是安全的普通文件"));
    }
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or_else(|| AppError::generic("无法读取冲突源文件修改时间"))?;
    let local_mtime = chrono::DateTime::from_timestamp_millis(mtime_ms)
        .ok_or_else(|| AppError::generic("冲突源文件修改时间超出支持范围"))?;
    Ok((
        local_mtime,
        LocalDestinationSnapshot {
            mtime_ms,
            size: metadata.len(),
        },
    ))
}

/// 确认普通文件仍与先前快照一致。
fn verify_conflict_source_snapshot(
    path: &std::path::Path,
    expected: &LocalDestinationSnapshot,
) -> AppResult<()> {
    let (_, actual) = inspect_conflict_source(path)?;
    if &actual != expected {
        return Err(AppError::generic("生成副本期间本地源文件已变化"));
    }
    Ok(())
}

/// 以 create-new 方式把源文件复制到隐藏 staging，并同步落盘、复核源快照。
async fn copy_to_hidden_conflict_staging(
    source: &std::path::Path,
    staging: &std::path::Path,
    expected: &LocalDestinationSnapshot,
) -> AppResult<()> {
    let source = source.to_path_buf();
    let staging = staging.to_path_buf();
    let expected = expected.clone();
    tokio::task::spawn_blocking(move || {
        let result = (|| -> AppResult<()> {
            let mut source_options = std::fs::OpenOptions::new();
            source_options.read(true);
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                source_options.custom_flags(libc::O_NOFOLLOW);
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                // Darwin fcntl.h: O_NOFOLLOW，避免为 macOS target 引入 Linux-only libc 依赖。
                const DARWIN_O_NOFOLLOW: i32 = 0x0000_0100;
                source_options.custom_flags(DARWIN_O_NOFOLLOW);
            }
            let mut source_file = source_options
                .open(&source)
                .map_err(|error| AppError::generic(format!("打开冲突源文件失败：{error}")))?;
            let source_metadata = source_file
                .metadata()
                .map_err(|error| AppError::generic(format!("读取冲突源句柄失败：{error}")))?;
            if !source_metadata.is_file() {
                return Err(AppError::generic("冲突源句柄不是普通文件"));
            }
            let source_mtime_ms = source_metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok());
            if source_metadata.len() != expected.size || source_mtime_ms != Some(expected.mtime_ms)
            {
                return Err(AppError::generic("复制前冲突源文件已变化"));
            }

            let mut target_file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .map_err(|error| AppError::generic(format!("创建冲突隐藏暂存失败：{error}")))?;
            target_file
                .set_permissions(source_metadata.permissions())
                .map_err(|error| AppError::generic(format!("复制冲突文件权限失败：{error}")))?;
            let copied = std::io::copy(&mut source_file, &mut target_file)
                .map_err(|error| AppError::generic(format!("复制冲突文件失败：{error}")))?;
            if copied != expected.size {
                return Err(AppError::generic(format!(
                    "冲突副本长度不一致：期望 {}，实际 {copied}",
                    expected.size
                )));
            }
            target_file
                .sync_all()
                .map_err(|error| AppError::generic(format!("同步冲突隐藏暂存失败：{error}")))?;

            let after = source_file
                .metadata()
                .map_err(|error| AppError::generic(format!("复核冲突源句柄失败：{error}")))?;
            let after_mtime_ms = after
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok());
            if after.len() != expected.size || after_mtime_ms != Some(expected.mtime_ms) {
                return Err(AppError::generic("复制期间冲突源文件已变化"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        result
    })
    .await
    .map_err(|error| AppError::generic(format!("冲突副本线程异常：{error}")))?
}

/// 本地优先覆盖失败时移除已发布的冗余云端旧版，避免每轮重试累积副本。
async fn remove_redundant_conflict_copy(copy_path: &std::path::Path) -> AppResult<()> {
    match tokio::fs::remove_file(copy_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::generic(format!(
                "清理冗余云端冲突副本失败：{error}"
            )))
        }
    }
    crate::drive::download_api::discard_resume_artifacts(copy_path);
    Ok(())
}

impl SyncExecutor {
    /// 并发执行全部动作。
    /// 对齐 dart `executor.executeAll`。
    pub async fn execute_all(&self, actions: &[SyncAction]) -> Vec<ActionResult> {
        // 历史修剪会修改数据库，因此单独申请活动许可；每个动作取得并发槽后仍会复查。
        if self.db.is_some() {
            let prune_activity = match self.begin_action_activity(None) {
                Ok(activity) => activity,
                Err(error) => {
                    return actions
                        .iter()
                        .map(|_| Self::engine_stopped_result(&error))
                        .collect()
                }
            };
            self.prune_transfer_history();
            drop(prune_activity);
        }

        let concurrency = self.concurrency.max(1) as usize;
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut indexed_results: Vec<(usize, ActionResult)> =
            stream::iter(actions.iter().cloned().enumerate())
                .map(|(action_id, action)| {
                    let sem = semaphore.clone();
                    let executor = self.clone_executor();
                    async move {
                        let execution = std::panic::AssertUnwindSafe(async {
                            let _slot = match sem.acquire_owned().await {
                                Ok(slot) => slot,
                                Err(error) => {
                                    return ActionResult {
                                        success: false,
                                        error_message: Some(format!("执行器已关闭：{error}")),
                                        deferred: true,
                                        cloud_file: None,
                                    }
                                }
                            };
                            // 关闭先封门再等待旧任务，排队动作不得在此后启动回调。
                            let _activity = match executor
                                .begin_action_activity(action.relative_path.as_deref())
                            {
                                Ok(activity) => activity,
                                Err(error) => return Self::engine_stopped_result(&error),
                            };
                            executor.execute_one(&action).await
                        })
                        .catch_unwind()
                        .await;
                        let result = match execution {
                            Ok(result) => result,
                            Err(_) => ActionResult {
                                success: false,
                                error_message: Some(format!(
                                    "动作 {action_id} 执行 panic，已按原身份记录失败"
                                )),
                                deferred: false,
                                cloud_file: None,
                            },
                        };
                        (action_id, result)
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;
        indexed_results.sort_by_key(|(action_id, _)| *action_id);
        indexed_results
            .into_iter()
            .map(|(_, result)| result)
            .collect()
    }

    /// 在动作真正执行前通过引擎活动门。
    fn begin_action_activity(
        &self,
        relative_path: Option<&str>,
    ) -> AppResult<Option<Box<dyn Send>>> {
        self.action_activity_gate
            .as_ref()
            .map(|gate| gate.begin(relative_path))
            .transpose()
    }

    /// 为冲突副本登记路径活动，防止 FUSE 在副本落位期间并发写入。
    fn begin_copy_activity(&self, copy_path: &std::path::Path) -> AppResult<Option<Box<dyn Send>>> {
        let mount = self
            .mount
            .as_ref()
            .ok_or_else(|| AppError::generic("mount manager 未初始化，无法保护冲突副本路径"))?;
        let relative = crate::core::paths::relative_path_from_mount(mount.mount_dir(), copy_path)?;
        self.begin_action_activity(Some(&relative))
    }

    /// 使用 no-replace 原子重命名分配冲突副本；EEXIST 时重新选名，绝不覆盖竞态目标。
    fn rename_to_unique_copy(
        &self,
        source: &std::path::Path,
        local_path: &std::path::Path,
        side_label: &str,
        stamp: &chrono::DateTime<chrono::Utc>,
    ) -> AppResult<(PathBuf, Option<Box<dyn Send>>)> {
        for _ in 0..32 {
            let candidate = crate::sync::conflict::dedupe_copy_path(local_path, side_label, stamp);
            let activity = self.begin_copy_activity(&candidate)?;
            match crate::sync::path_recovery::rename_no_replace(source, &candidate) {
                Ok(()) => return Ok((candidate, activity)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    drop(activity);
                }
                Err(error) => {
                    return Err(AppError::generic(format!("原子创建冲突副本失败：{error}")))
                }
            }
        }
        Err(AppError::generic("冲突副本路径持续被占用，请稍后重试"))
    }

    /// 在原文件同目录分配 watcher/FUSE 都会忽略的冲突下载暂存路径。
    fn allocate_conflict_staging(local_path: &std::path::Path) -> AppResult<PathBuf> {
        let parent = local_path
            .parent()
            .ok_or_else(|| AppError::generic("冲突文件缺少父目录"))?;
        for _ in 0..32 {
            let candidate = parent.join(format!(
                ".hwcloud_conflict-{}-{:016x}",
                std::process::id(),
                rand::random::<u64>()
            ));
            match std::fs::symlink_metadata(&candidate) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
                Ok(_) => continue,
                Err(error) => {
                    return Err(AppError::generic(format!(
                        "检查冲突下载暂存路径失败：{error}"
                    )))
                }
            }
        }
        Err(AppError::generic("无法分配冲突下载暂存路径"))
    }

    /// 将引擎关闭拒绝转为不记录同步失败的延迟结果。
    fn engine_stopped_result(error: &AppError) -> ActionResult {
        ActionResult {
            success: false,
            error_message: Some(error.to_string()),
            // 取消不是同步失败，不应生成 FAILED 兼容基线。
            deferred: true,
            cloud_file: None,
        }
    }

    /// 执行单个动作。
    async fn execute_one(&self, action: &SyncAction) -> ActionResult {
        tracing::debug!(rel = action.relative_path.as_deref(), action_type = ?action.action_type, "executor: 开始执行");

        if matches!(
            action.action_type,
            SyncActionType::Upload | SyncActionType::Download
        ) {
            return self.execute_transfer_action(action).await;
        }

        let result = match action.action_type {
            SyncActionType::Upload | SyncActionType::Download => unreachable!(),
            SyncActionType::CreatePlaceholder => self.do_create_placeholder(action).await,
            SyncActionType::CreateFolder => self.do_create_folder(action).await,
            SyncActionType::MoveInCloud => self.do_move_in_cloud(action).await,
            SyncActionType::DeleteFromCloud => self.do_delete_from_cloud(action).await,
            SyncActionType::DeleteFromLocal => self.do_delete_from_local(action).await,
            SyncActionType::CreateConflictCopy => self.do_conflict(action).await,
            SyncActionType::BackupBeforeCloudDelete => {
                self.do_backup_before_cloud_delete(action).await
            }
            SyncActionType::Skip => ActionResult {
                success: true,
                error_message: Some(action.reason.clone().unwrap_or_default()),
                deferred: false,
                cloud_file: None,
            },
        };

        result
    }

    /// 修剪传输历史（保留最近 100 条已结束任务）。
    fn prune_transfer_history(&self) {
        if let Some(db) = &self.db {
            {
                let conn = db.lock();
                let _ = repository::prune_transfer_history(&conn, 100);
            }
        }
    }

    // ===== 各动作实现 =====

    /// 统一记录动作执行结果日志（成功 info / 失败 warn）。
    ///
    /// 替代各 do_* 方法中重复的 `if result.success { info } else { warn }` 模式。
    /// - `deferred` 为 true 时跳过失败日志（仅 do_upload/do_download 的延迟场景用）
    pub(super) fn log_action_result(
        rel: &str,
        verb_success: &str,
        verb_fail: &str,
        result: &ActionResult,
    ) {
        if result.success {
            tracing::info!(rel, "{verb_success}");
        } else if !result.deferred {
            tracing::warn!(
                rel,
                error = result.error_message.as_deref().unwrap_or("?"),
                "{verb_fail}"
            );
        }
    }

    /// 为云端文件创建本地占位符并记录 cloud-only 基线。
    async fn do_create_placeholder(&self, action: &SyncAction) -> ActionResult {
        // 占位符必须同时具备云端元数据和可定位的相对路径。
        let cloud = match &action.cloud_file {
            Some(c) => c,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少云端文件元数据".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let rel_path = match &action.relative_path {
            Some(p) => p,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少相对路径".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        // 先确保磁盘占位符，再写入 cloud-only 基线。
        if let Some(m) = &self.mount {
            match m
                .create_placeholder_if_needed(rel_path, &cloud.id, cloud.size)
                .await
            {
                Ok(()) => {
                    // 对齐 dart：写 DB 记录（status=cloudOnly），防止孤儿占位符
                    if let (Some(db), Some(local_path)) = (&self.db, &action.local_path) {
                        {
                            let conn = db.lock();
                            let _ = repository::upsert(
                                &conn,
                                &SyncItem {
                                    file_id: cloud.id.clone(),
                                    local_path: local_path.clone(),
                                    parent_folder_id: cloud
                                        .parent_folder
                                        .as_ref()
                                        .and_then(|v| v.first().cloned()),
                                    name: cloud.name.clone(),
                                    is_folder: false,
                                    size: cloud.size,
                                    local_size: None,
                                    sha256: None,
                                    local_mtime: None,
                                    cloud_edited_time: cloud
                                        .edited_time
                                        .map(|t| t.timestamp_millis()),
                                    last_sync_time: None,
                                    status: sync_status::CLOUD_ONLY,
                                    error_message: None,
                                },
                            );
                        }
                    }
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    }
                }
                Err(e) => ActionResult {
                    success: false,
                    error_message: Some(e.to_string()),
                    deferred: false,
                    cloud_file: None,
                },
            }
        } else {
            ActionResult {
                success: false,
                error_message: Some("mount manager 未初始化".into()),
                deferred: false,
                cloud_file: None,
            }
        }
    }

    /// 确保本地目录存在，或在云端安全创建新目录。
    async fn do_create_folder(&self, action: &SyncAction) -> ActionResult {
        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 本地新文件夹（无云端文件）→ 调 createFolder API
        let result = if let Some(cloud_file) = &action.cloud_file {
            // 云端已有文件夹 → 本地 ensure
            if !cloud_file.is_folder() {
                return ActionResult {
                    success: false,
                    error_message: Some("创建本地目录动作携带的云端资源不是文件夹".into()),
                    deferred: false,
                    cloud_file: None,
                };
            }
            if let Some(m) = &self.mount {
                match m.ensure_folder_with_file_id(rel, &cloud_file.id) {
                    Ok(_) => ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    },
                    Err(e) => ActionResult {
                        success: false,
                        error_message: Some(e.to_string()),
                        deferred: false,
                        cloud_file: None,
                    },
                }
            } else {
                ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
        } else {
            // 云端文件名只取相对路径最后一段，避免路径分隔符触发接口校验失败。
            // 之前 name = "学习/程序设计"（含 /）撞华为文件名校验 → 400 21004002
            // 该取值与引擎写入数据库时提取末段名称的规则保持一致。
            let full = action.relative_path.as_deref().unwrap_or("新建文件夹");
            let name = full.rsplit('/').next().unwrap_or(full);

            // FilesApi 内部统一执行父目录范围查重、严格响应核验和响应丢失后的唯一收敛；
            // 列表失败时会 fail closed，绝不会继续发送非幂等 POST。
            match self
                .files_api
                .create_folder(name, action.parent_file_id.as_deref())
                .await
            {
                Ok(f) => {
                    // 远端身份确认后立即回填本地目录 xattr。若本地已被另一身份占用，
                    // 远端结果由下轮查重收敛，但本轮不得写入错误基线。
                    if let Some(mount) = &self.mount {
                        if let Err(error) = mount.ensure_folder_with_file_id(rel, &f.id) {
                            ActionResult {
                                success: false,
                                error_message: Some(error.to_string()),
                                deferred: true,
                                cloud_file: Some(f),
                            }
                        } else {
                            ActionResult {
                                success: true,
                                error_message: None,
                                deferred: false,
                                cloud_file: Some(f),
                            }
                        }
                    } else {
                        ActionResult {
                            success: true,
                            error_message: None,
                            deferred: false,
                            cloud_file: Some(f),
                        }
                    }
                }
                Err(e) => ActionResult {
                    success: false,
                    error_message: Some(e.to_string()),
                    deferred: false,
                    cloud_file: None,
                },
            }
        };
        Self::log_action_result(rel, "创建目录成功", "创建目录失败", &result);
        result
    }

    /// 在本地身份与目标去重校验通过后更新云端文件路径。
    async fn do_move_in_cloud(&self, action: &SyncAction) -> ActionResult {
        // 所有不确定前置条件均延迟重规划，不直接标记永久失败。
        let deferred = |technical_message: String| {
            let user_message = crate::sync::user_messages::simplify_sync_error(&technical_message);
            tracing::info!(
                technical_reason = %technical_message,
                user_message = %user_message,
                "云端路径变更等待重新检查"
            );
            ActionResult {
                success: false,
                error_message: Some(user_message.into_owned()),
                deferred: true,
                cloud_file: None,
            }
        };
        // 远端身份、目标父目录和本地路径缺一不可。
        let Some(file_id) = action.file_id.as_deref() else {
            return deferred("云端路径变更缺少 fileId，等待重新规划".to_string());
        };
        let Some(target_parent) = action.parent_file_id.as_deref() else {
            return deferred("云端路径变更的目标父目录尚未取得 fileId，等待重新规划".to_string());
        };
        let Some(relative_path) = action.relative_path.as_deref() else {
            return deferred("云端路径变更缺少目标相对路径，等待重新规划".to_string());
        };
        let Some(local_path) = action.local_path.as_deref().map(PathBuf::from) else {
            return deferred("云端路径变更缺少本地路径，等待重新规划".to_string());
        };
        let Some(planned_cloud_file) = action.cloud_file.as_ref() else {
            return deferred("云端路径变更缺少规划时的稳定身份与类型，等待重新规划".to_string());
        };
        if planned_cloud_file.id != file_id {
            return deferred("云端路径变更的规划身份与 fileId 不一致，等待重新规划".to_string());
        }

        // 写入前确认目标仍携带同一远端身份；本地再次移动时等待重新规划。
        let local_identity = std::fs::symlink_metadata(&local_path)
            .ok()
            .filter(|metadata| {
                !metadata.file_type().is_symlink()
                    && if planned_cloud_file.is_folder() {
                        metadata.is_dir()
                    } else {
                        metadata.is_file()
                    }
            })
            .and_then(|_| {
                crate::platform::xattr::get(&local_path, crate::mount::manager::XATTR_FILE_ID)
                    .ok()
                    .flatten()
            })
            .and_then(|bytes| String::from_utf8(bytes).ok());
        if local_identity.as_deref() != Some(file_id) {
            return deferred(
                "云端路径变更执行前本地路径或 fileId 已变化，等待重新规划".to_string(),
            );
        }

        // 目标名称来自规范化相对路径的末段。
        let target_name = relative_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(relative_path);
        // 写入前列出目标目录，存在同名不同 ID 时禁止覆盖。
        let target_files = match self.files_api.list_all(Some(target_parent)).await {
            Ok(files) => files,
            Err(error) => {
                return deferred(format!("核验移动目标目录失败，未发送远端写入：{error}"))
            }
        };
        if target_files
            .iter()
            .any(|file| file.id.as_str() != file_id && file.name.as_str() == target_name)
        {
            return deferred(format!(
                "目标目录已存在同名云端文件 {target_name}，拒绝覆盖并等待重新规划"
            ));
        }

        // 写入后按 fileId 复核名称和父目录，使响应丢失的路径变更仍可幂等收敛。
        let result = match self
            .files_api
            .update(file_id, Some(target_name), Some(target_parent), None)
            .await
        {
            Ok(file)
                if file.id == file_id
                    && file.is_folder() == planned_cloud_file.is_folder() =>
            {
                ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: Some(file),
                }
            }
            Ok(_) => deferred(
                "云端路径变更响应的稳定身份或文件类型不一致，等待重新核验".to_string(),
            ),
            Err(error) => match self.files_api.get(file_id).await {
                Ok(file)
                    if file.id.as_str() == file_id
                        && file.is_folder() == planned_cloud_file.is_folder()
                        && file.name.as_str() == target_name
                        && file.parent_folder.as_deref().is_some_and(|parents| {
                            parents.len() == 1 && parents[0].as_str() == target_parent
                        }) =>
                {
                    tracing::info!(
                        file_id,
                        target_parent,
                        target_name,
                        %error,
                        "路径变更响应不确定，但 fileId GET 已确认目标名称与父目录"
                    );
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: Some(file),
                    }
                }
                Ok(_) => deferred(format!(
                    "远端路径变更尚未生效，保留原基线等待重新规划：{error}"
                )),
                Err(verification_error) => deferred(format!(
                    "远端路径变更结果不确定，保留原基线等待重新规划：{error}；核验失败：{verification_error}"
                )),
            },
        };
        Self::log_action_result(
            relative_path,
            "更新云端文件路径成功",
            "更新云端文件路径失败",
            &result,
        );
        result
    }

    /// 回收云端文件，并在响应不确定时核实真实结果。
    async fn do_delete_from_cloud(&self, action: &SyncAction) -> ActionResult {
        let file_id = match &action.file_id {
            Some(id) => id.clone(),
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("缺少 fileId".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        let result = match self.files_api.delete(&file_id).await {
            Ok(()) => ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            },
            Err(e) => match self.files_api.verify_deleted(&file_id).await {
                Ok(true) => {
                    tracing::info!(
                        rel,
                        file_id,
                        "删除响应不确定，但 fileId 核验已确认回收/不存在"
                    );
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: None,
                    }
                }
                Ok(false) => ActionResult {
                    success: false,
                    error_message: Some(format!("{e}；远端核验显示文件仍未回收")),
                    deferred: false,
                    cloud_file: None,
                },
                Err(verification_error) => ActionResult {
                    success: false,
                    error_message: Some(format!("{e}；删除结果核验失败：{verification_error}")),
                    deferred: true,
                    cloud_file: None,
                },
            },
        };
        Self::log_action_result(rel, "删除云端文件成功", "删除云端文件失败", &result);
        result
    }

    /// 按冲突决策保留正本并创建败方副本。
    async fn do_conflict(&self, action: &SyncAction) -> ActionResult {
        let local_path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突处理缺少本地路径".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let cloud_file = match &action.cloud_file {
            Some(c) => c,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突处理缺少云端文件元数据".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };

        // 冲突副本和下载替换都绑定同一份严格源快照，禁止用“当前时间”掩盖 stat 失败。
        let (local_mtime, local_snapshot) = match inspect_conflict_source(&local_path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ActionResult {
                    success: false,
                    error_message: Some(error.to_string()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };

        // 解析冲突
        let mut resolution = if let Some(conflict) = &self.conflict {
            if let Ok(mut resolver) = conflict.lock() {
                resolver.resolve(&local_path, cloud_file, &local_mtime)
            } else {
                return ActionResult {
                    success: false,
                    error_message: Some("冲突解决器获取失败".into()),
                    deferred: false,
                    cloud_file: None,
                };
            }
        } else {
            return ActionResult {
                success: false,
                error_message: Some("冲突解决器未初始化".into()),
                deferred: false,
                cloud_file: None,
            };
        };

        let rel = action.relative_path.as_deref().unwrap_or("?");
        // 对齐 dart：cloud-wins → 本地副本保存到 copyPath，云端下载到 localPath
        // 本地胜出时先把云端副本下载到冲突副本路径，再用本地内容覆盖云端。
        let result = match resolution.winner {
            crate::sync::conflict::ConflictSide::Cloud => {
                // 云端获胜：主文件保持原位，先复制到隐藏 staging 并清空身份，随后才原子
                // 发布普通副本。任何崩溃点都不会让可见副本携带原 fileId。
                match Self::allocate_conflict_staging(&local_path) {
                    Err(error) => ActionResult {
                        success: false,
                        error_message: Some(format!("分配本地冲突副本暂存路径失败：{error}")),
                        deferred: false,
                        cloud_file: None,
                    },
                    Ok(staging_path) => {
                        if let Err(error) = copy_to_hidden_conflict_staging(
                            &local_path,
                            &staging_path,
                            &local_snapshot,
                        )
                        .await
                        {
                            ActionResult {
                                success: false,
                                error_message: Some(format!(
                                    "创建隐藏本地冲突副本失败，主文件保持不变：{error}"
                                )),
                                deferred: false,
                                cloud_file: None,
                            }
                        } else {
                            let identity_cleanup = match &self.mount {
                                Some(mount) => mount.clear_placeholder_xattr(&staging_path).await,
                                None => Err(AppError::generic(
                                    "冲突副本已暂存，但 mount manager 未初始化",
                                )),
                            };
                            if let Err(error) = identity_cleanup {
                                let cleanup = remove_redundant_conflict_copy(&staging_path).await;
                                ActionResult {
                                    success: false,
                                    error_message: Some(match cleanup {
                                        Ok(()) => format!(
                                            "隐藏冲突副本身份清理失败，主文件保持不变：{error}"
                                        ),
                                        Err(cleanup_error) => format!(
                                            "隐藏冲突副本身份清理失败，主文件保持不变：{error}；{cleanup_error}"
                                        ),
                                    }),
                                    deferred: false,
                                    cloud_file: None,
                                }
                            } else {
                                match self.rename_to_unique_copy(
                                    &staging_path,
                                    &local_path,
                                    "本地副本",
                                    &local_mtime,
                                ) {
                                    Err(error) => {
                                        let cleanup =
                                            remove_redundant_conflict_copy(&staging_path).await;
                                        ActionResult {
                                            success: false,
                                            error_message: Some(match cleanup {
                                                Ok(()) => format!(
                                                    "发布本地冲突副本失败，主文件保持不变：{error}"
                                                ),
                                                Err(cleanup_error) => format!(
                                                    "发布本地冲突副本失败，主文件保持不变：{error}；{cleanup_error}"
                                                ),
                                            }),
                                            deferred: false,
                                            cloud_file: None,
                                        }
                                    }
                                    Ok((copy_path, copy_activity)) => {
                                        resolution.copy_path = copy_path;
                                        let _copy_activity = copy_activity;
                                        let expectation = DownloadExpectation {
                                            edited_time_ms: cloud_file
                                                .edited_time
                                                .map(|time| time.timestamp_millis()),
                                            size: u64::try_from(cloud_file.size).ok(),
                                            content_hash: cloud_file.content_hash.clone(),
                                            // main 从未移动；只允许替换仍与冲突决策时相同的原文件。
                                            destination_snapshot: Some(local_snapshot.clone()),
                                            placeholder_file_id: None,
                                        };
                                        match self
                                            .download_api
                                            .download_with_expectation(
                                                &cloud_file.id,
                                                &local_path,
                                                Some(&expectation),
                                                None,
                                            )
                                            .await
                                        {
                                            Ok(()) => {
                                                let finalized: AppResult<()> = async {
                                                    let mount =
                                                        self.mount.as_ref().ok_or_else(|| {
                                                            AppError::generic(
                                                                "冲突下载已完成，但 mount manager 未初始化",
                                                            )
                                                        })?;
                                                    // 下载会原子替换 inode，先恢复稳定身份，再发布 downloaded。
                                                    mount
                                                        .set_file_id_xattr(
                                                            &local_path,
                                                            &cloud_file.id,
                                                        )
                                                        .await?;
                                                    mount.mark_downloaded(&local_path).await?;
                                                    Ok(())
                                                }
                                                .await;
                                                match finalized {
                                                    Ok(()) => ActionResult {
                                                        success: true,
                                                        error_message: None,
                                                        deferred: false,
                                                        cloud_file: None,
                                                    },
                                                    Err(error) => ActionResult {
                                                        success: false,
                                                        error_message: Some(format!(
                                                            "云端冲突版本已下载，但本地身份结算失败：{error}"
                                                        )),
                                                        deferred: false,
                                                        cloud_file: None,
                                                    },
                                                }
                                            }
                                            Err(error) => {
                                                // main 仍是原内容，已发布副本此时只是冗余；
                                                // 趁 copy activity 仍持有时删除，避免重试累积副本。
                                                let cleanup = remove_redundant_conflict_copy(
                                                    &resolution.copy_path,
                                                )
                                                .await;
                                                ActionResult {
                                                    success: false,
                                                    error_message: Some(match cleanup {
                                                        Ok(()) => error.to_string(),
                                                        Err(cleanup_error) => {
                                                            format!("{error}；{cleanup_error}")
                                                        }
                                                    }),
                                                    deferred: false,
                                                    cloud_file: None,
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            crate::sync::conflict::ConflictSide::Local => {
                // 本地获胜：先下载云端旧版到隐藏暂存，再用 no-replace 发布可见副本。
                // 只有副本已安全落位，才允许覆盖云端。
                match Self::allocate_conflict_staging(&local_path) {
                    Err(error) => ActionResult {
                        success: false,
                        error_message: Some(format!("分配云端冲突副本暂存路径失败：{error}")),
                        deferred: false,
                        cloud_file: None,
                    },
                    Ok(staging_path) => {
                        let expectation = DownloadExpectation {
                            edited_time_ms: cloud_file
                                .edited_time
                                .map(|time| time.timestamp_millis()),
                            size: u64::try_from(cloud_file.size).ok(),
                            content_hash: cloud_file.content_hash.clone(),
                            destination_snapshot: None,
                            placeholder_file_id: Some(cloud_file.id.clone()),
                        };
                        if let Err(error) = self
                            .download_api
                            .download_with_expectation(
                                &cloud_file.id,
                                &staging_path,
                                Some(&expectation),
                                None,
                            )
                            .await
                        {
                            crate::drive::download_api::discard_resume_artifacts(&staging_path);
                            let cleanup = match tokio::fs::remove_file(&staging_path).await {
                                Ok(()) => None,
                                Err(cleanup_error)
                                    if cleanup_error.kind() == std::io::ErrorKind::NotFound =>
                                {
                                    None
                                }
                                Err(cleanup_error) => Some(cleanup_error),
                            };
                            ActionResult {
                                success: false,
                                error_message: Some(match cleanup {
                                    Some(cleanup_error) => format!(
                                        "冲突副本（云端旧版）下载失败，跳过覆盖以保云端旧版：{error}；清理隐藏暂存失败：{cleanup_error}"
                                    ),
                                    None => format!(
                                        "冲突副本（云端旧版）下载失败，跳过覆盖以保云端旧版：{error}"
                                    ),
                                }),
                                deferred: false,
                                cloud_file: None,
                            }
                        } else {
                            let cleanup = match &self.mount {
                                Some(mount) => mount.clear_placeholder_xattr(&staging_path).await,
                                None => Err(AppError::generic(
                                    "冲突副本已下载，但 mount manager 未初始化",
                                )),
                            };
                            if let Err(error) = cleanup {
                                let remove_error =
                                    tokio::fs::remove_file(&staging_path).await.err();
                                crate::drive::download_api::discard_resume_artifacts(&staging_path);
                                ActionResult {
                                    success: false,
                                    error_message: Some(match remove_error {
                                        Some(remove_error) => format!(
                                            "冲突副本身份清理失败，未覆盖云端：{error}；清理隐藏暂存失败：{remove_error}"
                                        ),
                                        None => {
                                            format!("冲突副本身份清理失败，未覆盖云端：{error}")
                                        }
                                    }),
                                    deferred: false,
                                    cloud_file: None,
                                }
                            } else {
                                let cloud_time =
                                    cloud_file.edited_time.unwrap_or_else(chrono::Utc::now);
                                match self.rename_to_unique_copy(
                                    &staging_path,
                                    &local_path,
                                    "云端副本",
                                    &cloud_time,
                                ) {
                                    Err(error) => {
                                        let remove_error =
                                            tokio::fs::remove_file(&staging_path).await.err();
                                        crate::drive::download_api::discard_resume_artifacts(
                                            &staging_path,
                                        );
                                        ActionResult {
                                            success: false,
                                            error_message: Some(match remove_error {
                                                Some(remove_error) => format!(
                                                    "发布云端冲突副本失败，未覆盖云端：{error}；清理隐藏暂存失败：{remove_error}"
                                                ),
                                                None => format!(
                                                    "发布云端冲突副本失败，未覆盖云端：{error}"
                                                ),
                                            }),
                                            deferred: false,
                                            cloud_file: None,
                                        }
                                    }
                                    Ok((copy_path, copy_activity)) => {
                                        resolution.copy_path = copy_path;
                                        let _copy_activity = copy_activity;
                                        let parent_id = cloud_file
                                            .parent_folder
                                            .as_ref()
                                            .and_then(|v| v.first().map(|s| s.as_str()));
                                        match self
                                            .upload_api
                                            .upload_update(
                                                &cloud_file.id,
                                                &local_path,
                                                parent_id,
                                                None,
                                            )
                                            .await
                                        {
                                            Ok(updated_cloud_file) => ActionResult {
                                                success: true,
                                                error_message: None,
                                                deferred: false,
                                                // 覆盖上传会产生新的 editedTime/hash/size；
                                                // 结算必须采用 PATCH 返回的新版本，不能沿用旧冲突快照。
                                                cloud_file: Some(updated_cloud_file),
                                            },
                                            Err(error) => {
                                                // PATCH 失败时远端旧版仍是正本；已下载的旧版副本
                                                // 只是冗余。趁 copy activity 仍在持有时删除，防止
                                                // 下一轮重试持续生成带序号的重复副本。
                                                let cleanup = remove_redundant_conflict_copy(
                                                    &resolution.copy_path,
                                                )
                                                .await;
                                                ActionResult {
                                                    success: false,
                                                    error_message: Some(match cleanup {
                                                        Ok(()) => error.to_string(),
                                                        Err(cleanup_error) => {
                                                            format!("{error}；{cleanup_error}")
                                                        }
                                                    }),
                                                    deferred: false,
                                                    cloud_file: None,
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        Self::log_action_result(rel, "冲突处理完成", "冲突处理失败", &result);
        result
    }

    /// 云端已删除但本地有未上传修改：改名备份副本（保内容），原路径腾空即满足云端删除。
    /// 副本清掉占位 xattr，下轮作为全新本地文件上传（救援用户改动）。
    async fn do_backup_before_cloud_delete(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            }
            Err(error) => {
                return ActionResult {
                    success: false,
                    error_message: Some(format!("检查删除冲突源文件失败：{error}")),
                    deferred: false,
                    cloud_file: None,
                }
            }
        }
        let mount = match self.mount.as_ref() {
            Some(mount) => mount,
            None => {
                return ActionResult {
                    success: false,
                    error_message: Some("mount manager 未初始化，未改动本地原文件".into()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let (local_mtime, local_snapshot) = match inspect_conflict_source(&path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ActionResult {
                    success: false,
                    error_message: Some(error.to_string()),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        let staging_path = match Self::allocate_conflict_staging(&path) {
            Ok(staging_path) => staging_path,
            Err(error) => {
                return ActionResult {
                    success: false,
                    error_message: Some(format!("分配删除冲突隐藏暂存失败：{error}")),
                    deferred: false,
                    cloud_file: None,
                }
            }
        };
        if let Err(error) =
            copy_to_hidden_conflict_staging(&path, &staging_path, &local_snapshot).await
        {
            return ActionResult {
                success: false,
                error_message: Some(format!("创建删除冲突隐藏副本失败，原文件保持不变：{error}")),
                deferred: false,
                cloud_file: None,
            };
        }
        if let Err(error) = mount.clear_placeholder_xattr(&staging_path).await {
            let cleanup = remove_redundant_conflict_copy(&staging_path).await;
            return ActionResult {
                success: false,
                error_message: Some(match cleanup {
                    Ok(()) => format!("删除冲突副本身份清理失败，原文件保持不变：{error}"),
                    Err(cleanup_error) => {
                        format!("删除冲突副本身份清理失败：{error}；{cleanup_error}")
                    }
                }),
                deferred: false,
                cloud_file: None,
            };
        }
        let (copy_path, copy_activity) =
            match self.rename_to_unique_copy(&staging_path, &path, "本地副本", &local_mtime) {
                Ok(published) => published,
                Err(error) => {
                    let cleanup = remove_redundant_conflict_copy(&staging_path).await;
                    return ActionResult {
                        success: false,
                        error_message: Some(match cleanup {
                            Ok(()) => format!("发布删除冲突副本失败，原文件保持不变：{error}"),
                            Err(cleanup_error) => format!("{error}；{cleanup_error}"),
                        }),
                        deferred: false,
                        cloud_file: None,
                    };
                }
            };
        let _copy_activity = copy_activity;

        // 发布副本后最后一次复核主文件；只有它仍与复制来源一致才可删除。
        if let Err(error) = verify_conflict_source_snapshot(&path, &local_snapshot) {
            let cleanup = remove_redundant_conflict_copy(&copy_path).await;
            return ActionResult {
                success: false,
                error_message: Some(match cleanup {
                    Ok(()) => format!("{error}；已移除冗余副本，原文件保持不变"),
                    Err(cleanup_error) => format!("{error}；{cleanup_error}"),
                }),
                deferred: false,
                cloud_file: None,
            };
        }
        if let Err(error) = tokio::fs::remove_file(&path).await {
            let cleanup = remove_redundant_conflict_copy(&copy_path).await;
            return ActionResult {
                success: false,
                error_message: Some(match cleanup {
                    Ok(()) => format!("删除原路径失败，已移除冗余副本：{error}"),
                    Err(cleanup_error) => format!("删除原路径失败：{error}；{cleanup_error}"),
                }),
                deferred: false,
                cloud_file: None,
            };
        }
        tracing::info!(
            src = %path.display(),
            backup = %copy_path.display(),
            "云端删除但本地有未上传修改，已备份普通副本并移除原云端身份路径"
        );
        ActionResult {
            success: true,
            error_message: None,
            deferred: false,
            cloud_file: None,
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use crate::mount::manager::{
        MountManager, STATE_DOWNLOADED, XATTR_FILE_ID, XATTR_SIZE, XATTR_STATE,
    };

    /// 副本先在隐藏路径落盘并清身份，发布后主文件仍保有身份而可见副本绝不保有。
    #[tokio::test]
    async fn hidden_copy_is_identity_free_before_visible_publication() {
        let temp = tempfile::tempdir().unwrap();
        let mount = MountManager::new(temp.path());
        let main = temp.path().join("file.txt");
        let staging = temp.path().join(".hwcloud_conflict-test");
        let visible = temp.path().join("file (本地副本).txt");
        std::fs::write(&main, b"local-version").unwrap();
        for (key, value) in [
            (XATTR_FILE_ID, b"cloud-file-id".as_slice()),
            (XATTR_STATE, STATE_DOWNLOADED.as_bytes()),
            (XATTR_SIZE, b"13".as_slice()),
        ] {
            crate::platform::xattr::set(&main, key, value).unwrap();
        }
        let (_, snapshot) = inspect_conflict_source(&main).unwrap();

        copy_to_hidden_conflict_staging(&main, &staging, &snapshot)
            .await
            .unwrap();
        assert!(staging
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".hwcloud_"));
        assert!(!visible.exists(), "身份清理前不得存在可见冲突副本");

        mount.clear_placeholder_xattr(&staging).await.unwrap();
        crate::sync::path_recovery::rename_no_replace(&staging, &visible).unwrap();

        assert_eq!(std::fs::read(&main).unwrap(), b"local-version");
        assert_eq!(std::fs::read(&visible).unwrap(), b"local-version");
        assert_eq!(
            crate::platform::xattr::get(&main, XATTR_FILE_ID)
                .unwrap()
                .as_deref(),
            Some(b"cloud-file-id".as_slice())
        );
        assert_eq!(
            crate::platform::xattr::get(&main, XATTR_STATE)
                .unwrap()
                .as_deref(),
            Some(STATE_DOWNLOADED.as_bytes())
        );
        for key in [XATTR_FILE_ID, XATTR_STATE, XATTR_SIZE] {
            assert_eq!(
                crate::platform::xattr::get(&visible, key).unwrap(),
                None,
                "可见冲突副本不得携带 {key}"
            );
        }
    }

    /// 源文件在快照后变化时不得生成可发布的隐藏副本。
    #[tokio::test]
    async fn hidden_copy_rejects_changed_source_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("file.txt");
        let staging = temp.path().join(".hwcloud_conflict-test");
        std::fs::write(&main, b"version-one").unwrap();
        let (_, snapshot) = inspect_conflict_source(&main).unwrap();
        std::fs::write(&main, b"a-different-longer-version").unwrap();

        let error = copy_to_hidden_conflict_staging(&main, &staging, &snapshot)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("源文件已变化"));
        assert!(!staging.exists());
    }

    /// 覆盖上传失败后的云端旧版副本应被移除，后续重试不会累积重复副本。
    #[tokio::test]
    async fn failed_local_wins_upload_removes_redundant_copy() {
        let temp = tempfile::tempdir().unwrap();
        let copy = temp.path().join("file (云端副本).txt");
        std::fs::write(&copy, b"remote-old-version").unwrap();

        remove_redundant_conflict_copy(&copy).await.unwrap();

        assert!(!copy.exists());
    }
}
