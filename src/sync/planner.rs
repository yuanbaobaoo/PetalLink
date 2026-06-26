//! 同步规划器 —— 3-way diff（本地 vs 云端 vs DB）。
//!
//! 对齐 `legacy/lib/sync/sync_planner.dart` 的 `_decide` 决策表。
//!
//! 输入：`SyncSnapshot { local, cloud, db, is_startup_resume }`
//! 输出：按路径过滤后的 `Vec<SyncAction>`（不含 skip/null 类型）。

use std::collections::{HashMap, HashSet};

use crate::drive::models::DriveFile;
use crate::mount::manager::LocalFileEntry;
use crate::sync::state::{SyncAction, SyncActionType};

/// 同步快照（3 方数据视图）
pub struct SyncSnapshot {
    /// 本地文件条目（rel_path → entry）
    pub local: HashMap<String, LocalFileEntry>,
    /// 云端文件树（rel_path → DriveFile）
    pub cloud: HashMap<String, DriveFile>,
    /// DB 同步记录（rel_path → DB record，含 mtime/size/status）
    pub db: HashMap<String, DbSnapshotEntry>,
    /// 是否为启动恢复期（影响删除语义）
    pub is_startup_resume: bool,
}

/// DB 记录快照（只取 plan 需要的字段）
#[derive(Debug, Clone)]
pub struct DbSnapshotEntry {
    pub file_id: String,
    pub local_mtime: Option<i64>,
    pub local_size: Option<i64>,
    pub cloud_edited_time: Option<i64>,
    pub status: i32,
    pub is_folder: bool,
}

/// 同步规划器
pub struct SyncPlanner;

impl SyncPlanner {
    /// 执行 diff，返回动作列表（跳过 null/skip 类型）。
    /// 对齐 dart `plan(SyncSnapshot)`。
    pub fn plan(&self, snapshot: &SyncSnapshot) -> Vec<SyncAction> {
        // 收集全部路径（local ∪ cloud ∪ db）
        let mut all_paths: HashSet<&str> = HashSet::new();
        for k in snapshot.local.keys() {
            all_paths.insert(k.as_str());
        }
        for k in snapshot.cloud.keys() {
            all_paths.insert(k.as_str());
        }
        for k in snapshot.db.keys() {
            all_paths.insert(k.as_str());
        }

        let mut actions = Vec::new();
        for rel_path in all_paths {
            if let Some(action) = self.decide(rel_path, snapshot) {
                // 过滤 Skip 类型（对齐 dart plan() 的 action.type != SyncActionType.skip 过滤）
                if action.action_type == SyncActionType::Skip {
                    continue;
                }
                actions.push(action);
            }
        }
        actions
    }

