//! 云端树 BFS 构建 + 可信 checkpoint 持久化。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart` 的 BFS 部分 + cloudtree 持久化。
//!
//! BFS 并发数 8（每个 folder 独立 fetch），失败节点重试 2 次。
//! BFS 只构建候选树，不直接替换磁盘 checkpoint。调用方必须在完整应用 Changes 后，
//! 将 tree/path map/final cursor 作为同一个 [`CloudTreeCache`] 原子提交，再安装到 live state。
//!
//! # 可信边界
//! - 刷新开始、分页失败、子树重试耗尽都不改正式 checkpoint。
//! - `complete=true`、非空 cursor、tree/path map 内部一致才可加载为 trusted。
//! - 严格完整扫描得到的空 tree 是合法的云盘状态，不能按条目数量判坏。
//! - 候选文件先写完并 fsync，随后同目录 rename，最后 fsync 父目录；调用方只在
//!   `persist_cloud_checkpoint` 成功后安装同一候选，避免 old-tree/new-cursor 撕裂。

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::core::cache_paths;
use crate::drive::files_api::FilesApi;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};
use crate::mount::manager::MountManager;

/// BFS 并发数
const INDEXING_CONCURRENCY: usize = 8;

/// 缓存 JSON 结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTreeCache {
    pub root_folder_id: Option<String>,
    pub tree: HashMap<String, DriveFile>,
    pub path_to_id: HashMap<String, String>,
    /// 与 tree/path map 同批应用完成的 Changes 末页 `newStartCursor`。
    /// 旧缓存没有该字段，因此反序列化为 None 并强制可信全量刷新。
    #[serde(default)]
    pub cursor: Option<String>,
    /// 候选是否已经完成全量/增量应用。旧缓存无此字段时默认为 false。
    #[serde(default)]
    pub complete: bool,
}

impl CloudTreeCache {
    /// 构造一个可提交的完整候选 checkpoint。
    pub fn new_trusted(
        root_folder_id: Option<String>,
        tree: HashMap<String, DriveFile>,
        mut path_to_id: HashMap<String, String>,
        cursor: String,
    ) -> AppResult<Self> {
        // 根目录本身不在 tree 中，但增量 merge 需要 rootId → "" 反查来解析根级新增项。
        if let Some(root_id) = &root_folder_id {
            path_to_id
                .entry(String::new())
                .or_insert_with(|| root_id.clone());
        }
        let checkpoint = Self {
            root_folder_id,
            tree,
            path_to_id,
            cursor: Some(cursor),
            complete: true,
        };
        checkpoint.validate_trusted()?;
        Ok(checkpoint)
    }

    /// 校验 checkpoint 是否足以作为删除决策的可信远端事实。
    pub fn validate_trusted(&self) -> AppResult<()> {
        if !self.complete {
            return Err(AppError::generic("云端 checkpoint 未完整提交"));
        }
        if !matches!(self.cursor.as_deref(), Some(cursor) if !cursor.trim().is_empty()) {
            return Err(AppError::generic("云端 checkpoint 缺少有效 cursor"));
        }

        let mut seen_ids = HashSet::with_capacity(self.tree.len());
        for (path, file) in &self.tree {
            if path.is_empty() || file.id.trim().is_empty() {
                return Err(AppError::generic("云端 checkpoint 包含空路径或空 fileId"));
            }
            if !seen_ids.insert(file.id.as_str()) {
                return Err(AppError::generic(format!(
                    "云端 checkpoint 中 fileId 重复：{}",
                    file.id
                )));
            }
            if self.path_to_id.get(path) != Some(&file.id) {
                return Err(AppError::generic(format!(
                    "云端 checkpoint 的路径索引不一致：{path}"
                )));
            }
        }

        for (path, file_id) in &self.path_to_id {
            if path.is_empty() {
                if self.root_folder_id.as_ref() != Some(file_id) {
                    return Err(AppError::generic("云端 checkpoint 的根目录索引不一致"));
                }
                continue;
            }
            if self.tree.get(path).map(|file| &file.id) != Some(file_id) {
                return Err(AppError::generic(format!(
                    "云端 checkpoint 包含孤立路径索引：{path}"
                )));
            }
        }
        Ok(())
    }
}

