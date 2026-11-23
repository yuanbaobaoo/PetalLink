//! 同步记录收敛。

use std::collections::HashMap;
use std::sync::atomic::Ordering;

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

/// 同路径重复基线分组（路径 → 该路径全部基线行）。
type DuplicateBaselines = HashMap<String, Vec<repository::SyncItem>>;

/// 从同路径重复基线中选择保留行：云端权威 fileId 命中优先；否则取最近成功结算的
/// 非墓碑行；全为墓碑行时取最近结算的一行。
fn select_baseline_to_keep<'a>(
    group: &'a [repository::SyncItem],
    authority_file_id: Option<&str>,
) -> &'a repository::SyncItem {
    if let Some(authority) = authority_file_id {
        if let Some(matched) = group.iter().find(|record| record.file_id == authority) {
            return matched;
        }
    }
    group
        .iter()
        .filter(|record| record.status != repository::sync_status::DELETED)
        .max_by_key(|record| record.last_sync_time.unwrap_or(0))
        .or_else(|| {
            group
                .iter()
                .max_by_key(|record| record.last_sync_time.unwrap_or(0))
        })
        .expect("重复基线分组非空")
}

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
        let cloud_guard = self.cloud.lock();
        let ct = &cloud_guard.tree;
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

        let (path_to_id, root_folder_id) = {
            let cloud = self.cloud.lock();
            (cloud.path_to_id.clone(), cloud.root_folder_id.clone())
        };
        let cloud_guard = self.cloud.lock();
        let ct = &cloud_guard.tree;
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
        drop(cloud_guard);

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

    /// 收敛已实际解决的冲突基线：冲突处理成功结算后状态停在 CONFLICT；若云端
    /// 权威身份与编辑时间均未再变、本地文件与基线快照一致且无未终态传输，
    /// 说明冲突两端内容已对齐，恢复 synced，避免冲突提示在解决后永久残留。
    pub(super) fn reconcile_resolved_conflicts(
        &self,
        local: &HashMap<String, LocalFileEntry>,
        db: &HashMap<String, DbSnapshotEntry>,
        cloud: &HashMap<String, DriveFile>,
    ) -> AppResult<usize> {
        let candidates: Vec<(String, String)> = db
            .iter()
            .filter(|(_, entry)| entry.status == repository::sync_status::CONFLICT)
            .filter_map(|(rel, entry)| {
                let cloud_file = cloud.get(rel)?;
                // 云端权威身份与基线一致，且云端版本自冲突结算后未再变化。
                if cloud_file.id != entry.file_id
                    || cloud_file.edited_time.map(|time| time.timestamp_millis())
                        != entry.cloud_edited_time
                {
                    return None;
                }
                // 本地文件存在且与基线快照一致（冲突处理后无新修改）。
                let local_file = local.get(rel)?;
                if local_file.is_placeholder || local_file.is_folder != entry.is_folder {
                    return None;
                }
                if Some(local_file.mtime) != entry.local_mtime
                    || Some(local_file.size as i64) != entry.local_size
                {
                    return None;
                }
                Some((rel.clone(), entry.file_id.clone()))
            })
            .collect();
        if candidates.is_empty() {
            return Ok(0);
        }
        let conn = self.db.lock();
        // 有未终态传输的路径不收敛，等传输结算后下一周期再判。
        let active_paths: std::collections::HashSet<String> =
            repository::list_all_transfers(&conn)?
                .into_iter()
                .filter(|task| {
                    task.state_kind()
                        .map(|state| !state.is_terminal())
                        .unwrap_or(true)
                })
                .filter_map(|task| task.relative_path)
                .collect();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始冲突收敛事务失败：{error}")))?;
        let mut resolved = 0;
        for (rel, file_id) in &candidates {
            if active_paths.contains(rel) {
                continue;
            }
            // 事务内复核仍是 CONFLICT 再收敛，防止与并发结算竞争。
            let updated = transaction
                .execute(
                    "UPDATE sync_items SET status=?1, error_message=NULL
                     WHERE file_id=?2 AND local_path=?3 AND status=?4",
                    rusqlite::params![
                        repository::sync_status::SYNCED,
                        file_id,
                        rel,
                        repository::sync_status::CONFLICT
                    ],
                )
                .map_err(|error| {
                    AppError::generic(format!("冲突收敛更新失败（{rel}）：{error}"))
                })?;
            if updated > 0 {
                resolved += updated;
                tracing::info!(rel = %rel, file_id = %file_id, "冲突已解决：基线状态收敛为已同步");
            }
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交冲突收敛事务失败：{error}")))?;
        Ok(resolved)
    }

    /// 加载并校验按本地路径唯一的数据库基线快照。
    ///
    /// 同路径出现多行基线（如服务器对覆盖上传换发新 fileId 后旧行残留）会锁死全部
    /// 规划，因此在加载时自愈收敛而不是拒绝启动；收敛失败才维持 fail-fast。
    pub(super) fn load_db_snapshot(&self) -> AppResult<HashMap<String, DbSnapshotEntry>> {
        let (snapshot, duplicates) = {
            let conn = self.db.lock();
            let records = repository::load_all(&conn)?;
            Self::build_path_unique_snapshot(records)?
        };
        if duplicates.is_empty() {
            return Ok(snapshot);
        }
        self.heal_duplicate_path_baselines(&duplicates)?;
        let conn = self.db.lock();
        let (snapshot, remaining) = Self::build_path_unique_snapshot(repository::load_all(&conn)?)?;
        if let Some(relative_path) = remaining.keys().next() {
            return Err(AppError::generic(format!(
                "同步基线自愈后仍存在重复本地路径，拒绝继续规划：{relative_path}"
            )));
        }
        Ok(snapshot)
    }

    /// 构建路径唯一的快照；返回快照与按路径分组的重复基线（组内 ≥2 行）。
    fn build_path_unique_snapshot(
        records: Vec<repository::SyncItem>,
    ) -> AppResult<(HashMap<String, DbSnapshotEntry>, DuplicateBaselines)> {
        let mut by_path: HashMap<String, Vec<repository::SyncItem>> = HashMap::new();
        for record in records {
            by_path
                .entry(record.local_path.clone())
                .or_default()
                .push(record);
        }
        let mut snapshot = HashMap::new();
        let mut duplicates = HashMap::new();
        for (relative_path, mut group) in by_path {
            if group.len() > 1 {
                // 全表扫描行序不稳定，按 fileId 排序保证裁决与日志输出确定。
                group.sort_by(|a, b| a.file_id.cmp(&b.file_id));
                duplicates.insert(relative_path, group);
                continue;
            }
            let record = group.into_iter().next().expect("分组非空");
            let entry = DbSnapshotEntry {
                file_id: record.file_id,
                local_mtime: record.local_mtime,
                local_size: record.local_size,
                cloud_edited_time: record.cloud_edited_time,
                status: record.status,
                is_folder: record.is_folder,
            };
            snapshot.insert(relative_path, entry);
        }
        Ok((snapshot, duplicates))
    }

    /// 收敛同路径重复基线：可信云树命中该路径身份的行保留；否则保留最近成功结算
    /// 的非墓碑行；全为墓碑行时保留最近结算的墓碑行。其余行（含被新身份取代的
    /// 墓碑行）在同一事务内删除。孤行墓碑不经过此处，防重建语义不受影响。
    fn heal_duplicate_path_baselines(&self, duplicates: &DuplicateBaselines) -> AppResult<()> {
        let conn = self.db.lock();
        let authority_ids: HashMap<String, Option<String>> = {
            let cloud = self.cloud.lock();
            let trusted = cloud.trusted;
            duplicates
                .keys()
                .map(|path| {
                    let authority = trusted
                        .then(|| cloud.tree.get(path).map(|file| file.id.clone()))
                        .flatten();
                    (path.clone(), authority)
                })
                .collect()
        };
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始基线自愈事务失败：{error}")))?;
        for (relative_path, group) in duplicates {
            let authority = authority_ids.get(relative_path).cloned().flatten();
            let keep = select_baseline_to_keep(group, authority.as_deref());
            let removed: Vec<&str> = group
                .iter()
                .filter(|record| record.file_id != keep.file_id)
                .map(|record| record.file_id.as_str())
                .collect();
            for file_id in &removed {
                transaction
                    .execute(
                        "DELETE FROM sync_items WHERE file_id=?1 AND local_path=?2",
                        rusqlite::params![file_id, relative_path],
                    )
                    .map_err(|error| {
                        AppError::generic(format!(
                            "基线自愈清理旧身份行失败（{relative_path}）：{error}"
                        ))
                    })?;
            }
            tracing::warn!(
                rel = %relative_path,
                kept = %keep.file_id,
                removed = ?removed,
                authority = authority.as_deref().unwrap_or("<latest>"),
                "基线自愈：同路径重复基线已收敛"
            );
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交基线自愈事务失败：{error}")))
    }

    /// 确认云端副本、本地文件与成功基线一致后允许释放空间。
    pub fn can_safely_free_up(&self, rel_path: &str, file_id: &str) -> FreeUpCheckResult {
        if !self.cloud_tree_is_trusted() {
            return FreeUpCheckResult::NotSynced;
        }
        let cloud = self.cloud.lock();
        if cloud.tree.get(rel_path).map(|file| file.id.as_str()) != Some(file_id) {
            return FreeUpCheckResult::NotInCloud;
        }
        drop(cloud);
        let conn = self.db.lock();
        if repository::list_all_transfers(&conn).is_ok_and(|tasks| {
            tasks.into_iter().any(|task| {
                task.relative_path.as_deref() == Some(rel_path)
                    // 删除类判定用保守的非终态口径；状态损坏按活动处理（fail-closed）。
                    && task
                        .state_kind()
                        .map(|state| !state.is_terminal())
                        .unwrap_or(true)
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
            let mtime = crate::core::fs_meta::metadata_mtime_ms(&meta);
            if record.local_mtime != mtime || record.local_size != Some(meta.len() as i64) {
                FreeUpCheckResult::NotSynced
            } else {
                FreeUpCheckResult::Safe
            }
        } else {
            FreeUpCheckResult::NotSynced
        }
    }

    /// 清理「本地已失效的占位文件」。
    ///
    /// 只在以下条件同时成立时删除本地文件，避免把用户刚恢复的内容二次删掉：
    /// 1. 本地仍是 0 字节占位符（xattr 标记未变）；
    /// 2. 云端确实没有该路径（以当前可信云树为准）；
    /// 3. 调用方传入了该相对路径（必须来自 UI 上用户看到的那一条）。
    ///
    /// 返回实际清理数量。
    pub(super) fn cleanup_invalid_placeholder_inner(&self, relative_path: &str) -> AppResult<bool> {
        crate::core::paths::validate_relative_path(relative_path, false)?;
        let mount_dir = self
            .mount_dir
            .lock()
            .clone()
            .ok_or_else(|| AppError::generic("同步挂载目录未就绪"))?;
        let mount_dir = std::path::PathBuf::from(crate::core::paths::expand_tilde(&mount_dir));
        // 云端仍存在 → 占位符有效，拒绝清理。
        if self.cloud.lock().tree.contains_key(relative_path) {
            return Err(AppError::generic("该文件的云端副本仍然存在，无需清理"));
        }
        let abs = mount_dir.join(relative_path);
        let metadata = std::fs::symlink_metadata(&abs)
            .map_err(|error| AppError::generic(format!("读取占位文件失败：{error}")))?;
        // 只允许删除 0 字节且带 placeholder 标记的文件：真实内容绝不在这里被删。
        if !metadata.is_file()
            || metadata.len() != 0
            || !crate::mount::manager::is_placeholder_file(&abs)
        {
            return Err(AppError::generic("该路径已不是失效占位文件，拒绝删除"));
        }
        std::fs::remove_file(&abs)
            .map_err(|error| AppError::generic(format!("清理占位文件失败：{error}")))?;
        tracing::info!(rel = relative_path, "已清理失效占位文件");
        Ok(true)
    }

    /// 云端删除前的四道闸门，返回被拦下的条目数。
    ///
    /// 失效方向是**最危险**的那一侧：一旦本地树被误判成「文件都没了」
    /// （挂载卷被卸载、目录被清空/替换、权限变化），planner 会为整库文件生成
    /// `DeleteFromCloud`，用户云端数据就被真正删除。因此这里宁可拒绝：
    ///
    /// 1. 空扫描闸门：本地扫描为空而基线非空 → 整体拒绝（典型未挂载/空目录）；
    /// 2. 批量闸门：单轮要删的云端条目超过阈值且用户未确认 → 整体拒绝；
    /// 3. 逐条复核：删除前重新 stat 本地路径，存在（含占位符）或无法证明缺失 → 拒绝；
    /// 4. 执行侧还有一次同样语义的复核（`do_delete_from_cloud`）。
    pub(super) fn guard_cloud_deletes(
        &self,
        actions: &mut [crate::sync::state::SyncAction],
        local_len: usize,
        db_len: usize,
        cloud: &std::collections::HashMap<String, crate::drive::models::DriveFile>,
    ) -> usize {
        // 本批涉及的云端删除动作下标。
        let delete_indexes: Vec<usize> = actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.action_type == crate::sync::state::SyncActionType::DeleteFromCloud)
            .map(|(index, _)| index)
            .collect();
        if delete_indexes.is_empty() {
            // 本轮没有任何云端删除提议：清空宽限期记录（文件已恢复或删除已执行）。
            self.pending_cloud_deletes.lock().clear();
            return 0;
        }
        // 没有挂载目录就无法做任何本地核验：一律拒绝，绝不盲删云端。
        if self.mount_dir.lock().is_none() {
            tracing::error!(
                deletes = delete_indexes.len(),
                "同步挂载目录缺失，拒绝全部云端删除"
            );
            for index in &delete_indexes {
                downgrade_to_skip(
                    &mut actions[*index],
                    "同步挂载目录未就绪，无法核验本地状态，已拒绝云端删除",
                );
            }
            return delete_indexes.len();
        }
        // 这批删除会波及的云端条目数（目录按子树展开）。
        let affected = affected_cloud_entries(actions, &delete_indexes, cloud);
        // 用户确认是一次性令牌：只有真正放行本批时才消费。
        let approved = self.mass_delete_approved.swap(false, Ordering::AcqRel);
        match cloud_delete_gate(local_len, db_len, affected, approved) {
            CloudDeleteGate::Allow => {}
            CloudDeleteGate::BlockEmptyScan => {
                tracing::error!(
                    local_len,
                    db_len,
                    affected,
                    "本地扫描为空但同步基线非空，拒绝全部云端删除（疑似挂载目录未挂载或被清空）"
                );
                for index in &delete_indexes {
                    downgrade_to_skip(
                        &mut actions[*index],
                        "本地扫描为空，拒绝批量删除云端（疑似挂载目录异常）",
                    );
                }
                return affected;
            }
            CloudDeleteGate::BlockMassDelete { threshold } => {
                tracing::error!(
                    affected,
                    threshold,
                    local_len,
                    db_len,
                    "单轮云端删除超过安全阈值且未获确认，已全部拦下"
                );
                for index in &delete_indexes {
                    downgrade_to_skip(
                        &mut actions[*index],
                        "批量删除超出安全阈值，需用户确认后才会执行",
                    );
                }
                return affected;
            }
        }
        // 宽限期：给用户留出撤销误删的窗口（本地删了又马上恢复）。
        // 只有同一路径「连续」缺席超过阈值才真正删除；文件一旦回来，记录被清掉。
        if self.defer_undeleted_paths(actions, &delete_indexes, approved) {
            // 本批全部处于宽限期：不做逐条复核，也不计入「被阻止」。
            return 0;
        }
        self.validate_delete_from_cloud(actions);
        0
    }

    /// 对首次观察到缺席的路径延迟删除，返回是否仍有路径处于宽限期。
    ///
    /// 已获用户确认（`approved`）时跳过宽限：用户已经明确表态。
    fn defer_undeleted_paths(
        &self,
        actions: &mut [crate::sync::state::SyncAction],
        delete_indexes: &[usize],
        approved: bool,
    ) -> bool {
        // 本批提出的路径集合，用于清理已经恢复/已删除的旧记录。
        let proposed: std::collections::HashSet<String> = delete_indexes
            .iter()
            .filter_map(|index| actions[*index].relative_path.clone())
            .collect();
        // 当前时间（毫秒），同时驱动宽限期判断与过期记录清理。
        let now = chrono::Utc::now().timestamp_millis();
        let mut pending = self.pending_cloud_deletes.lock();
        // 只保留本批仍在提的路径，文件恢复后记录自动消失。
        pending.retain(|path, _| proposed.contains(path));
        if approved {
            pending.clear();
            return false;
        }
        let mut deferred = 0usize;
        for index in delete_indexes {
            let Some(path) = actions[*index].relative_path.clone() else {
                continue;
            };
            match pending.get(&path).copied() {
                Some(first_seen) if now.saturating_sub(first_seen) >= CLOUD_DELETE_GRACE_MS => {
                    // 已连续缺席超过宽限期，允许进入逐条复核。
                }
                Some(first_seen) => {
                    deferred += 1;
                    tracing::info!(
                        rel = %path,
                        absent_ms = now.saturating_sub(first_seen),
                        grace_ms = CLOUD_DELETE_GRACE_MS,
                        "本地删除处于宽限期，暂不删除云端"
                    );
                    downgrade_to_skip(
                        &mut actions[*index],
                        "本地删除处于宽限期，暂时保留云端文件（可撤销误删）",
                    );
                }
                None => {
                    // 首次观察到缺席：先记账，本轮绝不删除。
                    pending.insert(path.clone(), now);
                    deferred += 1;
                    tracing::info!(rel = %path, "首次观察到本地删除，进入宽限期等待确认");
                    downgrade_to_skip(
                        &mut actions[*index],
                        "首次检测到本地删除，等待宽限期结束后再同步删除云端",
                    );
                }
            }
        }
        deferred > 0
    }

    /// 云端删除前逐条复核本地路径，存在内容或无法证明缺失时改为 Skip。
    ///
    /// 占位符同样视为「本地仍存在」：占位符代表该条目的内容本来就在云端，
    /// 用户并没有在本地删除它，因此不能据此删掉云端唯一副本。
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
                    // 占位符（0 字节 + xattr state=placeholder）也计入「本地存在」：
                    // 它的内容本来就只在云端，删掉云端就等于删掉唯一副本。
                    let placeholder = size == 0 && crate::mount::manager::is_placeholder_file(&abs);
                    tracing::info!(
                        rel = %rel,
                        size,
                        placeholder,
                        reason = a.reason.as_deref().unwrap_or("?"),
                        "DeleteFromCloud 防误删：本地路径仍存在，改为 Skip"
                    );
                    downgrade_to_skip(
                        a,
                        if placeholder {
                            "防误删：本地占位符仍在，拒绝对唯一云端副本执行删除"
                        } else {
                            "防误删：本地文件实际存在，跳过 DeleteFromCloud"
                        },
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(rel = %rel, %error, "无法确认本地文件不存在，拒绝云端删除");
                    downgrade_to_skip(a, "本地路径访问异常，无法证明文件已删除，拒绝云端删除");
                    let _ = error;
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

    fn count_rows_by_local_path(db: &Arc<Mutex<Connection>>, path: &str) -> i64 {
        db.lock()
            .query_row(
                "SELECT COUNT(*) FROM sync_items WHERE local_path=?1",
                rusqlite::params![path],
                |row| row.get(0),
            )
            .unwrap()
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

    /// 同路径重复基线（服务器对覆盖上传换发新 fileId 后旧行残留）必须在加载时自愈
    /// 收敛，不能拒绝规划导致整个引擎瘫痪。
    #[test]
    fn load_db_snapshot_heals_duplicate_path_baselines() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        let mut stale = sync_item("old-cloud-id", "docs/manual.pdf", false);
        stale.status = sync_status::CLOUD_ONLY;
        stale.last_sync_time = Some(10);
        let mut fresh = sync_item("new-cloud-id", "docs/manual.pdf", false);
        fresh.status = sync_status::CONFLICT;
        fresh.last_sync_time = Some(20);
        repository::upsert(&db.lock(), &stale).unwrap();
        repository::upsert(&db.lock(), &fresh).unwrap();

        let snapshot = engine.load_db_snapshot().unwrap();

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot["docs/manual.pdf"].file_id, "new-cloud-id");
        assert!(
            repository::find_by_file_id(&db.lock(), "old-cloud-id")
                .unwrap()
                .is_none(),
            "旧身份残留行必须被自愈清除"
        );
        assert_eq!(
            count_rows_by_local_path(&db, "docs/manual.pdf"),
            1,
            "自愈后该路径只剩一行基线"
        );
    }

    /// 云树可信且命中该路径身份时，自愈必须保留云端权威行，即使它不是最近结算的。
    #[test]
    fn load_db_snapshot_heal_prefers_cloud_authority() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        let mut authoritative = sync_item("authoritative-id", "docs/manual.pdf", false);
        authoritative.last_sync_time = Some(10);
        let mut newer = sync_item("newer-orphan-id", "docs/manual.pdf", false);
        newer.last_sync_time = Some(20);
        repository::upsert(&db.lock(), &authoritative).unwrap();
        repository::upsert(&db.lock(), &newer).unwrap();
        engine.cloud_tree_insert(
            "docs/manual.pdf".into(),
            cloud_file("authoritative-id", "manual.pdf", "docs-id", false),
        );
        engine.set_cloud_tree_trusted(true);

        let snapshot = engine.load_db_snapshot().unwrap();

        assert_eq!(snapshot["docs/manual.pdf"].file_id, "authoritative-id");
        assert!(repository::find_by_file_id(&db.lock(), "newer-orphan-id")
            .unwrap()
            .is_none());
    }

    /// 墓碑行是该路径唯一基线时必须原样保留（用户主动删除的防重建语义）。
    #[test]
    fn load_db_snapshot_keeps_sole_tombstone_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        let mut tombstone = sync_item("deleted-id", "docs/manual.pdf", false);
        tombstone.status = sync_status::DELETED;
        repository::upsert(&db.lock(), &tombstone).unwrap();

        let snapshot = engine.load_db_snapshot().unwrap();

        assert_eq!(snapshot["docs/manual.pdf"].status, sync_status::DELETED);
        assert_eq!(count_rows_by_local_path(&db, "docs/manual.pdf"), 1);
    }

    /// 墓碑行与新身份基线并存时（同路径已被新文件取代），自愈保留正常基线并清除
    /// 过期墓碑，避免"合法并存"永远锁死规划。
    #[test]
    fn load_db_snapshot_heal_supersedes_stale_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        let mut tombstone = sync_item("deleted-id", "docs/manual.pdf", false);
        tombstone.status = sync_status::DELETED;
        tombstone.last_sync_time = Some(99);
        let mut active = sync_item("active-id", "docs/manual.pdf", false);
        active.last_sync_time = Some(20);
        repository::upsert(&db.lock(), &tombstone).unwrap();
        repository::upsert(&db.lock(), &active).unwrap();

        let snapshot = engine.load_db_snapshot().unwrap();

        assert_eq!(snapshot["docs/manual.pdf"].file_id, "active-id");
        assert!(
            repository::find_by_file_id(&db.lock(), "deleted-id")
                .unwrap()
                .is_none(),
            "被新身份取代的墓碑必须清除"
        );
    }

    /// 为冲突收敛测试建 transfer_queue 空表（test_engine 只建 sync_items）。
    fn ensure_transfer_queue_table(db: &Arc<Mutex<Connection>>) {
        db.lock()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS transfer_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    direction INTEGER NOT NULL,
                    file_id TEXT,
                    local_path TEXT,
                    name TEXT NOT NULL,
                    total_size INTEGER NOT NULL DEFAULT 0,
                    transferred INTEGER NOT NULL DEFAULT 0,
                    state INTEGER NOT NULL DEFAULT 0,
                    error_message TEXT,
                    created_at INTEGER NOT NULL,
                    finished_at INTEGER,
                    server_id TEXT,
                    upload_id TEXT,
                    resume_offset INTEGER NOT NULL DEFAULT 0,
                    session_url TEXT,
                    relative_path TEXT,
                    parent_file_id TEXT,
                    operation INTEGER,
                    source_mtime INTEGER,
                    source_size INTEGER,
                    expected_cloud_edited_time INTEGER,
                    attempt_count INTEGER NOT NULL DEFAULT 0,
                    verify_attempt_count INTEGER NOT NULL DEFAULT 0,
                    next_retry_at INTEGER,
                    error_kind INTEGER,
                    remote_result_file_id TEXT,
                    state_revision INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
    }

    /// 冲突已实际解决（云端权威身份与版本未变、本地与基线一致）时，
    /// 过期的 CONFLICT 状态必须收敛回已同步，冲突提示不得永久残留。
    #[test]
    fn reconcile_resolved_conflicts_recovers_synced_status() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        ensure_transfer_queue_table(&db);
        let rel = "docs/manual.pdf";
        let mut conflicted = sync_item("conflict-id", rel, false);
        conflicted.status = sync_status::CONFLICT;
        repository::upsert(&db.lock(), &conflicted).unwrap();

        let mut cloud = cloud_file("conflict-id", "manual.pdf", "docs-id", false);
        cloud.edited_time = chrono::DateTime::from_timestamp_millis(22);
        let cloud = HashMap::from([(rel.to_string(), cloud)]);
        let local = HashMap::from([(
            rel.to_string(),
            LocalFileEntry {
                absolute_path: temp.path().join(rel),
                relative_path: rel.to_string(),
                size: 7,
                mtime: 11,
                is_folder: false,
                is_placeholder: false,
            },
        )]);
        let db_snapshot = engine.load_db_snapshot().unwrap();

        let resolved = engine
            .reconcile_resolved_conflicts(&local, &db_snapshot, &cloud)
            .unwrap();

        assert_eq!(resolved, 1);
        assert_eq!(
            repository::find_by_file_id(&db.lock(), "conflict-id")
                .unwrap()
                .unwrap()
                .status,
            sync_status::SYNCED
        );
    }

    /// 云端版本已再变化（edited_time 与基线不一致）时不得收敛，留给规划器处理。
    #[test]
    fn reconcile_resolved_conflicts_skips_when_cloud_changed() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        ensure_transfer_queue_table(&db);
        let rel = "docs/manual.pdf";
        let mut conflicted = sync_item("conflict-id", rel, false);
        conflicted.status = sync_status::CONFLICT;
        repository::upsert(&db.lock(), &conflicted).unwrap();

        let mut cloud = cloud_file("conflict-id", "manual.pdf", "docs-id", false);
        cloud.edited_time = chrono::DateTime::from_timestamp_millis(99);
        let cloud = HashMap::from([(rel.to_string(), cloud)]);
        let local = HashMap::from([(
            rel.to_string(),
            LocalFileEntry {
                absolute_path: temp.path().join(rel),
                relative_path: rel.to_string(),
                size: 7,
                mtime: 11,
                is_folder: false,
                is_placeholder: false,
            },
        )]);
        let db_snapshot = engine.load_db_snapshot().unwrap();

        let resolved = engine
            .reconcile_resolved_conflicts(&local, &db_snapshot, &cloud)
            .unwrap();

        assert_eq!(resolved, 0);
        assert_eq!(
            repository::find_by_file_id(&db.lock(), "conflict-id")
                .unwrap()
                .unwrap()
                .status,
            sync_status::CONFLICT
        );
    }

    /// 本地文件在冲突结算后又被修改（mtime 与基线不一致）时不得收敛。
    #[test]
    fn reconcile_resolved_conflicts_skips_when_local_edited() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, db) = test_engine(temp.path());
        ensure_transfer_queue_table(&db);
        let rel = "docs/manual.pdf";
        let mut conflicted = sync_item("conflict-id", rel, false);
        conflicted.status = sync_status::CONFLICT;
        repository::upsert(&db.lock(), &conflicted).unwrap();

        let mut cloud = cloud_file("conflict-id", "manual.pdf", "docs-id", false);
        cloud.edited_time = chrono::DateTime::from_timestamp_millis(22);
        let cloud = HashMap::from([(rel.to_string(), cloud)]);
        let local = HashMap::from([(
            rel.to_string(),
            LocalFileEntry {
                absolute_path: temp.path().join(rel),
                relative_path: rel.to_string(),
                size: 7,
                mtime: 99,
                is_folder: false,
                is_placeholder: false,
            },
        )]);
        let db_snapshot = engine.load_db_snapshot().unwrap();

        let resolved = engine
            .reconcile_resolved_conflicts(&local, &db_snapshot, &cloud)
            .unwrap();

        assert_eq!(resolved, 0);
    }
}

