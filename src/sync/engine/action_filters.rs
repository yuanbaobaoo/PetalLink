//! 同步动作的纯过滤、补全与规划去重。

use std::collections::{HashMap, HashSet};

use crate::sync::state::{SyncAction, SyncActionType};

/// 丢弃近期远端删除路径上的回摆动作，但保留继续确认云端删除的动作。
pub(super) fn filter_anti_oscillation(actions: &mut Vec<SyncAction>, rdp: &HashMap<String, i64>) {
    actions.retain(|a| {
        let rel = match &a.relative_path {
            Some(p) => p,
            None => return true,
        };
        !rdp.contains_key(rel) || matches!(a.action_type, SyncActionType::DeleteFromCloud)
    });
}

/// 依据父路径映射，为尚未指定父目录的嵌套动作补充云端目录标识。
pub(super) fn fill_parent_file_ids(actions: &mut [SyncAction], p2i: &HashMap<String, String>) {
    for a in actions {
        if a.parent_file_id.is_some() || a.relative_path.is_none() {
            continue;
        }
        let rel = a.relative_path.as_ref().unwrap();
        if let Some(pos) = rel.rfind('/') {
            if let Some(pid) = p2i.get(&rel[..pos]) {
                a.parent_file_id = Some(pid.clone());
            }
        }
    }
}

/// 为「云端已删除目录下、有云端内容要创建」的动作补建目录链到云端。
///
/// 场景：云端删了整个目录 B（含 B/sub），本地改过 B/sub/f2.txt → f2 走
/// BackupBeforeCloudDelete 改名备份（本地目录链已保留）。但副本下轮 Upload、
/// 或本轮有 Upload/冲突副本/本地新目录落在被删目录下时，父目录已不在云端
/// path_to_id → 内容会落到云端根目录。本方法为这些「被删但有内容要放进去」的
/// 祖先目录补 CreateFolder（cloud_file=None，本地→云端重建），execute_actions_ordered
/// 阶段 1 会先建它们并回填 path_to_id，内容即落到正确目录。
pub(super) fn add_rescue_folder_recreations(
    actions: &mut Vec<SyncAction>,
    snapshot: &crate::sync::planner::SyncSnapshot,
    recently_deleted: &HashMap<String, i64>,
) {
    // 仅对「创建云端内容」的动作（上传/备份副本/冲突副本/本地新建目录）补建父目录链；
    // 下载/删除/占位不创建云端内容，无需为其父目录重建（避免误重建正在清理的目录）。
    let rescue_paths: Vec<String> = actions
        .iter()
        .filter(|a| {
            matches!(
                a.action_type,
                SyncActionType::Upload
                    | SyncActionType::MoveInCloud
                    | SyncActionType::BackupBeforeCloudDelete
                    | SyncActionType::CreateConflictCopy
            ) || (a.action_type == SyncActionType::CreateFolder && a.cloud_file.is_none())
        })
        .filter_map(|a| a.relative_path.clone())
        .collect();
    if rescue_paths.is_empty() {
        return;
    }

    // 已有动作的路径（owned，避免与下方 push 的可变借用冲突）
    let existing: HashSet<String> = actions
        .iter()
        .filter_map(|a| a.relative_path.clone())
        .collect();

    let mut to_recreate: HashSet<String> = HashSet::new();
    for path in &rescue_paths {
        // 枚举所有祖先目录前缀
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let ancestor = parts[..i].join("/");
            if existing.contains(&ancestor) {
                continue;
            }
            // 祖先是「云端已删除的本地目录」：本地有目录 + 云端无 + db 有
            // ★ 跳过用户主动删除的目录（已在 recently_deleted 中），避免"救援重建"已删除目录
            let is_deleted_folder = snapshot
                .local
                .get(&ancestor)
                .map(|e| e.is_folder)
                .unwrap_or(false)
                && !snapshot.cloud.contains_key(&ancestor)
                && snapshot.db.contains_key(&ancestor)
                && !recently_deleted.contains_key(&ancestor);
            if is_deleted_folder {
                to_recreate.insert(ancestor);
            }
        }
    }
    if to_recreate.is_empty() {
        return;
    }

    // 按深度升序加入（父先建）；execute_actions_ordered 阶段 1 会再排一次并回填 path_to_id
    let mut folders: Vec<String> = to_recreate.into_iter().collect();
    folders.sort_by_key(|p| p.matches('/').count());
    for rel in folders {
        let Some(entry) = snapshot.local.get(&rel) else {
            continue;
        };
        actions.push(SyncAction {
            action_type: SyncActionType::CreateFolder,
            relative_path: Some(rel.clone()),
            file_id: None,
            parent_file_id: None,
            local_path: Some(entry.absolute_path.to_string_lossy().to_string()),
            cloud_file: None,
            reason: Some("云端已删除但内有内容需救援 → 重建目录到云端".to_string()),
        });
        tracing::info!(rel = %rel, "为救援内容补建云端目录");
    }
}

