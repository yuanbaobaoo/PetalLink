//! 同步动作的并发编排与非传输动作实现。

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::{stream, FutureExt, StreamExt};
use tokio::sync::Semaphore;

use crate::data::repository::{self, sync_status, SyncItem};
use crate::drive::download_api::DownloadExpectation;
use crate::error::{AppError, AppResult};
use crate::sync::state::{ActionResult, SyncAction, SyncActionType};

use super::SyncExecutor;

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
            let _cloud = cloud_file;
            if let Some(m) = &self.mount {
                match m.ensure_folder(rel) {
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
                Ok(f) => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: Some(f),
                },
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

    /// 在本地身份与目标去重校验通过后移动云端文件。
    async fn do_move_in_cloud(&self, action: &SyncAction) -> ActionResult {
        let deferred = |message: String| ActionResult {
            success: false,
            error_message: Some(message),
            deferred: true,
            cloud_file: None,
        };
        let Some(file_id) = action.file_id.as_deref() else {
            return deferred("跨目录移动缺少 fileId，等待重新规划".to_string());
        };
        let Some(target_parent) = action.parent_file_id.as_deref() else {
            return deferred("跨目录移动的目标父目录尚未取得 fileId，等待重新规划".to_string());
        };
        let Some(relative_path) = action.relative_path.as_deref() else {
            return deferred("跨目录移动缺少目标相对路径，等待重新规划".to_string());
        };
        let Some(local_path) = action.local_path.as_deref().map(PathBuf::from) else {
            return deferred("跨目录移动缺少本地路径，等待重新规划".to_string());
        };

        // 写入前确认目标仍携带同一远端身份；本地再次移动时等待重新规划。
        let local_identity = std::fs::metadata(&local_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|_| {
                xattr::get(&local_path, crate::mount::manager::XATTR_FILE_ID)
                    .ok()
                    .flatten()
            })
            .and_then(|bytes| String::from_utf8(bytes).ok());
        if local_identity.as_deref() != Some(file_id) {
            return deferred("跨目录移动执行前本地路径或 fileId 已变化，等待重新规划".to_string());
        }

        let target_name = relative_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(relative_path);
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

        // 更新前先读取当前父目录，使已提交但响应丢失的重试收敛为幂等改名。
        let result = match self
            .files_api
            .update(file_id, Some(target_name), Some(target_parent), None)
            .await
        {
            Ok(file) => ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: Some(file),
            },
            Err(error) => match self.files_api.get(file_id).await {
                Ok(file)
                    if file.id.as_str() == file_id
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
                        "移动响应不确定，但 fileId GET 已确认目标名称与父目录"
                    );
                    ActionResult {
                        success: true,
                        error_message: None,
                        deferred: false,
                        cloud_file: Some(file),
                    }
                }
                Ok(_) => deferred(format!(
                    "远端跨目录移动尚未生效，保留原基线等待重新规划：{error}"
                )),
                Err(verification_error) => deferred(format!(
                    "远端跨目录移动结果不确定，保留原基线等待重新规划：{error}；核验失败：{verification_error}"
                )),
            },
        };
        Self::log_action_result(
            relative_path,
            "移动云端文件成功",
            "移动云端文件失败",
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

        // 获取本地 mtime
        let local_mtime = tokio::fs::metadata(&local_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .unwrap_or(chrono::Utc::now())
            })
            .unwrap_or(chrono::Utc::now());

        // 解析冲突
        let resolution = if let Some(conflict) = &self.conflict {
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
                    let expectation = DownloadExpectation {
                        edited_time_ms: cloud_file.edited_time.map(|time| time.timestamp_millis()),
                        size: u64::try_from(cloud_file.size).ok(),
                        content_hash: cloud_file.content_hash.clone(),
                        destination_snapshot: None,
                        placeholder_file_id: Some(cloud_file.id.clone()),
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
                            if let Some(m) = &self.mount {
                                let _ = m.mark_downloaded(&local_path).await;
                            }
                            if let Some(m) = &self.mount {
                                let _ = m.clear_placeholder_xattr(&resolution.copy_path).await;
                            }
                            ActionResult {
                                success: true,
                                error_message: None,
                                deferred: false,
                                cloud_file: None,
                            }
                        }
                        Err(e) => {
                            let _ = tokio::fs::rename(&resolution.copy_path, &local_path).await;
                            ActionResult {
                                success: false,
                                error_message: Some(e.to_string()),
                                deferred: false,
                                cloud_file: None,
                            }
                        }
                    }
                }
            }
            crate::sync::conflict::ConflictSide::Local => {
                // 本地获胜：下载云端旧版 → copyPath（败方副本），上传本地覆盖云端。
                let expectation = DownloadExpectation {
                    edited_time_ms: cloud_file.edited_time.map(|time| time.timestamp_millis()),
                    size: u64::try_from(cloud_file.size).ok(),
                    content_hash: cloud_file.content_hash.clone(),
                    destination_snapshot: None,
                    placeholder_file_id: Some(cloud_file.id.clone()),
                };
                if let Err(e) = self
                    .download_api
                    .download_with_expectation(
                        &cloud_file.id,
                        &resolution.copy_path,
                        Some(&expectation),
                        None,
                    )
                    .await
                {
                    ActionResult {
                        success: false,
                        error_message: Some(format!(
                            "冲突副本（云端旧版）下载失败，跳过覆盖以保云端旧版：{e}"
                        )),
                        deferred: false,
                        cloud_file: None,
                    }
                } else {
                    if let Some(m) = &self.mount {
                        let _ = m.clear_placeholder_xattr(&resolution.copy_path).await;
                    }
                    let parent_id = cloud_file
                        .parent_folder
                        .as_ref()
                        .and_then(|v| v.first().map(|s| s.as_str()));
                    match self
                        .upload_api
                        .upload_update(&cloud_file.id, &local_path, parent_id, None)
                        .await
                    {
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
        if !path.exists() {
            return ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            };
        }
        // 本地 mtime 作为副本时间戳（败方=本地的修改时间）
        let local_mtime = tokio::fs::metadata(&path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                chrono::DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .unwrap_or(chrono::Utc::now())
            })
            .unwrap_or_else(chrono::Utc::now);
        let copy_path = crate::sync::conflict::dedupe_copy_path(&path, "本地副本", &local_mtime);
        match tokio::fs::rename(&path, &copy_path).await {
            Ok(()) => {
                if let Some(m) = &self.mount {
                    let _ = m.clear_placeholder_xattr(&copy_path).await;
                }
                tracing::info!(
                    src = %path.display(),
                    backup = %copy_path.display(),
                    "云端删除但本地有未上传修改，已备份副本"
                );
                ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
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
