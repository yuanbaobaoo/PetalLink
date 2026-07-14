//! 云树缓存、可信 checkpoint 与全量/增量刷新。
//!
//! 只有完整持久化并安装的 checkpoint 才能驱动破坏性决策；刷新失败时保持不可信。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::data::repository;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::sync::cloud_tree;
use crate::sync::task_runner::RecoveredCloudFile;

use super::action_filters::is_blocked_path_identity;
use super::{SyncEngine, INCREMENTAL_FORCED_FULL_THRESHOLD};

impl SyncEngine {
    /// 将任务恢复确认的远端写入合并并原子持久化到当前可信检查点。
    pub(super) fn commit_recovered_cloud_files(
        &self,
        recovered: &[RecoveredCloudFile],
    ) -> AppResult<()> {
        if recovered.is_empty() {
            return Ok(());
        }
        if !self.cloud_tree_is_trusted() {
            return Err(AppError::generic("云端检查点不可信，拒绝发布恢复任务结果"));
        }

        let mut tree = self.cloud_tree.lock().clone();
        let mut path_to_id = self.path_to_id.lock().clone();
        for recovered_file in recovered {
            let stale_paths = tree
                .iter()
                .filter(|(path, file)| {
                    path.as_str() != recovered_file.relative_path
                        && file.id == recovered_file.file.id
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            for stale_path in stale_paths {
                tree.remove(&stale_path);
                path_to_id.remove(&stale_path);
            }
            tree.insert(
                recovered_file.relative_path.clone(),
                recovered_file.file.clone(),
            );
            path_to_id.insert(
                recovered_file.relative_path.clone(),
                recovered_file.file.id.clone(),
            );
        }

        let cursor = self
            .cloud_cursor
            .lock()
            .clone()
            .filter(|cursor| !cursor.trim().is_empty())
            .ok_or_else(|| AppError::generic("可信云端检查点缺少 cursor"))?;
        let checkpoint = cloud_tree::CloudTreeCache::new_trusted(
            self.root_folder_id.lock().clone(),
            tree,
            path_to_id,
            cursor,
        )?;
        let mount_dir = self
            .mount_dir
            .lock()
            .clone()
            .ok_or_else(|| AppError::generic("同步挂载尚未初始化，无法持久化云端检查点"))?;
        let absolute_mount = crate::core::paths::expand_tilde(&mount_dir);
        if let Err(error) = cloud_tree::persist_cloud_checkpoint(&absolute_mount, &checkpoint) {
            self.set_cloud_tree_trusted(false);
            return Err(error);
        }
        self.install_cloud_checkpoint(checkpoint);
        tracing::info!(
            recovered = recovered.len(),
            "已将恢复任务的权威远端结果提交到云端检查点"
        );
        Ok(())
    }

    /// 向云树插入条目。
    pub fn cloud_tree_insert(&self, rel: String, file: DriveFile) {
        self.cloud_tree.lock().insert(rel, file);
    }

    /// 向路径索引插入条目。
    pub fn path_to_id_insert(&self, rel: String, id: String) {
        self.path_to_id.lock().insert(rel, id);
    }

    /// 从云树移除条目。
    pub fn cloud_tree_remove(&self, rel: &str) {
        self.cloud_tree.lock().remove(rel);
    }

    /// 从路径索引移除条目。
    pub fn path_to_id_remove(&self, rel: &str) {
        self.path_to_id.lock().remove(rel);
    }

    /// 记录近期删除路径，抑制监听器振荡。
    pub fn add_recently_deleted(&self, rel: &str) {
        self.recently_deleted_paths
            .lock()
            .insert(rel.to_string(), chrono::Utc::now().timestamp_millis());
    }

    /// 获取云树写锁。
    pub fn cloud_tree_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, DriveFile>> {
        self.cloud_tree.lock()
    }

    /// 返回云树 checkpoint 是否可信。
    pub(crate) fn cloud_tree_is_trusted(&self) -> bool {
        self.cloud_tree_trusted.load(Ordering::Acquire)
    }

    /// 更新云树 checkpoint 信任状态。
    fn set_cloud_tree_trusted(&self, trusted: bool) {
        self.cloud_tree_trusted.store(trusted, Ordering::Release);
    }

    /// 按固定顺序安装完整 checkpoint。
    fn install_cloud_checkpoint(&self, checkpoint: cloud_tree::CloudTreeCache) {
        self.set_cloud_tree_trusted(false);
        *self.cloud_tree.lock() = checkpoint.tree;
        *self.path_to_id.lock() = checkpoint.path_to_id;
        *self.root_folder_id.lock() = checkpoint.root_folder_id;
        *self.cloud_cursor.lock() = checkpoint.cursor;
        self.set_cloud_tree_trusted(true);
    }

    /// 获取路径索引写锁。
    pub fn path_to_id_lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, String>> {
        self.path_to_id.lock()
    }

    /// 返回云盘根目录 ID。
    pub(crate) fn root_folder_id(&self) -> Option<String> {
        self.root_folder_id.lock().clone()
    }

    /// 加载 checkpoint：返回 true 表示仍需增量 catch-up；返回 false 表示已构建、重放并提交全量 checkpoint。
    pub(super) async fn load_or_refresh_cloud_tree(&self, mount_dir: &str) -> AppResult<bool> {
        let _activity = self.begin_external_activity()?;
        let abs_dir = crate::core::paths::expand_tilde(mount_dir);
        let loaded_from_cache = if let Some(cache) = cloud_tree::load_persisted_cloud_tree(&abs_dir)
        {
            self.install_cloud_checkpoint(cache);
            // 持久化 checkpoint 可供增量重放，但 catch-up 完成前不能驱动破坏性决策。
            self.set_cloud_tree_trusted(false);
            true
        } else {
            self.set_cloud_tree_trusted(false);
            self.update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = true;
                runtime.sync_phase = Some("indexing-startup".to_string());
            })?;
            let refresh_result = self.build_and_commit_full_checkpoint(&abs_dir).await;
            let reset_result = self.update_runtime_and_broadcast(|runtime| {
                runtime.is_indexing = false;
                runtime.sync_phase = None;
            });
            if refresh_result.is_err() {
                self.set_cloud_tree_trusted(false);
                self.restore_idle_runtime_after_error();
            }
            refresh_result?;
            reset_result?;
            false
        };
        Ok(loaded_from_cache)
    }

