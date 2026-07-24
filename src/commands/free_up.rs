//! 释放空间与按需下载命令。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::data::repository;
use crate::error::{AppError, AppResult};
use crate::sync::state::FreeUpCheckResult;

use super::{mount, sync_engine, try_sync_engine, DB, FILES_API};

/// 可释放空间候选项（基于 DB 基线枚举，实际释放前再逐项安全核验）。
/// 与前端 `FreeableItem` interface 的合同为 camelCase，序列化/反序列化必须保持一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeableItem {
    /// 云端文件 ID
    pub file_id: String,
    /// 相对挂载目录的路径
    pub rel_path: String,
    /// 文件名
    pub name: String,
    /// 本地已下载字节数
    pub size: i64,
}

/// 批量释放空间结果统计。
/// 与前端 `FreeUpBatchResult` interface 的合同为 camelCase，缺失时前端读不到计数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeUpBatchResult {
    /// 成功释放的文件数
    pub freed_count: usize,
    /// 因不满足条件被跳过的文件数
    pub skipped_count: usize,
    /// 成功释放的总字节数
    pub freed_bytes: i64,
    /// 被跳过项的错误原因（与跳过项一一对应，便于前端提示）
    pub errors: Vec<String>,
}

/// 记录释放空间的技术拒绝原因，并返回用户可理解的错误。
fn free_up_rejection(file_id: &str, rel_path: &str, technical_reason: &str) -> AppError {
    let user_message = crate::sync::user_messages::simplify_sync_error(technical_reason);
    tracing::warn!(
        file_id,
        rel_path,
        technical_reason,
        user_message = %user_message,
        "释放空间安全检查未通过"
    );
    AppError::generic(user_message.into_owned())
}

/// 检查文件是否可安全释放本地空间。
#[tauri::command]
pub async fn sync_check_safe_free_up(rel_path: String, file_id: String) -> AppResult<String> {
    // 引擎已启动 → 用 cloud_tree + DB 精确校验
    if let Some(e) = try_sync_engine() {
        return Ok(match e.can_safely_free_up(&rel_path, &file_id) {
            FreeUpCheckResult::Safe => "safe",
            FreeUpCheckResult::NotInCloud => "not_in_cloud",
            FreeUpCheckResult::NotSynced => "not_synced",
        }
        .to_string());
    }
    // 未启动引擎时缺少可信云端 checkpoint 与 activity gate，按不安全处理。
    let _ = (rel_path, file_id);
    Ok("not_synced".to_string())
}

/// 在原文件同目录分配不存在的释放空间暂存路径。
fn allocate_free_up_staging_path(local_path: &std::path::Path) -> AppResult<std::path::PathBuf> {
    let parent = local_path
        .parent()
        .ok_or_else(|| AppError::generic("待释放文件缺少父目录"))?;
    for _ in 0..16 {
        let candidate = parent.join(format!(
            ".hwcloud_freeup-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        match crate::sync::path_recovery::optional_metadata(&candidate)? {
            None => return Ok(candidate),
            Some(_) => continue,
        }
    }
    Err(AppError::generic("无法分配释放空间临时路径"))
}

/// 仅在原路径空缺或仍是本文件占位符时恢复暂存内容。
async fn restore_staged_free_up(
    local_path: &std::path::Path,
    staging_path: &std::path::Path,
    file_id: &str,
) -> AppResult<()> {
    if let Some(metadata) = crate::sync::path_recovery::optional_metadata(local_path)? {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AppError::generic(format!(
                "原路径已出现非普通文件，已保留旧内容于 {}",
                staging_path.display()
            )));
        }
        let state = xattr::get(local_path, crate::mount::manager::XATTR_STATE)
            .map_err(|error| AppError::generic(format!("读取回滚占位状态失败：{error}")))?;
        let owner = xattr::get(local_path, crate::mount::manager::XATTR_FILE_ID)
            .map_err(|error| AppError::generic(format!("读取回滚占位身份失败：{error}")))?;
        let is_owned_placeholder = state.as_deref()
            == Some(crate::mount::manager::STATE_PLACEHOLDER.as_bytes())
            && owner.as_deref() == Some(file_id.as_bytes());
        if !is_owned_placeholder {
            return Err(AppError::generic(format!(
                "原路径已出现新的用户文件，已保留旧内容于 {}",
                staging_path.display()
            )));
        }
        tokio::fs::remove_file(local_path)
            .await
            .map_err(|error| AppError::generic(format!("移除回滚占位符失败：{error}")))?;
    }
    tokio::fs::rename(staging_path, local_path)
        .await
        .map_err(|error| AppError::generic(format!("恢复释放空间原文件失败：{error}")))?;
    let _ = xattr::remove(
        local_path,
        crate::mount::manager::XATTR_FREE_UP_RELATIVE_PATH,
    );
    Ok(())
}