/// 目录级联删除只做纯规划去重：仅当 planner 已明确产生一个真实云端目录的
/// `DeleteFromCloud` 时，才移除其子孙删除动作。绝不能把“直接文件恰好都删除”扩大为
/// 父目录删除，也绝不能在远端确认前修改成功基线。
pub(super) fn dedupe_directory_deletes(
    actions: &mut Vec<SyncAction>,
    cloud_tree: &HashMap<String, crate::drive::models::DriveFile>,
) {
    let explicit_directory_deletes: Vec<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::DeleteFromCloud)
        .filter_map(|action| action.relative_path.as_ref())
        .filter(|path| {
            cloud_tree
                .get(path.as_str())
                .is_some_and(|entry| entry.is_folder())
        })
        .cloned()
        .collect();
    if explicit_directory_deletes.is_empty() {
        return;
    }

    let mut removed = 0usize;
    actions.retain(|action| {
        if action.action_type != SyncActionType::DeleteFromCloud {
            return true;
        }
        let Some(path) = action.relative_path.as_deref() else {
            return true;
        };
        let covered = explicit_directory_deletes
            .iter()
            .any(|directory| path != directory && path.starts_with(&format!("{directory}/")));
        if covered {
            removed += 1;
        }
        !covered
    });
    if removed > 0 {
        tracing::info!(removed, "显式目录删除覆盖子孙动作；仅去重，不提前结算");
    }
}

/// §2.13 目录删除保护：若云端目录下有文件被 BackupBeforeCloudDelete（本地修改过
/// 需要备份保存），则移除该目录的 DeleteFromLocal，保留目录作为备份副本的栖身之所。
/// 其余无本地修改的目录正常删除。
pub(super) fn preserve_dirs_with_pending_backups(actions: &mut Vec<SyncAction>) {
    // 收集所有 BackupBeforeCloudDelete 的目标路径（owned，避免 borrow 冲突）
    let backup_paths: HashSet<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete)
        .filter_map(|a| a.relative_path.clone())
        .collect();
    if backup_paths.is_empty() {
        return;
    }
    // 找出哪些 DeleteFromLocal 目标是目录，且其下有文件需要备份
    let mut preserved = 0usize;
    actions.retain(|a| {
        if a.action_type != SyncActionType::DeleteFromLocal {
            return true;
        }
        let Some(rel) = &a.relative_path else {
            return true;
        };
        // 检查是否有 BackupBeforeCloudDelete 的文件在此目录下
        let has_backup_child = backup_paths
            .iter()
            .any(|bp| bp.starts_with(&format!("{}/", rel)));
        if has_backup_child {
            tracing::info!(
                dir = %rel,
                "保留本地目录：目录下有文件需 BackupBeforeCloudDelete（备份副本需要栖身目录）"
            );
            preserved += 1;
            return false; // 移除 DeleteFromLocal
        }
        true
    });
    if preserved > 0 {
        tracing::info!(
            preserved,
            "目录删除保护：保留 {} 个有备份子文件的目录",
            preserved
        );
    }
}

/// DeleteFromLocal 祖先去重：若目录自身已在 DeleteFromLocal 列表中，
/// 则其子孙的文件删除动作是多余的（目录 delete 会级联清空）。移除它们，
/// 避免并发执行时报 "No such file or directory"。
pub(super) fn dedupe_local_descendants(actions: &mut Vec<SyncAction>) {
    // 收集所有 DeleteFromLocal 的路径（owned，避免 borrow 冲突）
    let delete_paths: Vec<String> = actions
        .iter()
        .filter(|a| a.action_type == SyncActionType::DeleteFromLocal)
        .filter_map(|a| a.relative_path.clone())
        .collect();
    let ancestor_set: HashSet<&str> = delete_paths.iter().map(|s| s.as_str()).collect();
    let mut skipped = 0usize;
    actions.retain(|a| {
        if a.action_type != SyncActionType::DeleteFromLocal {
            return true;
        }
        let Some(rel) = &a.relative_path else {
            return true;
        };
        let has_ancestor = (0..rel.len())
            .any(|i| rel.as_bytes().get(i) == Some(&b'/') && ancestor_set.contains(&rel[..i]));
        if has_ancestor {
            skipped += 1;
            return false;
        }
        true
    });
    if skipped > 0 {
        tracing::info!(
            skipped,
            "DeleteFromLocal 祖先去重：跳过 {} 个被子目录删除覆盖的文件",
            skipped
        );
    }
}
