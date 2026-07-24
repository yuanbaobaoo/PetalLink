//! 云盘命令。

use std::collections::HashMap;

use tauri::{AppHandle, Emitter};

use crate::data::repository::{self, transfer_direction, TransferTask};
use crate::drive::about_api::AboutApi;
use crate::drive::models::{DriveAbout, DriveFile, FileListResult};
use crate::error::{AppError, AppResult};
use crate::sync::transfer_state::{TransferOperation, TransferState};

use super::{
    emit_folder_content_changed, mount, sync_engine, try_sync_engine, DB, DRIVE_CLIENT, FILES_API,
    THUMBNAIL_API,
};

/// 分页列出云盘目录内容。
#[tauri::command]
pub async fn drive_list(
    parent_id: Option<String>,
    cursor: Option<String>,
    page_size: Option<u32>,
) -> AppResult<FileListResult> {
    FILES_API
        .list(
            parent_id.as_deref(),
            cursor.as_deref(),
            page_size.unwrap_or(100),
        )
        .await
}

/// 列出云盘目录的全部内容。
#[tauri::command]
pub async fn drive_list_all(parent_id: Option<String>) -> AppResult<Vec<DriveFile>> {
    FILES_API.list_all(parent_id.as_deref()).await
}

/// 获取云盘文件信息。
#[tauri::command]
pub async fn drive_get_file(id: String) -> AppResult<DriveFile> {
    FILES_API.get(&id).await
}

/// 创建云盘目录。
#[tauri::command]
pub async fn drive_create_folder(name: String, parent_id: Option<String>) -> AppResult<DriveFile> {
    // FilesApi 保证创建前查重、严格写响应合同和丢响应后的 parent+name 唯一收敛。
    FILES_API.create_folder(&name, parent_id.as_deref()).await
}

/// 拒绝操作仍有活动任务的文件身份或路径子树。
fn ensure_no_active_transfer_for_identity(
    file_id: Option<&str>,
    relative_path: Option<&str>,
) -> AppResult<()> {
    let active = repository::list_all_transfers(&DB.lock())?
        .into_iter()
        .any(|task| {
            (file_id.is_some_and(|id| task.file_id.as_deref() == Some(id))
                || relative_path.is_some_and(|path| {
                    task.relative_path.as_deref().is_some_and(|task_path| {
                        task_path == path
                            || task_path
                                .strip_prefix(path)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                            || path
                                .strip_prefix(task_path)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                    })
                }))
                && task.state_kind().is_ok_and(|state| {
                    !matches!(
                        state,
                        TransferState::Completed | TransferState::Failed | TransferState::Canceled
                    )
                })
        });
    if active {
        return Err(AppError::generic("该文件存在活动或待恢复任务，请稍后重试"));
    }
    Ok(())
}

