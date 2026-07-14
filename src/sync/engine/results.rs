//! 同步结果结算。

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::executor::SyncExecutor;

use super::action_filters::fill_parent_file_ids;
use super::SyncEngine;

impl SyncEngine {
    /// 将动作执行结果结算到数据库与内存缓存。
    /// 先提交持久化基线，再发布内存缓存变化。
    pub fn apply_results(
        &self,
        actions: &[crate::sync::state::SyncAction],
        results: &[crate::sync::state::ActionResult],
    ) -> AppResult<()> {
        use crate::sync::state::SyncActionType;

        // 修改缓存前先捕获显式目录子树并结算数据库记录；仅从内存移除结算成功的根路径。
        let delete_subtrees: std::collections::HashMap<String, (bool, Vec<String>)> = {
            let cloud = self.cloud_tree.lock();
            actions
                .iter()
                .zip(results.iter())
                .filter(|(action, result)| {
                    result.success && action.action_type == SyncActionType::DeleteFromCloud
                })
                .filter_map(|(action, _)| {
                    let root = action.relative_path.as_ref()?;
                    let prefix = format!("{root}/");
                    let is_directory = cloud.get(root).is_some_and(|file| file.is_folder());
                    let paths = if is_directory {
                        cloud
                            .keys()
                            .filter(|path| *path == root || path.starts_with(&prefix))
                            .cloned()
                            .collect()
                    } else {
                        vec![root.clone()]
                    };
                    Some((root.clone(), (is_directory, paths)))
                })
                .collect()
        };
        let mut settled_cloud_deletes = std::collections::HashSet::new();

        // 修改内存缓存前先更新持久化基线。
        let conn = self.db.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始同步结果结算事务失败：{error}")))?;
        let move_baselines = if actions
            .iter()
            .any(|action| action.action_type == SyncActionType::MoveInCloud)
        {
            repository::load_all(&transaction)?
        } else {
            Vec::new()
        };
        for (action, result) in actions.iter().zip(results.iter()) {
            let Some(rel) = &action.relative_path else {
                continue;
            };

            // 删除/备份动作成功 → 清 DB 记录（按 local_path；file_id 可选，覆盖"双方都删清理"file_id=None 场景）
            // BackupBeforeCloudDelete：原文件改名走，原路径腾空 + 云端已删 → 同样清掉原 DB 记录，
            // 让下轮该路径「全缺席」无动作；副本是全新路径，下轮正常 Upload。
            if result.success
                && matches!(
                    action.action_type,
                    SyncActionType::DeleteFromCloud
                        | SyncActionType::DeleteFromLocal
                        | SyncActionType::BackupBeforeCloudDelete
                )
            {
                let fid = action.file_id.as_deref().unwrap_or("");
                let delete_result = if action.action_type == SyncActionType::DeleteFromCloud
                    && delete_subtrees
                        .get(rel)
                        .is_some_and(|(is_directory, _)| *is_directory)
                {
                    let prefix = format!("{rel}/");
                    transaction.execute(
                        "DELETE FROM sync_items
                         WHERE local_path=?1 OR substr(local_path, 1, length(?2))=?2",
                        rusqlite::params![rel, prefix],
                    )
                } else {
                    transaction.execute(
                        "DELETE FROM sync_items WHERE local_path=?1 AND (?2='' OR file_id=?2)",
                        rusqlite::params![rel, fid],
                    )
                };
                delete_result.map_err(|error| {
                    AppError::generic(format!("删除已确认但基线结算失败（{rel}）：{error}"))
                })?;
                if action.action_type == SyncActionType::DeleteFromCloud {
                    settled_cloud_deletes.insert(rel.clone());
                }
                continue;
            }

            // 失败或延期的持久化任务不得推进最后确认成功的基线。永久失败只能更新兼容状态与消息；
            // mtime、size、fileId 和云端版本事实保持不变，TaskRunner 是权威失败来源。
            if !result.success {
                if !result.deferred {
                    if let Some(file_id) = action
                        .file_id
                        .as_deref()
                        .filter(|file_id| !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX))
                    {
                        transaction
                            .execute(
                                "UPDATE sync_items SET status=?1, error_message=?2
                             WHERE file_id=?3 AND local_path=?4",
                                rusqlite::params![
                                    repository::sync_status::FAILED,
                                    result.error_message.as_deref(),
                                    file_id,
                                    rel,
                                ],
                            )
                            .map_err(|error| {
                                AppError::generic(format!("记录同步失败状态失败：{error}"))
                            })?;
                    }
                }
                continue;
            }

            // Skip 通常是有意的空操作，不得制造成功基线。唯一允许结算的是旧版
            // `pending:<path>` 上传恢复：同路径有明确云端文件，并已重新读取本地元数据。
            if action.action_type == SyncActionType::Skip {
                let pending_file_id = format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel);
                let pending_exists = transaction
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM sync_items WHERE file_id=?1 AND local_path=?2
                         )",
                        rusqlite::params![pending_file_id, rel],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|error| {
                        AppError::generic(format!("核验待确认同步基线失败：{error}"))
                    })?;
                if !pending_exists || action.cloud_file.is_none() {
                    continue;
                }
            }

            // 上传与下载已由 TaskRunner 按任务 ID、版本和源快照原子结算；此处重新读取文件状态
            // 会让远端响应后的编辑覆盖已核验成功基线。
            if matches!(
                action.action_type,
                SyncActionType::Upload | SyncActionType::Download
            ) {
                if action.action_type == SyncActionType::Upload
                    && action
                        .reason
                        .as_deref()
                        .is_some_and(|reason| reason.starts_with("同目录改名检测："))
                {
                    let file_id = action
                        .file_id
                        .as_deref()
                        .ok_or_else(|| AppError::generic("同目录改名已完成但动作缺少 fileId"))?;
                    transaction
                        .execute(
                            "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                            rusqlite::params![file_id, rel],
                        )
                        .map_err(|error| {
                            AppError::generic(format!("清理同目录改名旧路径基线失败：{error}"))
                        })?;
                }
                continue;
            }

            let default_status = if action.action_type == SyncActionType::CreatePlaceholder {
                repository::sync_status::CLOUD_ONLY // 占位符 → cloudOnly（非 synced）
            } else if action.action_type == SyncActionType::CreateConflictCopy {
                repository::sync_status::CONFLICT
            } else {
                repository::sync_status::SYNCED // upload/download/createFolder → synced
            };

            // 云端元数据：成功时优先用 executor 返回的（新上传/建文件夹的 fileId 由此得到），
            // 否则用 action 携带的（download/placeholder 的 cloud_file）。
            let cloud_file = result.cloud_file.as_ref().or(action.cloud_file.as_ref());
            // 已确认成功必须具备真实远端 ID。不完整上传响应由 TaskRunner 转入 VerifyingRemote，
            // 此处合成待确认 ID 会制造虚假成功基线。
            let file_id = cloud_file
                .map(|file| file.id.clone())
                .or_else(|| action.file_id.clone());
            let file_id = match file_id {
                Some(fid) => fid,
                None => {
                    tracing::warn!(rel = %rel, status = default_status, "跳过成功基线写入：缺少真实 fileId");
                    continue;
                }
            };

            // 结构性移动必须保留最后实际同步的内容版本，不能把移动前后的编辑误认为已同步；
            // 紧随的下一轮周期负责上传内容差异。
            let move_baseline = if action.action_type == SyncActionType::MoveInCloud {
                Some(
                    move_baselines
                        .iter()
                        .find(|item| item.file_id == file_id && item.local_path != rel.as_str())
                        .or_else(|| move_baselines.iter().find(|item| item.file_id == file_id))
                        .ok_or_else(|| {
                            AppError::generic(format!(
                                "云端移动已确认但缺少原内容基线（fileId={file_id}）"
                            ))
                        })?,
                )
            } else {
                None
            };

            // 读取本地真实 mtime/size（对齐 dart _updateDbFromResults 从本地文件 stat）。
            // 写死 None 会导致 is_local_changed 恒 true（db.local_mtime.is_none()），每轮重传。
            let (local_mtime, local_size, sha256, is_folder) = match move_baseline {
                Some(baseline) => (
                    baseline.local_mtime,
                    baseline.local_size,
                    baseline.sha256.clone(),
                    baseline.is_folder,
                ),
                None => {
                    let settlement_path = if let Some(path) = action.local_path.as_ref() {
                        Some(std::path::PathBuf::from(path))
                    } else if matches!(
                        action.action_type,
                        SyncActionType::CreatePlaceholder
                            | SyncActionType::CreateFolder
                            | SyncActionType::Skip
                    ) {
                        Some(
                            self.mount
                                .as_ref()
                                .ok_or_else(|| {
                                    AppError::generic("同步挂载未初始化，无法结算本地动作")
                                })?
                                .mount_dir()
                                .join(rel),
                        )
                    } else {
                        None
                    };
                    let (local_mtime, local_size) = match settlement_path {
                        Some(p) => {
                            let metadata = std::fs::symlink_metadata(p).map_err(|error| {
                                AppError::generic(format!(
                                    "成功动作结算时无法读取本地目标（{rel}）：{error}"
                                ))
                            })?;
                            let expects_folder = action.action_type == SyncActionType::CreateFolder;
                            if metadata.file_type().is_symlink()
                                || (expects_folder && !metadata.is_dir())
                                || (!expects_folder && !metadata.is_file())
                            {
                                return Err(AppError::generic(format!(
                                    "成功动作结算时本地目标类型不一致：{rel}"
                                )));
                            }
                            let modified = metadata
                                .modified()
                                .ok()
                                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|duration| duration.as_millis() as i64)
                                .ok_or_else(|| {
                                    AppError::generic(format!(
                                        "成功动作结算时无法读取本地修改时间：{rel}"
                                    ))
                                })?;
                            if action.action_type == SyncActionType::Skip
                                && cloud_file.is_some_and(|file| file.size != metadata.len() as i64)
                            {
                                return Err(AppError::generic(format!(
                                    "待确认上传的本地大小与云端结果不一致，拒绝收敛成功：{rel}"
                                )));
                            }
                            (Some(modified), Some(metadata.len() as i64))
                        }
                        None => (None, None),
                    };
                    (
                        local_mtime,
                        local_size,
                        None,
                        matches!(action.action_type, SyncActionType::CreateFolder),
                    )
                }
            };
            let status = move_baseline
                .map(|baseline| baseline.status)
                .unwrap_or(default_status);
            let error_message = move_baseline.and_then(|baseline| baseline.error_message.clone());
            let last_sync_time = match move_baseline {
                Some(baseline) => baseline.last_sync_time,
                None => Some(chrono::Utc::now().timestamp_millis()),
            };

            // 重建云端已删目录（CreateFolder && cloud_file=None 成功）：新 folderId 与旧 db
            // 记录不同（旧目录已删，重建得新 id），先清掉同路径旧记录，避免 dual 记录污染。
            if result.success
                && action.action_type == SyncActionType::CreateFolder
                && action.cloud_file.is_none()
            {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE local_path=?1",
                        rusqlite::params![rel],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理重建目录旧基线失败：{error}"))
                    })?;
            }
            if action.action_type == SyncActionType::MoveInCloud {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                        rusqlite::params![file_id.as_str(), rel],
                    )
                    .map_err(|error| {
                        AppError::generic(format!("清理云端移动旧路径基线失败：{error}"))
                    })?;
            }
            // 成功上传/覆盖上传 → 清掉同路径的 pending: 占位孤儿行。
            // PK 是 (file_id, local_path)，真实 fileId 与 pending: 占位 fileId 是不同主键，
            // upsert 不会覆盖占位行 → 必须显式删除，避免残留孤儿导致 planner 误判。
            if !file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE local_path=?1 AND file_id=?2",
                        rusqlite::params![
                            rel,
                            format!("{}{}", repository::PENDING_FILE_ID_PREFIX, rel)
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("清理待确认基线失败：{error}")))?;
            }

            // upsert（对齐 dart insertOnConflictUpdate）
            repository::upsert(
                &transaction,
                &repository::SyncItem {
                    file_id,
                    local_path: rel.clone(),
                    parent_folder_id: cloud_file
                        .and_then(|file| {
                            file.parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned())
                        })
                        .or_else(|| action.parent_file_id.clone()),
                    name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
                    is_folder,
                    size: cloud_file.map(|f| f.size).unwrap_or(0),
                    local_size,
                    sha256,
                    local_mtime,
                    cloud_edited_time: cloud_file
                        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis())),
                    last_sync_time,
                    status,
                    // 结构性移动保留已有内容失败，其他已确认成功会清理过期兼容错误。
                    error_message,
                },
            )?;
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交同步结果结算失败：{error}")))?;
        drop(conn);

        // 只有持久化基线写入成功后，才发布缓存增量。
        {
            let mut recently_deleted = self.recently_deleted_paths.lock();
            let mut cloud = self.cloud_tree.lock();
            let mut path_to_id = self.path_to_id.lock();
            for (action, result) in actions.iter().zip(results.iter()) {
                let Some(relative_path) = &action.relative_path else {
                    continue;
                };
                if result.success
                    && matches!(
                        action.action_type,
                        SyncActionType::DeleteFromCloud
                            | SyncActionType::DeleteFromLocal
                            | SyncActionType::BackupBeforeCloudDelete
                    )
                {
                    recently_deleted
                        .insert(relative_path.clone(), chrono::Utc::now().timestamp_millis());
                }
                if result.success && action.action_type == SyncActionType::DeleteFromCloud {
                    if !settled_cloud_deletes.contains(relative_path) {
                        continue;
                    }
                    for path in delete_subtrees
                        .get(relative_path)
                        .map(|(_, paths)| paths)
                        .into_iter()
                        .flatten()
                    {
                        cloud.remove(path);
                        path_to_id.remove(path);
                    }
                    recently_deleted
                        .insert(relative_path.clone(), chrono::Utc::now().timestamp_millis());
                    continue;
                }
                if result.success {
                    if let Some(file) = result.cloud_file.as_ref().or(action.cloud_file.as_ref()) {
                        let stale_paths: Vec<String> = cloud
                            .iter()
                            .filter(|(path, existing)| {
                                path.as_str() != relative_path && existing.id == file.id
                            })
                            .map(|(path, _)| path.clone())
                            .collect();
                        for stale_path in stale_paths {
                            cloud.remove(&stale_path);
                            path_to_id.remove(&stale_path);
                        }
                        cloud.insert(relative_path.clone(), file.clone());
                        path_to_id.insert(relative_path.clone(), file.id.clone());
                    }
                }
            }
            let expire_before = chrono::Utc::now().timestamp_millis() - 300_000;
            recently_deleted.retain(|_, timestamp| *timestamp > expire_before);
        }
        Ok(())
    }
}