/// 仅在释放空间基线未被并发改写时恢复已同步状态。
fn rollback_free_up_baseline(
    file_id: &str,
    rel_path: &str,
    source_mtime: i64,
    source_size: i64,
) -> AppResult<()> {
    let conn = DB.lock();
    let changed = conn
        .execute(
            "UPDATE sync_items
             SET status=?1, local_size=?2, error_message=NULL
             WHERE file_id=?3 AND local_path=?4 AND status=?5
               AND local_mtime=?6 AND local_size=0",
            rusqlite::params![
                repository::sync_status::SYNCED,
                source_size,
                file_id,
                rel_path,
                repository::sync_status::CLOUD_ONLY,
                source_mtime,
            ],
        )
        .map_err(|error| AppError::generic(format!("回滚释放空间基线失败：{error}")))?;
    if changed != 1 {
        return Err(AppError::generic("释放空间基线已并发变化，无法自动回滚"));
    }
    Ok(())
}

/// 将已同步文件替换为按需下载占位符。
#[tauri::command]
pub async fn sync_free_up_space(
    file_id: String,
    rel_path: String,
    local_path: String,
    _name: String,
    size: i64,
) -> AppResult<()> {
    let engine = sync_engine()?;
    let m = mount()?;
    // 校验前端传入的绝对路径与 rel_path 一致，避免路径错配释放错误文件。
    let frontend_rel = crate::core::paths::relative_path_from_mount(
        m.mount_dir(),
        &std::path::PathBuf::from(&local_path),
    )?;
    if frontend_rel != rel_path {
        return Err(AppError::config(format!(
            "释放空间路径不一致：rel_path={rel_path}, local_path={local_path}"
        )));
    }
    free_up_one(&engine, &m, &file_id, &rel_path, size).await?;
    Ok(())
}