/// 判断相对路径是否等于根路径或位于其子树内。
fn is_path_in_subtree(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// 校验路径迁移不会进入自身子树或覆盖其他同步基线。
fn ensure_no_db_path_collision(old_root: &str, new_root: &str) -> AppResult<()> {
    crate::core::paths::validate_relative_path(old_root, false)?;
    crate::core::paths::validate_relative_path(new_root, false)?;
    if old_root == new_root {
        return Ok(());
    }
    if is_path_in_subtree(new_root, old_root) {
        return Err(AppError::generic("拒绝把目录移动到自身子树"));
    }
    let collision = repository::load_all(&DB.lock())?.into_iter().any(|record| {
        !is_path_in_subtree(&record.local_path, old_root)
            && is_path_in_subtree(&record.local_path, new_root)
    });
    if collision {
        return Err(AppError::generic("目标同步基线已被其他文件或目录占用"));
    }
    Ok(())
}

/// 删除云盘文件并结算本地同步状态。
#[tauri::command]
pub async fn drive_delete_file(app: AppHandle, id: String, name: Option<String>) -> AppResult<()> {
    // 索引中（云端树 BFS 重建）：删除会与索引并发改云端，且 cloud_tree 不完整
    // 无法正确反映删除后的状态 → 拒绝，等索引完成。
    ensure_not_indexing()?;
    // 查询本地同步记录，用于同步删除本地文件。
    let local_info = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?
    };
    let _path_lease = match (try_sync_engine(), local_info.as_ref()) {
        (Some(engine), Some(record)) => {
            Some(engine.begin_exclusive_path_activity(&record.local_path)?)
        }
        _ => None,
    };
    ensure_no_active_transfer_for_identity(
        Some(&id),
        local_info.as_ref().map(|record| record.local_path.as_str()),
    )?;
    if let Err(write_error) = FILES_API.delete(&id).await {
        match FILES_API.verify_deleted(&id).await {
            Ok(true) => tracing::info!(file_id = %id, "删除响应丢失，但 fileId 核验确认已回收"),
            Ok(false) => return Err(write_error),
            Err(verification_error) => {
                return Err(crate::error::AppError::generic(format!(
                    "删除结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        }
    }

    tracing::info!(file_id = %id, "删除云端文件已核验");
    if let Some(engine) = try_sync_engine() {
        let roots = {
            let cloud = engine.cloud_tree_lock();
            cloud
                .iter()
                .filter(|(_, file)| file.id == id)
                .map(|(path, file)| (path.clone(), file.is_folder()))
                .collect::<Vec<_>>()
        };
        for (root, is_folder) in roots {
            if is_folder {
                let prefix = format!("{root}/");
                engine
                    .cloud_tree_lock()
                    .retain(|path, _| path != &root && !path.starts_with(&prefix));
                engine
                    .path_to_id_lock()
                    .retain(|path, _| path != &root && !path.starts_with(&prefix));
            } else {
                engine.cloud_tree_remove(&root);
                engine.path_to_id_remove(&root);
            }
        }
    }
    if let Some(record) = local_info.as_ref() {
        let local_path = record.local_path.clone();
        let is_folder = record.is_folder;
        let mount = mount()?;
        let absolute_path =
            crate::core::paths::safe_join_under(mount.mount_dir(), &local_path, false)?;
        match std::fs::symlink_metadata(&absolute_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::generic(format!(
                    "云端已回收，但无法安全读取本地路径，已保留内容：{error}"
                )))
            }
            Ok(_) => {
                let baselines = {
                    let conn = DB.lock();
                    let mut by_path = HashMap::new();
                    for item in repository::load_all(&conn)? {
                        let path = item.local_path.clone();
                        if by_path.insert(path.clone(), item).is_some() {
                            return Err(AppError::generic(format!(
                                "云端已回收，但同步基线存在重复路径，已保留本地内容：{path}"
                            )));
                        }
                    }
                    by_path
                };
                crate::sync::executor::verify_local_delete_snapshot(
                    &absolute_path,
                    &local_path,
                    &baselines,
                    false,
                )
                .map_err(|error| {
                    AppError::generic(format!(
                        "云端已回收，但本地内容在操作期间发生变化，已保留并等待冲突救援：{error}"
                    ))
                })?;
                mount
                    .delete_local_confirmed(&absolute_path)
                    .await
                    .map_err(|error| {
                        AppError::generic(format!("云端已回收，但本地删除失败：{error}"))
                    })?;
            }
        }

        {
            let conn = DB.lock();
            if is_folder {
                let prefix = format!("{local_path}/");
                conn.execute(
                    "UPDATE sync_items SET status=?1
                     WHERE local_path=?2 OR substr(local_path, 1, length(?3))=?3",
                    rusqlite::params![repository::sync_status::DELETED, local_path, prefix],
                )
            } else {
                conn.execute(
                    "UPDATE sync_items SET status=?1 WHERE file_id=?2",
                    rusqlite::params![repository::sync_status::DELETED, id],
                )
            }
            .map_err(|error| {
                crate::error::AppError::generic(format!("结算删除基线失败：{error}"))
            })?;
        }

        if let Some(engine) = try_sync_engine() {
            engine.add_recently_deleted(&local_path);
        }
    }
    // 文件已删除，先刷新文件列表（无论留痕是否成功，删除结果都已落地）。
    emit_folder_content_changed(&app);
    // 留痕：写入 Completed 删除记录使传输队列可见。留痕失败时返回错误（带固定前缀，
    // 前端据此区分「文件未删」与「文件已删但记录未写入」）；不进入 TaskRunner 执行体系。
    record_completed_delete(&DB.lock(), &id, name.as_deref(), local_info.as_ref())?;
    // 留痕写入成功后广播传输队列变化，使已打开的传输面板实时刷新出删除记录。
    let _ = app.emit("transfer_update", ());
    Ok(())
}

/// 留痕失败错误的稳定标识符，前端据此区分「文件未删」（真失败）与「文件已删但记录未写入」。
/// 必须与 app/api/drive.ts 的 DELETE_TRACE_ERROR_PREFIX 保持完全一致，改动任一侧需同步另一侧。
pub(crate) const DELETE_TRACE_ERROR_PREFIX: &str = "TRACE_FAILED:";

/// 删除成功后写入一条 Completed 删除记录到传输队列，仅作历史可见性，不进入 TaskRunner 执行体系。
///
/// 仅负责 DB 写入与修剪；留痕失败时返回带 `DELETE_TRACE_ERROR_PREFIX` 前缀的错误，
/// 让前端区分两种失败。广播 transfer_update 由调用处负责。
fn record_completed_delete(
    conn: &rusqlite::Connection,
    file_id: &str,
    fallback_name: Option<&str>,
    local_info: Option<&repository::SyncItem>,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let relative_path = local_info.map(|record| record.local_path.clone());
    // 名称优先级：DB 基线 > 命令传入（前端已知文件名）> fileId 兜底。
    let name = local_info
        .map(|record| record.name.clone())
        .or_else(|| fallback_name.map(str::to_string))
        .unwrap_or_else(|| file_id.to_string());
    let task = TransferTask {
        id: 0,
        direction: transfer_direction::DELETE,
        file_id: Some(file_id.to_string()),
        local_path: None,
        name,
        total_size: 0,
        transferred: 0,
        state: i32::from(TransferState::Completed),
        error_message: None,
        created_at: now,
        finished_at: Some(now),
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path,
        parent_file_id: None,
        operation: Some(i32::from(TransferOperation::Delete)),
        source_mtime: None,
        source_size: None,
        expected_cloud_edited_time: None,
        attempt_count: 0,
        next_retry_at: None,
        error_kind: None,
        remote_result_file_id: None,
        state_revision: 0,
    };
    if let Err(error) = repository::insert_transfer(conn, &task) {
        return Err(AppError::generic(format!(
            "{DELETE_TRACE_ERROR_PREFIX}文件已删除，但传输记录写入失败：{error}"
        )));
    }
    // 修剪历史（保留最近 100 条已结束任务），与同步执行器一致；失败仅记录不阻断。
    if let Err(error) = repository::prune_transfer_history(conn, 100) {
        tracing::warn!(file_id, %error, "修剪传输历史失败，不影响删除留痕");
    }
    Ok(())
}

/// 在远端写入已核验后迁移本地路径、数据库基线与内存索引。
async fn settle_verified_remote_path_change(
    file_id: &str,
    new_relative_path: &str,
    verified: &DriveFile,
) -> AppResult<()> {
    crate::core::paths::validate_relative_path(new_relative_path, false)?;
    let old_record = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, file_id)?
    };
    let Some(old_record) = old_record else {
        return Ok(());
    };
    let old_relative_path = old_record.local_path.clone();
    if old_relative_path == new_relative_path {
        return Ok(());
    }
    ensure_no_db_path_collision(&old_relative_path, new_relative_path)?;

    let mount = mount()?;
    let old_absolute =
        crate::core::paths::safe_join_under(mount.mount_dir(), &old_relative_path, false)?;
    let new_absolute =
        crate::core::paths::safe_join_under(mount.mount_dir(), new_relative_path, false)?;
    crate::sync::path_recovery::ensure_safe_target_parent(mount.mount_dir(), new_relative_path)?;
    let old_metadata = crate::sync::path_recovery::optional_metadata(&old_absolute)?;
    let new_metadata = crate::sync::path_recovery::optional_metadata(&new_absolute)?;
    if let Some(metadata) = old_metadata.as_ref() {
        crate::sync::path_recovery::validate_local_type(metadata, &old_record, &old_absolute)?;
        if new_metadata.is_some() {
            return Err(AppError::generic("目标本地路径已存在，拒绝覆盖"));
        }
        crate::sync::path_recovery::rename_no_replace(&old_absolute, &new_absolute)
            .map_err(|error| AppError::generic(format!("同步本地路径变更失败：{error}")))?;
    } else if let Some(metadata) = new_metadata.as_ref() {
        crate::sync::path_recovery::validate_local_type(metadata, &old_record, &new_absolute)?;
        let target_id = crate::sync::path_recovery::read_file_id(&new_absolute)?;
        if target_id.as_deref() != Some(file_id) {
            return Err(AppError::generic("目标路径已存在且无法证明是同一云端文件"));
        }
    }

    let affected = {
        let conn = DB.lock();
        let prefix = format!("{old_relative_path}/");
        repository::load_all(&conn)?
            .into_iter()
            .filter(|record| {
                record.local_path == old_relative_path || record.local_path.starts_with(&prefix)
            })
            .collect::<Vec<_>>()
    };
    {
        let conn = DB.lock();
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| AppError::generic(format!("开始路径结算事务失败：{error}")))?;
        for record in &affected {
            transaction
                .execute(
                    "DELETE FROM sync_items WHERE file_id=?1 AND local_path=?2",
                    rusqlite::params![record.file_id, record.local_path],
                )
                .map_err(|error| AppError::generic(format!("删除旧路径基线失败：{error}")))?;
        }
        for mut record in affected {
            let suffix = record
                .local_path
                .strip_prefix(&old_relative_path)
                .unwrap_or_default();
            record.local_path = format!("{new_relative_path}{suffix}");
            if record.file_id == file_id {
                record.name = verified.name.clone();
                record.parent_folder_id = verified
                    .parent_folder
                    .as_ref()
                    .and_then(|parents| parents.first().cloned());
                record.cloud_edited_time = verified
                    .edited_time
                    .map(|edited_time| edited_time.timestamp_millis());
            }
            repository::upsert(&transaction, &record)?;
        }
        transaction
            .commit()
            .map_err(|error| AppError::generic(format!("提交路径结算事务失败：{error}")))?;
    }

    if let Some(engine) = try_sync_engine() {
        let prefix = format!("{old_relative_path}/");
        let mut cloud = engine.cloud_tree_lock();
        let mut path_to_id = engine.path_to_id_lock();
        let stale_paths: Vec<String> = cloud
            .keys()
            .filter(|path| *path == &old_relative_path || path.starts_with(&prefix))
            .cloned()
            .collect();
        let mut moved = Vec::with_capacity(stale_paths.len());
        for old_path in stale_paths {
            if let Some(file) = cloud.remove(&old_path) {
                path_to_id.remove(&old_path);
                let suffix = old_path
                    .strip_prefix(&old_relative_path)
                    .unwrap_or_default();
                moved.push((format!("{new_relative_path}{suffix}"), file));
            }
        }
        for (path, mut file) in moved {
            if file.id == file_id {
                file = verified.clone();
            }
            path_to_id.insert(path.clone(), file.id.clone());
            cloud.insert(path, file);
        }
        drop(path_to_id);
        drop(cloud);
        engine.add_recently_deleted(&old_relative_path);
    }
    Ok(())
}