impl SyncEngine {
    /// 先顺序结算目录结构动作，再回填 parent ID 并并发执行其余动作。
    pub(super) async fn execute_actions_ordered(
        &self,
        exec: &SyncExecutor,
        actions: &mut [crate::sync::state::SyncAction],
    ) -> AppResult<Vec<crate::sync::state::ActionResult>> {
        use crate::sync::state::{ActionResult, SyncActionType};
        let n = actions.len();
        let mut results: Vec<Option<ActionResult>> = (0..n).map(|_| None).collect();

        // 收集本地新建目录，并按路径深度排序。
        let mut folder_idxs: Vec<usize> = (0..n)
            .filter(|&i| {
                actions[i].action_type == SyncActionType::CreateFolder
                    && actions[i].cloud_file.is_none()
            })
            .collect();
        folder_idxs.sort_by_key(|&i| {
            actions[i]
                .relative_path
                .as_deref()
                .map(|p| p.matches('/').count())
                .unwrap_or(0)
        });

        // 父目录先顺序创建并持久化，随后回填路径索引。
        for &i in &folder_idxs {
            self.ensure_cycle_active()?;
            let _activity = self.begin_external_activity()?;
            // 父目录可能刚完成结算，执行前重新填充 parent。
            fill_parent_file_ids(&mut actions[i..=i], &self.path_to_id.lock());
            let mut res = exec
                .execute_all(&[actions[i].clone()])
                .await
                .into_iter()
                .next()
                .unwrap_or_else(|| ActionResult {
                    success: false,
                    error_message: Some("目录创建未返回结果".into()),
                    deferred: false,
                    cloud_file: None,
                });
            self.ensure_cycle_active()?;
            if res.success {
                // 发布 parent ID 前先提交持久化基线，避免缓存与数据库分裂。
                if let Err(error) = self.apply_results(
                    std::slice::from_ref(&actions[i]),
                    std::slice::from_ref(&res),
                ) {
                    res.success = false;
                    res.deferred = true;
                    res.error_message = Some(format!(
                        "云端目录已创建，但本地基线结算失败，等待重新收敛：{error}"
                    ));
                } else if let Some(cloud_file) = res.cloud_file.as_ref() {
                    tracing::info!(rel = %actions[i].relative_path.as_deref().unwrap_or("?"),
                        folder_id = %cloud_file.id, "本地新建目录已持久化并回填 path_to_id");
                }
            }
            results[i] = Some(res);
        }

        // 目录创建完成后立即发布内容变化。
        if !folder_idxs.is_empty() {
            if let Err(error) = self.update_runtime_and_broadcast(|runtime| {
                runtime.content_changed = true;
            }) {
                tracing::warn!(%error, "目录创建后重算全局状态失败");
            }
        }

        // 用最新路径索引回填其余动作，再并发执行。
        fill_parent_file_ids(actions, &self.path_to_id.lock());
        let other_idxs: Vec<usize> = (0..n).filter(|&i| results[i].is_none()).collect();
        let other_actions: Vec<crate::sync::state::SyncAction> =
            other_idxs.iter().map(|&i| actions[i].clone()).collect();
        let other_results = if other_actions.is_empty() {
            Vec::new()
        } else {
            self.ensure_cycle_active()?;
            exec.execute_all(&other_actions).await
        };
        for (k, &i) in other_idxs.iter().enumerate() {
            if let Some(r) = other_results.get(k) {
                results[i] = Some(r.clone());
            }
        }

        Ok(results
            .into_iter()
            .map(|r| {
                r.unwrap_or_else(|| ActionResult {
                    success: false,
                    error_message: Some("动作未执行".into()),
                    deferred: false,
                    cloud_file: None,
                })
            })
            .collect())
    }
}
