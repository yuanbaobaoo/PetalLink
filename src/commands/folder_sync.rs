//! 目录同步命令。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::data::repository;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::engine::SyncEngine;

use super::{mount, sync_engine, FILES_API};

/// 目录同步进度事件载荷。
#[derive(Clone, Serialize)]
pub struct FolderSyncProgress {
    /// 已完成任务数。
    pub done: usize,
    /// 总任务数。
    pub total: usize,
}

/// 递归同步云端目录子树与本地目录。
#[tauri::command]
pub async fn sync_folder_recursive(
    app: AppHandle,
    folder_id: String,
    rel_path: String,
) -> AppResult<i64> {
    let eng = sync_engine()?;
    let activity = eng.begin_external_activity()?;
    // 全局索引读取中（云端树 BFS 重建中）：选择目录同步会基于不完整的 cloud_tree/path_to_id，
    // 且与全局 BFS 并发拉取易冲突 → 拒绝，等索引完成。
    if eng.current_state().is_indexing {
        return Err(AppError::generic(
            "正在读取云端索引，请稍后再试".to_string(),
        ));
    }
    let Some(folder_guard) = eng.try_begin_folder_sync_guard() else {
        return Err(AppError::generic(
            "已有同步周期或目录同步正在运行，本次请求未开始",
        ));
    };
    // 后台异步执行并立即返回，传输项实时进入传输队列。
    // 后台任务结束时释放目录同步锁并广播内容变更。
    let eng_clone = eng.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let _activity = activity;
        let _folder = folder_guard;
        let result =
            sync_folder_recursive_impl(&app_clone, &eng_clone, &folder_id, &rel_path).await;
        // 完成后广播内容变更，让前端刷新目录。
        if let Err(error) = eng_clone.update_runtime_and_broadcast(|runtime| {
            runtime.content_changed = true;
            runtime.last_sync_time = Some(chrono::Utc::now().timestamp_millis());
        }) {
            tracing::warn!(%error, "目录同步完成后重算全局状态失败");
        }
        match &result {
            Ok(done) => tracing::info!(done, rel = %rel_path, "sync_folder_recursive（后台）完成"),
            Err(e) => {
                tracing::warn!(error = %e, rel = %rel_path, "sync_folder_recursive（后台）失败")
            }
        }
    });
    Ok(0)
}