/// 释放单个已同步文件的本地空间：安全核验通过后把原文件替换为占位符。
///
/// 所有前置条件（可信云树、本地非占位、基线一致、远端核验、无活动传输）与原子 staging/回滚
/// 逻辑均集中在此，供单文件命令与目录批量释放复用。返回成功释放的字节数。
///
/// - `engine` 同步引擎，提供可信云树与路径租约
/// - `m` 挂载管理器，用于占位符创建
/// - `file_id` 云端文件 ID
/// - `rel_path` 相对挂载目录的路径
/// - `size` 预期本地字节数（与基线核验）
async fn free_up_one(
    engine: &crate::sync::engine::SyncEngine,
    m: &crate::mount::manager::MountManager,
    file_id: &str,
    rel_path: &str,
    size: i64,
) -> AppResult<i64> {
    // 转为拥有所有权的 String，使下游 baseline/staging/DB 等处的引用与格式化写法保持一致。
    let file_id = file_id.to_string();
    let rel_path = rel_path.to_string();
    let lp = crate::core::paths::safe_join_under(m.mount_dir(), &rel_path, false)?;
    let _path_lease = engine.begin_exclusive_path_activity(&rel_path)?;

    if size < 0 || !engine.cloud_tree_is_trusted() {
        return Err(free_up_rejection(
            &file_id,
            &rel_path,
            "云端索引尚未追平，拒绝释放本地唯一副本",
        ));
    }

    let metadata_snapshot = std::fs::symlink_metadata(&lp)
        .map_err(|error| AppError::generic(format!("读取待释放文件失败：{error}")))?;
    if !metadata_snapshot.file_type().is_file() || crate::mount::manager::is_placeholder_file(&lp) {
        return Err(AppError::generic("待释放目标不是已下载的普通文件"));
    }
    let source_mtime = metadata_snapshot
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .ok_or_else(|| AppError::generic("无法读取待释放文件修改时间"))?;
    let source_size = metadata_snapshot.len() as i64;
    if source_size != size {
        return Err(AppError::generic("待释放文件大小已变化，请刷新后重试"));
    }

    let baseline = {
        let conn = DB.lock();
        let active = repository::list_all_transfers(&conn)?
            .into_iter()
            .any(|task| {
                (task.relative_path.as_deref() == Some(rel_path.as_str())
                    || task.file_id.as_deref() == Some(file_id.as_str()))
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            crate::sync::transfer_state::TransferState::Completed
                                | crate::sync::transfer_state::TransferState::Failed
                                | crate::sync::transfer_state::TransferState::Canceled
                        )
                    })
            });
        if active {
            return Err(AppError::generic("该文件存在活动传输任务，暂不能释放空间"));
        }
        repository::find_by_file_id(&conn, &file_id)?
            .filter(|record| record.local_path == rel_path)
            .ok_or_else(|| {
                free_up_rejection(&file_id, &rel_path, "找不到与路径匹配的成功同步基线")
            })?
    };
    if baseline.status != repository::sync_status::SYNCED
        || baseline.local_mtime != Some(source_mtime)
        || baseline.local_size != Some(source_size)
        || baseline.size != size
    {
        return Err(free_up_rejection(
            &file_id,
            &rel_path,
            "本地内容与最后成功同步基线不一致，拒绝释放",
        ));
    }
    {
        let cloud = engine.cloud_tree_lock();
        if cloud.get(&rel_path).map(|file| file.id.as_str()) != Some(file_id.as_str()) {
            return Err(free_up_rejection(
                &file_id,
                &rel_path,
                "可信云树中不存在同一 fileId",
            ));
        }
    }

    // 远端核验位于两次本地与数据库检查之间，期间发生变化会使租约失效。
    let remote = FILES_API.get(&file_id).await?;
    let remote_edited_time = remote
        .edited_time
        .map(|edited_time| edited_time.timestamp_millis());
    if remote.id != file_id
        || remote.size != size
        || baseline.cloud_edited_time.is_none()
        || remote_edited_time != baseline.cloud_edited_time
        || FILES_API.verify_deleted(&file_id).await?
    {
        return Err(free_up_rejection(
            &file_id,
            &rel_path,
            "远端副本不存在、已回收、大小或版本与成功基线不一致",
        ));
    }

    let current_metadata = std::fs::symlink_metadata(&lp)
        .map_err(|error| AppError::generic(format!("释放前复核本地文件失败：{error}")))?;
    let current_mtime = current_metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64);
    if !current_metadata.file_type().is_file()
        || current_metadata.len() as i64 != source_size
        || current_mtime != Some(source_mtime)
    {
        return Err(free_up_rejection(
            &file_id,
            &rel_path,
            "远端核验期间本地文件已变化，拒绝删除",
        ));
    }
    {
        let conn = DB.lock();
        let active = repository::list_all_transfers(&conn)?
            .into_iter()
            .any(|task| {
                (task.relative_path.as_deref() == Some(rel_path.as_str())
                    || task.file_id.as_deref() == Some(file_id.as_str()))
                    && task.state_kind().is_ok_and(|state| {
                        !matches!(
                            state,
                            crate::sync::transfer_state::TransferState::Completed
                                | crate::sync::transfer_state::TransferState::Failed
                                | crate::sync::transfer_state::TransferState::Canceled
                        )
                    })
            });
        let current = repository::find_by_file_id(&conn, &file_id)?;
        if active
            || current.as_ref().map_or(true, |record| {
                record.local_path != baseline.local_path
                    || record.status != baseline.status
                    || record.local_mtime != baseline.local_mtime
                    || record.local_size != baseline.local_size
                    || record.cloud_edited_time != baseline.cloud_edited_time
            })
        {
            return Err(free_up_rejection(
                &file_id,
                &rel_path,
                "释放租约已失效，请刷新后重试",
            ));
        }
    }

    // 原文件先原子移入 watcher 忽略的同目录 staging；占位或 DB 结算失败时可恢复，
    // 同时正确处理真实的 0 字节文件（delete_local 会有意保留这类文件，不能用于此处）。
    let staging_path = allocate_free_up_staging_path(&lp)?;
    xattr::set(
        &lp,
        crate::mount::manager::XATTR_FREE_UP_RELATIVE_PATH,
        rel_path.as_bytes(),
    )
    .map_err(|error| AppError::generic(format!("写入释放空间恢复标记失败：{error}")))?;
    std::fs::File::open(&lp)
        .and_then(|file| file.sync_all())
        .map_err(|error| AppError::generic(format!("持久化释放空间恢复标记失败：{error}")))?;
    tokio::fs::rename(&lp, &staging_path)
        .await
        .map_err(|error| AppError::generic(format!("暂存待释放文件失败：{error}")))?;
    if let Err(error) = m.create_placeholder_strict(&rel_path, &file_id, size).await {
        let rollback = restore_staged_free_up(&lp, &staging_path, &file_id).await;
        return Err(AppError::generic(format!(
            "创建占位符失败：{error}；文件恢复结果：{}",
            rollback
                .map(|_| "已恢复".to_string())
                .unwrap_or_else(|rollback_error| rollback_error.to_string())
        )));
    }

    // 更新 DB
    let changed_result = {
        let conn = DB.lock();
        conn.execute(
            "UPDATE sync_items
             SET status=?1, local_size=0, error_message=NULL
             WHERE file_id=?2 AND local_path=?3 AND status=?4
               AND local_mtime=?5 AND local_size=?6",
            rusqlite::params![
                repository::sync_status::CLOUD_ONLY,
                file_id,
                rel_path,
                repository::sync_status::SYNCED,
                source_mtime,
                source_size,
            ],
        )
        .map_err(|error| AppError::generic(format!("提交释放空间基线失败：{error}")))
    };
    let changed = match changed_result {
        Ok(changed) => changed,
        Err(error) => {
            let rollback = restore_staged_free_up(&lp, &staging_path, &file_id).await;
            return Err(AppError::generic(format!(
                "{error}；文件恢复结果：{}",
                rollback
                    .map(|_| "已恢复".to_string())
                    .unwrap_or_else(|rollback_error| rollback_error.to_string())
            )));
        }
    };
    if changed != 1 {
        let rollback = restore_staged_free_up(&lp, &staging_path, &file_id).await;
        return Err(AppError::generic(format!(
            "释放空间后基线发生并发变化；文件恢复结果：{}",
            rollback
                .map(|_| "已恢复".to_string())
                .unwrap_or_else(|rollback_error| rollback_error.to_string())
        )));
    }

    if let Err(remove_error) = tokio::fs::remove_file(&staging_path).await {
        let restore_result = restore_staged_free_up(&lp, &staging_path, &file_id).await;
        let baseline_result = if restore_result.is_ok() {
            rollback_free_up_baseline(&file_id, &rel_path, source_mtime, source_size)
        } else {
            Ok(())
        };
        return Err(AppError::generic(format!(
            "清理释放空间暂存文件失败：{remove_error}；文件恢复：{}；基线恢复：{}",
            restore_result
                .map(|_| "成功".to_string())
                .unwrap_or_else(|error| error.to_string()),
            baseline_result
                .map(|_| "成功".to_string())
                .unwrap_or_else(|error| error.to_string())
        )));
    }

    // 实际释放的字节数（供批量释放统计）
    Ok(source_size)
}