/// 构建云端文件树（BFS）。
/// 返回 (tree: rel_path→DriveFile, path_to_id: rel_path→file_id, root_folder_id)。
pub async fn refresh_cloud_tree(
    files_api: &Arc<FilesApi>,
    _mount: &Option<Arc<MountManager>>,
    _abs_mount_dir: &str,
) -> AppResult<(
    HashMap<String, DriveFile>,
    HashMap<String, String>,
    Option<String>,
)> {
    let mut tree: HashMap<String, DriveFile> = HashMap::new();
    let mut path_to_id: HashMap<String, String> = HashMap::new();
    let mut root_folder_id: Option<String> = None;
    let mut visited: HashSet<String> = HashSet::new();

    // 根目录入队
    let mut queue: VecDeque<BfsNode> = VecDeque::new();
    queue.push_back(BfsNode {
        folder_id: None,
        path: String::new(),
        retries: 0,
    });

    let mut processed_folders: usize = 0;

    tracing::info!("开始 BFS 云端树构建");

    while !queue.is_empty() {
        let batch_size = std::cmp::min(INDEXING_CONCURRENCY, queue.len());
        let batch: Vec<BfsNode> = queue.drain(..batch_size).collect();
        let futures: Vec<_> = batch
            .iter()
            .map(|node| {
                let api = files_api.clone();
                let node = node.clone();
                async move {
                    let parent_id = if node.path.is_empty() {
                        None
                    } else {
                        node.folder_id.as_deref()
                    };
                    match api.list_all(parent_id).await {
                        Ok(files) => Ok((node, files)),
                        Err(e) => Err((node, e)),
                    }
                }
            })
            .collect();

        let results = join_all(futures).await;

        for result in results {
            match result {
                Ok((node, files)) => {
                    // 根目录第一层：动态发现 root folder ID
                    if node.path.is_empty() && root_folder_id.is_none() {
                        root_folder_id = detect_root_folder_id(&files);
                    }

                    for f in &files {
                        // 跳过 .hwcloud_ 前缀内部文件
                        if f.name.starts_with(crate::constants::INTERNAL_FILE_PREFIX) {
                            continue;
                        }
                        crate::core::paths::validate_path_segment(&f.name)?;
                        let rel_path = if node.path.is_empty() {
                            f.name.clone()
                        } else {
                            format!("{}/{}", node.path, f.name)
                        };
                        tree.insert(rel_path.clone(), f.clone());
                        path_to_id.insert(rel_path.clone(), f.id.clone());

                        // 候选扫描必须无本地副作用。目录/占位符只能由可信 checkpoint
                        // 安装后的 planner/executor 创建，否则扫描失败或 replay 删除会在
                        // 本地留下半棵树，并把已删除的远端对象反向上传复活。

                        if f.is_folder() && !visited.contains(&f.id) {
                            visited.insert(f.id.clone());
                            queue.push_back(BfsNode {
                                folder_id: Some(f.id.clone()),
                                path: rel_path,
                                retries: 0,
                            });
                        }
                    }
                }
                Err((node, e)) => {
                    if node.retries < 2 {
                        tracing::warn!(path = %node.path, retries = node.retries, "BFS 单文件夹失败，重试");
                        queue.push_back(BfsNode {
                            retries: node.retries + 1,
                            ..node
                        });
                    } else {
                        tracing::error!(path = %node.path, error = %e, "BFS 文件夹永久失败（子树将缺失）");
                        return Err(AppError::generic(format!(
                            "云端树刷新不完整：目录 {} 重试耗尽：{e}",
                            if node.path.is_empty() {
                                "/"
                            } else {
                                node.path.as_str()
                            }
                        )));
                    }
                }
            }
        }

        processed_folders += batch_size;
        // 对齐 dart _refreshCloudTree：每 5 个目录或队列耗尽时输出进度
        if processed_folders % 5 == 0 || queue.is_empty() {
            tracing::info!(
                scanned = processed_folders,
                items = tree.len(),
                pending = queue.len(),
                "云端刷新进度：已扫描 {} 个目录，累计 {} 项，队列剩余 {}",
                processed_folders,
                tree.len(),
                queue.len(),
            );
        }
    }

    tracing::info!(
        files = tree.len(),
        folders = processed_folders,
        "云端全量刷新完成：{} 项（{} 个目录）",
        tree.len(),
        processed_folders,
    );

    // 此处只返回完整候选。调用方必须先从扫描前 cursor 重放 Changes，再把最终 cursor
    // 与候选 tree/path map 一次性持久化，成功后才允许替换 live state。
    Ok((tree, path_to_id, root_folder_id))
}