/// 单轮云端删除的安全阈值：超过这个条目数就必须由用户显式确认。
///
/// 阈值不是「性能阈值」，而是「误删爆炸半径」：一次误判最多影响这么多条云端文件。
/// 正常的逐文件删除远低于它；删整棵子树会按子树条目数计入。
const MASS_DELETE_CONFIRM_THRESHOLD: usize = 100;
/// 本地删除到云端删除之间的宽限期（毫秒）。
///
/// 用户误删后常常在几秒到几十秒内从废纸篓恢复；宽限期让这类操作完全无害。
/// 取 60 秒：与默认自动周期一致，正常情况下删除会延后一轮周期执行。
const CLOUD_DELETE_GRACE_MS: i64 = 60_000;
/// 相对阈值：一次要删掉一半以上的同步基线时也必须确认。
///
/// 覆盖「挂载目录被清空但有少量残留文件」的形态：此时绝对阈值可能不触发，
/// 但「一半以上的文件同时消失」在真实使用里几乎只可能是环境异常。
/// 条目数下限避免小目录（如只有 4 个文件）频繁打扰。
const MASS_DELETE_RATIO_MIN_ENTRIES: usize = 10;

/// 云端删除闸门的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudDeleteGate {
    /// 允许继续逐条复核。
    Allow,
    /// 本地扫描为空而基线非空：整体拒绝。
    BlockEmptyScan,
    /// 超出批量删除阈值且未获用户确认：整体拒绝。
    BlockMassDelete {
        /// 当前生效的阈值，便于日志与前端提示对齐。
        threshold: usize,
    },
}