/// 在非幂等远端路径写入前持久化本地身份，供后续可信同步周期恢复。
async fn persist_remote_path_change_identity(
    file_id: &str,
    old_relative_path: &str,
) -> AppResult<()> {
    let mount = mount()?;
    let local_path =
        crate::core::paths::safe_join_under(mount.mount_dir(), old_relative_path, false)?;
    match std::fs::symlink_metadata(&local_path) {
        Ok(metadata)
            if !metadata.file_type().is_symlink()
                && (metadata.file_type().is_file() || metadata.file_type().is_dir()) =>
        {
            let existing_id = xattr::get(&local_path, crate::mount::manager::XATTR_FILE_ID)
                .map_err(|error| AppError::generic(format!("读取路径变更源身份失败：{error}")))?
                .map(String::from_utf8)
                .transpose()
                .map_err(|_| AppError::generic("路径变更源 fileId 标记损坏，拒绝修改远端"))?;
            if existing_id.as_deref().is_some_and(|id| id != file_id) {
                return Err(AppError::generic(
                    "路径变更源属于另一云端文件，拒绝修改远端",
                ));
            }
            mount.set_file_id_xattr(&local_path, file_id).await
        }
        Ok(_) => Err(AppError::generic("远端路径变更源不是安全的文件或目录")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::generic(format!(
            "持久化路径变更源身份失败：{error}"
        ))),
    }
}

