//! 同步记录收敛。

use std::collections::HashMap;

use crate::data::repository;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::mount::manager::LocalFileEntry;
use crate::sync::planner::DbSnapshotEntry;
use crate::sync::state::FreeUpCheckResult;
use crate::sync::transfer_state::TransferState;

use super::action_filters::is_blocked_path_identity;
use super::coordination::CycleRequest;
use super::{FailedRecordReconciliation, SyncEngine};
use crate::sync::path_recovery::BlockedPathChange;

impl SyncEngine {
    /// 用可信云树和本地身份补齐缺失的数据库基线。
    /// 仅在路径、类型与 fileId 可证明一致时创建或迁移记录。
    pub(super) fn reconcile_db_records(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        db: &HashMap<String, DbSnapshotEntry>,
        blocked_changes: &[BlockedPathChange],
    ) -> AppResult<()> {
        let conn = self.db.lock();
        let durable_records = repository::load_all(&conn)?;
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        for (rel, entry) in local {
            if is_blocked_path_identity(Some(rel), None, blocked_changes) {
                continue;
            }
            if let Some(db_entry) = db.get(rel) {
                if entry.is_folder && db_entry.is_folder {
                    if let Some(cloud_folder) = ct.get(rel) {
                        if !cloud_folder.is_folder() || cloud_folder.id != db_entry.file_id {
                            return Err(AppError::generic(format!(
                                "目录 {rel} 的数据库与云端稳定身份不一致，拒绝继续规划"
                            )));
                        }
                    }
                    let mount = self.mount.as_ref().ok_or_else(|| {
                        AppError::generic("同步挂载未初始化，无法核验目录稳定身份")
                    })?;
                    mount
                        .ensure_folder_with_file_id(rel, &db_entry.file_id)
                        .map_err(|error| {
                            AppError::generic(format!(
                                "目录 {rel} 的本地稳定身份不一致，拒绝继续规划：{error}"
                            ))
                        })?;
                }
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

            // 可信云树中同路径、同类型且本地 xattr 可绑定为同一 fileId 的目录，可以
            // 恢复创建成功但数据库尚未写入的崩溃窗口。不同 xattr 必须 fail closed，
            // 不能把恰好同名的另一目录覆盖成当前云端身份。
            if entry.is_folder {
                if let Some(cloud_folder) = ct.get(rel).filter(|file| file.is_folder()) {
                    let mount = self.mount.as_ref().ok_or_else(|| {
                        AppError::generic("同步挂载未初始化，无法核验目录稳定身份")
                    })?;
                    mount
                        .ensure_folder_with_file_id(rel, &cloud_folder.id)
                        .map_err(|error| {
                            AppError::generic(format!(
                                "目录 {rel} 无法绑定可信云端身份，拒绝恢复基线：{error}"
                            ))
                        })?;
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
                    crate::platform::xattr::get(&entry.absolute_path, XATTR_FILE_ID)
                        .ok()
                        .flatten()
                        .and_then(|b| String::from_utf8(b).ok())
                })
                .unwrap_or_default();
            if file_id.is_empty() {
                continue; // 无 fileId 无法 upsert（本地新增文件由 planner Upload 处理）
            }
            if is_blocked_path_identity(Some(rel), Some(&file_id), blocked_changes) {
                continue;
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
        blocked_changes: &[BlockedPathChange],
    ) -> AppResult<FailedRecordReconciliation> {
        // 失败修复与残余清理必须同事务提交，避免只完成其中一半。
        let conn = self.db.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始失败状态复核事务失败：{error}")))?;
        let items = repository::load_all(&transaction)?;
        let mut reconciliation = FailedRecordReconciliation::default();

        // 仅复核未被当前变更阻塞的失败记录。
        for item in items
            .iter()
            .filter(|item| item.status == repository::sync_status::FAILED)
            .filter(|item| {
                !is_blocked_path_identity(
                    Some(&item.local_path),
                    Some(&item.file_id),
                    blocked_changes,
                )
            })
        {
            // 两端同时存在才具备自动恢复为已同步的充分条件。
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
            // 目录没有内容哈希，身份与类型一致即可恢复基线。
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
            // 文件必须同时匹配本地与云端快照，不能仅凭 fileId 清错。
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

        // 两端均已消失的基线可清理，但活跃传输仍拥有该路径时必须保留。
        for item in items
            .iter()
            .filter(|item| {
                !is_blocked_path_identity(
                    Some(&item.local_path),
                    Some(&item.file_id),
                    blocked_changes,
                )
            })
            .filter(|item| {
                !local.contains_key(&item.local_path) && !cloud.contains_key(&item.local_path)
            })
        {
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
        // 在提交前统计最终失败数，保证日志与事务结果属于同一快照。
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
        // 先收集全体 DB 记录（按 fileId 索引）。重复 fileId 明确标成歧义，不能让
        // HashMap 静默覆盖后挑任一路径执行远端 move。
        let mut db_by_id: std::collections::HashMap<
            String,
            Option<crate::data::repository::SyncItem>,
        > = std::collections::HashMap::new();
        for record in repository::load_all(&db)?
            .into_iter()
            .filter(|record| !record.file_id.is_empty())
        {
            match db_by_id.entry(record.file_id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(record));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
            }
        }
        drop(db);

        let path_to_id = self.path_to_id.lock().clone();
        let root_folder_id = self.root_folder_id.lock().clone();
        let ct = self.cloud_tree.lock();
        let mount_dir =
            crate::core::paths::expand_tilde(&self.mount_dir.lock().clone().unwrap_or_default());
        let mut renamed_sources = std::collections::HashSet::new();
        let mut superseded_cloud_paths = std::collections::HashSet::new();
        let mut deferred_replacement_sources = std::collections::HashSet::new();
        let mut directory_moves = Vec::<(String, String, String)>::new();
        for action in actions.iter_mut() {
            // 接纳三类候选：上传新文件（Upload，file_id 为空）、本地新目录
            // （CreateFolder，file_id 为空），以及孤儿占位符删除
            // （DeleteFromLocal + file_id 为空）。Finder 改名的纯占位符（0 字节）会被
            // planner 判为孤儿占位符 → DeleteFromLocal，但它仍带 XATTR_FILE_ID，
            // 借此可识别为同一文件的改名。合法的云端驱动 DeleteFromLocal 带 file_id，不会误命中。
            let is_orphan_placeholder_delete =
                action.action_type == SyncActionType::DeleteFromLocal && action.file_id.is_none();
            let is_local_folder_create =
                action.action_type == SyncActionType::CreateFolder && action.cloud_file.is_none();
            if !(action.action_type == SyncActionType::Upload
                || is_local_folder_create
                || is_orphan_placeholder_delete)
                || action.file_id.is_some()
            {
                continue;
            }
            let local_path = match &action.local_path {
                Some(p) => std::path::PathBuf::from(p),
                None => continue,
            };
            let local_metadata = match std::fs::symlink_metadata(&local_path) {
                Ok(metadata)
                    if !metadata.file_type().is_symlink()
                        && if is_local_folder_create {
                            metadata.is_dir()
                        } else {
                            metadata.is_file()
                        } =>
                {
                    metadata
                }
                _ => continue,
            };
            let xattr_id = crate::platform::xattr::get(&local_path, XATTR_FILE_ID)
                .ok()
                .flatten()
                .and_then(|b| String::from_utf8(b).ok())
                .filter(|file_id| !file_id.trim().is_empty());
            let Some(fid) = xattr_id else { continue };
            let Some(old_record) = db_by_id.get(&fid) else {
                continue;
            };
            let Some(old_record) = old_record.as_ref() else {
                tracing::warn!(
                    file_id = %fid,
                    new = action.relative_path.as_deref().unwrap_or("?"),
                    "同一 fileId 存在多条同步基线，拒绝识别远端路径移动"
                );
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
            if old_record.is_folder != local_metadata.is_dir()
                || old_record.is_folder != cloud_file.is_folder()
            {
                tracing::warn!(
                    old = %old_record.local_path,
                    new = action.relative_path.as_deref().unwrap_or("?"),
                    file_id = %fid,
                    "稳定 fileId 的本地、数据库与云端类型不一致，拒绝识别路径移动"
                );
                continue;
            }
            // 只有旧路径仍携带同一 fileId 才能证明是复制；旧路径若已被其他文件占用，
            // 仍应把携带稳定身份的新路径识别为移动，随后由下一周期处理旧路径的新文件。
            let old_abs = std::path::PathBuf::from(&mount_dir).join(&old_record.local_path);
            match std::fs::symlink_metadata(&old_abs) {
                Ok(_) => {
                    let old_owner = match crate::platform::xattr::get(&old_abs, XATTR_FILE_ID) {
                        Ok(owner) => owner,
                        Err(error) => {
                            tracing::warn!(
                                old = %old_record.local_path,
                                new = action.relative_path.as_deref().unwrap_or("?"),
                                %error,
                                "无法核验旧路径身份，拒绝执行远端改名/移动"
                            );
                            continue;
                        }
                    };
                    if old_owner.as_deref() == Some(fid.as_bytes()) {
                        if let Err(error) =
                            crate::platform::xattr::remove(&local_path, XATTR_FILE_ID)
                        {
                            let already_absent =
                                crate::platform::xattr::get(&local_path, XATTR_FILE_ID)
                                    .map_err(|verify_error| {
                                        AppError::generic(format!(
                                            "复制检测清理 fileId 失败且无法复核（{}）：{error}；{verify_error}",
                                            local_path.display()
                                        ))
                                    })?
                                    .is_none();
                            if !already_absent {
                                return Err(AppError::generic(format!(
                                    "复制检测无法清除新路径的旧 fileId（{}）：{error}",
                                    local_path.display()
                                )));
                            }
                        }
                        if crate::platform::xattr::get(&local_path, XATTR_FILE_ID)
                            .map_err(|error| {
                                AppError::generic(format!(
                                    "复制检测复核新路径 fileId 失败（{}）：{error}",
                                    local_path.display()
                                ))
                            })?
                            .is_some()
                        {
                            return Err(AppError::generic(format!(
                                "复制检测清除后 fileId 仍存在：{}",
                                local_path.display()
                            )));
                        }
                        tracing::info!(
                            old = %old_record.local_path,
                            new = action.relative_path.as_deref().unwrap_or("?"),
                            "复制检测（旧路径仍属于同一 fileId），已清除新文件旧 xattr"
                        );
                        continue;
                    }
                    tracing::info!(
                        old = %old_record.local_path,
                        new = action.relative_path.as_deref().unwrap_or("?"),
                        "旧路径已被其他文件占用，按稳定 fileId 继续识别为移动"
                    );
                    // 旧路径的新文件不能在本周期继续用原 fileId 覆盖上传；移动结算后触发的
                    // 下一轮重扫会在旧基线清除后把它规划为全新上传。
                    deferred_replacement_sources
                        .insert((old_record.local_path.clone(), fid.clone()));
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
            if old_record.is_folder
                && new_relative_path
                    .strip_prefix(current_cloud_path.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
                tracing::warn!(
                    old = %current_cloud_path,
                    new = %new_relative_path,
                    file_id = %fid,
                    "拒绝把云端目录移动到自身子树"
                );
                continue;
            }
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
            // 同目录改名与跨目录移动都只修改远端路径元数据；内容差异留给下一轮版本校验。
            action.action_type = SyncActionType::MoveInCloud;
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
            if old_record.is_folder {
                directory_moves.push((
                    old_record.local_path.clone(),
                    new_relative_path.clone(),
                    fid.clone(),
                ));
            }
            renamed_sources.insert((old_record.local_path.clone(), fid));
            tracing::info!(reason = action.reason.as_deref(), "检测到本地路径变化");
        }
        drop(ct);

        // fileId 证明为同一文件后，移除旧路径及其删除祖先，避免与移动并发回收目标文件。
        // 目录移动只保留最外层根 MoveInCloud：旧/新两棵子树上的上传、建目录、删除、
        // 占位和后代 Move 全部交给根移动后的下一轮版本对账，避免拆成大量破坏性动作。
        actions.retain(|action| {
            if let Some(path) = action.relative_path.as_deref() {
                for (old_root, new_root, root_file_id) in &directory_moves {
                    let in_old_subtree = path == old_root
                        || path
                            .strip_prefix(old_root)
                            .is_some_and(|suffix| suffix.starts_with('/'));
                    let in_new_subtree = path == new_root
                        || path
                            .strip_prefix(new_root)
                            .is_some_and(|suffix| suffix.starts_with('/'));
                    if in_old_subtree || in_new_subtree {
                        let is_root_move = action.action_type == SyncActionType::MoveInCloud
                            && path == new_root
                            && action.file_id.as_deref() == Some(root_file_id.as_str());
                        if !is_root_move {
                            return false;
                        }
                    }
                }
            }
            if let (Some(path), Some(file_id)) =
                (action.relative_path.as_ref(), action.file_id.as_ref())
            {
                if deferred_replacement_sources.contains(&(path.clone(), file_id.clone())) {
                    return false;
                }
            }
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
                .scan_local(self.skip_matcher.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::service::AuthService;
    use crate::data::repository::{sync_status, SyncItem};
    use crate::drive::changes_api::ChangesApi;
    use crate::drive::client::DriveClient;
    use crate::drive::files_api::FilesApi;
    use crate::drive::models::FileCategory;
    use crate::mount::manager::{MountManager, XATTR_FILE_ID};
    use crate::sync::state::{SyncAction, SyncActionType};
    use crate::sync::status_aggregator::StatusAggregator;
    use parking_lot::Mutex;
    use rusqlite::Connection;
    use std::sync::Arc;

    const SYNC_ITEMS_DDL: &str = "
        CREATE TABLE sync_items (
            file_id TEXT NOT NULL,
            local_path TEXT NOT NULL,
            parent_folder_id TEXT,
            name TEXT NOT NULL,
            is_folder INTEGER NOT NULL DEFAULT 0,
            size INTEGER NOT NULL DEFAULT 0,
            local_size INTEGER,
            sha256 TEXT,
            local_mtime INTEGER,
            cloud_edited_time INTEGER,
            last_sync_time INTEGER,
            status INTEGER NOT NULL DEFAULT 0,
            error_message TEXT,
            PRIMARY KEY (file_id, local_path)
        );
    ";

    fn test_engine(mount_root: &std::path::Path) -> (SyncEngine, Arc<Mutex<Connection>>) {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SYNC_ITEMS_DDL).unwrap();
        let db = Arc::new(Mutex::new(connection));
        let auth = Arc::new(AuthService::new());
        let client = Arc::new(DriveClient::new(auth));
        let files_api = Arc::new(FilesApi::new(client.clone()));
        let changes_api = Arc::new(ChangesApi::new(client));
        let mut engine = SyncEngine::new(
            files_api,
            changes_api,
            db.clone(),
            Arc::new(StatusAggregator::default()),
            Vec::new(),
            0,
            0,
        );
        engine.set_mount(Arc::new(MountManager::new(mount_root)));
        (engine, db)
    }

    fn sync_item(file_id: &str, path: &str, is_folder: bool) -> SyncItem {
        SyncItem {
            file_id: file_id.into(),
            local_path: path.into(),
            parent_folder_id: None,
            name: path.rsplit('/').next().unwrap().into(),
            is_folder,
            size: if is_folder { 0 } else { 7 },
            local_size: Some(if is_folder { 0 } else { 7 }),
            sha256: (!is_folder).then(|| format!("sha-{file_id}")),
            local_mtime: Some(11),
            cloud_edited_time: Some(22),
            last_sync_time: Some(33),
            status: sync_status::SYNCED,
            error_message: None,
        }
    }

    fn cloud_file(file_id: &str, name: &str, parent_id: &str, is_folder: bool) -> DriveFile {
        DriveFile {
            id: file_id.into(),
            name: name.into(),
            category: if is_folder {
                FileCategory::Folder
            } else {
                FileCategory::Document
            },
            size: if is_folder { 0 } else { 7 },
            parent_folder: Some(vec![parent_id.into()]),
            ..Default::default()
        }
    }

    fn assert_detect_directory_move_collapses_subtree(
        old_root: &str,
        new_root: &str,
        source_parent_id: &str,
        target_parent_id: &str,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let new_nested = format!("{new_root}/nested");
        let new_file = format!("{new_nested}/note.txt");
        std::fs::create_dir_all(temp.path().join(&new_nested)).unwrap();
        std::fs::write(temp.path().join(&new_file), b"content").unwrap();
        crate::platform::xattr::set(
            &temp.path().join(new_root),
            XATTR_FILE_ID,
            b"folder-root-id",
        )
        .unwrap();

        let (engine, db) = test_engine(temp.path());
        let old_nested = format!("{old_root}/nested");
        let old_file = format!("{old_nested}/note.txt");
        for item in [
            sync_item("folder-root-id", old_root, true),
            sync_item("nested-folder-id", &old_nested, true),
            sync_item("nested-file-id", &old_file, false),
        ] {
            repository::upsert(&db.lock(), &item).unwrap();
        }
        for (path, file) in [
            (
                old_root.to_string(),
                cloud_file(
                    "folder-root-id",
                    old_root.rsplit('/').next().unwrap(),
                    source_parent_id,
                    true,
                ),
            ),
            (
                old_nested.clone(),
                cloud_file("nested-folder-id", "nested", "folder-root-id", true),
            ),
            (
                old_file.clone(),
                cloud_file("nested-file-id", "note.txt", "nested-folder-id", false),
            ),
        ] {
            engine.path_to_id_insert(path.clone(), file.id.clone());
            engine.cloud_tree_insert(path, file);
        }
        let target_parent_path = new_root.rsplit_once('/').map(|(parent, _)| parent);
        if let Some(target_parent_path) = target_parent_path {
            engine.path_to_id_insert(target_parent_path.into(), target_parent_id.into());
        }

        let mut actions = vec![
            SyncAction {
                action_type: SyncActionType::CreateFolder,
                relative_path: Some(new_root.into()),
                file_id: None,
                parent_file_id: None,
                local_path: Some(temp.path().join(new_root).to_string_lossy().into_owned()),
                cloud_file: None,
                reason: None,
            },
            SyncAction {
                action_type: SyncActionType::CreateFolder,
                relative_path: Some(new_nested.clone()),
                file_id: None,
                parent_file_id: None,
                local_path: Some(temp.path().join(&new_nested).to_string_lossy().into_owned()),
                cloud_file: None,
                reason: None,
            },
            SyncAction {
                action_type: SyncActionType::Upload,
                relative_path: Some(new_file.clone()),
                file_id: None,
                parent_file_id: None,
                local_path: Some(temp.path().join(&new_file).to_string_lossy().into_owned()),
                cloud_file: None,
                reason: None,
            },
            SyncAction {
                action_type: SyncActionType::DeleteFromCloud,
                relative_path: Some(old_root.into()),
                file_id: Some("folder-root-id".into()),
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: None,
            },
            SyncAction {
                action_type: SyncActionType::DeleteFromCloud,
                relative_path: Some(old_nested),
                file_id: Some("nested-folder-id".into()),
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: None,
            },
            SyncAction {
                action_type: SyncActionType::DeleteFromCloud,
                relative_path: Some(old_file),
                file_id: Some("nested-file-id".into()),
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: None,
            },
        ];

        engine.detect_renames(&mut actions).unwrap();

        assert_eq!(
            actions.len(),
            1,
            "目录根路径变化必须折叠旧/新子树全部派生动作"
        );
        let moved = &actions[0];
        assert_eq!(moved.action_type, SyncActionType::MoveInCloud);
        assert_eq!(moved.relative_path.as_deref(), Some(new_root));
        assert_eq!(moved.file_id.as_deref(), Some("folder-root-id"));
        assert_eq!(moved.parent_file_id.as_deref(), Some(target_parent_id));
        assert!(moved
            .cloud_file
            .as_ref()
            .is_some_and(|file| file.is_folder() && file.id == "folder-root-id"));
    }

    /// 同一父目录的嵌套目录改名只生成一次根 MoveInCloud。
    #[test]
    fn detect_same_parent_directory_rename_collapses_entire_subtree() {
        assert_detect_directory_move_collapses_subtree(
            "projects/old",
            "projects/new",
            "projects-id",
            "projects-id",
        );
    }

    /// 跨父目录移动同样只生成根 MoveInCloud，并解析新 parentFolder。
    #[test]
    fn detect_cross_parent_directory_move_collapses_entire_subtree() {
        assert_detect_directory_move_collapses_subtree(
            "projects/old",
            "archive/new",
            "projects-id",
            "archive-id",
        );
    }

    /// 重复 fileId 基线有歧义时必须保留普通本地动作，绝不能任选一路径远端移动。
    #[test]
    fn detect_directory_move_rejects_duplicate_file_id_baselines() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("new");
        std::fs::create_dir(&target).unwrap();
        crate::platform::xattr::set(&target, XATTR_FILE_ID, b"duplicate-folder-id").unwrap();
        let (engine, db) = test_engine(temp.path());
        repository::upsert(&db.lock(), &sync_item("duplicate-folder-id", "old-a", true)).unwrap();
        repository::upsert(&db.lock(), &sync_item("duplicate-folder-id", "old-b", true)).unwrap();
        engine.cloud_tree_insert(
            "old-a".into(),
            cloud_file("duplicate-folder-id", "old-a", "root", true),
        );
        let mut actions = vec![SyncAction {
            action_type: SyncActionType::CreateFolder,
            relative_path: Some("new".into()),
            file_id: None,
            parent_file_id: None,
            local_path: Some(target.to_string_lossy().into_owned()),
            cloud_file: None,
            reason: None,
        }];

        engine.detect_renames(&mut actions).unwrap();

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, SyncActionType::CreateFolder);
        assert!(actions[0].file_id.is_none());
    }

    /// 同路径目录已有不同稳定身份时必须阻断 reconcile，不能覆盖为云端身份。
    #[test]
    fn reconcile_rejects_directory_with_different_xattr_identity() {
        let temp = tempfile::tempdir().unwrap();
        let local_path = temp.path().join("occupied");
        std::fs::create_dir(&local_path).unwrap();
        crate::platform::xattr::set(&local_path, XATTR_FILE_ID, b"source-folder-id").unwrap();
        let (engine, db) = test_engine(temp.path());
        repository::upsert(
            &db.lock(),
            &sync_item("destination-folder-id", "occupied", true),
        )
        .unwrap();
        engine.cloud_tree_insert(
            "occupied".into(),
            cloud_file("destination-folder-id", "occupied", "root", true),
        );
        let local = HashMap::from([(
            "occupied".into(),
            LocalFileEntry {
                absolute_path: local_path,
                relative_path: "occupied".into(),
                size: 0,
                mtime: 11,
                is_folder: true,
                is_placeholder: false,
            },
        )]);
        let db_snapshot = engine.load_db_snapshot().unwrap();

        let error = engine
            .reconcile_db_records(&local, &db_snapshot, &[])
            .unwrap_err();

        assert!(error.to_string().contains("本地稳定身份不一致"));
        let record = repository::find_by_file_id(&db.lock(), "destination-folder-id")
            .unwrap()
            .unwrap();
        assert_eq!(record.local_path, "occupied");
        assert_eq!(
            crate::platform::xattr::get(&temp.path().join("occupied"), XATTR_FILE_ID)
                .unwrap()
                .as_deref(),
            Some(b"source-folder-id".as_slice()),
            "reconcile 不得覆盖不同目录身份"
        );
    }
}
