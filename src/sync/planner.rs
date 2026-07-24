//! 同步规划器 —— 3-way diff（本地 vs 云端 vs DB）。
//!
//! 对齐 `legacy/lib/sync/sync_planner.dart` 的 `_decide` 决策表。
//!
//! 输入：`SyncSnapshot { local, cloud, db, cloud_tree_trusted, is_startup_resume }`
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
    /// cloud 是否来自完整分页并与 cursor 同批原子提交的可信 checkpoint。
    /// false 时“云端不存在”只是未知事实，不能驱动任一方向的删除。
    pub cloud_tree_trusted: bool,
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

/// 比较本地、云端与数据库基线，生成有序同步动作。
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
                if !snapshot.cloud_tree_trusted
                    && matches!(
                        action.action_type,
                        SyncActionType::DeleteFromLocal | SyncActionType::DeleteFromCloud
                    )
                {
                    tracing::warn!(
                        path = rel_path,
                        action = ?action.action_type,
                        "云端 checkpoint 不可信，抑制删除动作"
                    );
                    continue;
                }
                // 过滤 Skip 类型（对齐 dart plan() 的 action.type != SyncActionType.skip 过滤）
                // ★ 例外：携带 cloud_file 的 Skip 是 pending 占位项的收敛动作（上次失败实为成功），
                //   必须放行到 engine，让其 upsert 真实 fileId + 清理 pending 孤儿行。
                if action.action_type == SyncActionType::Skip && action.cloud_file.is_none() {
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
                // 会话内本地删除目录 → 同步删除云端（用户主动行为，非系统保护场景）
                if db_exists && !snap.is_startup_resume {
                    return Some(SyncAction {
                        action_type: SyncActionType::DeleteFromCloud,
                        relative_path: Some(rel_path.to_string()),
                        file_id: cloud.unwrap().id.clone().into(),
                        parent_file_id: None,
                        local_path: None,
                        cloud_file: None,
                        reason: Some("本地目录已删除 → 同步删除云端".to_string()),
                    });
                }
                // 启动恢复期 + DELETED tombstone → 跳过（不重建）
                if db_exists
                    && snap.is_startup_resume
                    && db.unwrap().status == crate::data::repository::sync_status::DELETED
                {
                    return None;
                }
                // 否则本地缺失 → 创建文件夹
                return Some(SyncAction {
                    action_type: SyncActionType::CreateFolder,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: cloud
                        .unwrap()
                        .parent_folder
                        .as_ref()
                        .and_then(|v| v.first().cloned()),
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
            // ★ pending: 占位项 + 云端已有 → 上次「失败」其实成功（如 308 误判），收敛为已同步。
            // 用 Skip 携带真实 cloud_file：engine 结算时 upsert 真实 fileId + status=SYNCED +
            // 清理 pending 孤儿行。避免重复上传（华为不查重）和 Download 覆盖（cloud_edited_time=None 误判）。
            if db
                .unwrap()
                .file_id
                .starts_with(crate::data::repository::PENDING_FILE_ID_PREFIX)
            {
                return Some(SyncAction {
                    action_type: SyncActionType::Skip,
                    relative_path: Some(rel_path.to_string()),
                    file_id: cloud.unwrap().id.clone().into(),
                    parent_file_id: None,
                    local_path: None,
                    cloud_file: Some(cloud.unwrap().clone()),
                    reason: Some(
                        "pending 占位项发现云端已有 → 收敛为已同步（上次失败实为成功）".to_string(),
                    ),
                });
            }
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
                    // Update 必须携带规划时远端版本，执行前据此拒绝覆盖并发修改。
                    cloud_file: Some(cloud.unwrap().clone()),
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
                // ★ pending: 占位项（新增上传失败 / retry 后仍未成功）→ 重新计划上传。
                // 绝不能走下面的 BackupBeforeCloudDelete / DeleteFromLocal，否则会删本地文件（数据丢失）。
                // 占位 fileId 无真实云端对应，云端无此文件是「还没传上去」而非「云端删了」。
                //
                // ★★ 但 FAILED 状态的占位项不再自动重试（避免空间不足等原因导致的无限重试循环），
                // 留给用户在传输面板手动点"重试"。仅非 FAILED（如 SYNCING 延迟）的才自动重试。
                if db
                    .unwrap()
                    .file_id
                    .starts_with(crate::data::repository::PENDING_FILE_ID_PREFIX)
                {
                    if db.unwrap().status == crate::data::repository::sync_status::FAILED {
                        // FAILED → 不自动重试，等用户手动触发（传输面板重试按钮）
                        return None;
                    }
                    return Some(SyncAction {
                        action_type: SyncActionType::Upload,
                        relative_path: Some(rel_path.to_string()),
                        file_id: None,
                        parent_file_id: None,
                        local_path: Some(
                            local.unwrap().absolute_path.to_string_lossy().to_string(),
                        ),
                        cloud_file: None,
                        reason: Some("pending 占位项（上传待重试）→ 重新上传".to_string()),
                    });
                }
                // ★★ 启动恢复期删除守卫 ★★
                // 启动恢复期 cloud_tree 可能不可信（BFS 部分失败/缓存残缺）。
                // 对"DB 有真实 fileId（非 pending:）且本地未改"的文件，绝不直接删除，
                // 改为 Skip，等下一次 BFS 成功后重新判定。
                // 仅保护本地未改的文件（本地已改的走 BackupBeforeCloudDelete，本就不删内容）。
                // 注：文件夹因 local_mtime=None → is_local_changed=true → !is_local_changed=false，不走此守卫。
                if snap.is_startup_resume && !is_local_changed(local.unwrap(), db.unwrap()) {
                    return Some(SyncAction {
                        action_type: SyncActionType::Skip,
                        relative_path: Some(rel_path.to_string()),
                        file_id: db.unwrap().file_id.clone().into(),
                        parent_file_id: None,
                        local_path: None,
                        cloud_file: None,
                        reason: Some("启动恢复期 cloud_tree 不可信，跳过删除待复核".to_string()),
                    });
                }
                // 文件夹：同样生成 DeleteFromLocal，由 engine 层判断是否需要保留
                //（若目录内有文件被 BackupBeforeCloudDelete 需要栖身之所，engine 会过滤）
                if local.unwrap().is_folder {
                    return Some(SyncAction {
                        action_type: SyncActionType::DeleteFromLocal,
                        relative_path: Some(rel_path.to_string()),
                        file_id: db.unwrap().file_id.clone().into(),
                        parent_file_id: None,
                        local_path: Some(
                            local.unwrap().absolute_path.to_string_lossy().to_string(),
                        ),
                        cloud_file: None,
                        reason: Some("云端已删除文件夹 → 同步删除本地".to_string()),
                    });
                }
                // 文件：本地有未上传的真实修改 → 改名备份副本（冲突保护），原路径腾空即满足云端删除
                if local_has_content && is_local_changed(local.unwrap(), db.unwrap()) {
                    return Some(SyncAction {
                        action_type: SyncActionType::BackupBeforeCloudDelete,
                        relative_path: Some(rel_path.to_string()),
                        file_id: db.unwrap().file_id.clone().into(),
                        parent_file_id: None,
                        local_path: Some(
                            local.unwrap().absolute_path.to_string_lossy().to_string(),
                        ),
                        cloud_file: None,
                        reason: Some("云端已删除但本地有未上传修改 → 备份副本".to_string()),
                    });
                }
                // 未改 / 占位 → 删除本地（匹配云端删除）
                tracing::debug!(
                    rel = %rel_path,
                    local_has_content,
                    is_placeholder = local.map(|e| e.is_placeholder).unwrap_or(false),
                    "云端已删除文件 → 同步删除本地占位/未改文件"
                );
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
            // 启动恢复期 / 无 DB：检查是否是用户主动删除的 tombstone
            if db_exists
                && snap.is_startup_resume
                && db.unwrap().status == crate::data::repository::sync_status::DELETED
            {
                // 用户主动删除的 tombstone → 跳过（不重建占位符）
                return Some(SyncAction {
                    action_type: SyncActionType::Skip,
                    relative_path: Some(rel_path.to_string()),
                    file_id: None,
                    parent_file_id: None,
                    local_path: None,
                    cloud_file: None,
                    reason: Some("用户已删除（tombstone）→ 跳过".to_string()),
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

        // === 本地无 + 云端无 + DB 有（双方都删了，或云端树缓存滞后）===
        // 不发 API（云端大概率已 404），由 engine 在周期末尾统一清 DB 残余。
        if !local_exists && !cloud_exists && db_exists {
            return None;
        }

        None
    }
}

impl Default for SyncPlanner {
    /// 创建无额外运行状态的默认规划器。
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

/// 规划器私有决策合同测试。
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};

    use super::{DbSnapshotEntry, SyncPlanner, SyncSnapshot};
    use crate::data::repository;
    use crate::drive::models::{DriveFile, FileCategory};
    use crate::mount::manager::LocalFileEntry;
    use crate::sync::state::SyncActionType;

    /// 构造带稳定远端版本的文件元数据。
    fn cloud_file(edited_time: i64) -> DriveFile {
        DriveFile {
            id: "cloud-file".to_string(),
            name: "MEMORY.md".to_string(),
            category: FileCategory::Document,
            size: 12,
            parent_folder: Some(vec!["root".to_string()]),
            description: None,
            created_time: None,
            edited_time: Utc.timestamp_millis_opt(edited_time).single(),
            mime_type: Some("text/markdown".to_string()),
            content_hash: None,
            thumbnail_link: None,
        }
    }

    /// 本地单边修改生成的 Update 必须携带规划时远端版本快照。
    #[test]
    fn local_update_keeps_cloud_version_snapshot() {
        let relative_path = "MEMORY.md".to_string();
        let local = LocalFileEntry {
            relative_path: relative_path.clone(),
            absolute_path: PathBuf::from("/mount/MEMORY.md"),
            size: 12,
            mtime: 2_000,
            is_folder: false,
            is_placeholder: false,
        };
        let cloud = cloud_file(3_000);
        let db = DbSnapshotEntry {
            file_id: cloud.id.clone(),
            local_mtime: Some(1_000),
            local_size: Some(local.size as i64),
            cloud_edited_time: Some(3_000),
            status: repository::sync_status::SYNCED,
            is_folder: false,
        };
        let snapshot = SyncSnapshot {
            local: HashMap::from([(relative_path.clone(), local)]),
            cloud: HashMap::from([(relative_path.clone(), cloud)]),
            db: HashMap::from([(relative_path, db)]),
            cloud_tree_trusted: true,
            is_startup_resume: false,
        };

        let actions = SyncPlanner.plan(&snapshot);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, SyncActionType::Upload);
        assert_eq!(actions[0].file_id.as_deref(), Some("cloud-file"));
        assert_eq!(
            actions[0]
                .cloud_file
                .as_ref()
                .and_then(|file| file.edited_time)
                .map(|time| time.timestamp_millis()),
            Some(3_000)
        );
    }
}