/// 计算一批删除动作波及的云端条目数（目录按子树展开）。
fn affected_cloud_entries(
    actions: &[crate::sync::state::SyncAction],
    delete_indexes: &[usize],
    cloud: &std::collections::HashMap<String, crate::drive::models::DriveFile>,
) -> usize {
    let mut affected = 0usize;
    for index in delete_indexes {
        let Some(relative_path) = actions[*index].relative_path.as_deref() else {
            affected += 1;
            continue;
        };
        // 目录删除会被云端级联到整棵子树，按实际条目计入爆炸半径。
        let is_folder = cloud
            .get(relative_path)
            .map(|file| file.is_folder())
            .unwrap_or(false);
        if !is_folder {
            affected += 1;
            continue;
        }
        // 子树前缀。
        let prefix = format!("{relative_path}/");
        affected += 1 + cloud
            .keys()
            .filter(|path| path.starts_with(&prefix))
            .count();
    }
    affected
}

/// 计算闸门判定，纯函数便于单测覆盖。
fn cloud_delete_gate(
    local_len: usize,
    db_len: usize,
    affected: usize,
    approved: bool,
) -> CloudDeleteGate {
    // 空扫描 + 非空基线：唯一可能是本地树不可见（未挂载/被清空/权限变化）。
    if local_len == 0 && db_len > 0 {
        return CloudDeleteGate::BlockEmptyScan;
    }
    // 绝对阈值：一次误判的爆炸半径过大。
    if affected >= MASS_DELETE_CONFIRM_THRESHOLD && !approved {
        return CloudDeleteGate::BlockMassDelete {
            threshold: MASS_DELETE_CONFIRM_THRESHOLD,
        };
    }
    // 相对阈值：一次删掉一半以上基线，通常意味着本地树不可见而不是用户手删。
    if !approved
        && affected >= MASS_DELETE_RATIO_MIN_ENTRIES
        && affected.saturating_mul(2) >= db_len
    {
        return CloudDeleteGate::BlockMassDelete {
            threshold: MASS_DELETE_RATIO_MIN_ENTRIES,
        };
    }
    CloudDeleteGate::Allow
}