/// 重命名云盘文件并结算本地路径。
#[tauri::command]
pub async fn drive_rename_file(id: String, new_name: String) -> AppResult<DriveFile> {
    // 索引中拒绝操作，避免与重建中的 cloud_tree 冲突。
    ensure_not_indexing()?;
    crate::core::paths::validate_path_segment(&new_name)?;
    let old_relative_path = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?.map(|record| record.local_path)
    };
    let new_relative_path = old_relative_path.as_ref().map(|old_relative_path| {
        std::path::Path::new(old_relative_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.join(&new_name))
            .unwrap_or_else(|| std::path::PathBuf::from(&new_name))
            .to_string_lossy()
            .into_owned()
    });
    let engine_for_lease = try_sync_engine();
    let _source_lease = match (engine_for_lease.as_ref(), old_relative_path.as_deref()) {
        (Some(engine), Some(path)) => Some(engine.begin_exclusive_path_activity(path)?),
        _ => None,
    };
    let _target_lease = match (
        engine_for_lease.as_ref(),
        old_relative_path.as_deref(),
        new_relative_path.as_deref(),
    ) {
        (Some(engine), Some(old), Some(new)) if old != new => {
            Some(engine.begin_exclusive_path_activity(new)?)
        }
        _ => None,
    };
    ensure_no_active_transfer_for_identity(Some(&id), old_relative_path.as_deref())?;
    if new_relative_path.as_deref() != old_relative_path.as_deref() {
        ensure_no_active_transfer_for_identity(None, new_relative_path.as_deref())?;
    }
    if let (Some(old), Some(new)) = (old_relative_path.as_deref(), new_relative_path.as_deref()) {
        ensure_no_db_path_collision(old, new)?;
        let mount = mount()?;
        let old_absolute = crate::core::paths::safe_join_under(mount.mount_dir(), old, false)?;
        let new_absolute = crate::core::paths::safe_join_under(mount.mount_dir(), new, false)?;
        let old_metadata = crate::sync::path_recovery::optional_metadata(&old_absolute)?;
        let new_metadata = crate::sync::path_recovery::optional_metadata(&new_absolute)?;
        if old_absolute != new_absolute && new_metadata.is_some() {
            let target_id = crate::sync::path_recovery::read_file_id(&new_absolute)?;
            if old_metadata.is_some() || target_id.as_deref() != Some(id.as_str()) {
                return Err(AppError::generic("目标本地路径已存在，拒绝先修改云端"));
            }
        }
    }
    if let Some(old_relative_path) = old_relative_path.as_deref() {
        persist_remote_path_change_identity(&id, old_relative_path).await?;
    }
    let file = match FILES_API.rename_file(&id, &new_name).await {
        Ok(file) => file,
        Err(write_error) => match FILES_API.get(&id).await {
            Ok(file) if file.id == id && file.name == new_name => file,
            Ok(_) => return Err(write_error),
            Err(verification_error) => {
                return Err(AppError::generic(format!(
                    "重命名结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        },
    };
    if let Some(new_relative_path) = new_relative_path {
        settle_verified_remote_path_change(&id, &new_relative_path, &file).await?;
    }
    tracing::info!(file_id = %id, new_name = %new_name, "重命名已核验并结算");
    Ok(file)
}

/// 移动云盘文件并结算本地路径。
#[tauri::command]
pub async fn drive_move_file(id: String, new_parent_folder: String) -> AppResult<DriveFile> {
    // 索引中拒绝：移动改 parentFolder，与重建中的 path_to_id/cloud_tree 冲突。
    ensure_not_indexing()?;
    let new_parent_folder = if new_parent_folder == "root" {
        try_sync_engine()
            .and_then(|engine| engine.root_folder_id())
            .unwrap_or(new_parent_folder)
    } else {
        new_parent_folder
    };
    let old_relative_path = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &id)?.map(|record| record.local_path)
    };
    let target_parent_path = if old_relative_path.is_some() {
        let mut resolved = (new_parent_folder == "root").then(String::new);
        if let Some(engine) = try_sync_engine() {
            if new_parent_folder == "root"
                || engine.root_folder_id().as_deref() == Some(new_parent_folder.as_str())
            {
                resolved = Some(String::new());
            } else {
                resolved = engine.path_to_id_lock().iter().find_map(|(path, file_id)| {
                    (file_id == &new_parent_folder).then_some(path.clone())
                });
            }
        }
        if resolved.is_none() && new_parent_folder != "root" {
            let conn = DB.lock();
            resolved = repository::find_by_file_id(&conn, &new_parent_folder)?
                .filter(|record| record.is_folder)
                .map(|record| record.local_path);
        }
        resolved.ok_or_else(|| AppError::generic("无法解析目标云端目录的本地路径"))?
    } else {
        String::new()
    };
    let new_relative_path = old_relative_path
        .as_deref()
        .map(|old_relative_path| -> AppResult<String> {
            let name = std::path::Path::new(old_relative_path)
                .file_name()
                .ok_or_else(|| AppError::generic("移动源路径缺少文件名"))?;
            Ok(if target_parent_path.is_empty() {
                std::path::PathBuf::from(name)
            } else {
                std::path::Path::new(&target_parent_path).join(name)
            }
            .to_string_lossy()
            .into_owned())
        })
        .transpose()?;
    let engine_for_lease = try_sync_engine();
    let _source_lease = match (engine_for_lease.as_ref(), old_relative_path.as_deref()) {
        (Some(engine), Some(path)) => Some(engine.begin_exclusive_path_activity(path)?),
        _ => None,
    };
    let _target_lease = match (
        engine_for_lease.as_ref(),
        old_relative_path.as_deref(),
        new_relative_path.as_deref(),
    ) {
        (Some(engine), Some(old), Some(new)) if old != new => {
            Some(engine.begin_exclusive_path_activity(new)?)
        }
        _ => None,
    };
    ensure_no_active_transfer_for_identity(Some(&id), old_relative_path.as_deref())?;
    if new_relative_path.as_deref() != old_relative_path.as_deref() {
        ensure_no_active_transfer_for_identity(None, new_relative_path.as_deref())?;
    }
    if let (Some(old), Some(new)) = (old_relative_path.as_deref(), new_relative_path.as_deref()) {
        ensure_no_db_path_collision(old, new)?;
        let mount = mount()?;
        let old_absolute = crate::core::paths::safe_join_under(mount.mount_dir(), old, false)?;
        let new_absolute = crate::core::paths::safe_join_under(mount.mount_dir(), new, false)?;
        let old_metadata = crate::sync::path_recovery::optional_metadata(&old_absolute)?;
        let new_metadata = crate::sync::path_recovery::optional_metadata(&new_absolute)?;
        if old_absolute != new_absolute && new_metadata.is_some() {
            let target_id = crate::sync::path_recovery::read_file_id(&new_absolute)?;
            if old_metadata.is_some() || target_id.as_deref() != Some(id.as_str()) {
                return Err(AppError::generic("目标本地路径已存在，拒绝先修改云端"));
            }
        }
    }
    if let Some(old_relative_path) = old_relative_path.as_deref() {
        persist_remote_path_change_identity(&id, old_relative_path).await?;
    }
    let file = match FILES_API
        .update(&id, None, Some(&new_parent_folder), None)
        .await
    {
        Ok(file) => file,
        Err(write_error) => match FILES_API.get(&id).await {
            Ok(file)
                if file.id == id
                    && file.parent_folder.as_deref().is_some_and(|parents| {
                        parents.len() == 1 && parents[0] == new_parent_folder
                    }) =>
            {
                file
            }
            Ok(_) => return Err(write_error),
            Err(verification_error) => {
                return Err(AppError::generic(format!(
                    "移动结果不确定：{write_error}；核验失败：{verification_error}"
                )))
            }
        },
    };
    if let Some(new_relative_path) = new_relative_path {
        settle_verified_remote_path_change(&id, &new_relative_path, &file).await?;
    }
    tracing::info!(file_id = %id, target_folder = %new_parent_folder, "移动已核验并结算");
    Ok(file)
}

/// 搜索云盘文件。
#[tauri::command]
pub async fn drive_search(
    keyword: String,
    parent_id: Option<String>,
    page_size: Option<u32>,
) -> AppResult<FileListResult> {
    FILES_API
        .search(&keyword, parent_id.as_deref(), page_size.unwrap_or(100))
        .await
}

/// 获取云盘文件缩略图。
#[tauri::command]
pub async fn drive_get_thumbnail(file_id: String) -> AppResult<String> {
    THUMBNAIL_API.get_data_url(&file_id).await.map_err(|error| {
        tracing::warn!(file_id, %error, "获取缩略图失败");
        error
    })
}

/// 获取云盘容量信息。
#[tauri::command]
pub async fn drive_get_about() -> AppResult<DriveAbout> {
    AboutApi::new(DRIVE_CLIENT.clone()).get().await
}

/// 下载云盘文件到挂载目录。
#[tauri::command]
pub async fn drive_download_file(file_id: String, dest_path: String) -> AppResult<()> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    let m = mount()?;
    let dest = std::path::PathBuf::from(&dest_path);
    let rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &dest)?;
    let dest = crate::core::paths::safe_join_under(m.mount_dir(), &rel, false)?;
    let cloud = FILES_API.get(&file_id).await?;
    let is_update = dest.is_file();
    let operation = if is_update {
        crate::sync::transfer_state::TransferOperation::DownloadUpdate
    } else {
        crate::sync::transfer_state::TransferOperation::Download
    };
    let result = engine
        .task_runner()?
        .enqueue_and_run(repository::TransferTask {
            id: 0,
            direction: if is_update {
                repository::transfer_direction::DOWNLOAD_UPDATE
            } else {
                repository::transfer_direction::DOWNLOAD
            },
            file_id: Some(file_id),
            local_path: Some(dest.to_string_lossy().into_owned()),
            name: cloud.name,
            total_size: cloud.size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(rel),
            parent_file_id: cloud
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned()),
            operation: Some(i32::from(operation)),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: cloud.edited_time.map(|time| time.timestamp_millis()),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        })
        .await?;
    if result.outcome.disposition == crate::sync::task_runner::TaskDisposition::Completed {
        Ok(())
    } else {
        let user_message = result.outcome.disposition.user_message();
        tracing::info!(
            disposition = ?result.outcome.disposition,
            user_message,
            "下载未立即完成，已保留在传输队列"
        );
        Err(AppError::generic(user_message))
    }
}