/// BFS 节点
#[derive(Debug, Clone)]
struct BfsNode {
    folder_id: Option<String>,
    path: String,
    retries: u32,
}

/// 动态发现根目录的真实 folder ID。
/// 取根级条目 parentFolder 中唯一的最高频值；最高频并列则 fail closed。
fn detect_root_folder_id(files: &[DriveFile]) -> Option<String> {
    let mut counter: HashMap<String, usize> = HashMap::new();
    for f in files {
        if let Some(pf) = &f.parent_folder {
            for id in pf {
                *counter.entry(id.clone()).or_default() += 1;
            }
        }
    }
    let mut candidates: Vec<(String, usize)> = counter.into_iter().collect();
    candidates
        .sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (root_id, max_count) = candidates.first()?;
    if matches!(candidates.get(1), Some((_, count)) if count == max_count) {
        return None;
    }
    Some(root_id.clone())
}

/// 加载可信云端 checkpoint。不存在、旧格式或内部不一致 → None。
///
/// 条目数量不是可信条件：严格完成的空云盘是合法 checkpoint。旧版独立 cursor 文件
/// 不会在这里拼接进旧 tree；缺少同版本 cursor 的缓存必须先做可信全量刷新。
pub fn load_persisted_cloud_tree(abs_mount_dir: &str) -> Option<CloudTreeCache> {
    let cache_file = cache_paths::cloud_tree_cache_file(abs_mount_dir).ok()?;
    if !cache_file.exists() {
        // 尝试旧版迁移
        crate::core::cache_paths::migrate_legacy_cache(abs_mount_dir);
    }
    let file = cache_file;
    if !file.exists() {
        return None;
    }
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::warn!(%error, "读取云端 checkpoint 失败，将全量刷新");
            return None;
        }
    };
    let cache: CloudTreeCache = match serde_json::from_str(&raw) {
        Ok(cache) => cache,
        Err(error) => {
            tracing::warn!(%error, "解析云端 checkpoint 失败，将全量刷新");
            return None;
        }
    };
    if let Err(error) = cache.validate_trusted() {
        tracing::warn!(%error, "云端 checkpoint 不可信，将全量刷新");
        return None;
    }
    tracing::info!(files = cache.tree.len(), "从缓存加载可信云端 checkpoint");
    Some(cache)
}

/// 原子提交完整可信 checkpoint。
///
/// 候选先在同目录临时文件完整写入并 fsync，之后才 rename 覆盖正式文件并 fsync
/// 父目录。rename 之前的任何失败都不会修改上一份正式 checkpoint。调用方必须把
/// 此函数的 `Ok(())` 当作安装 live tree/path/cursor 的唯一提交门槛。
pub fn persist_cloud_checkpoint(abs_mount_dir: &str, checkpoint: &CloudTreeCache) -> AppResult<()> {
    checkpoint.validate_trusted()?;
    let cache_file = cache_paths::cloud_tree_cache_file(abs_mount_dir)?;
    let parent = cache_file
        .parent()
        .ok_or_else(|| AppError::generic("云端 checkpoint 路径缺少父目录"))?;
    std::fs::create_dir_all(parent)?;

    let json = serde_json::to_vec_pretty(checkpoint)?;
    let tmp_file = cache_file.with_extension("json.tmp");
    let backup_file = cache_file.with_extension("json.bak");
    if backup_file.exists() {
        std::fs::remove_file(&backup_file)?;
    }
    {
        let mut candidate = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_file)?;
        candidate.write_all(&json)?;
        candidate.sync_all()?;
    }

    // 保留旧 inode，便于 rename 后父目录 fsync 失败时恢复旧正式文件。
    // backup 只用于本次提交回滚，loader 永远只读取正式 checkpoint。
    let had_previous = cache_file.exists();
    if had_previous {
        std::fs::hard_link(&cache_file, &backup_file)?;
        sync_parent_directory(parent)?;
    }

    if let Err(error) = std::fs::rename(&tmp_file, &cache_file) {
        let _ = std::fs::remove_file(&backup_file);
        return Err(error.into());
    }
    if let Err(error) = sync_parent_directory(parent) {
        let rollback = if had_previous {
            std::fs::rename(&backup_file, &cache_file)
        } else {
            std::fs::remove_file(&cache_file)
        };
        if let Err(rollback_error) = rollback {
            tracing::error!(%rollback_error, "云端 checkpoint 提交失败且旧版本回滚失败");
        } else {
            let _ = sync_parent_directory(parent);
        }
        return Err(error);
    }
    if had_previous {
        let _ = std::fs::remove_file(&backup_file);
    }
    tracing::info!(files = checkpoint.tree.len(), "可信云端 checkpoint 已提交");
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &std::path::Path) -> AppResult<()> {
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &std::path::Path) -> AppResult<()> {
    // Windows 不支持以普通 File 打开目录；同目录 rename 仍保证不会暴露半截 JSON。
    Ok(())
}