/// 把云端删除动作降级为 Skip，并记录面向用户的原因。
fn downgrade_to_skip(action: &mut crate::sync::state::SyncAction, reason: &str) {
    action.action_type = crate::sync::state::SyncActionType::Skip;
    action.reason = Some(reason.to_string());
}

#[cfg(test)]
mod delete_gate_tests {
    use super::{affected_cloud_entries, cloud_delete_gate, CloudDeleteGate};
    use crate::drive::models::DriveFile;
    use crate::sync::state::{SyncAction, SyncActionType};
    use std::collections::HashMap;

    /// 构造最小动作。
    fn action(action_type: SyncActionType, path: &str) -> SyncAction {
        SyncAction {
            action_type,
            relative_path: Some(path.to_string()),
            file_id: Some(format!("id-{path}")),
            parent_file_id: None,
            local_path: None,
            cloud_file: None,
            reason: None,
        }
    }

    /// 构造最小云端条目（目录或文件）。
    fn cloud_file(id: &str, name: &str, folder: bool) -> DriveFile {
        DriveFile {
            id: id.to_string(),
            name: name.to_string(),
            category: if folder {
                crate::drive::models::FileCategory::Folder
            } else {
                crate::drive::models::FileCategory::Video
            },
            size: 0,
            parent_folder: None,
            description: None,
            created_time: None,
            edited_time: None,
            mime_type: None,
            content_hash: None,
            thumbnail_link: None,
        }
    }