    /// 单路径决策（对齐 dart `_decide`）。
    fn decide(&self, rel_path: &str, snap: &SyncSnapshot) -> Option<SyncAction> {
        let local = snap.local.get(rel_path);
        let cloud = snap.cloud.get(rel_path);
        let db = snap.db.get(rel_path);

        let local_exists = local.is_some();
        let local_has_content = local.map(|e| !e.is_placeholder).unwrap_or(false);
        let cloud_exists = cloud.is_some();
        let db_exists = db.is_some();

        // === 文件夹 ===
        if cloud.map(|c| c.is_folder()).unwrap_or(false) {
            if !local_exists {
                // 文件夹双向删除禁用（安全：级联丢失风险）
                // 对齐 dart：db 有记录且非启动恢复期 → skip
                if db_exists && !snap.is_startup_resume {
                    return None; // skip — 用户手动删除云端文件夹的语义已禁用
                }
                // 否则本地缺失 → 创建文件夹
                return Some(SyncAction {
                    action_type: SyncActionType::CreateFolder,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: cloud.unwrap().parent_folder.as_ref().and_then(|v| v.first().cloned()),
                    local_path: None,
                    cloud_file: Some(cloud.unwrap().clone()),
                    reason: Some("云端文件夹 → 本地创建".to_string()),
                });
            }
            if local_exists && cloud_exists {
                return None; // 双方都已有文件夹 → skip
            }
        }

        // === 全缺席 ===
        if !local_exists && !cloud_exists && !db_exists {
            return None;
        }

        // === 三方都存在（文件）===
        if local_has_content && cloud_exists && db_exists {
            let local_changed = is_local_changed(local.unwrap(), db.unwrap());
            let cloud_changed = is_cloud_changed(cloud.unwrap(), db.unwrap());
            if local_changed && cloud_changed {
                return Some(SyncAction {
                    action_type: SyncActionType::CreateConflictCopy,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: Some(cloud.unwrap().clone()),
                    reason: Some("三方都存在，本地/云端均已修改 → 冲突".to_string()),
                });
            } else if local_changed {
                return Some(SyncAction {
                    action_type: SyncActionType::Upload,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: None,
                    reason: Some("本地已修改 → 上传".to_string()),
                });
            } else if cloud_changed {
                return Some(SyncAction {
                    action_type: SyncActionType::Download,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: Some(cloud.unwrap().clone()),
                    reason: Some("云端已修改 → 下载".to_string()),
                });
            } else {
                return None; // 未变化 → skip
            }
        }

        // === 本地有内容 + 云端有 + 无 DB（首次记录兜底，不做动作，让 reconcile 补 DB）===
        if local_exists && cloud_exists && !db_exists {
            return Some(SyncAction {
                action_type: SyncActionType::Skip,
                relative_path: Some(rel_path.to_string()),
                file_id: cloud.unwrap().id.clone().into(),
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: Some("双方都有但无 DB 记录 → skip，由 reconcile 补 DB".to_string()),
            });
        }

        // === 本地有 + 云端无 ===
        if local_exists && !cloud_exists {
            if db_exists {
                // 文件夹：云端删除也不删本地（双向删除禁用 + 保目录结构）。
                // 关键：若云端删了整个目录但本地改了其内某文件，该文件会走 BackupBeforeCloudDelete
                // 改名备份，此时父目录链必须存在——否则副本无家可归。目录不删即保留栖身之所。
                if local.unwrap().is_folder {
                    return None; // skip — 不传播云端文件夹删除
                }
                // 文件：本地有未上传的真实修改 → 改名备份副本（冲突保护），原路径腾空即满足云端删除
                if local_has_content && is_local_changed(local.unwrap(), db.unwrap()) {
                    return Some(SyncAction {
                        action_type: SyncActionType::BackupBeforeCloudDelete,
                        relative_path: Some(rel_path.to_string()),
                        file_id: db.unwrap().file_id.clone().into(),
                        parent_file_id: None,
                        local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                        cloud_file: None,
                        reason: Some("云端已删除但本地有未上传修改 → 备份副本".to_string()),
                    });
                }
                // 未改 / 占位 → 删除本地（匹配云端删除）
                return Some(SyncAction {
                    action_type: SyncActionType::DeleteFromLocal,
                    relative_path: Some(rel_path.to_string()),
                    file_id: db.unwrap().file_id.clone().into(),
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: None,
                    reason: Some("云端已删除 → 删除本地".to_string()),
                });
            }
            if !local_has_content {
                // 本地占位符且无 DB → 孤儿占位符清理
                return Some(SyncAction {
                    action_type: SyncActionType::DeleteFromLocal,
                    relative_path: Some(rel_path.to_string()),
                    file_id: None,
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: None,
                    reason: Some("孤儿占位符 → 清理".to_string()),
                });
            }
            // 本地新文件夹 → 新建云端文件夹
            if local.unwrap().is_folder {
                return Some(SyncAction {
                    action_type: SyncActionType::CreateFolder,
                    relative_path: Some(rel_path.to_string()),
                    file_id: None,
                    parent_file_id: None,
                    local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                    cloud_file: None,
                    reason: Some("本地新增文件夹 → 创建云端文件夹".to_string()),
                });
            }
            // 本地新文件 → 上传
            return Some(SyncAction {
                action_type: SyncActionType::Upload,
                relative_path: Some(rel_path.to_string()),
                file_id: None,
                parent_file_id: None,
                local_path: Some(local.unwrap().absolute_path.to_string_lossy().to_string()),
                cloud_file: None,
                reason: Some("本地新文件 → 上传".to_string()),
            });
        }

        // === 本地无 + 云端有 ===
        if !local_exists && cloud_exists {
            if db_exists && !snap.is_startup_resume {
                // 会话内删除 → 双向删除云端
                return Some(SyncAction {
                    action_type: SyncActionType::DeleteFromCloud,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: None,
                    local_path: None,
                    cloud_file: None,
                    reason: Some("会话内删除 → 双向删除云端".to_string()),
                });
            }
            // 启动恢复期 或 无 DB → 创建占位符
            let reason = if snap.is_startup_resume && db_exists {
                "启动后恢复删除 → 重建占位".to_string()
            } else {
                "云端新文件 → 创建占位".to_string()
            };
            return Some(SyncAction {
                action_type: SyncActionType::CreatePlaceholder,
                relative_path: Some(rel_path.to_string()),
                file_id: cloud.unwrap().id.clone().into(),
                parent_file_id: None,
                local_path: None,
                cloud_file: Some(cloud.unwrap().clone()),
                reason: Some(reason),
            });
        }

        // === 本地无 + 云端无 + DB 有（双方都删了，清理 DB 记录） ===
        if !local_exists && !cloud_exists && db_exists {
            return Some(SyncAction {
                action_type: SyncActionType::DeleteFromLocal,
                relative_path: Some(rel_path.to_string()),
                file_id: None,
                parent_file_id: None,
                local_path: None,
                cloud_file: None,
                reason: Some("双方都已删除 → 清理 DB 记录".to_string()),
            });
        }

        None
    }
}