/// 枚举目录（含子树）下可释放空间的文件候选项。
///
/// 仅基于 DB 成功同步基线筛选 status=SYNCED 且非目录的记录，供前端弹窗预览；
/// 实际释放前由 `free_up_one` 逐项重新核验，避免预览与释放之间状态漂移造成误释放。
/// 路径匹配用精确前缀加路径分隔符边界，避免 `docs` 误匹配 `docs-backup` 这类同级异名目录。
///
/// - `folder_rel_path` 目录相对挂载根的路径，传空串表示从根枚举
#[tauri::command]
pub fn sync_list_freeable_in_folder(folder_rel_path: String) -> AppResult<Vec<FreeableItem>> {
    let conn = DB.lock();
    // 根目录（空路径）枚举全部 SYNCED 文件；子目录用「等于自身 或 以「dir/」为前缀」匹配，
    // 带分隔符边界杜绝 `docs` 误匹配 `docs-backup` 这类同级异名。
    let rows: Vec<repository::SyncItem> = if folder_rel_path.is_empty() {
        let mut stmt = conn
            .prepare("SELECT * FROM sync_items WHERE status=?1 AND is_folder=0")
            .map_err(|error| AppError::generic(format!("查询可释放候选项失败：{error}")))?;
        let collected = stmt
            .query_map(
                rusqlite::params![repository::sync_status::SYNCED],
                repository::SyncItem::from_row,
            )
            .map_err(|error| AppError::generic(format!("读取可释放候选项失败：{error}")))?;
        collected
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::generic(format!("解析可释放候选项失败：{error}")))?
    } else {
        let prefix = format!("{folder_rel_path}/");
        let mut stmt = conn
            .prepare(
                "SELECT * FROM sync_items
                 WHERE status=?1 AND is_folder=0
                   AND (local_path=?2 OR substr(local_path, 1, length(?3))=?3)",
            )
            .map_err(|error| AppError::generic(format!("查询可释放候选项失败：{error}")))?;
        let collected = stmt
            .query_map(
                rusqlite::params![repository::sync_status::SYNCED, folder_rel_path, prefix,],
                repository::SyncItem::from_row,
            )
            .map_err(|error| AppError::generic(format!("读取可释放候选项失败：{error}")))?;
        collected
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::generic(format!("解析可释放候选项失败：{error}")))?
    };
    Ok(rows
        .into_iter()
        .map(|record| FreeableItem {
            file_id: record.file_id,
            rel_path: record.local_path,
            name: record.name,
            size: record.local_size.unwrap_or(0),
        })
        .collect())
}