    /// 空扫描 + 非空基线必须整体拒绝：这是「挂载目录被清空」的典型形态。
    #[test]
    fn empty_scan_blocks_every_cloud_delete() {
        assert_eq!(
            cloud_delete_gate(0, 1024, 1, false),
            CloudDeleteGate::BlockEmptyScan
        );
        // 用户确认也不能放行：空扫描不是「用户要删很多」，而是「本地看不见」。
        assert_eq!(
            cloud_delete_gate(0, 1024, 1, true),
            CloudDeleteGate::BlockEmptyScan
        );
        // 基线也为空（首轮同步）时没有可删的东西，放行。
        assert_eq!(cloud_delete_gate(0, 0, 0, false), CloudDeleteGate::Allow);
    }

    /// 一次删掉一半以上基线也要确认：覆盖「目录被清空但有残留」的形态。
    #[test]
    fn deleting_half_of_baseline_requires_approval() {
        // 80 / 85：绝对阈值没到，但超过一半 → 拦下。
        assert!(matches!(
            cloud_delete_gate(85, 85, 80, false),
            CloudDeleteGate::BlockMassDelete { .. }
        ));
        // 少量删除（低于条目下限）不打扰。
        assert_eq!(cloud_delete_gate(85, 85, 9, false), CloudDeleteGate::Allow);
        // 大库里删少量文件不受影响。
        assert_eq!(
            cloud_delete_gate(1000, 1000, 50, false),
            CloudDeleteGate::Allow
        );
        // 用户确认后放行。
        assert_eq!(cloud_delete_gate(85, 85, 80, true), CloudDeleteGate::Allow);
    }