impl Default for SyncPlanner {
    fn default() -> Self {
        Self
    }
}

/// 本地是否变更（mtime 或 size 与 DB 不同）。
/// 对齐 dart `_isLocalChanged`。
pub fn is_local_changed(local: &LocalFileEntry, db: &DbSnapshotEntry) -> bool {
    if db.local_mtime.is_none() {
        return true; // 首次记录
    }
    if local.mtime != db.local_mtime.unwrap() {
        return true;
    }
    // 同时检查 localSize（v3，避免 mtime 精度不足漏判）
    if let Some(db_size) = db.local_size {
        if local.size as i64 != db_size {
            return true;
        }
    }
    false
}

/// 云端是否变更（仅比较 editedTime，用云端时间为权威基准）。
/// 对齐 dart `_isCloudChanged`。
pub fn is_cloud_changed(cloud: &DriveFile, db: &DbSnapshotEntry) -> bool {
    let cloud_edited_ms = cloud.edited_time.map(|t| t.timestamp_millis());
    if cloud_edited_ms.is_none() {
        return false;
    }
    if db.cloud_edited_time.is_none() {
        return true;
    }
    cloud_edited_ms.unwrap() != db.cloud_edited_time.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::repository::sync_status;
    use std::path::PathBuf;

    fn make_local(path: &str, size: u64, mtime: i64, is_placeholder: bool) -> LocalFileEntry {
        LocalFileEntry {
            absolute_path: PathBuf::from(format!("/mount/{path}")),
            relative_path: path.to_string(),
            size,
            mtime,
            is_folder: false,
            is_placeholder,
        }
    }

    fn make_local_folder(path: &str) -> LocalFileEntry {
        LocalFileEntry {
            absolute_path: PathBuf::from(format!("/mount/{path}")),
            relative_path: path.to_string(),
            size: 0,
            mtime: 1000,
            is_folder: true,
            is_placeholder: false,
        }
    }

    fn make_cloud(id: &str, name: &str, is_folder: bool, edited_time_ms: i64) -> DriveFile {
        use crate::drive::models::FileCategory;
        DriveFile {
            id: id.to_string(),
            name: name.to_string(),
            category: if is_folder { FileCategory::Folder } else { FileCategory::None },
            size: if is_folder { 0 } else { 100 },
            parent_folder: None,
            description: None,
            created_time: None,
            edited_time: chrono::DateTime::from_timestamp_millis(edited_time_ms),
            mime_type: if is_folder { Some("application/vnd.huawei-apps.folder".into()) } else { Some("text/plain".into()) },
            content_hash: None,
            thumbnail_link: None,
        }
    }

    fn make_db(file_id: &str, local_mtime: i64, local_size: i64, cloud_edited_time: i64, status: i32) -> DbSnapshotEntry {
        DbSnapshotEntry {
            file_id: file_id.to_string(),
            local_mtime: Some(local_mtime),
            local_size: Some(local_size),
            cloud_edited_time: Some(cloud_edited_time),
            status,
            is_folder: false,
        }
    }

    #[test]
    fn test_local_new_file_upload() {
        let mut local = HashMap::new();
        local.insert("new.txt".to_string(), make_local("new.txt", 100, 1000, false));
        let snapshot = SyncSnapshot {
            local,
            cloud: HashMap::new(),
            db: HashMap::new(),
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, SyncActionType::Upload);
        assert_eq!(actions[0].relative_path.as_deref(), Some("new.txt"));
    }

    #[test]
    fn test_cloud_new_file_placeholder() {
        let mut cloud = HashMap::new();
        cloud.insert("cloud-file.txt".to_string(), make_cloud("f1", "cloud-file.txt", false, 1000));
        let snapshot = SyncSnapshot {
            local: HashMap::new(),
            cloud,
            db: HashMap::new(),
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, SyncActionType::CreatePlaceholder);
    }

    #[test]
    fn test_local_changed_upload() {
        let mut local = HashMap::new();
        local.insert("f.txt".to_string(), make_local("f.txt", 200, 3000, false)); // size changed (was 100)
        let mut cloud = HashMap::new();
        cloud.insert("f.txt".to_string(), make_cloud("f1", "f.txt", false, 1000));
        let mut db = HashMap::new();
        db.insert("f.txt".to_string(), make_db("f1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot {
            local,
            cloud,
            db,
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(actions.iter().any(|a| a.action_type == SyncActionType::Upload));
    }

    #[test]
    fn test_cloud_changed_download() {
        let mut local = HashMap::new();
        local.insert("f.txt".to_string(), make_local("f.txt", 100, 1000, false));
        let mut cloud = HashMap::new();
        cloud.insert("f.txt".to_string(), make_cloud("f1", "f.txt", false, 5000)); // editedTime changed
        let mut db = HashMap::new();
        db.insert("f.txt".to_string(), make_db("f1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot {
            local,
            cloud,
            db,
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(actions.iter().any(|a| a.action_type == SyncActionType::Download));
    }

    #[test]
    fn test_conflict_when_both_changed() {
        let mut local = HashMap::new();
        local.insert("f.txt".to_string(), make_local("f.txt", 200, 3000, false));
        let mut cloud = HashMap::new();
        cloud.insert("f.txt".to_string(), make_cloud("f1", "f.txt", false, 5000));
        let mut db = HashMap::new();
        db.insert("f.txt".to_string(), make_db("f1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot {
            local,
            cloud,
            db,
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(actions.iter().any(|a| a.action_type == SyncActionType::CreateConflictCopy));
    }

    #[test]
    fn test_cloud_folder_creates_local_folder() {
        let mut cloud = HashMap::new();
        cloud.insert("docs".to_string(), make_cloud("folder-1", "docs", true, 0));
        let snapshot = SyncSnapshot {
            local: HashMap::new(),
            cloud,
            db: HashMap::new(),
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, SyncActionType::CreateFolder);
    }

    #[test]
    fn test_no_action_when_both_synced() {
        let mut local = HashMap::new();
        local.insert("f.txt".to_string(), make_local("f.txt", 100, 1000, false));
        let mut cloud = HashMap::new();
        cloud.insert("f.txt".to_string(), make_cloud("f1", "f.txt", false, 1000));
        let mut db = HashMap::new();
        db.insert("f.txt".to_string(), make_db("f1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot {
            local,
            cloud,
            db,
            is_startup_resume: false,
        };
        let actions = SyncPlanner.plan(&snapshot);
        // 未变化 → 应无动作（或 skip 被过滤）
        assert!(!actions.iter().any(|a| a.action_type != SyncActionType::Skip));
    }

    /// 云端删除整个目录（云端无、本地有文件夹、db 有）→ 不删本地（保目录结构供副本栖身）。
    #[test]
    fn test_cloud_deleted_folder_skipped() {
        let mut local = HashMap::new();
        local.insert("B".to_string(), make_local_folder("B"));
        let mut db = HashMap::new();
        db.insert(
            "B".to_string(),
            DbSnapshotEntry {
                file_id: "folder-B".into(),
                local_mtime: None,
                local_size: None,
                cloud_edited_time: Some(1000),
                status: sync_status::SYNCED,
                is_folder: true,
            },
        );
        let snapshot = SyncSnapshot { local, cloud: HashMap::new(), db, is_startup_resume: false };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(actions.is_empty(), "云端删除的目录不应产生删除动作（保目录结构）");
    }

    /// 云端删除文件 + 本地有未上传修改 → 备份副本（不直接删，保内容）。
    #[test]
    fn test_cloud_deleted_modified_file_backed_up() {
        let mut local = HashMap::new();
        // 本地 mtime=5000 ≠ db mtime=1000 → is_local_changed
        local.insert("A/f.txt".to_string(), make_local("A/f.txt", 200, 5000, false));
        let mut db = HashMap::new();
        db.insert("A/f.txt".to_string(), make_db("fid-1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot { local, cloud: HashMap::new(), db, is_startup_resume: false };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(
            actions.iter().any(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete),
            "本地有未上传修改 → 应备份副本而非直接删"
        );
        assert!(
            !actions.iter().any(|a| a.action_type == SyncActionType::DeleteFromLocal),
            "改过的文件不应直接删除"
        );
    }

    /// 云端删除文件 + 本地未改 → 删除本地（匹配云端）。
    #[test]
    fn test_cloud_deleted_unchanged_file_deleted() {
        let mut local = HashMap::new();
        // mtime/size 与 db 一致 → 未改
        local.insert("A/f.txt".to_string(), make_local("A/f.txt", 100, 1000, false));
        let mut db = HashMap::new();
        db.insert("A/f.txt".to_string(), make_db("fid-1", 1000, 100, 1000, sync_status::SYNCED));
        let snapshot = SyncSnapshot { local, cloud: HashMap::new(), db, is_startup_resume: false };
        let actions = SyncPlanner.plan(&snapshot);
        assert!(actions.iter().any(|a| a.action_type == SyncActionType::DeleteFromLocal));
        assert!(!actions.iter().any(|a| a.action_type == SyncActionType::BackupBeforeCloudDelete));
    }

    /// 云端删除整个目录 + 目录内某文件本地改过 → 目录 skip + 改过文件备份 + 未改文件删除。
    /// 验证：副本所在目录链（B、B/sub）保留，副本有家可归。
    #[test]
    fn test_cloud_deleted_folder_with_modified_file_preserves_chain() {
        let mut local = HashMap::new();
        local.insert("B".to_string(), make_local_folder("B"));
        local.insert("B/sub".to_string(), make_local_folder("B/sub"));
        // f2.txt 本地改过（mtime≠db）
        local.insert("B/sub/f2.txt".to_string(), make_local("B/sub/f2.txt", 300, 9000, false));
        // f1.txt 未改
        local.insert("B/sub/f1.txt".to_string(), make_local("B/sub/f1.txt", 100, 1000, false));

        let mut db = HashMap::new();
        db.insert(
            "B".to_string(),
            DbSnapshotEntry {
                file_id: "fb".into(),
                local_mtime: None,
                local_size: None,
                cloud_edited_time: Some(1000),
                status: sync_status::SYNCED,
                is_folder: true,
            },
        );
        db.insert(
            "B/sub".to_string(),
            DbSnapshotEntry {
                file_id: "fbs".into(),
                local_mtime: None,
                local_size: None,
                cloud_edited_time: Some(1000),
                status: sync_status::SYNCED,
                is_folder: true,
            },
        );
        db.insert("B/sub/f2.txt".to_string(), make_db("fid2", 1000, 100, 1000, sync_status::SYNCED));
        db.insert("B/sub/f1.txt".to_string(), make_db("fid1", 1000, 100, 1000, sync_status::SYNCED));

        let snapshot = SyncSnapshot { local, cloud: HashMap::new(), db, is_startup_resume: false };
        let actions = SyncPlanner.plan(&snapshot);

        // 目录 B、B/sub → 不删（保栖身之所）
        assert!(!actions.iter().any(|a| a.relative_path.as_deref() == Some("B")
            && a.action_type == SyncActionType::DeleteFromLocal));
        assert!(!actions.iter().any(|a| a.relative_path.as_deref() == Some("B/sub")
            && a.action_type == SyncActionType::DeleteFromLocal));
        // 改过的 f2.txt → 备份；未改的 f1.txt → 删除
        assert!(actions.iter().any(|a| a.relative_path.as_deref() == Some("B/sub/f2.txt")
            && a.action_type == SyncActionType::BackupBeforeCloudDelete));
        assert!(actions.iter().any(|a| a.relative_path.as_deref() == Some("B/sub/f1.txt")
            && a.action_type == SyncActionType::DeleteFromLocal));
    }
}