    /// 仅在云树可信时清理已删除墓碑。
    pub(super) fn purge_deleted_tombstones_if_trusted(
        &self,
        blocked_changes: &[crate::sync::path_recovery::BlockedPathChange],
    ) -> AppResult<()> {
        if !self.cloud_tree_is_trusted() {
            return Err(AppError::generic("云端树尚未 catch-up，拒绝清理墓碑"));
        }
        let conn = self.db.lock();
        let cloud = self.cloud_tree.lock();
        let to_purge: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT local_path, file_id FROM sync_items WHERE status=?1")
                .map_err(|error| AppError::generic(format!("查询墓碑失败：{error}")))?;
            let rows = statement
                .query_map(rusqlite::params![repository::sync_status::DELETED], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| AppError::generic(format!("读取墓碑失败：{error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::generic(format!("读取墓碑失败：{error}")))?
                .into_iter()
                .filter(|(path, file_id)| {
                    !is_blocked_path_identity(Some(path), Some(file_id), blocked_changes)
                })
                .filter(|(path, _)| !cloud.contains_key(path))
                .map(|(path, _)| path)
                .collect()
        };
        drop(cloud);
        for path in &to_purge {
            conn.execute(
                "DELETE FROM sync_items WHERE local_path=?1 AND status=?2",
                rusqlite::params![path, repository::sync_status::DELETED],
            )
            .map_err(|error| AppError::generic(format!("清理墓碑失败：{error}")))?;
        }
        if !to_purge.is_empty() {
            tracing::info!(count = to_purge.len(), "已清理可信云树中不存在的墓碑");
        }
        Ok(())
    }

    /// 设置当前同步阶段并广播。
    fn set_phase(&self, phase: &str) -> AppResult<()> {
        self.update_runtime_and_broadcast(|runtime| {
            runtime.sync_phase = Some(phase.to_string());
        })?;
        Ok(())
    }

    /// 构建、重放并原子安装全量 checkpoint。
    async fn build_and_commit_full_checkpoint(&self, abs_dir: &str) -> AppResult<()> {
        let result = async {
            self.ensure_cycle_active()?;
            let start_cursor = self.start_cursor_source.get_start_cursor().await?;
            self.ensure_cycle_active()?;
            let (mut tree, mut path_to_id, root_folder_id) =
                cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, abs_dir).await?;
            self.ensure_cycle_active()?;
            let (changes, final_cursor) = self.changes_api.list_all_changes(&start_cursor).await?;
            Self::apply_changes_to_candidate(
                &mut tree,
                &mut path_to_id,
                root_folder_id.as_deref(),
                &changes,
            )?;
            let checkpoint = cloud_tree::CloudTreeCache::new_trusted(
                root_folder_id,
                tree,
                path_to_id,
                final_cursor,
            )?;
            self.ensure_cycle_active()?;
            cloud_tree::persist_cloud_checkpoint(abs_dir, &checkpoint)?;
            self.ensure_cycle_active()?;
            self.install_cloud_checkpoint(checkpoint);
            if let Ok(legacy_cursor) = crate::core::cache_paths::changes_cursor_file(abs_dir) {
                let _ = std::fs::remove_file(legacy_cursor);
            }
            self.incremental_since_full.store(0, Ordering::Relaxed);
            Ok(())
        }
        .await;
        if result.is_err() {
            // 失败时保留旧树用于非破坏性展示，同时撤销删除与清理所需的信任。
            self.set_cloud_tree_trusted(false);
        }
        result
    }

    /// 在同步周期中执行全量云树刷新。
    pub(super) async fn refresh_cloud_full_for_cycle(&self) -> AppResult<()> {
        let _activity = self.begin_external_activity()?;
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = crate::core::paths::expand_tilde(&mount_dir);

        // 刷新期间显式发布索引状态。
        self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = true;
            runtime.sync_phase = Some("indexing-manual".to_string());
        })?;
        // 无论成功失败都复位索引状态。
        let refresh_result = self.build_and_commit_full_checkpoint(&abs_dir).await;
        self.ensure_cycle_active()?;
        let reset_result = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = false;
            runtime.sync_phase = None;
        });
        match refresh_result {
            Ok(()) => {
                reset_result?;
            }
            Err(error) => {
                let _ = reset_result;
                return Err(error);
            }
        }
        Ok(())
    }

    /// 运行容错的自动云树刷新。
    pub(super) async fn run_auto_cloud_refresh(self: &Arc<Self>) {
        let result = self.run_sync_cycle("auto-cloud-refresh").await;
        (self.cycle_observer)("auto-cycle-returned");
        if let Err(e) = result {
            tracing::warn!(error = %e, "自动云端刷新失败（忽略，下次定时重试）");
        }
    }

    /// 在同步周期中优先执行增量云树刷新。
    pub(super) async fn refresh_cloud_incremental_for_cycle(&self) -> AppResult<()> {
        let _activity = self.begin_external_activity()?;
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = crate::core::paths::expand_tilde(&mount_dir);

        // 具体阶段由增量/全量选择逻辑设置。
        self.update_runtime_and_broadcast(|runtime| runtime.is_indexing = true)?;

        // 增量失败或缺少 cursor 时回退全量刷新。
        let refresh_result = self.try_incremental_or_full_refresh(&abs_dir).await;

        // 无论成败都复位索引状态并通知前端刷新内容。
        let reset_result = self.update_runtime_and_broadcast(|runtime| {
            runtime.is_indexing = false;
            runtime.sync_phase = None;
            runtime.content_changed = true;
        });

        match refresh_result {
            Ok(()) => {
                reset_result?;
            }
            Err(error) => {
                let _ = reset_result;
                return Err(error);
            }
        }
        self.set_phase("syncing-auto-incremental")?;
        Ok(())
    }

    /// 优先增量刷新，必要时 fail-closed 回退全量刷新。
    async fn try_incremental_or_full_refresh(&self, abs_dir: &str) -> AppResult<()> {
        let saved_cursor = self.cloud_cursor.lock().clone();
        let consecutive = self.incremental_since_full.load(Ordering::Relaxed);
        let force_full = consecutive >= INCREMENTAL_FORCED_FULL_THRESHOLD;
        if force_full {
            tracing::info!(
                consecutive,
                threshold = INCREMENTAL_FORCED_FULL_THRESHOLD,
                "连续增量达阈值，强制全量 BFS 纠偏"
            );
        }

        // 已校验 checkpoint 可用于增量重放，但 catch-up 前仍保持不可信。
        if !force_full {
            if let Some(cursor) = saved_cursor.filter(|cursor| !cursor.trim().is_empty()) {
                self.set_phase("querying-changes")?;
                let incremental = async {
                    let (changes, final_cursor) =
                        self.changes_api.list_all_changes(&cursor).await?;
                    self.ensure_cycle_active()?;
                    let mut tree = self.cloud_tree.lock().clone();
                    let mut path_to_id = self.path_to_id.lock().clone();
                    let root_folder_id = self.root_folder_id.lock().clone();
                    Self::apply_changes_to_candidate(
                        &mut tree,
                        &mut path_to_id,
                        root_folder_id.as_deref(),
                        &changes,
                    )?;
                    let checkpoint = cloud_tree::CloudTreeCache::new_trusted(
                        root_folder_id,
                        tree,
                        path_to_id,
                        final_cursor,
                    )?;
                    self.ensure_cycle_active()?;
                    cloud_tree::persist_cloud_checkpoint(abs_dir, &checkpoint)?;
                    self.ensure_cycle_active()?;
                    self.install_cloud_checkpoint(checkpoint);
                    self.incremental_since_full.fetch_add(1, Ordering::Relaxed);
                    Ok::<(), AppError>(())
                }
                .await;
                match incremental {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        self.set_cloud_tree_trusted(false);
                        tracing::warn!(%error, "增量 checkpoint 失败，保留旧盘并回退可信全量刷新");
                    }
                }
            }
        }

        self.set_phase("indexing-auto-full")?;
        self.build_and_commit_full_checkpoint(abs_dir).await
    }

    /// 将 changes 批量应用到候选树；无法安全解析时直接失败。
    fn apply_changes_to_candidate(
        tree: &mut HashMap<String, DriveFile>,
        path_to_id: &mut HashMap<String, String>,
        root_folder_id: Option<&str>,
        changes: &[crate::drive::changes_api::Change],
    ) -> AppResult<()> {
        use crate::drive::changes_api::ChangeKind;
        let mut id_to_path: HashMap<String, String> = path_to_id
            .iter()
            .map(|(path, id)| (id.clone(), path.clone()))
            .collect();
        if let Some(root_id) = root_folder_id.filter(|id| !id.trim().is_empty()) {
            id_to_path.insert(root_id.to_string(), String::new());
        }

        for change in changes {
            match change.kind {
                ChangeKind::Removed => {
                    let Some(relative_path) = id_to_path.get(change.file_id()).cloned() else {
                        // 已从候选树删除的墓碑按幂等空操作处理。
                        continue;
                    };
                    if relative_path.is_empty() {
                        return Err(AppError::generic("Changes 试图删除云盘根目录"));
                    }

                    let prefix = format!("{relative_path}/");
                    let removed_paths: Vec<String> = tree
                        .keys()
                        .filter(|path| *path == &relative_path || path.starts_with(&prefix))
                        .cloned()
                        .collect();
                    for path in removed_paths {
                        tree.remove(&path);
                        if let Some(id) = path_to_id.remove(&path) {
                            id_to_path.remove(&id);
                        }
                    }
                }
                ChangeKind::Modified => {
                    let file = change.file().ok_or_else(|| {
                        AppError::generic(format!(
                            "非删除 Change 缺少完整文件：{}",
                            change.file_id()
                        ))
                    })?;
                    crate::core::paths::validate_path_segment(&file.name)?;
                    let parents = file.parent_folder.as_ref().ok_or_else(|| {
                        AppError::generic(format!("Change {} 缺少 parentFolder", change.file_id()))
                    })?;
                    if parents.len() != 1 || parents[0].trim().is_empty() {
                        return Err(AppError::generic(format!(
                            "Change {} 的多父目录/空父目录语义不受支持",
                            change.file_id()
                        )));
                    }
                    let parent_id = &parents[0];
                    if parent_id == change.file_id() {
                        return Err(AppError::generic("Change 的 parentFolder 指向自身"));
                    }
                    let parent_path = id_to_path.get(parent_id).cloned().ok_or_else(|| {
                        AppError::generic(format!(
                            "Change {} 的 parentFolder {} 无法映射到可信路径",
                            change.file_id(),
                            parent_id
                        ))
                    })?;
                    let desired_path = if parent_path.is_empty() {
                        file.name.clone()
                    } else {
                        format!("{parent_path}/{}", file.name)
                    };

                    if let Some(existing_path) = id_to_path.get(change.file_id()).cloned() {
                        if existing_path.is_empty() {
                            return Err(AppError::generic("Changes 不支持修改云盘根目录"));
                        }
                        if existing_path != desired_path {
                            if desired_path.starts_with(&format!("{existing_path}/")) {
                                return Err(AppError::generic("Change 试图把目录移动到自身子树"));
                            }
                            Self::rekey_candidate_subtree(
                                tree,
                                path_to_id,
                                &mut id_to_path,
                                &existing_path,
                                &desired_path,
                            )?;
                        }
                    } else if let Some(existing_id) = path_to_id.get(&desired_path) {
                        if existing_id != change.file_id() {
                            return Err(AppError::generic(format!(
                                "Change 目标路径冲突：{desired_path}"
                            )));
                        }
                    };

                    tree.insert(desired_path.clone(), file.clone());
                    path_to_id.insert(desired_path.clone(), change.file_id().to_string());
                    id_to_path.insert(change.file_id().to_string(), desired_path);
                }
            }
        }
        Ok(())
    }

    /// 在候选树及双向索引中原子重键子树。
    fn rekey_candidate_subtree(
        tree: &mut HashMap<String, DriveFile>,
        path_to_id: &mut HashMap<String, String>,
        id_to_path: &mut HashMap<String, String>,
        old_root: &str,
        new_root: &str,
    ) -> AppResult<()> {
        let old_prefix = format!("{old_root}/");
        let moved_paths: Vec<String> = tree
            .keys()
            .filter(|path| path.as_str() == old_root || path.starts_with(&old_prefix))
            .cloned()
            .collect();
        if moved_paths.is_empty() {
            return Err(AppError::generic(format!(
                "Change 引用的旧路径不在候选树：{old_root}"
            )));
        }
        let moved_set: std::collections::HashSet<&str> =
            moved_paths.iter().map(String::as_str).collect();
        let targets: Vec<(String, String)> = moved_paths
            .iter()
            .map(|old_path| {
                let suffix = old_path.strip_prefix(old_root).unwrap_or_default();
                (old_path.clone(), format!("{new_root}{suffix}"))
            })
            .collect();
        for (_, target) in &targets {
            if tree.contains_key(target) && !moved_set.contains(target.as_str()) {
                return Err(AppError::generic(format!(
                    "Change 移动/改名目标路径已存在：{target}"
                )));
            }
        }

        let mut moved = Vec::with_capacity(targets.len());
        for (old_path, new_path) in targets {
            let file = tree
                .remove(&old_path)
                .ok_or_else(|| AppError::generic(format!("候选树移动时路径消失：{old_path}")))?;
            let file_id = path_to_id.remove(&old_path).ok_or_else(|| {
                AppError::generic(format!("候选路径索引移动时路径消失：{old_path}"))
            })?;
            id_to_path.remove(&file_id);
            moved.push((new_path, file_id, file));
        }
        for (new_path, file_id, file) in moved {
            id_to_path.insert(file_id.clone(), new_path.clone());
            path_to_id.insert(new_path.clone(), file_id);
            tree.insert(new_path, file);
        }
        Ok(())
    }
}
