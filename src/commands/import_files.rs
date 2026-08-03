//! 拖拽导入命令 —— 把挂载目录外的文件/目录复制进当前同步文件夹。
//!
//! 复制完成后由后台同步周期自动上传（watcher 防抖周期兜底），
//! 目标已存在的同名项拒绝覆盖并计入失败列表，部分失败不影响其余导入源。

use serde::Serialize;
use specta::Type;

use crate::error::{AppError, AppResult};
use crate::mount::import_copy::copy_into_mount;

use super::{mount, sync_engine};

/// 拖拽导入的单个失败项。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportFailure {
    /// 导入源的原始路径。
    pub source: String,
    /// 用户可读的中文失败原因。
    pub reason: String,
}

/// 拖拽导入结果汇总。
#[derive(Debug, Clone, Serialize, Type)]
pub struct ImportFilesResult {
    /// 成功复制的文件数（目录按其中文件逐个计）。
    pub imported: i64,
    /// 命中跳过规则或为符号链接而被跳过的条目数。
    pub skipped: i64,
    /// 失败的导入源及原因（含同名冲突拒绝覆盖）。
    pub failures: Vec<ImportFailure>,
}

/// 拖拽导入外部文件/目录到指定同步文件夹（相对挂载根路径，根目录为空串）。
///
/// 逐源独立复制，任一源失败不阻塞其余源；全部复制完后后台触发一次全量同步周期，
/// command 不等待上传完成即返回，传输进度经传输队列实时可见。
#[tauri::command]
#[specta::specta]
pub async fn drive_import_files(
    source_paths: Vec<String>,
    target_rel_path: String,
) -> AppResult<ImportFilesResult> {
    if source_paths.is_empty() {
        return Err(AppError::generic("没有可导入的文件"));
    }
    let engine = sync_engine()?;
    let _activity = engine.begin_external_activity()?;
    // 云端索引重建期间拒绝导入，避免复制完成后同步周期基于不完整的云端树规划。
    if engine.current_state().is_indexing {
        let user_message = "正在读取云端文件，请稍后再试";
        tracing::debug!(user_message, "云端索引构建期间拒绝拖拽导入");
        return Err(AppError::generic(user_message));
    }
    let m = mount()?;
    let target_dir = crate::core::paths::safe_join_under(m.mount_dir(), &target_rel_path, true)?;
    // 拖拽落点只能是当前浏览的真实文件夹，目标不存在直接失败（不为调用方建目录）。
    if !target_dir.is_dir() {
        return Err(AppError::generic(format!(
            "导入目标目录不存在：{target_rel_path}"
        )));
    }
    let skip = engine.skip_matcher();

    // 复制是阻塞 IO，移出 async runtime；逐源独立，部分失败归入失败列表继续。
    let (imported, skipped, failures) = tauri::async_runtime::spawn_blocking(move || {
        let mut imported = 0_i64;
        let mut skipped = 0_i64;
        let mut failures: Vec<ImportFailure> = Vec::new();
        for source in source_paths {
            let path = std::path::PathBuf::from(&source);
            if !path.is_absolute() {
                failures.push(ImportFailure {
                    reason: "路径不是绝对路径".to_string(),
                    source,
                });
                continue;
            }
            match copy_into_mount(&path, &target_dir, &skip) {
                Ok(stats) => {
                    imported += stats.copied_files;
                    skipped += stats.skipped;
                    // 冲突按“拒绝覆盖”归入失败列表，展示挂载内相对路径。
                    failures.extend(stats.conflicts.into_iter().map(|conflict| ImportFailure {
                        source: source.clone(),
                        reason: format!("目标已存在同名项，拒绝覆盖：{conflict}"),
                    }));
                }
                Err(error) => failures.push(ImportFailure {
                    source,
                    reason: error.to_string(),
                }),
            }
        }
        (imported, skipped, failures)
    })
    .await
    .map_err(|error| AppError::generic(format!("拖拽导入任务失败：{error}")))?;

    // 复制完成立即后台触发全量同步周期上传；不 await，watcher 防抖周期作为兜底。
    if imported > 0 {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = engine.trigger_manual_sync().await {
                tracing::warn!(%error, "拖拽导入后触发同步周期失败");
            }
        });
    }
    tracing::info!(imported, skipped, failures = failures.len(), "拖拽导入完成");
    Ok(ImportFilesResult {
        imported,
        skipped,
        failures,
    })
}