/// 批量释放多个文件的本地空间，逐项独立执行。
///
/// 单项失败（如并发改动、远端版本漂移）只记录原因并跳过，不中断整体释放；
/// 每项独立持有路径租约，互不阻塞。返回成功/跳过计数与释放总字节。
///
/// - `items` 由前端弹窗确认的可释放候选项清单
#[tauri::command]
pub async fn sync_free_up_batch(items: Vec<FreeableItem>) -> AppResult<FreeUpBatchResult> {
    let engine = sync_engine()?;
    let m = mount()?;
    let mut result = FreeUpBatchResult {
        freed_count: 0,
        skipped_count: 0,
        freed_bytes: 0,
        errors: Vec::new(),
    };
    for item in items {
        match free_up_one(&engine, &m, &item.file_id, &item.rel_path, item.size).await {
            Ok(bytes) => {
                result.freed_count += 1;
                result.freed_bytes += bytes;
            }
            Err(error) => {
                result.skipped_count += 1;
                result.errors.push(format!("{}：{}", item.name, error));
            }
        }
    }
    Ok(result)
}

/// 按需下载占位文件。
#[tauri::command]
pub async fn sync_download_on_demand(
    _app: AppHandle,
    file_id: String,
    dest_path: String,
) -> AppResult<bool> {
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    // 全局索引读取中：禁止按需下载（同 sync_folder_recursive，cloud_tree 构建中）
    if engine.current_state().is_indexing {
        let user_message = "正在读取云端文件，请稍后再试";
        tracing::debug!(user_message, "云端索引构建期间拒绝按需下载");
        return Err(AppError::generic(user_message));
    }
    let m = mount()?;
    let frontend_dest = PathBuf::from(&dest_path);
    let frontend_rel = crate::core::paths::relative_path_from_mount(m.mount_dir(), &frontend_dest)?;
    let record = {
        let conn = DB.lock();
        repository::find_by_file_id(&conn, &file_id)?
    };
    let dest_rel = match record.as_ref().map(|record| record.local_path.clone()) {
        Some(rel) => {
            crate::core::paths::validate_relative_path(&rel, false)?;
            if rel != frontend_rel {
                return Err(AppError::config(format!(
                    "下载路径不一致：file_id={file_id}, rel_path={rel}, dest_path={dest_path}"
                )));
            }
            rel
        }
        None => frontend_rel,
    };
    let dest = crate::core::paths::safe_join_under(m.mount_dir(), &dest_rel, false)?;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // 云端元数据必须包含真实 editedTime 与 size，以支持变更判断和传输进度。
    // 云端查询失败时仅允许回退到具备完整版本信息的可信 DB 缓存；缓存也不完整时必须
    // 传播原始 GET 错误，不能构造 size=0 / editedTime=None 的不可执行任务。
    let cached_metadata = record
        .as_ref()
        .filter(|record| record.size >= 0 && record.cloud_edited_time.is_some());
    let cloud_file = match FILES_API.get(&file_id).await {
        Ok(file) => Some(file),
        Err(error) if cached_metadata.is_some() => {
            tracing::warn!(
                file_id,
                error = %error,
                "按需下载获取实时元数据失败，使用可信同步基线"
            );
            None
        }
        Err(error) => return Err(error),
    };
    let cloud_edited_time = cloud_file
        .as_ref()
        .and_then(|f| f.edited_time.map(|t| t.timestamp_millis()))
        .or_else(|| cached_metadata.and_then(|record| record.cloud_edited_time))
        .ok_or_else(|| AppError::generic("按需下载缺少可信云端 editedTime，拒绝创建任务"))?;
    let cloud_size = cloud_file
        .as_ref()
        .map(|file| file.size)
        .filter(|size| *size >= 0)
        .or_else(|| cached_metadata.map(|record| record.size))
        .ok_or_else(|| AppError::generic("按需下载缺少可信云端文件大小，拒绝创建任务"))?;
    let destination_snapshot = match std::fs::symlink_metadata(&dest) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(AppError::generic("按需下载目标不是安全的普通文件"));
            }
            if metadata.len() == 0 && crate::mount::manager::is_placeholder_file(&dest) {
                None
            } else {
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64)
                    .ok_or_else(|| AppError::generic("无法读取按需下载目标修改时间"))?;
                Some((mtime, metadata.len() as i64))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(AppError::generic(format!("读取按需下载目标失败：{error}"))),
    };
    let is_update = destination_snapshot.is_some();
    let operation = if is_update {
        crate::sync::transfer_state::TransferOperation::DownloadUpdate
    } else {
        crate::sync::transfer_state::TransferOperation::Download
    };
    let direction = if is_update {
        repository::transfer_direction::DOWNLOAD_UPDATE
    } else {
        repository::transfer_direction::DOWNLOAD
    };
    let task = repository::TransferTask {
        id: 0,
        direction,
        file_id: Some(file_id),
        local_path: Some(dest.to_string_lossy().into_owned()),
        name,
        total_size: cloud_size,
        transferred: 0,
        state: i32::from(crate::sync::transfer_state::TransferState::Pending),
        error_message: None,
        created_at: chrono::Utc::now().timestamp_millis(),
        finished_at: None,
        server_id: None,
        upload_id: None,
        resume_offset: 0,
        session_url: None,
        relative_path: Some(dest_rel),
        parent_file_id: cloud_file
            .as_ref()
            .and_then(|file| file.parent_folder.as_ref())
            .and_then(|parents| parents.first().cloned())
            .or_else(|| cached_metadata.and_then(|record| record.parent_folder_id.clone())),
        operation: Some(i32::from(operation)),
        source_mtime: destination_snapshot.map(|snapshot| snapshot.0),
        source_size: destination_snapshot.map(|snapshot| snapshot.1),
        expected_cloud_edited_time: Some(cloud_edited_time),
        attempt_count: 0,
        next_retry_at: None,
        error_kind: None,
        remote_result_file_id: None,
        state_revision: 0,
    };
    let result = engine.task_runner()?.enqueue_and_run(task).await?;
    match result.outcome.disposition {
        crate::sync::task_runner::TaskDisposition::Completed => Ok(true),
        disposition => {
            let user_message = disposition.user_message();
            tracing::info!(
                disposition = ?disposition,
                user_message,
                "按需下载未立即完成，已保留在传输队列"
            );
            Err(AppError::generic(user_message))
        }
    }
}