/// 执行受活动门禁保护的云端子树遍历与本地任务投递。
async fn sync_folder_recursive_impl(
    app: &AppHandle,
    eng: &SyncEngine,
    folder_id: &str,
    rel_path: &str,
) -> AppResult<i64> {
    let m = mount()?;
    let task_runner = eng.task_runner()?;
    crate::core::paths::validate_relative_path(rel_path, true)?;
    let dest_dir = crate::core::paths::safe_join_under(m.mount_dir(), rel_path, true)?;
    tracing::info!(folder_id, rel = %rel_path, "sync_folder_recursive: 开始递归同步");

    // BFS 读取云端子树。
    let mut cloud_files: Vec<(String, DriveFile)> = Vec::new(); // (subrel, DriveFile)
    let mut cloud_folders: Vec<String> = Vec::new();
    let mut folder_rel_to_id: HashMap<String, String> = HashMap::new();
    folder_rel_to_id.insert(String::new(), folder_id.to_string());
    let mut queue: Vec<(String, String)> = vec![(folder_id.to_string(), String::new())];
    while !queue.is_empty() {
        let (id, path) = queue.remove(0);
        eng.ensure_cycle_active()?;
        let _operation = eng.begin_external_activity()?;
        let children = FILES_API.list_all(Some(id.as_str())).await?;
        for f in children {
            if f.name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                continue;
            }
            crate::core::paths::validate_path_segment(&f.name)?;
            let subrel = if path.is_empty() {
                f.name.clone()
            } else {
                format!("{path}/{}", f.name)
            };
            if f.is_folder() {
                cloud_folders.push(subrel.clone());
                folder_rel_to_id.insert(subrel.clone(), f.id.clone());
                queue.push((f.id.clone(), subrel));
            } else {
                cloud_files.push((subrel, f));
            }
        }
    }
    tracing::info!(
        files = cloud_files.len(),
        folders = cloud_folders.len(),
        "sync_folder_recursive: 云端子树"
    );

    // 创建本地目录。
    tokio::fs::create_dir_all(&dest_dir).await.ok();
    for sub in &cloud_folders {
        let path = crate::core::paths::safe_join_under(&dest_dir, sub, false)?;
        let _ = tokio::fs::create_dir_all(path).await;
    }

    // 扫描本地真实文件，排除临时文件与占位符。
    let dest_dir_clone = dest_dir.clone();
    let local_files: HashMap<String, PathBuf> = tokio::task::spawn_blocking(move || {
        let mut out: HashMap<String, PathBuf> = HashMap::new();
        let _ = scan_dir_for_real_files(&dest_dir_clone, &dest_dir_clone, &mut out);
        out
    })
    .await
    .unwrap_or_default();

    // 计算待下载与待上传文件。
    let cloud_map: HashMap<String, DriveFile> = cloud_files
        .iter()
        .map(|(r, f)| (r.clone(), f.clone()))
        .collect();
    let to_download: Vec<(String, DriveFile)> = cloud_files
        .into_iter()
        .filter(|(r, _)| !local_files.contains_key(r))
        .collect();
    let to_upload: Vec<(String, PathBuf)> = local_files
        .into_iter()
        .filter(|(r, _)| !cloud_map.contains_key(r))
        .collect();
    let total = to_download.len() + to_upload.len();
    tracing::info!(
        download = to_download.len(),
        upload = to_upload.len(),
        "sync_folder_recursive: 任务"
    );

    let mut done: i64 = 0;

    // 为本地独有文件补建云端父目录链，保留目录层级。
    {
        // 收集所有需要上传的文件的祖先目录路径
        let mut missing_dirs: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for (subrel, _) in &to_upload {
            let parts: Vec<&str> = subrel.split('/').collect();
            // 最后一段是文件名，之前的是目录层级。
            for i in 1..parts.len() {
                let ancestor = parts[..i].join("/");
                if !cloud_folders.contains(&ancestor) && !folder_rel_to_id.contains_key(&ancestor) {
                    missing_dirs.insert(ancestor);
                }
            }
        }
        // 按深度升序创建（父目录先建，子目录才能找到父 ID）
        let mut sorted_dirs: Vec<String> = missing_dirs.into_iter().collect();
        sorted_dirs.sort_by_key(|p| p.matches('/').count());
        for dir_rel in &sorted_dirs {
            let dir_name = dir_rel.rsplit('/').next().unwrap_or(dir_rel);
            let parent_rel = parent_subrel_of(dir_rel);
            let parent_id = if parent_rel.is_empty() {
                Some(folder_id)
            } else {
                folder_rel_to_id.get(&parent_rel).map(|s| s.as_str())
            };
            let create_result = {
                eng.ensure_cycle_active()?;
                let _operation = eng.begin_external_activity()?;
                FILES_API.create_folder(dir_name, parent_id).await
            };
            match create_result {
                Ok(f) => {
                    folder_rel_to_id.insert(dir_rel.clone(), f.id.clone());
                    // 本地也建目录（确保后续 scan 能看到）
                    let _ = tokio::fs::create_dir_all(dest_dir.join(dir_rel)).await;
                    tracing::info!(dir = %dir_rel, cloud_id = %f.id, "sync_folder_recursive: 已补建云端父目录");
                }
                Err(e) => {
                    // 409/400 时查同名已存在目录，存在则视为成功
                    if matches!(e.drive_status(), Some(400 | 409)) {
                        if let Some(pid) = parent_id {
                            eng.ensure_cycle_active()?;
                            let _operation = eng.begin_external_activity()?;
                            if let Ok(list) = FILES_API.list_all(Some(pid)).await {
                                if let Some(existing) =
                                    list.iter().find(|c| c.is_folder() && c.name == dir_name)
                                {
                                    folder_rel_to_id.insert(dir_rel.clone(), existing.id.clone());
                                    let _ = tokio::fs::create_dir_all(dest_dir.join(dir_rel)).await;
                                    tracing::info!(dir = %dir_rel, cloud_id = %existing.id, "sync_folder_recursive: 父目录已存在（409容错）");
                                    continue;
                                }
                            }
                        }
                    }
                    tracing::warn!(dir = %dir_rel, error = %e, "sync_folder_recursive: 补建云端父目录失败，其内文件将继续上传（可能平铺）");
                }
            }
        }
    }

    // 下载云端独有文件。
    for (subrel, f) in &to_download {
        eng.ensure_cycle_active()?;
        let dest = crate::core::paths::safe_join_under(&dest_dir, subrel, false)?;
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        let is_update = std::fs::metadata(&dest)
            .ok()
            .is_some_and(|metadata| metadata.is_file() && metadata.len() > 0);
        let task = repository::TransferTask {
            id: 0,
            direction: if is_update {
                repository::transfer_direction::DOWNLOAD_UPDATE
            } else {
                repository::transfer_direction::DOWNLOAD
            },
            file_id: Some(f.id.clone()),
            local_path: Some(dest.to_string_lossy().into_owned()),
            name: f.name.clone(),
            total_size: f.size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(full_rel.clone()),
            parent_file_id: f
                .parent_folder
                .as_ref()
                .and_then(|parents| parents.first().cloned()),
            operation: Some(i32::from(if is_update {
                crate::sync::transfer_state::TransferOperation::DownloadUpdate
            } else {
                crate::sync::transfer_state::TransferOperation::Download
            })),
            source_mtime: None,
            source_size: None,
            expected_cloud_edited_time: f.edited_time.map(|time| time.timestamp_millis()),
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };
        match task_runner.enqueue_and_run(task).await {
            Ok(result)
                if result.outcome.disposition
                    == crate::sync::task_runner::TaskDisposition::Completed => {}
            Ok(result) => tracing::warn!(
                subrel = %subrel,
                disposition = ?result.outcome.disposition,
                "sync_folder_recursive: 下载进入恢复队列"
            ),
            Err(error) => tracing::warn!(
                subrel = %subrel,
                %error,
                "sync_folder_recursive: 下载失败"
            ),
        }
        done += 1;
        let _ = app.emit(
            "folder_sync_progress",
            FolderSyncProgress {
                done: done as usize,
                total,
            },
        );
    }
    // 上传本地独有文件。
    for (subrel, local_path) in &to_upload {
        eng.ensure_cycle_active()?;
        let parent_subrel = parent_subrel_of(subrel);
        let parent_id = folder_rel_to_id
            .get(&parent_subrel)
            .map(|s| s.as_str())
            .unwrap_or(folder_id);
        let full_rel = if rel_path.is_empty() {
            subrel.clone()
        } else {
            format!("{rel_path}/{subrel}")
        };
        let file_size = local_path.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let source_mtime = local_path
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        let task = repository::TransferTask {
            id: 0,
            direction: repository::transfer_direction::UPLOAD,
            file_id: None,
            local_path: Some(local_path.to_string_lossy().into_owned()),
            name: subrel.rsplit('/').next().unwrap_or(subrel).to_string(),
            total_size: file_size,
            transferred: 0,
            state: i32::from(crate::sync::transfer_state::TransferState::Pending),
            error_message: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            finished_at: None,
            server_id: None,
            upload_id: None,
            resume_offset: 0,
            session_url: None,
            relative_path: Some(full_rel.clone()),
            parent_file_id: Some(parent_id.to_string()),
            operation: Some(i32::from(
                crate::sync::transfer_state::TransferOperation::Create,
            )),
            source_mtime,
            source_size: Some(file_size),
            expected_cloud_edited_time: None,
            attempt_count: 0,
            next_retry_at: None,
            error_kind: None,
            remote_result_file_id: None,
            state_revision: 0,
        };
        match task_runner.enqueue_and_run(task).await {
            Ok(result)
                if result.outcome.disposition
                    == crate::sync::task_runner::TaskDisposition::Completed =>
            {
                if let Some(uploaded) = result.outcome.cloud_file {
                    eng.cloud_tree_insert(full_rel.clone(), uploaded.clone());
                    eng.path_to_id_insert(full_rel.clone(), uploaded.id);
                }
            }
            Ok(result) => tracing::warn!(
                subrel = %subrel,
                disposition = ?result.outcome.disposition,
                "sync_folder_recursive: 上传进入恢复队列"
            ),
            Err(error) => tracing::warn!(
                subrel = %subrel,
                %error,
                "sync_folder_recursive: 上传失败"
            ),
        }
        done += 1;
        let _ = app.emit(
            "folder_sync_progress",
            FolderSyncProgress {
                done: done as usize,
                total,
            },
        );
    }

    tracing::info!(done, total, "sync_folder_recursive: 完成");
    Ok(done)
}

/// 递归扫描目录，收集"真实内容文件"（subrel → PathBuf）：跳过 .tmp、.hwcloud_ 前缀、占位符（需 xattr 验证）。
fn scan_dir_for_real_files(
    base: &Path,
    current: &Path,
    out: &mut HashMap<String, PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            scan_dir_for_real_files(base, &path, out)?;
        } else if ft.is_file() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tmp") || name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                continue;
            }
            let meta = entry.metadata()?;
            // 跳过占位符（xattr state=placeholder），0 字节用户文件（如空配置）不是占位符
            if meta.len() == 0 && crate::mount::manager::is_placeholder_file(&path) {
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            out.insert(rel, path);
        }
    }
    Ok(())
}

/// 取 subrel 的父 subrel（最后一个 / 之前；无 / 则空串）。
fn parent_subrel_of(subrel: &str) -> String {
    match subrel.rfind('/') {
        Some(i) => subrel[..i].to_string(),
        None => String::new(),
    }
}
