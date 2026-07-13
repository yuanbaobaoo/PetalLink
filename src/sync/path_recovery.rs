//! Crash recovery for direct remote rename/move commands.
//!
//! The command persists the cloud fileId as an xattr on the local source before the remote write.
//! Once a trusted cloud tree places that same fileId at another path, this module can safely finish
//! the local rename and atomically re-key the `sync_items` subtree without inventing a new content
//! baseline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::data::repository::{self, SyncItem};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::mount::manager::XATTR_FILE_ID;
use crate::sync::transfer_state::TransferState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathRecoverySummary {
    pub rekeyed_roots: usize,
    pub skipped_unmarked: usize,
}

/// Finish direct path changes only from a current, complete cloud tree. Callers own that trust
/// decision; this function deliberately performs no network fallback.
pub(crate) fn recover_verified_remote_path_changes<F, G>(
    mount_root: &Path,
    conn: &Connection,
    cloud_tree: &HashMap<String, DriveFile>,
    mut acquire_path_leases: F,
) -> AppResult<PathRecoverySummary>
where
    F: FnMut(&str, &str) -> AppResult<G>,
{
    let initial_records = repository::load_all(conn)?;
    let mut cloud_by_id: HashMap<String, Option<(String, DriveFile)>> = HashMap::new();
    for (path, file) in cloud_tree {
        match cloud_by_id.entry(file.id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some((path.clone(), file.clone())));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                // A duplicated fileId makes path identity non-unique. Never choose either path.
                entry.insert(None);
            }
        }
    }

    let mut candidates = initial_records
        .into_iter()
        .filter_map(|record| {
            let (new_path, cloud_file) = cloud_by_id.get(&record.file_id)?.as_ref()?;
            (new_path != &record.local_path).then(|| (record, new_path.clone(), cloud_file.clone()))
        })
        .collect::<Vec<_>>();
    // A moved folder re-keys its descendants. Process shallow folders first so stale child
    // candidates disappear before they can attempt a second filesystem rename.
    candidates.sort_by_key(|(record, _, _)| {
        (
            Path::new(&record.local_path).components().count(),
            !record.is_folder,
        )
    });

    let mut summary = PathRecoverySummary::default();
    for (candidate, new_path, cloud_file) in candidates {
        let records = repository::load_all(conn)?;
        let Some(current) = records.iter().find(|record| {
            record.file_id == candidate.file_id && record.local_path == candidate.local_path
        }) else {
            continue;
        };
        // Hold source and target leases across the final identity checks, filesystem rename and
        // DB transaction. This closes the race with a newly admitted transfer or direct command.
        let _path_leases = acquire_path_leases(&current.local_path, &new_path)?;
        match recover_one(
            mount_root,
            conn,
            cloud_tree,
            &cloud_by_id,
            &records,
            current,
            &new_path,
            &cloud_file,
        )? {
            RecoveryOutcome::Rekeyed => summary.rekeyed_roots += 1,
            RecoveryOutcome::Unmarked => summary.skipped_unmarked += 1,
        }
    }
    Ok(summary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryOutcome {
    Rekeyed,
    Unmarked,
}

#[allow(clippy::too_many_arguments)]
fn recover_one(
    mount_root: &Path,
    conn: &Connection,
    cloud_tree: &HashMap<String, DriveFile>,
    cloud_by_id: &HashMap<String, Option<(String, DriveFile)>>,
    records: &[SyncItem],
    root: &SyncItem,
    new_root: &str,
    cloud_root: &DriveFile,
) -> AppResult<RecoveryOutcome> {
    crate::core::paths::validate_relative_path(&root.local_path, false)?;
    crate::core::paths::validate_relative_path(new_root, false)?;
    if records
        .iter()
        .filter(|record| record.file_id == root.file_id)
        .count()
        != 1
    {
        return Err(AppError::generic(format!(
            "同一 fileId 存在多条本地基线，拒绝猜测路径：{}",
            root.file_id
        )));
    }
    if cloud_root.is_folder() != root.is_folder {
        return Err(AppError::generic(format!(
            "云端与本地基线类型不一致，拒绝路径恢复：{}",
            root.file_id
        )));
    }
    if root.is_folder && is_in_subtree(new_root, &root.local_path) {
        return Err(AppError::generic(format!(
            "远端路径恢复拒绝把目录移入自身：{} -> {new_root}",
            root.local_path
        )));
    }

    let old_absolute = crate::core::paths::safe_join_under(mount_root, &root.local_path, false)?;
    let new_absolute = crate::core::paths::safe_join_under(mount_root, new_root, false)?;
    let old_metadata = optional_metadata(&old_absolute)?;
    let new_metadata = optional_metadata(&new_absolute)?;
    let old_owner = if old_metadata.is_some() {
        read_file_id(&old_absolute)?
    } else {
        None
    };
    let new_owner = if new_metadata.is_some() {
        read_file_id(&new_absolute)?
    } else {
        None
    };

    let local_already_moved = match (old_metadata.as_ref(), new_metadata.as_ref()) {
        (Some(_), Some(_)) => {
            return Err(AppError::generic(format!(
                "远端路径已变更，但源和目标同时存在，拒绝覆盖：{} -> {new_root}",
                root.local_path
            )))
        }
        (Some(metadata), None) => {
            if old_owner.as_deref() != Some(root.file_id.as_str()) {
                tracing::warn!(
                    old = %root.local_path,
                    new = %new_root,
                    file_id = %root.file_id,
                    "路径变化缺少本地持久身份，交回既有同步逻辑"
                );
                return Ok(RecoveryOutcome::Unmarked);
            }
            validate_local_type(metadata, root, &old_absolute)?;
            false
        }
        (None, Some(metadata)) => {
            if new_owner.as_deref() != Some(root.file_id.as_str()) {
                return Err(AppError::generic(format!(
                    "远端路径已变更，但目标被其他本地内容占用：{new_root}"
                )));
            }
            validate_local_type(metadata, root, &new_absolute)?;
            true
        }
        (None, None) => return Ok(RecoveryOutcome::Unmarked),
    };

    let subtree = records
        .iter()
        .filter(|record| is_in_subtree(&record.local_path, &root.local_path))
        .cloned()
        .collect::<Vec<_>>();
    for record in &subtree {
        let expected_path = rekey_path(&record.local_path, &root.local_path, new_root)?;
        if let Some(Some((actual_path, _))) = cloud_by_id.get(&record.file_id) {
            if actual_path != &expected_path {
                return Err(AppError::generic(format!(
                    "远端子树路径不一致，拒绝本地重键：{} 期望 {expected_path}，实际 {actual_path}",
                    record.local_path
                )));
            }
        }
    }
    if records.iter().any(|record| {
        !is_in_subtree(&record.local_path, &root.local_path)
            && is_in_subtree(&record.local_path, new_root)
    }) {
        return Err(AppError::generic(format!(
            "目标 DB 子树已被其他记录占用，拒绝覆盖：{new_root}"
        )));
    }
    if cloud_tree
        .get(new_root)
        .map_or(true, |file| file.id != root.file_id)
    {
        return Err(AppError::generic(format!(
            "可信云树无法确认目标 fileId，拒绝路径恢复：{new_root}"
        )));
    }
    ensure_no_active_transfer(conn, &root.file_id, &root.local_path, new_root)?;

    if !local_already_moved {
        ensure_safe_target_parent(mount_root, new_root)?;
        rename_no_replace(&old_absolute, &new_absolute).map_err(|error| {
            AppError::generic(format!(
                "本地路径恢复失败，源内容保持不覆盖：{} -> {new_root}：{error}",
                root.local_path
            ))
        })?;
    }

    rekey_db_subtree(conn, &subtree, &root.local_path, new_root, cloud_root)?;
    tracing::info!(
        old = %root.local_path,
        new = %new_root,
        file_id = %root.file_id,
        descendants = subtree.len().saturating_sub(1),
        "已收敛中断的远端路径变更"
    );
    Ok(RecoveryOutcome::Rekeyed)
}

fn ensure_no_active_transfer(
    conn: &Connection,
    file_id: &str,
    old_root: &str,
    new_root: &str,
) -> AppResult<()> {
    let active = repository::list_all_transfers(conn)?
        .into_iter()
        .any(|task| {
            let state_is_active = task.state_kind().is_ok_and(|state| {
                !matches!(
                    state,
                    TransferState::Completed | TransferState::Failed | TransferState::Canceled
                )
            });
            state_is_active
                && (task.file_id.as_deref() == Some(file_id)
                    || task.relative_path.as_deref().is_some_and(|path| {
                        is_in_subtree(path, old_root) || is_in_subtree(path, new_root)
                    }))
        });
    if active {
        return Err(AppError::generic(format!(
            "路径恢复涉及活动传输，拒绝并发重键：{old_root} -> {new_root}"
        )));
    }
    Ok(())
}

fn rekey_db_subtree(
    conn: &Connection,
    subtree: &[SyncItem],
    old_root: &str,
    new_root: &str,
    cloud_root: &DriveFile,
) -> AppResult<()> {
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| AppError::generic(format!("开始路径恢复事务失败：{error}")))?;
    for record in subtree {
        transaction
            .execute(
                "DELETE FROM sync_items WHERE file_id=?1 AND local_path=?2",
                rusqlite::params![record.file_id, record.local_path],
            )
            .map_err(|error| AppError::generic(format!("删除旧路径基线失败：{error}")))?;
    }
    for record in subtree {
        let mut moved = record.clone();
        moved.local_path = rekey_path(&record.local_path, old_root, new_root)?;
        if moved.file_id == cloud_root.id {
            moved.name = cloud_root.name.clone();
            moved.parent_folder_id = cloud_root
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned());
            moved.size = cloud_root.size;
            moved.cloud_edited_time = cloud_root
                .edited_time
                .map(|edited_time| edited_time.timestamp_millis());
        }
        // Preserve local mtime/size/hash and sync status: a structural move proves no content
        // version and must not manufacture a new successful content baseline.
        repository::upsert(&transaction, &moved)?;
    }
    transaction
        .commit()
        .map_err(|error| AppError::generic(format!("提交路径恢复事务失败：{error}")))?;
    Ok(())
}

