//! 同步记录收敛。

use std::collections::HashMap;

use crate::data::repository;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::mount::manager::LocalFileEntry;
use crate::sync::planner::DbSnapshotEntry;
use crate::sync::state::FreeUpCheckResult;
use crate::sync::transfer_state::TransferState;

use super::coordination::CycleRequest;
use super::{FailedRecordReconciliation, SyncEngine};

impl SyncEngine {
    /// 用可信云树和本地身份补齐缺失的数据库基线。
    /// 仅在路径、类型与 fileId 可证明一致时创建或迁移记录。
    pub(super) fn reconcile_db_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        db: &HashMap<String, DbSnapshotEntry>,
    ) -> AppResult<()> {
        let conn = self.db.lock();
        let durable_records = repository::load_all(&conn)?;
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        for (rel, entry) in local {
            if let Some(db_entry) = db.get(rel) {
                // 若 DB 记录标记为 DELETED 但用户重新粘贴了文件 → 复活为正常状态
                if db_entry.status == repository::sync_status::DELETED {
                    let status = if entry.is_placeholder {
                        repository::sync_status::CLOUD_ONLY
                    } else {
                        repository::sync_status::SYNCED
                    };
                    conn.execute(
                        "UPDATE sync_items SET status=?1 WHERE local_path=?2",
                        rusqlite::params![status, rel],
                    )
                    .map_err(|error| AppError::generic(format!("复活删除墓碑失败：{error}")))?;
                    tracing::info!(rel = %rel, "DELETED 墓碑已复活");
                }
                continue;
            }

            // 可信云树中同路径、同类型的目录足以恢复目录身份；目录没有 fileId xattr，
            // 可借此收敛远端创建成功但数据库基线尚未写入的崩溃窗口。
            if entry.is_folder {
                if let Some(cloud_folder) = ct.get(rel).filter(|file| file.is_folder()) {
                    let metadata =
                        std::fs::symlink_metadata(&entry.absolute_path).map_err(|error| {
                            AppError::generic(format!("读取待恢复目录基线失败：{error}"))
                        })?;
                    repository::upsert(
                        &conn,
                        &repository::SyncItem {
                            file_id: cloud_folder.id.clone(),
                            local_path: rel.clone(),
                            parent_folder_id: cloud_folder
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned()),
                            name: cloud_folder.name.clone(),
                            is_folder: true,
                            size: cloud_folder.size,
                            local_size: Some(metadata.len() as i64),
                            sha256: None,
                            local_mtime: Some(entry.mtime),
                            cloud_edited_time: cloud_folder
                                .edited_time
                                .map(|time| time.timestamp_millis()),
                            last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                            status: repository::sync_status::SYNCED,
                            error_message: None,
                        },
                    )?;
                    continue;
                }
            }
            let status = if entry.is_placeholder {
                repository::sync_status::CLOUD_ONLY
            } else {
                repository::sync_status::SYNCED
            };
            // 尝试从 xattr 获取 fileId（占位符有 xattr）
            let file_id = std::fs::metadata(&entry.absolute_path)
                .ok()
                .and_then(|_| {
                    use crate::mount::manager::XATTR_FILE_ID;
                    xattr::get(&entry.absolute_path, XATTR_FILE_ID)
                        .ok()
                        .flatten()
                        .and_then(|b| String::from_utf8(b).ok())
                })
                .unwrap_or_default();
            if file_id.is_empty() {
                continue; // 无 fileId 无法 upsert（本地新增文件由 planner Upload 处理）
            }
            // 仅当 xattr fileId 与可信云树同路径记录一致时创建基线；复制文件交由正常上传规划。
            let Some(cloud_file) = ct.get(rel) else {
                tracing::info!(
                    rel = %rel,
                    xattr_fid = %file_id,
                    "reconcile 跳过：可信 cloud_tree 同路径不存在，禁止制造已同步 baseline"
                );
                continue;
            };
            if cloud_file.id != file_id {
                tracing::info!(
                    rel = %rel,
                    xattr_fid = %file_id,
                    cloud_fid = %cloud_file.id,
                    "reconcile 跳过：xattr fileId 与 cloud_tree 不一致（可能为复制文件），由 planner 正常处理"
                );
                continue;
            }

            // 恢复已确认远端移动的基线时，只有旧本地路径确实消失才迁移键，
            // 并保留旧内容版本，不把目标当前 mtime 和 size 误记为已同步。
            if let Some(previous) = durable_records
                .iter()
                .find(|record| record.file_id == file_id && record.local_path != rel.as_str())
            {
                let previous_path = std::path::PathBuf::from(&mount_dir).join(&previous.local_path);
                match std::fs::symlink_metadata(&previous_path) {
                    Ok(_) => {
                        tracing::warn!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            "同一 fileId 的旧本地路径仍存在，按复制/歧义处理，拒绝迁移成功基线"
                        );
                        continue;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::warn!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            %error,
                            "无法证明旧本地路径已消失，拒绝迁移成功基线"
                        );
                        continue;
                    }
                }

                let migration = (|| -> AppResult<()> {
                    let transaction = conn.unchecked_transaction().map_err(|error| {
                        AppError::generic(format!("开始恢复云端移动基线事务失败：{error}"))
                    })?;
                    transaction
                        .execute(
                            "DELETE FROM sync_items WHERE file_id=?1 AND local_path<>?2",
                            rusqlite::params![file_id, rel],
                        )
                        .map_err(|error| {
                            AppError::generic(format!("清理移动前旧路径基线失败：{error}"))
                        })?;
                    repository::upsert(
                        &transaction,
                        &repository::SyncItem {
                            file_id: file_id.clone(),
                            local_path: rel.clone(),
                            parent_folder_id: cloud_file
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first().cloned()),
                            name: cloud_file.name.clone(),
                            is_folder: previous.is_folder,
                            size: cloud_file.size,
                            local_size: previous.local_size,
                            sha256: previous.sha256.clone(),
                            local_mtime: previous.local_mtime,
                            cloud_edited_time: cloud_file
                                .edited_time
                                .map(|time| time.timestamp_millis()),
                            last_sync_time: previous.last_sync_time,
                            status: previous.status,
                            error_message: previous.error_message.clone(),
                        },
                    )?;
                    transaction.commit().map_err(|error| {
                        AppError::generic(format!("提交恢复云端移动基线失败：{error}"))
                    })?;
                    Ok(())
                })();
                match migration {
                    Ok(()) => {
                        tracing::info!(
                            old = %previous.local_path,
                            new = %rel,
                            file_id,
                            "已恢复响应丢失/进程中断后的云端移动基线"
                        );
                        self.cycle.request(CycleRequest::LOCAL_RESCAN);
                    }
                    Err(error) => {
                        tracing::warn!(old = %previous.local_path, new = %rel, file_id, %error, "恢复云端移动基线失败");
                    }
                }
                continue;
            }
            repository::upsert(
                &conn,
                &repository::SyncItem {
                    file_id,
                    local_path: rel.clone(),
                    parent_folder_id: None,
                    name: entry
                        .relative_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.relative_path)
                        .to_string(),
                    is_folder: entry.is_folder,
                    size: 0,
                    local_size: if entry.is_placeholder {
                        None
                    } else {
                        Some(entry.size as i64)
                    },
                    sha256: None,
                    local_mtime: Some(entry.mtime),
                    cloud_edited_time: None,
                    last_sync_time: Some(chrono::Utc::now().timestamp_millis()),
                    status,
                    error_message: None,
                },
            )?;
        }
        drop(conn);
        Ok(())
    }

    /// 复核失败记录，并在事务中清理两端均缺失的残余基线。
    pub(super) fn reconcile_failed_and_purge_stale_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        cloud: &HashMap<String, DriveFile>,
    ) -> AppResult<FailedRecordReconciliation> {
        let conn = self.db.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始失败状态复核事务失败：{error}")))?;
        let items = repository::load_all(&transaction)?;
        let mut reconciliation = FailedRecordReconciliation::default();

        for item in items
            .iter()
            .filter(|item| item.status == repository::sync_status::FAILED)
        {
            let (local_entry, cloud_file) =
                match (local.get(&item.local_path), cloud.get(&item.local_path)) {
                    (None, None) => continue,
                    (Some(local_entry), Some(cloud_file)) => (local_entry, cloud_file),
                    _ => {
                        reconciliation.missing_side += 1;
                        if item.file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                            reconciliation.pending_id += 1;
                        }
                        continue;
                    }
                };
            if item.file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                reconciliation.pending_id += 1;
                continue;
            }
            if item.file_id != cloud_file.id {
                reconciliation.id_mismatch += 1;
                continue;
            }
            if local_entry.is_folder != cloud_file.is_folder() {
                reconciliation.type_conflict += 1;
                continue;
            }
            if local_entry.is_folder && cloud_file.is_folder() {
                let updated = transaction
                    .execute(
                        "UPDATE sync_items SET
                            parent_folder_id=?1, name=?2, is_folder=1, size=?3,
                            local_size=?4, sha256=NULL, local_mtime=?5, cloud_edited_time=?6,
                            last_sync_time=?7, status=?8, error_message=NULL
                         WHERE file_id=?9 AND local_path=?10 AND status=?11
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue
                               WHERE relative_path=?10 AND state NOT IN (?12, ?13)
                           )",
                        rusqlite::params![
                            cloud_file
                                .parent_folder
                                .as_ref()
                                .and_then(|parents| parents.first()),
                            cloud_file.name,
                            cloud_file.size,
                            local_entry.size as i64,
                            local_entry.mtime,
                            cloud_file.edited_time.map(|time| time.timestamp_millis()),
                            chrono::Utc::now().timestamp_millis(),
                            repository::sync_status::SYNCED,
                            item.file_id,
                            item.local_path,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Completed),
                            i32::from(TransferState::Canceled),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("恢复目录失败状态失败：{error}")))?;
                reconciliation.healed += updated;
                if updated == 0 {
                    reconciliation.transfer_blocked += 1;
                }
                continue;
            }

            let cloud_edited_time = cloud_file.edited_time.map(|time| time.timestamp_millis());
            if item.is_folder {
                reconciliation.type_conflict += 1;
                continue;
            }
            let file_converged = !local_entry.is_placeholder
                && item.local_size == Some(local_entry.size as i64)
                && item.local_mtime == Some(local_entry.mtime)
                && item.size == cloud_file.size
                && item.cloud_edited_time.is_some()
                && item.cloud_edited_time == cloud_edited_time;
            if file_converged {
                let updated = transaction
                    .execute(
                        "UPDATE sync_items SET status=?1, error_message=NULL
                         WHERE file_id=?2 AND local_path=?3 AND status=?4
                           AND NOT EXISTS (
                               SELECT 1 FROM transfer_queue
                               WHERE relative_path=?3 AND state NOT IN (?5, ?6)
                           )",
                        rusqlite::params![
                            repository::sync_status::SYNCED,
                            item.file_id,
                            item.local_path,
                            repository::sync_status::FAILED,
                            i32::from(TransferState::Completed),
                            i32::from(TransferState::Canceled),
                        ],
                    )
                    .map_err(|error| AppError::generic(format!("恢复文件失败状态失败：{error}")))?;
                reconciliation.healed += updated;
                if updated == 0 {
                    reconciliation.transfer_blocked += 1;
                }
            } else {
                reconciliation.baseline_changed += 1;
            }
        }

        for item in items.iter().filter(|item| {
            !local.contains_key(&item.local_path) && !cloud.contains_key(&item.local_path)
        }) {
            let deleted = transaction
                .execute(
                    "DELETE FROM sync_items
                     WHERE local_path=?1
                       AND NOT EXISTS (
                           SELECT 1 FROM transfer_queue
                           WHERE relative_path=?1 AND state NOT IN (?2, ?3)
                       )",
                    rusqlite::params![
                        item.local_path,
                        i32::from(TransferState::Completed),
                        i32::from(TransferState::Canceled),
                    ],
                )
                .map_err(|error| AppError::generic(format!("清理残余基线失败：{error}")))?;
            reconciliation.purged += deleted;
            if deleted == 0 {
                reconciliation.transfer_blocked += 1;
                reconciliation.stale_transfer_blocked += 1;
            }
        }
        reconciliation.remaining_failed = transaction
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE status=?1",
                rusqlite::params![repository::sync_status::FAILED],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| AppError::generic(format!("统计剩余失败状态失败：{error}")))?
            as usize;
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交失败状态复核事务失败：{error}")))?;
        tracing::debug!(
            healed = reconciliation.healed,
            purged = reconciliation.purged,
            remaining_failed = reconciliation.remaining_failed,
            pending_id = reconciliation.pending_id,
            missing_side = reconciliation.missing_side,
            id_mismatch = reconciliation.id_mismatch,
            type_conflict = reconciliation.type_conflict,
            baseline_changed = reconciliation.baseline_changed,
            transfer_blocked = reconciliation.transfer_blocked,
            stale_transfer_blocked = reconciliation.stale_transfer_blocked,
            "失败状态复核完成"
        );
        Ok(reconciliation)
    }

    /// 通过稳定 fileId 识别本地改名，并将上传与删除动作收敛为远端移动。
    pub(super) fn detect_renames(
        &self,
        actions: &mut Vec<crate::sync::state::SyncAction>,
    ) -> AppResult<()> {
        use crate::mount::manager::XATTR_FILE_ID;
        use crate::sync::state::SyncActionType;
        let db = self.db.lock();
        // 先收集全体 DB 记录（按 fileId 索引）
        let db_by_id: std::collections::HashMap<String, crate::data::repository::SyncItem> =
            repository::load_all(&db)?
                .into_iter()
                .filter(|r| !r.file_id.is_empty())
                .map(|r| (r.file_id.clone(), r))
                .collect();
        drop(db);

        let path_to_id = self.path_to_id.lock().clone();
        let root_folder_id = self.root_folder_id.lock().clone();
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        let mut renamed_sources = std::collections::HashSet::new();
        let mut superseded_cloud_paths = std::collections::HashSet::new();
        for action in actions.iter_mut() {
            if action.action_type != SyncActionType::Upload || action.file_id.is_some() {
                continue;
            }
            let local_path = match &action.local_path {
                Some(p) => std::path::PathBuf::from(p),
                None => continue,
            };
            let xattr_id = std::fs::metadata(&local_path).ok().and_then(|_| {
                xattr::get(&local_path, XATTR_FILE_ID)
                    .ok()
                    .flatten()
                    .and_then(|b| String::from_utf8(b).ok())
            });
            let Some(fid) = xattr_id else { continue };
            let Some(old_record) = db_by_id.get(&fid) else {
                continue;
            };
            if Some(&old_record.local_path) == action.relative_path.as_ref() {
                continue;
            }
            // 按稳定 fileId 而非旧数据库路径解析，可收敛远端已移动但本地结算未完成的状态。
            let Some((current_cloud_path, cloud_file)) =
                ct.iter().find(|(_, cloud_file)| cloud_file.id == fid)
            else {
                continue;
            };
            // 旧文件仍在本地 → 复制（非改名）：新文件应作为全新上传，
            // 不能复用旧 fileId 走 update/rename 路径（否则云端文件被移动/覆盖）。
            // 同时清除新文件上的旧 xattr fileId，避免下轮又被误判为改名。
            let old_abs = std::path::PathBuf::from(&mount_dir).join(&old_record.local_path);
            match std::fs::symlink_metadata(&old_abs) {
                Ok(_) => {
                    let _ = xattr::remove(&local_path, XATTR_FILE_ID);
                    tracing::info!(
                        old = %old_record.local_path,
                        new = action.relative_path.as_deref().unwrap_or("?"),
                        "复制检测（旧文件仍在本地），已清除新文件旧 xattr，按全新文件上传");
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(
                        old = %old_record.local_path,
                        new = action.relative_path.as_deref().unwrap_or("?"),
                        %error,
                        "无法证明旧路径已经消失，拒绝执行远端改名/移动");
                    continue;
                }
            }

            let new_relative_path = action.relative_path.clone().unwrap_or_default();
            let current_parent_path = current_cloud_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let new_parent_path = new_relative_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");

            action.file_id = Some(fid.clone());
            action.cloud_file = Some(cloud_file.clone());
            if current_parent_path == new_parent_path {
                action.parent_file_id = cloud_file
                    .parent_folder
                    .as_ref()
                    .and_then(|v| v.first().cloned());
                action.reason = Some(format!(
                    "同目录改名检测：{} → {}（fileId={}，先于内容同步）",
                    old_record.local_path, new_relative_path, fid,
                ));
            } else {
                action.action_type = SyncActionType::MoveInCloud;
                action.parent_file_id = if new_parent_path.is_empty() {
                    root_folder_id.clone()
                } else {
                    path_to_id.get(new_parent_path).cloned()
                };
                action.reason = Some(format!(
                    "跨目录移动检测：{} → {}（fileId={}，目标 parent={:?}）",
                    old_record.local_path, new_relative_path, fid, action.parent_file_id,
                ));
            }
            if current_cloud_path.as_str() != new_relative_path.as_str() {
                superseded_cloud_paths.insert((current_cloud_path.clone(), fid.clone()));
            }
            renamed_sources.insert((old_record.local_path.clone(), fid));
            tracing::info!(reason = action.reason.as_deref(), "检测到本地文件路径变化");
        }
        drop(ct);

        // fileId 证明为同一文件后，移除旧路径及其删除祖先，避免与移动并发回收目标文件。
        actions.retain(|action| {
            if action.action_type == SyncActionType::CreatePlaceholder {
                if let (Some(path), Some(file_id)) =
                    (action.relative_path.as_ref(), action.file_id.as_ref())
                {
                    if superseded_cloud_paths.contains(&(path.clone(), file_id.clone())) {
                        return false;
                    }
                }
            }
            if action.action_type != SyncActionType::DeleteFromCloud {
                return true;
            }
            let Some(old_path) = action.relative_path.as_ref() else {
                return true;
            };
            let Some(file_id) = action.file_id.as_ref() else {
                return true;
            };
            !renamed_sources.iter().any(|(source_path, source_file_id)| {
                // 推迟精确源删除及其祖先目录删除；移动延迟时不得回收源文件，
                // 成功后由下一周期清理空目录。
                (source_path == old_path && source_file_id == file_id)
                    || source_path
                        .strip_prefix(old_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        });
        Ok(())
    }

    /// 扫描挂载目录并按相对路径构建本地快照。
    pub(super) async fn scan_local(&self) -> AppResult<HashMap<String, LocalFileEntry>> {
        match &self.mount {
            Some(m) => Ok(m
                .scan_local(&self.skip_patterns)
                .await?
                .into_iter()
                .map(|e| (e.relative_path.clone(), e))
                .collect()),
            None => Err(AppError::generic("同步挂载尚未初始化，拒绝按空本地树规划")),
        }
    }

    /// 加载并校验按本地路径唯一的数据库基线快照。
    pub(super) fn load_db_snapshot(&self) -> AppResult<HashMap<String, DbSnapshotEntry>> {
        let conn = self.db.lock();
        let mut snapshot = HashMap::new();
        for record in repository::load_all(&conn)? {
            let relative_path = record.local_path.clone();
            let entry = DbSnapshotEntry {
                file_id: record.file_id,
                local_mtime: record.local_mtime,
                local_size: record.local_size,
                cloud_edited_time: record.cloud_edited_time,
                status: record.status,
                is_folder: record.is_folder,
            };
            if snapshot.insert(relative_path.clone(), entry).is_some() {
                return Err(AppError::generic(format!(
                    "同步基线存在重复本地路径，拒绝继续规划：{relative_path}"
                )));
            }
        }
        Ok(snapshot)
    }

    /// 确认云端副本、本地文件与成功基线一致后允许释放空间。
    pub fn can_safely_free_up(&self, rel_path: &str, file_id: &str) -> FreeUpCheckResult {
        if !self.cloud_tree_is_trusted() {
            return FreeUpCheckResult::NotSynced;
        }
        let tree = self.cloud_tree.lock();
        if tree.get(rel_path).map(|file| file.id.as_str()) != Some(file_id) {
            return FreeUpCheckResult::NotInCloud;
        }
        drop(tree);
        let conn = self.db.lock();
        if repository::list_all_transfers(&conn).is_ok_and(|tasks| {
            tasks.into_iter().any(|task| {
                task.relative_path.as_deref() == Some(rel_path)
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            TransferState::Completed
                                | TransferState::Failed
                                | TransferState::Canceled
                        )
                    })
            })
        }) {
            return FreeUpCheckResult::NotSynced;
        }
        if let Ok(Some(record)) = repository::find_by_file_id(&conn, file_id) {
            if record.local_path != rel_path || record.status != repository::sync_status::SYNCED {
                return FreeUpCheckResult::NotSynced;
            }
            let Some(mount) = &self.mount else {
                return FreeUpCheckResult::NotSynced;
            };
            let path = mount.mount_dir().join(&record.local_path);
            // 本地文件必须存在且与数据库基线一致；缺失文件或占位符均不可释放。
            let Ok(meta) = std::fs::metadata(path) else {
                return FreeUpCheckResult::NotSynced;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            if record.local_mtime != mtime || record.local_size != Some(meta.len() as i64) {
                FreeUpCheckResult::NotSynced
            } else {
                FreeUpCheckResult::Safe
            }
        } else {
            FreeUpCheckResult::NotSynced
        }
    }

    /// 云端删除前复核本地路径，存在内容或无法证明缺失时改为 Skip。
    /// 占位符没有唯一内容，可以继续删除。
    pub(super) fn validate_delete_from_cloud(
        &self,
        actions: &mut [crate::sync::state::SyncAction],
    ) {
        let mount_dir = match self.mount_dir.lock().clone() {
            Some(d) => std::path::PathBuf::from(crate::core::paths::expand_tilde(&d)),
            None => return,
        };
        for a in actions.iter_mut() {
            if a.action_type != crate::sync::state::SyncActionType::DeleteFromCloud {
                continue;
            }
            let Some(rel) = &a.relative_path else {
                continue;
            };
            let abs = mount_dir.join(rel);
            match std::fs::symlink_metadata(&abs) {
                Ok(meta) => {
                    let size = meta.len();
                    // 占位符（0 字节 + xattr state=placeholder）→ 仍执行删除（无实际内容）
                    // 其余（size>0 或无 xattr）→ 本地文件实际存在，跳过删除
                    if size == 0 && crate::mount::manager::is_placeholder_file(&abs) {
                        continue;
                    }
                    tracing::info!(
                        rel = %rel,
                        size,
                        reason = a.reason.as_deref().unwrap_or("?"),
                        "DeleteFromCloud 防误删：本地文件实际存在，改为 Skip"
                    );
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason = Some(format!(
                        "防误删：本地文件实际存在（{} 字节），跳过 DeleteFromCloud",
                        size,
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(rel = %rel, %error, "无法确认本地文件不存在，拒绝云端删除");
                    a.action_type = crate::sync::state::SyncActionType::Skip;
                    a.reason = Some(format!("本地路径访问异常，无法证明文件已删除：{error}"));
                }
            }
        }
    }
}