    /// 批量删除超过阈值必须等用户确认。
    #[test]
    fn mass_delete_requires_explicit_approval() {
        assert_eq!(
            cloud_delete_gate(1024, 1024, 99, false),
            CloudDeleteGate::Allow
        );
        assert!(matches!(
            cloud_delete_gate(1024, 1024, 100, false),
            CloudDeleteGate::BlockMassDelete { .. }
        ));
        // 用户确认后放行。
        assert_eq!(
            cloud_delete_gate(1024, 1024, 100, true),
            CloudDeleteGate::Allow
        );
    }

    /// 目录删除按整棵子树计入爆炸半径：一条动作可能代表上千个文件。
    #[test]
    fn folder_delete_counts_whole_subtree() {
        let cloud = HashMap::from([
            ("photos".to_string(), cloud_file("f-photos", "photos", true)),
            (
                "photos/a.jpg".to_string(),
                cloud_file("f-a", "a.jpg", false),
            ),
            (
                "photos/b.jpg".to_string(),
                cloud_file("f-b", "b.jpg", false),
            ),
            (
                "photos/sub/c.jpg".to_string(),
                cloud_file("f-c", "c.jpg", false),
            ),
            (
                "other.txt".to_string(),
                cloud_file("f-other", "other.txt", false),
            ),
        ]);
        let actions = vec![
            action(SyncActionType::DeleteFromCloud, "photos"),
            action(SyncActionType::DeleteFromCloud, "other.txt"),
        ];

        // photos 自身 + 3 个后代，加 other.txt 共 5 条。
        assert_eq!(affected_cloud_entries(&actions, &[0, 1], &cloud), 5);
    }
}