fn rekey_path(path: &str, old_root: &str, new_root: &str) -> AppResult<String> {
    if path == old_root {
        return Ok(new_root.to_string());
    }
    let suffix = path
        .strip_prefix(old_root)
        .filter(|suffix| suffix.starts_with('/'))
        .ok_or_else(|| AppError::generic(format!("路径不属于待重键子树：{path}")))?;
    Ok(format!("{new_root}{suffix}"))
}

fn is_in_subtree(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(crate) fn optional_metadata(path: &Path) -> AppResult<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::generic(format!(
            "读取路径恢复元数据失败（{}）：{error}",
            path.display()
        ))),
    }
}

pub(crate) fn validate_local_type(
    metadata: &std::fs::Metadata,
    record: &SyncItem,
    path: &Path,
) -> AppResult<()> {
    let file_type = metadata.file_type();
    if file_type.is_symlink()
        || (record.is_folder && !file_type.is_dir())
        || (!record.is_folder && !file_type.is_file())
    {
        return Err(AppError::generic(format!(
            "路径恢复目标类型不一致，拒绝操作：{}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn read_file_id(path: &Path) -> AppResult<Option<String>> {
    xattr::get(path, XATTR_FILE_ID)
        .map_err(|error| {
            AppError::generic(format!(
                "读取路径恢复 fileId 失败（{}）：{error}",
                path.display()
            ))
        })?
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| AppError::generic("路径恢复 fileId 标记损坏，拒绝继续"))
}

/// Create each missing parent component without traversing an existing symlink.
pub(crate) fn ensure_safe_target_parent(mount_root: &Path, new_root: &str) -> AppResult<()> {
    let parent = Path::new(new_root)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Ok(());
    };
    let mut current = PathBuf::from(mount_root);
    for component in parent.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(AppError::generic("目标父目录包含不安全路径片段"));
        };
        current.push(segment);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(AppError::generic(format!(
                    "目标父路径不是安全目录：{}",
                    current.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| {
                    AppError::generic(format!("创建路径恢复父目录失败：{error}"))
                })?;
            }
            Err(error) => {
                return Err(AppError::generic(format!(
                    "检查路径恢复父目录失败：{error}"
                )))
            }
        }
    }
    Ok(())
}

/// Atomic no-clobber rename on macOS. The fallback keeps the explicit existence check for
/// non-macOS development builds; PetalLink's production target uses `RENAME_EXCL`.
pub fn rename_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::raw::{c_char, c_int, c_uint};
        use std::os::unix::ffi::OsStrExt;

        const AT_FDCWD: c_int = -2;
        const RENAME_EXCL: c_uint = 0x0000_0004;
        extern "C" {
            fn renameatx_np(
                from_fd: c_int,
                from: *const c_char,
                to_fd: c_int,
                to: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }

        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "源路径含 NUL"))?;
        let target = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "目标路径含 NUL"))?;
        // SAFETY: both C strings are NUL-terminated and remain alive for the call; AT_FDCWD and
        // RENAME_EXCL are the constants declared by the macOS SDK.
        let result = unsafe {
            renameatx_np(
                AT_FDCWD,
                source.as_ptr(),
                AT_FDCWD,
                target.as_ptr(),
                RENAME_EXCL,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if std::fs::symlink_metadata(target).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "目标路径已存在",
            ));
        }
        std::fs::rename(source, target)
    }
}
