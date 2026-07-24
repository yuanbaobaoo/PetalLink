//! 本地删除前的持久基线快照校验与安全执行。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::data::repository::{self, SyncItem};
use crate::error::{AppError, AppResult};
use crate::sync::state::{ActionResult, SyncAction};

use super::SyncExecutor;

/// 将文件元数据的修改时间转为 epoch 毫秒。
fn metadata_mtime_ms(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
}

/// 核验待删除内容仍与持久化同步基线一致。
pub(crate) fn verify_local_delete_snapshot(
    path: &Path,
    relative_path: &str,
    baselines: &HashMap<String, SyncItem>,
    allow_orphan_placeholder: bool,
) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| AppError::generic(format!("读取待删除路径失败：{error}")))?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::generic(format!(
            "待删除路径已变为符号链接：{relative_path}"
        )));
    }

    if metadata.is_dir() {
        let baseline = baselines
            .get(relative_path)
            .ok_or_else(|| AppError::generic(format!("目录不在同步基线中：{relative_path}")))?;
        if !baseline.is_folder
            || baseline.local_mtime != metadata_mtime_ms(&metadata)
            || baseline.local_size != Some(metadata.len() as i64)
        {
            return Err(AppError::generic(format!(
                "目录在删除执行前发生变化：{relative_path}"
            )));
        }
        for entry in std::fs::read_dir(path)
            .map_err(|error| AppError::generic(format!("读取目录失败：{error}")))?
        {
            let entry =
                entry.map_err(|error| AppError::generic(format!("读取目录项失败：{error}")))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                AppError::generic(format!("目录包含非 UTF-8 名称：{relative_path}"))
            })?;
            let child_relative = if relative_path.is_empty() {
                name.to_string()
            } else {
                format!("{relative_path}/{name}")
            };
            verify_local_delete_snapshot(&entry.path(), &child_relative, baselines, false)?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        return Err(AppError::generic(format!(
            "拒绝删除非普通文件：{relative_path}"
        )));
    }
    if crate::mount::manager::is_placeholder_file(path) {
        if allow_orphan_placeholder {
            return Ok(());
        }
        let baseline = baselines
            .get(relative_path)
            .ok_or_else(|| AppError::generic(format!("占位符不在同步基线中：{relative_path}")))?;
        if baseline.is_folder {
            return Err(AppError::generic(format!(
                "占位符类型与同步基线不一致：{relative_path}"
            )));
        }
        return Ok(());
    }

    let baseline = baselines
        .get(relative_path)
        .ok_or_else(|| AppError::generic(format!("文件不在同步基线中：{relative_path}")))?;
    if baseline.is_folder
        || baseline.local_mtime != metadata_mtime_ms(&metadata)
        || baseline.local_size != Some(metadata.len() as i64)
    {
        return Err(AppError::generic(format!(
            "文件在删除执行前发生变化：{relative_path}"
        )));
    }
    Ok(())
}

impl SyncExecutor {
    /// 仅在本地内容仍匹配持久基线时执行递归删除。
    pub(super) async fn do_delete_from_local(&self, action: &SyncAction) -> ActionResult {
        let path = match &action.local_path {
            Some(p) => PathBuf::from(p),
            None => {
                return ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                }
            } // DB 清理场景
        };
        let rel = action.relative_path.as_deref().unwrap_or("?");
        let fail = |technical_message: String, deferred: bool| {
            let user_message = crate::sync::user_messages::simplify_sync_error(&technical_message);
            tracing::warn!(
                rel,
                technical_reason = %technical_message,
                user_message = %user_message,
                "本地删除安全检查未通过"
            );
            ActionResult {
                success: false,
                error_message: Some(user_message.into_owned()),
                deferred,
                cloud_file: None,
            }
        };
        let Some(mount) = &self.mount else {
            return fail("mount manager 未初始化，拒绝删除本地内容".into(), false);
        };

        let baselines = match &self.db {
            Some(db) => {
                let conn = db.lock();
                match repository::load_all(&conn) {
                    Ok(items) => {
                        let mut by_path = HashMap::with_capacity(items.len());
                        let mut duplicate = None;
                        for item in items {
                            let path = item.local_path.clone();
                            if by_path.insert(path.clone(), item).is_some() {
                                duplicate = Some(path);
                                break;
                            }
                        }
                        if let Some(path) = duplicate {
                            return fail(format!("同步基线存在重复路径，拒绝删除：{path}"), true);
                        }
                        by_path
                    }
                    Err(error) => {
                        return fail(format!("读取同步基线失败，保留本地内容：{error}"), true)
                    }
                }
            }
            None => return fail("同步数据库未初始化，拒绝删除本地内容".into(), false),
        };

        let mut path_exists = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return fail(format!("无法读取待删除路径，保留本地内容：{error}"), true),
            Ok(_) => {
                let allow_orphan_placeholder = action.file_id.is_none();
                if let Err(error) =
                    verify_local_delete_snapshot(&path, rel, &baselines, allow_orphan_placeholder)
                {
                    return fail(error.to_string(), true);
                }
                true
            }
        };

        // 远端删除证明尽量贴近不可逆的本地删除动作。
        if let Some(file_id) = action.file_id.as_deref() {
            if file_id.starts_with(repository::PENDING_FILE_ID_PREFIX) {
                return fail("待上传记录没有可核验的远端删除事实".into(), true);
            }
            match self.files_api.verify_deleted(file_id).await {
                Ok(true) => {}
                Ok(false) => {
                    return fail("云端文件仍存在，取消本地删除并等待重新规划".into(), true)
                }
                Err(error) => {
                    return fail(format!("无法确认云端已删除，保留本地内容：{error}"), true)
                }
            }
        }

        // 远端核验返回后重新检查完整本地快照。
        if path_exists {
            path_exists = match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return fail(
                        format!("远端核验后无法读取待删除路径，保留本地内容：{error}"),
                        true,
                    )
                }
                Ok(_) => {
                    let allow_orphan_placeholder = action.file_id.is_none();
                    if let Err(error) = verify_local_delete_snapshot(
                        &path,
                        rel,
                        &baselines,
                        allow_orphan_placeholder,
                    ) {
                        return fail(
                            format!("远端核验期间本地内容发生变化，已取消删除：{error}"),
                            true,
                        );
                    }
                    true
                }
            };
        }

        let result = if !path_exists {
            ActionResult {
                success: true,
                error_message: None,
                deferred: false,
                cloud_file: None,
            }
        } else {
            match mount.delete_local_confirmed(&path).await {
                Ok(()) => ActionResult {
                    success: true,
                    error_message: None,
                    deferred: false,
                    cloud_file: None,
                },
                Err(error) => fail(error.to_string(), true),
            }
        };
        Self::log_action_result(rel, "删除本地文件成功", "删除本地文件失败", &result);
        result
    }
}