/// 上传挂载目录中的本地文件。
#[tauri::command]
pub async fn drive_upload_file(
    local_path: String,
    parent_id: Option<String>,
) -> AppResult<DriveFile> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    let m = mount()?;
    let path = std::path::PathBuf::from(&local_path);
    let rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &path)?;
    let path = crate::core::paths::safe_join_under(m.mount_dir(), &rel, false)?;
    let parent_id = parent_id.filter(|id| !id.trim().is_empty());
    let metadata = std::fs::metadata(&path)
        .map_err(|error| AppError::generic(format!("读取上传源失败：{error}")))?;
    let source_mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    let result = engine
        .task_runner()?
        .enqueue_and_run(repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::UPLOAD,
            file_id: None,
            local_path: Some(path.to_string_lossy().into_owned()),
            name: rel.rsplit('/').next().unwrap_or(&rel).to_string(),
            total_size: metadata.len() as i64,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(rel),
            parent_file_id: parent_id,
            operation: Some(i32::from(
                crate::sync::transfer_state::TransferOperation::Create,
            )),
            source_mtime,
            source_size: Some(metadata.len() as i64),
            expected_cloud_edited_time: None,
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        })
        .await?;
    if result.outcome.disposition != crate::sync::task_runner::TaskDisposition::Completed {
        let user_message = result.outcome.disposition.user_message();
        tracing::info!(
            disposition = ?result.outcome.disposition,
            user_message,
            "上传未立即完成，已保留在传输队列"
        );
        return Err(AppError::generic(user_message));
    }
    result
        .outcome
        .cloud_file
        .ok_or_else(|| AppError::generic("上传完成但缺少云端文件结果"))
}