/// 兼容旧 shutdown 调用：只清理未提交候选，不再破坏最后可信 checkpoint。
///
/// 新模型下刷新从不把 partial state 写进正式文件，所以正常退出时把正式 checkpoint
/// 改成 `complete=false` 反而会丢失最后可信基线。未提交 `.tmp` 永远不会被 loader
/// 读取；退出时尽力清理即可。
pub fn mark_cache_incomplete_if_exists() {
    let Ok(config) = crate::core::config_store::ConfigStore::load() else {
        return;
    };
    let abs_dir = config.expanded_mount_dir().to_string_lossy().to_string();
    let Ok(cache_file) = cache_paths::cloud_tree_cache_file(&abs_dir) else {
        return;
    };
    let tmp_file = cache_file.with_extension("json.tmp");
    if tmp_file.exists() {
        let _ = std::fs::remove_file(tmp_file);
    }
    let backup_file = cache_file.with_extension("json.bak");
    if backup_file.exists() {
        let _ = std::fs::remove_file(backup_file);
    }
}

impl Default for DriveFile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            category: crate::drive::models::FileCategory::None,
            size: 0,
            parent_folder: None,
            description: None,
            created_time: None,
            edited_time: None,
            mime_type: None,
            content_hash: None,
            thumbnail_link: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== 索引完整性守卫测试（load_persisted_cloud_tree 严格校验）=====

    /// 构造一个非空 DriveFile 供缓存填充
    fn sample_file() -> DriveFile {
        use crate::drive::models::FileCategory;
        DriveFile {
            id: "f1".into(),
            name: "学习".into(),
            category: FileCategory::Folder,
            ..Default::default()
        }
    }

    /// 把给定 CloudTreeCache 写入临时挂载目录的缓存文件路径，供 load 测试。
    fn write_cache_raw(abs_mount_dir: &str, json: &str) {
        let cache_file = cache_paths::cloud_tree_cache_file(abs_mount_dir).unwrap();
        std::fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        std::fs::write(&cache_file, json).unwrap();
    }

    /// complete=false（哨兵/中断）→ load 应返回 None（强制全量重跑）
    #[test]
    fn test_load_rejects_incomplete_cache() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let mut tree = HashMap::new();
        tree.insert("学习".into(), sample_file());
        let cache = CloudTreeCache {
            root_folder_id: Some("root".into()),
            tree,
            path_to_id: HashMap::new(),
            cursor: Some("c1".into()),
            complete: false,
        };
        write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
        assert!(
            load_persisted_cloud_tree(&abs).is_none(),
            "complete=false 的缓存必须被拒绝，否则 startup 会拿残缺缓存触发文件同步"
        );
    }

    /// 严格完成的空云盘是合法可信 checkpoint。
    #[test]
    fn test_load_accepts_complete_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let cache = CloudTreeCache {
            root_folder_id: Some("root".into()),
            tree: HashMap::new(),
            path_to_id: HashMap::new(),
            cursor: Some("c-empty".into()),
            complete: true,
        };
        write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
        assert!(
            load_persisted_cloud_tree(&abs).is_some(),
            "完整空盘必须可作为可信 checkpoint"
        );
    }

    /// complete=true 且 tree 非空 → load 应返回 Some
    #[test]
    fn test_load_accepts_complete_cache() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let mut tree = HashMap::new();
        tree.insert("学习".into(), sample_file());
        let mut path_to_id = HashMap::new();
        path_to_id.insert("学习".into(), "f1".into());
        let cache = CloudTreeCache {
            root_folder_id: Some("root".into()),
            tree,
            path_to_id,
            cursor: Some("c1".into()),
            complete: true,
        };
        write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
        let loaded = load_persisted_cloud_tree(&abs);
        assert!(loaded.is_some(), "完整缓存应被接受");
        assert_eq!(loaded.unwrap().tree.len(), 1);
    }

    /// 旧版缓存无 complete 字段 → serde default false → 视为不完整 → 返回 None
    /// （安全升级：旧缓存强制重跑一次 BFS，之后自然带上新字段）
    #[test]
    fn test_old_cache_without_complete_field_treated_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        // 手写无 complete 字段的旧格式 JSON
        let old_json = r#"{
            "root_folder_id": "root",
            "tree": {"学习": {"id": "f1", "name": "学习"}},
            "path_to_id": {"学习": "f1"}
        }"#;
        write_cache_raw(&abs, old_json);
        assert!(
            load_persisted_cloud_tree(&abs).is_none(),
            "旧格式缓存（无 complete 字段）应被视为不完整，强制重跑 BFS"
        );
    }

    /// persist_cloud_checkpoint 原子写：写入后文件存在且可被 load 正确读回，
    /// 且无残留 .tmp 文件。
    #[test]
    fn test_persist_internal_atomic_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let mut tree = HashMap::new();
        tree.insert("学习".into(), sample_file());
        let mut p2i = HashMap::new();
        p2i.insert("学习".into(), "f1".into());
        let checkpoint =
            CloudTreeCache::new_trusted(Some("root".into()), tree, p2i, "c1".into()).unwrap();
        persist_cloud_checkpoint(&abs, &checkpoint).unwrap();

        let cache_file = cache_paths::cloud_tree_cache_file(&abs).unwrap();
        assert!(cache_file.exists(), "缓存文件应存在");
        // 无残留 .tmp
        let tmp = cache_file.with_extension("json.tmp");
        assert!(!tmp.exists(), "原子写后不应残留 .tmp 文件");
        // 可被 load 读回
        let loaded = load_persisted_cloud_tree(&abs);
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().root_folder_id.as_deref(), Some("root"));
    }

    #[test]
    fn test_detect_root_folder_id_most_common() {
        use crate::drive::models::FileCategory;
        let files = vec![
            DriveFile {
                id: "f1".into(),
                name: "a".into(),
                category: FileCategory::Folder,
                size: 0,
                parent_folder: Some(vec!["root-real-123".into()]),
                ..Default::default()
            },
            DriveFile {
                id: "f2".into(),
                name: "b".into(),
                category: FileCategory::None,
                size: 100,
                parent_folder: Some(vec!["root-real-123".into()]),
                ..Default::default()
            },
            DriveFile {
                id: "f3".into(),
                name: "c".into(),
                category: FileCategory::None,
                size: 50,
                parent_folder: Some(vec!["other-id".into()]),
                ..Default::default()
            },
        ];
        let root = detect_root_folder_id(&files);
        // root-real-123 出现 2 次，other-id 仅 1 次 → root-real-123 当选
        assert_eq!(root.as_deref(), Some("root-real-123"));
    }

    #[test]
    fn test_detect_root_folder_id_no_consensus() {
        use crate::drive::models::FileCategory;
        let files = vec![
            DriveFile {
                id: "f1".into(),
                name: "a".into(),
                category: FileCategory::None,
                size: 0,
                parent_folder: Some(vec!["id-a".into()]),
                ..Default::default()
            },
            DriveFile {
                id: "f2".into(),
                name: "b".into(),
                category: FileCategory::None,
                size: 0,
                parent_folder: Some(vec!["id-b".into()]),
                ..Default::default()
            },
        ];
        let root = detect_root_folder_id(&files);
        // 都只出现 1 次 → None
        assert!(root.is_none());
    }
}