/// 云端删除「逐条本地复核」的回归测试：只有本地确实不存在（且不是占位符）才允许删云端。
#[cfg(test)]
mod local_presence_guard_tests {
    use super::*;
    use crate::drive::models::FileCategory;
    use crate::sync::state::{SyncAction, SyncActionType};
    use std::sync::Arc;

    /// 建一个带挂载目录的最小引擎。
    fn engine_with_mount(root: &std::path::Path) -> SyncEngine {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sync_items (
                    file_id TEXT NOT NULL, local_path TEXT NOT NULL, parent_folder_id TEXT,
                    name TEXT NOT NULL, is_folder INTEGER NOT NULL DEFAULT 0,
                    size INTEGER NOT NULL DEFAULT 0, local_size INTEGER, sha256 TEXT,
                    local_mtime INTEGER, cloud_edited_time INTEGER, last_sync_time INTEGER,
                    status INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                    PRIMARY KEY (file_id, local_path)
                );
                CREATE TABLE transfer_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, direction INTEGER NOT NULL,
                    file_id TEXT, local_path TEXT, name TEXT NOT NULL,
                    total_size INTEGER NOT NULL DEFAULT 0, transferred INTEGER NOT NULL DEFAULT 0,
                    state INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                    created_at INTEGER NOT NULL, finished_at INTEGER, server_id TEXT,
                    upload_id TEXT, resume_offset INTEGER NOT NULL DEFAULT 0, session_url TEXT,
                    relative_path TEXT, parent_file_id TEXT, operation INTEGER,
                    source_mtime INTEGER, source_size INTEGER, expected_cloud_edited_time INTEGER,
                    attempt_count INTEGER NOT NULL DEFAULT 0,
                    verify_attempt_count INTEGER NOT NULL DEFAULT 0, next_retry_at INTEGER,
                    error_kind INTEGER, remote_result_file_id TEXT,
                    state_revision INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
        let db = Arc::new(parking_lot::Mutex::new(connection));
        let auth = Arc::new(crate::auth::service::AuthService::new());
        let client = Arc::new(crate::drive::client::DriveClient::new(auth));
        let files_api = Arc::new(crate::drive::files_api::FilesApi::new(client.clone()));
        let changes_api = Arc::new(crate::drive::changes_api::ChangesApi::new(client));
        let mut engine = SyncEngine::new(
            files_api,
            changes_api,
            db,
            Arc::new(crate::sync::status_aggregator::StatusAggregator::default()),
            Vec::new(),
            3,
            0,
        );
        engine.set_mount(Arc::new(crate::mount::manager::MountManager::new(root)));
        engine
    }

    /// 构造一条云端删除动作。
    fn delete_action(path: &str) -> SyncAction {
        SyncAction {
            action_type: SyncActionType::DeleteFromCloud,
            relative_path: Some(path.to_string()),
            file_id: Some(format!("id-{path}")),
            parent_file_id: None,
            local_path: None,
            cloud_file: None,
            reason: Some("会话内删除 → 双向删除云端".to_string()),
        }
    }

    /// 构造云端缓存：单文件 + 一个目录子树。
    fn cloud_tree() -> HashMap<String, DriveFile> {
        // 最小 DriveFile。
        let file = |id: &str, name: &str, folder: bool| DriveFile {
            id: id.to_string(),
            name: name.to_string(),
            category: if folder {
                FileCategory::Folder
            } else {
                FileCategory::Video
            },
            size: 0,
            parent_folder: None,
            description: None,
            created_time: None,
            edited_time: None,
            mime_type: None,
            content_hash: None,
            thumbnail_link: None,
        };
        HashMap::from([
            ("gone.txt".to_string(), file("f-gone", "gone.txt", false)),
            ("docs".to_string(), file("f-docs", "docs", true)),
            ("docs/a.txt".to_string(), file("f-a", "a.txt", false)),
        ])
    }

    /// 没有挂载目录时无法核验本地状态，必须一律拒绝。
    #[test]
    fn missing_mount_blocks_every_delete() {
        // 与 engine_with_mount 相同的构造，但不绑定挂载目录。
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sync_items (
                    file_id TEXT NOT NULL, local_path TEXT NOT NULL, parent_folder_id TEXT,
                    name TEXT NOT NULL, is_folder INTEGER NOT NULL DEFAULT 0,
                    size INTEGER NOT NULL DEFAULT 0, local_size INTEGER, sha256 TEXT,
                    local_mtime INTEGER, cloud_edited_time INTEGER, last_sync_time INTEGER,
                    status INTEGER NOT NULL DEFAULT 0, error_message TEXT,
                    PRIMARY KEY (file_id, local_path)
                );",
            )
            .unwrap();
        let db = Arc::new(parking_lot::Mutex::new(connection));
        let auth = Arc::new(crate::auth::service::AuthService::new());
        let client = Arc::new(crate::drive::client::DriveClient::new(auth));
        let files_api = Arc::new(crate::drive::files_api::FilesApi::new(client.clone()));
        let changes_api = Arc::new(crate::drive::changes_api::ChangesApi::new(client));
        let engine = SyncEngine::new(
            files_api,
            changes_api,
            db,
            Arc::new(crate::sync::status_aggregator::StatusAggregator::default()),
            Vec::new(),
            3,
            0,
        );
        let cloud = cloud_tree();
        let mut actions = vec![delete_action("gone.txt")];

        let blocked = engine.guard_cloud_deletes(&mut actions, 10, 10, &cloud);