/// 云端树索引重建时拒绝文件操作。
fn ensure_not_indexing() -> AppResult<()> {
    if sync_engine()
        .ok()
        .map(|e| e.current_state().is_indexing)
        .unwrap_or(false)
    {
        let user_message = "正在读取云端文件，请稍后再试";
        tracing::debug!(user_message, "云端索引构建期间拒绝文件操作");
        return Err(AppError::generic(user_message));
    }
    Ok(())
}

/// 删除留痕的私有 DB 合同测试。record_completed_delete 依赖私有函数与内存 DB，
/// 无法通过公开接口覆盖，按 coding-rules 第四章「确实依赖私有实现」例外保留在 src 内。
/// 前后端前缀一致性合同由 app/api/drive.contract.test.ts 跨语言校验。
#[cfg(test)]
mod tests {
    use super::*;

    /// 用正式迁移建表的内存数据库，确保 transfer_queue schema 与生产一致。
    fn open_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::data::migrations::run(&conn).unwrap();
        conn
    }

    /// 无同步基线时，留痕 name 回退为传入的 fallback_name，relative_path 为空。
    #[test]
    fn delete_trace_uses_fallback_name_without_baseline() {
        let conn = open_test_db();
        record_completed_delete(&conn, "file-A", Some("report.pdf"), None).unwrap();
        let record = &repository::list_all_transfers(&conn).unwrap()[0];
        assert_eq!(record.name, "report.pdf");
        assert_eq!(record.relative_path, None);
        assert_eq!(record.direction, transfer_direction::DELETE);
        assert_eq!(record.state, i32::from(TransferState::Completed));
    }

    /// 有同步基线时，留痕 name 与 relative_path 取自基线（优先于 fallback）。
    #[test]
    fn delete_trace_prefers_baseline_over_fallback() {
        let conn = open_test_db();
        let baseline = repository::SyncItem {
            file_id: "file-B".to_string(),
            local_path: "docs/b.txt".to_string(),
            parent_folder_id: None,
            name: "b.txt".to_string(),
            is_folder: false,
            size: 0,
            local_size: Some(0),
            sha256: None,
            local_mtime: None,
            cloud_edited_time: None,
            last_sync_time: None,
            status: repository::sync_status::SYNCED,
            error_message: None,
        };
        repository::upsert(&conn, &baseline).unwrap();
        record_completed_delete(&conn, "file-B", Some("ignored"), Some(&baseline)).unwrap();
        let record = &repository::list_all_transfers(&conn).unwrap()[0];
        assert_eq!(record.name, "b.txt");
        assert_eq!(record.relative_path.as_deref(), Some("docs/b.txt"));
    }

    /// 留痕写入失败（transfer_queue 表缺失模拟 DB 异常）时，必须返回带固定前缀的错误，
    /// 前端据此区分「文件未删」与「文件已删但记录未写入」。此测试锁定该核心合同。
    #[test]
    fn delete_trace_insert_failure_returns_prefixed_error() {
        // 未建表的连接：insert_transfer 会因表不存在失败。
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = record_completed_delete(&conn, "file-C", Some("c.txt"), None);
        let error = result.unwrap_err().to_string();
        assert!(
            error.starts_with(DELETE_TRACE_ERROR_PREFIX),
            "留痕失败错误必须以 {DELETE_TRACE_ERROR_PREFIX} 开头，实际：{error}"
        );
    }
}
