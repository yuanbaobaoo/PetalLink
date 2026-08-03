//! 拖拽导入复制 —— 把挂载目录外的文件/目录递归复制进挂载目录。
//!
//! 复制全程使用 `.tmp` 临时文件 + 原子 rename，保证 watcher 防抖窗口内不会扫到半截文件；
//! 目标已存在一律拒绝覆盖，按条目计入冲突列表返回，不中断其余条目。

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::mount::skip::SkipMatcher;

/// 单个导入源的复制结果统计。
#[derive(Clone, Debug, Default)]
pub struct CopyStats {
    /// 实际复制完成的文件数。
    pub copied_files: i64,
    /// 实际创建的目录数。
    pub copied_dirs: i64,
    /// 命中跳过规则、符号链接或非法名称而被跳过的条目数。
    pub skipped: i64,
    /// 因目标已存在而拒绝覆盖的条目展示路径（相对导入根）。
    pub conflicts: Vec<String>,
}

/// 把 source（文件或目录）复制到 target_dir 下，目标名取 source 的文件名。
///
/// 失败语义：源不存在、源为符号链接、源名非法或命中跳过规则、源与目标位置重叠时返回 Err；
/// 目标已存在不报错，计入 `CopyStats.conflicts` 并跳过该项，保证部分失败不中断整体导入。
pub fn copy_into_mount(
    source: &Path,
    target_dir: &Path,
    skip: &SkipMatcher,
) -> AppResult<CopyStats> {
    // 准入校验：源必须真实存在且不是符号链接（不跟随，避免把挂载外内容链进来）。
    let source_meta = fs::symlink_metadata(source)
        .map_err(|error| AppError::generic(format!("读取导入源失败：{error}")))?;
    if source_meta.file_type().is_symlink() {
        return Err(AppError::generic(format!(
            "导入源不能是符号链接：{}",
            source.display()
        )));
    }
    let name = source
        .file_name()
        .and_then(|segment| segment.to_str())
        .ok_or_else(|| AppError::generic(format!("导入源缺少有效文件名：{}", source.display())))?;
    crate::core::paths::validate_path_segment(name)?;
    if skip.should_skip(name) {
        return Err(AppError::generic(format!("导入源命中跳过规则：{name}")));
    }

    // 防自我复制：源与目标 canonical 后相同，或源是目标目录的祖先（复制进自身）时拒绝。
    let canonical_source = fs::canonicalize(source)
        .map_err(|error| AppError::generic(format!("解析导入源路径失败：{error}")))?;
    let canonical_target_dir = fs::canonicalize(target_dir)
        .map_err(|error| AppError::generic(format!("解析导入目标目录失败：{error}")))?;
    if canonical_source == canonical_target_dir.join(name)
        || canonical_target_dir.starts_with(&canonical_source)
    {
        return Err(AppError::generic(format!(
            "导入源与目标位置重叠，拒绝复制：{}",
            source.display()
        )));
    }

    let mut stats = CopyStats::default();
    let target = target_dir.join(name);
    if source_meta.is_dir() {
        copy_dir_recursive(source, &target, name, skip, &mut stats)?;
    } else {
        copy_file_atomic(source, &target, name, &mut stats)?;
    }
    Ok(stats)
}

/// 递归复制目录；目标目录已存在时整体记冲突并拒绝合并，避免覆盖云端镜像中的既有内容。
fn copy_dir_recursive(
    source_dir: &Path,
    target_dir: &Path,
    display_path: &str,
    skip: &SkipMatcher,
    stats: &mut CopyStats,
) -> AppResult<()> {
    if target_dir.symlink_metadata().is_ok() {
        stats.conflicts.push(display_path.to_string());
        return Ok(());
    }
    fs::create_dir_all(target_dir)
        .map_err(|error| AppError::generic(format!("创建导入目录失败：{error}")))?;
    stats.copied_dirs += 1;

    let entries = fs::read_dir(source_dir)
        .map_err(|error| AppError::generic(format!("读取导入源目录失败：{error}")))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| AppError::generic(format!("读取导入源目录项失败：{error}")))?;
        // 无法读取或非 UTF-8 名称的条目无法安全落盘，按跳过处理。
        let Some(child_name) = entry.file_name().to_str().map(str::to_owned) else {
            stats.skipped += 1;
            continue;
        };
        if skip.should_skip(&child_name)
            || crate::core::paths::validate_path_segment(&child_name).is_err()
        {
            stats.skipped += 1;
            continue;
        }
        // entry.file_type 不跟随符号链接；符号链接跳过，防止把挂载外目标链入镜像。
        let file_type = entry
            .file_type()
            .map_err(|error| AppError::generic(format!("读取导入源类型失败：{error}")))?;
        let child_display = format!("{display_path}/{child_name}");
        if file_type.is_symlink() {
            stats.skipped += 1;
        } else if file_type.is_dir() {
            copy_dir_recursive(
                &entry.path(),
                &target_dir.join(&child_name),
                &child_display,
                skip,
                stats,
            )?;
        } else {
            copy_file_atomic(
                &entry.path(),
                &target_dir.join(&child_name),
                &child_display,
                stats,
            )?;
        }
    }
    Ok(())
}

/// 以 `.tmp` + 原子 rename 复制单个文件；目标已存在记冲突，rename 失败时清理临时文件。
fn copy_file_atomic(
    source_file: &Path,
    target_file: &Path,
    display_path: &str,
    stats: &mut CopyStats,
) -> AppResult<()> {
    if target_file.symlink_metadata().is_ok() {
        stats.conflicts.push(display_path.to_string());
        return Ok(());
    }
    // 临时文件与目标同目录，保证 rename 是同卷原子操作；`.tmp` 后缀被 watcher/scanner 跳过。
    let tmp_file = temp_sibling(target_file);
    fs::copy(source_file, &tmp_file)
        .map_err(|error| AppError::generic(format!("复制导入文件失败：{error}")))?;
    if let Err(error) = fs::rename(&tmp_file, target_file) {
        let _ = fs::remove_file(&tmp_file);
        return Err(AppError::generic(format!("落盘导入文件失败：{error}")));
    }
    stats.copied_files += 1;
    Ok(())
}

/// 生成目标文件旁的 `.tmp` 临时路径（沿用下载落盘的 `.tmp` 后缀约定）。
fn temp_sibling(target_file: &Path) -> PathBuf {
    let name = target_file
        .file_name()
        .map(|segment| segment.to_string_lossy().into_owned())
        .unwrap_or_default();
    target_file.with_file_name(format!("{name}{}", crate::constants::TMP_SUFFIX))
}