        assert_eq!(blocked, 1);
        assert_eq!(actions[0].action_type, SyncActionType::Skip);
    }

    /// 失效占位清理必须三重守卫：真文件、云端仍存在、已不是占位符都不能删。
    #[test]
    fn cleanup_only_removes_genuinely_stale_placeholder() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine_with_mount(temp.path());
        // 云端仍存在该路径 → 拒绝清理。
        let existing = temp.path().join("kept.txt");
        std::fs::write(&existing, b"").unwrap();
        assert!(engine
            .cleanup_invalid_placeholder_inner("gone.txt")
            .is_err());

        // 云端不存在，但本地是普通空文件（无 placeholder 标记）→ 拒绝清理。
        let empty_file = temp.path().join("empty.txt");
        std::fs::write(&empty_file, b"").unwrap();
        assert!(engine
            .cleanup_invalid_placeholder_inner("empty.txt")
            .is_err());

        // 本地是真实内容 → 拒绝清理。
        let real_file = temp.path().join("real.txt");
        std::fs::write(&real_file, b"content").unwrap();
        assert!(engine
            .cleanup_invalid_placeholder_inner("real.txt")
            .is_err());
        assert!(real_file.exists(), "真实内容绝不能被这里删掉");
    }

    /// 场景一：本地误删后很快恢复 —— 宽限期挡住删除，恢复后不再提出删除且记录清空。
    #[test]
    fn quick_restore_within_grace_window_cancels_cloud_delete() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine_with_mount(temp.path());
        let cloud = cloud_tree();
        let mut actions = vec![delete_action("gone.txt")];

        // 第一次观察：进入宽限期，拒绝删除。
        let blocked = engine.guard_cloud_deletes(&mut actions, 10, 10, &cloud);
        assert_eq!(blocked, 0, "宽限期不算「被安全闸门阻止」");
        assert_eq!(actions[0].action_type, SyncActionType::Skip);

        // 用户在宽限期内恢复文件：planner 不再为该路径提出删除。
        std::fs::write(temp.path().join("gone.txt"), b"restored").unwrap();
        let mut no_actions: Vec<SyncAction> = Vec::new();
        engine.guard_cloud_deletes(&mut no_actions, 10, 10, &cloud);
        assert!(
            engine.pending_cloud_deletes.lock().is_empty(),
            "文件恢复后宽限期记录必须清空"
        );

        // 即使某轮仍然错误地提出了删除，逐条复核也会因文件存在而拦住。
        let mut stale_actions = vec![delete_action("gone.txt")];
        engine.guard_cloud_deletes(&mut stale_actions, 10, 10, &cloud);
        assert_eq!(stale_actions[0].action_type, SyncActionType::Skip);
    }

    /// 宽限期到期后才允许真正删除云端。
    #[test]
    fn cloud_delete_proceeds_after_grace_window() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine_with_mount(temp.path());
        let cloud = cloud_tree();
        let mut actions = vec![delete_action("gone.txt")];
        engine.guard_cloud_deletes(&mut actions, 10, 10, &cloud);

        // 把首次观察时间改成「很久以前」，模拟宽限期已过。
        {
            let mut pending = engine.pending_cloud_deletes.lock();
            let first_seen = pending.get("gone.txt").copied().expect("应已记账");
            pending.insert("gone.txt".to_string(), first_seen - 120_000);
        }
        let mut actions = vec![delete_action("gone.txt")];
        engine.guard_cloud_deletes(&mut actions, 10, 10, &cloud);

        assert_eq!(actions[0].action_type, SyncActionType::DeleteFromCloud);
    }

    /// 本地文件仍在 → 拒绝云端删除；本地确实不存在 → 允许（需已过宽限期）。
    #[test]
    fn existing_local_file_blocks_cloud_delete() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("gone.txt"), b"still here").unwrap();
        let engine = engine_with_mount(temp.path());
        // 预先让宽限期过期，聚焦「逐条本地复核」这一层。
        {
            let mut pending = engine.pending_cloud_deletes.lock();
            for path in ["gone.txt", "docs"] {
                pending.insert(path.to_string(), 0);
            }
        }
        let cloud = cloud_tree();
        let mut actions = vec![
            delete_action("gone.txt"),
            // 本地不存在的目录可以删。
            delete_action("docs"),
        ];

        let blocked = engine.guard_cloud_deletes(&mut actions, 10, 10, &cloud);

        assert_eq!(blocked, 0, "逐条复核不产生「批量拦截」计数");
        assert_eq!(
            actions[0].action_type,
            SyncActionType::Skip,
            "本地文件仍存在时必须降级为 Skip"
        );
        assert_eq!(actions[1].action_type, SyncActionType::DeleteFromCloud);
    }

    /// 本地为空而基线非空 → 整批拒绝（挂载目录被清空/未挂载的典型形态）。
    #[test]
    fn empty_local_scan_blocks_every_delete() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine_with_mount(temp.path());
        let cloud = cloud_tree();
        let mut actions = vec![delete_action("gone.txt"), delete_action("docs")];

        let blocked = engine.guard_cloud_deletes(&mut actions, 0, 500, &cloud);

        // gone.txt 1 条 + docs 子树（docs 自身 + docs/a.txt）2 条。
        assert_eq!(blocked, 3);
        assert!(actions
            .iter()
            .all(|action| action.action_type == SyncActionType::Skip));
    }

    /// 批量阈值：未确认时整批拦下，确认后放行（确认同时跳过宽限期）。
    #[test]
    fn mass_delete_requires_approval_then_allows() {
        let temp = tempfile::tempdir().unwrap();
        let engine = engine_with_mount(temp.path());
        // 构造 100 个云端文件。
        let mut cloud = HashMap::new();
        for index in 0..100 {
            // 每个云端条目的路径。
            let path = format!("bulk-{index}.txt");
            cloud.insert(
                path.clone(),
                DriveFile {
                    id: format!("f-{index}"),
                    name: path.clone(),
                    category: FileCategory::Video,
                    size: 0,
                    parent_folder: None,
                    description: None,
                    created_time: None,
                    edited_time: None,
                    mime_type: None,
                    content_hash: None,
                    thumbnail_link: None,
                },
            );
        }
        let mut actions: Vec<SyncAction> = (0..100)
            .map(|index| delete_action(&format!("bulk-{index}.txt")))
            .collect();

        // 未确认：全部拦下。
        let blocked = engine.guard_cloud_deletes(&mut actions, 1000, 1000, &cloud);
        assert_eq!(blocked, 100);
        assert!(actions
            .iter()
            .all(|action| action.action_type == SyncActionType::Skip));

        // 用户确认后放行（本地均不存在）。
        let mut actions: Vec<SyncAction> = (0..100)
            .map(|index| delete_action(&format!("bulk-{index}.txt")))
            .collect();
        engine.approve_mass_delete();
        let blocked = engine.guard_cloud_deletes(&mut actions, 1000, 1000, &cloud);
        assert_eq!(blocked, 0);
        assert!(actions
            .iter()
            .all(|action| action.action_type == SyncActionType::DeleteFromCloud));
    }
}
