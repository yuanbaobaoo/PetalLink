//! 云端树 BFS 构建 + 缓存持久化。
//!
//! 对齐 `legacy/lib/sync/sync_engine.dart` 的 BFS 部分 + cloudtree 持久化。
//!
//! BFS 并发数 8（每个 folder 独立 fetch），失败节点重试 2 次。
//! 完成后写入 `<Application Support>/cloudtree_<escaped>.json` 缓存。
//!
//! # 索引完整性守卫
//! BFS 全量扫描可能耗时几十秒。若中途被强退（关机/注销/Ctrl-C），缓存写盘是
//! all-or-nothing 且只在成功结尾执行 → 新缓存丢失，留下旧/空缓存。若 startup
//! 盲信这份缓存跳过 BFS，就会拿残缺 `cloud_tree` 跑 `run_sync_cycle` → planner
//! 把全部本地文件误判为「本地新增」→ 疯狂上传/建目录（见 commit 历史）。
//!
//! 守卫机制（用明确标记而非启发式判断）：
//! - `CloudTreeCache.complete: bool`：仅当 BFS 成功跑完才置 true。
//! - BFS 入口先写一份 `complete:false` 哨兵（原子写）。中途被杀 → 哨兵留在盘上。
//! - shutdown flush 再补一道把缓存标记为不完整（双保险，覆盖 BFS 尚未开始的窗口）。
//! - `load_persisted_cloud_tree` 见到 `complete:false` / 空 tree → 返回 None →
//!   engine 强制全量重跑 BFS，绝不进文件同步路径。
//!   旧缓存无 `complete` 字段 → `#[serde(default)]` 得 false → 视为不完整 → 重跑一次后自然带上新字段。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::core::cache_paths;
use crate::drive::files_api::FilesApi;
use crate::drive::models::DriveFile;
use crate::error::AppResult;
use crate::mount::manager::MountManager;

/// BFS 并发数
const INDEXING_CONCURRENCY: usize = 8;

/// 缓存 JSON 结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudTreeCache {
    pub root_folder_id: Option<String>,
    pub tree: HashMap<String, DriveFile>,
    pub path_to_id: HashMap<String, String>,
    /// BFS 是否成功跑完。旧缓存无此字段 → default false → 视为不完整 → 强制重跑。
    /// 见模块级「索引完整性守卫」文档。
    #[serde(default)]
    pub complete: bool,
}

/// 构建云端文件树（BFS）。
/// 返回 (tree: rel_path→DriveFile, path_to_id: rel_path→file_id, root_folder_id)。
pub async fn refresh_cloud_tree(
    files_api: &Arc<FilesApi>,
    mount: &Option<Arc<MountManager>>,
    abs_mount_dir: &str,
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

    // 索引完整性哨兵：BFS 开始前先写一份 complete:false 的空缓存（原子写）。
    // 若本次 BFS 中途被强退，哨兵留在盘上 → 下次 startup load 见 complete:false
    // → 视为不可用 → 强制全量重跑，绝不拿残缺缓存去触发文件同步/上传。
    // （complete:true 的完整缓存由 BFS 成功结尾覆盖写入。）
    let _ = persist_cloud_tree_internal(
        abs_mount_dir,
        &HashMap::new(),
        &HashMap::new(),
        &None,
        false,
    );

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

                        // #6 BFS 渐进建本地目录+占位符（对齐 dart _ensureLocalChildren）
                        if let Some(ref m) = mount {
                            if f.is_folder() {
                                // ensure_folder 只建目录不写 xattr，需补写 cloud folderId。
                                // reconcile_db_records 无 xattr 则无法为目录建 DB 记录 →
                                // 用户随后删本地目录时 planner 看「本地无/云端有/DB 无」
                                // → 误判为「云端文件夹→本地创建」把目录复原回来。
                                // 设 xattr 后 reconcile 能建 DB 记录 → 删除走 skip 分支
                                // （文件夹级联删除禁用），不再误复原。
                                if let Ok(abs) = m.ensure_folder(&rel_path) {
                                    if !f.id.is_empty() {
                                        let _ = m.set_file_id_xattr(&abs, &f.id).await;
                                    }
                                }
                            } else {
                                let _ = m
                                    .create_placeholder_if_needed(&rel_path, &f.id, f.size)
                                    .await;
                            }
                        }

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

    // 持久化缓存（complete:true：BFS 已成功跑完）
    let _ = persist_cloud_tree(abs_mount_dir, &tree, &path_to_id, &root_folder_id);

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
/// 统计根文件列表中出现次数 ≥ 2 的 parentFolder 值（对齐 dart `_detectRootFolderId`）。
fn detect_root_folder_id(files: &[DriveFile]) -> Option<String> {
    let mut counter: HashMap<String, usize> = HashMap::new();
    for f in files {
        if let Some(pf) = &f.parent_folder {
            for id in pf {
                *counter.entry(id.clone()).or_default() += 1;
            }
        }
    }
    counter
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .max_by_key(|(_, count)| *count)
        .map(|(id, _)| id)
}

/// 加载缓存的云端树。不存在或损坏 → None。
///
/// **完整性守卫**：仅当 `complete=true` 且 tree 非空时返回 `Some`；否则返回 `None`
/// （调用方 engine 会据此强制全量重跑 BFS）。这保证 startup 永远不会拿残缺/空/
/// 未完成的缓存去跑文件同步，杜绝「索引中被退出 → 下次误判全量新增」。
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
    let raw = std::fs::read_to_string(&file).ok()?;
    let cache: CloudTreeCache = serde_json::from_str(&raw).ok()?;
    // 完整性校验：未标记完成（上次 BFS 中断/被强退，或旧版无此字段）→ 视为不可用
    if !cache.complete {
        tracing::warn!("云端树缓存未标记完成（上次索引可能中断或为旧格式），将全量重跑 BFS");
        return None;
    }
    // 兜底：complete=true 但 tree 空（理论上不应发生，防御异常数据）
    if cache.tree.is_empty() {
        tracing::warn!("云端树缓存为空，将全量重跑 BFS");
        return None;
    }
    tracing::info!(files = cache.tree.len(), "从缓存加载云端树");
    Some(cache)
}

/// 持久化云端树到磁盘（complete=true，BFS 成功跑完时调用）。
///
/// 内部走 [`persist_cloud_tree_internal`] 的原子写。
fn persist_cloud_tree(
    abs_mount_dir: &str,
    tree: &HashMap<String, DriveFile>,
    path_to_id: &HashMap<String, String>,
    root_folder_id: &Option<String>,
) -> AppResult<()> {
    persist_cloud_tree_internal(abs_mount_dir, tree, path_to_id, root_folder_id, true)
}

/// 原子写云端树缓存。
///
/// 写到 `cache_file + TMP_SUFFIX` → 同步落盘 → rename 覆盖 `cache_file`。
/// 保证磁盘上要么是完整新版、要么是上一版，绝不出现写到一半的半截 JSON
/// （对齐项目已有的 `.tmp` + rename 原子写范式，见 `constants::TMP_SUFFIX`、
/// `download_api`）。`complete=false` 时写空 tree 哨兵。
fn persist_cloud_tree_internal(
    abs_mount_dir: &str,
    tree: &HashMap<String, DriveFile>,
    path_to_id: &HashMap<String, String>,
    root_folder_id: &Option<String>,
    complete: bool,
) -> AppResult<()> {
    let cache_file = cache_paths::cloud_tree_cache_file(abs_mount_dir)?;
    if let Some(parent) = cache_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache = CloudTreeCache {
        root_folder_id: root_folder_id.clone(),
        tree: tree.clone(),
        path_to_id: path_to_id.clone(),
        complete,
    };
    let json = serde_json::to_string_pretty(&cache)?;
    // 原子写：tmp → fsync → rename
    let tmp_file = cache_file.with_extension("json.tmp");
    std::fs::write(&tmp_file, &json)?;
    // fsync tmp 确保内容落盘，再 rename 保证目标文件原子替换
    {
        let f = std::fs::File::open(&tmp_file)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_file, &cache_file)?;
    tracing::info!(complete, "云端树已持久化");
    Ok(())
}

/// 把当前配置挂载目录的云端树缓存标记为不完整（complete=false）。
///
/// 供 `platform::shutdown::flush_with_timeout` 在退出时调用（双保险）：
/// 即使退出发生在 BFS 尚未开始的窗口（哨兵还没写），也能确保下次 startup
/// 检测到「未完成」并强制全量重跑 BFS。纯本地文件操作，无网络、非阻塞，
/// 3.2s 超时兜底内必完成。
///
/// 语义：仅当缓存文件存在且当前 `complete=true` 时才改写为 false ——
/// 这样不会破坏已经写下的哨兵，也避免在根本没有缓存的场景下凭空创建空文件。
pub fn mark_cache_incomplete_if_exists() {
    // 读当前配置得到挂载目录；配置缺失则无操作（无挂载目录 = 无缓存可标记）
    let Ok(config) = crate::core::config_store::ConfigStore::load() else {
        return;
    };
    let abs_dir = config.expanded_mount_dir().to_string_lossy().to_string();
    let Ok(cache_file) = cache_paths::cloud_tree_cache_file(&abs_dir) else {
        return;
    };
    if !cache_file.exists() {
        return;
    }
    // 已是不完整（哨兵/旧格式）→ 无需改写
    let raw = match std::fs::read_to_string(&cache_file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut cache: CloudTreeCache = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !cache.complete {
        return;
    }
    cache.complete = false;
    let json = match serde_json::to_string_pretty(&cache) {
        Ok(s) => s,
        Err(_) => return,
    };
    // 原子写覆盖（tmp → fsync → rename）
    let tmp_file = cache_file.with_extension("json.tmp");
    if std::fs::write(&tmp_file, &json).is_err() {
        return;
    }
    if std::fs::File::open(&tmp_file)
        .and_then(|f| f.sync_all())
        .is_err()
    {
        return;
    }
    let _ = std::fs::rename(&tmp_file, &cache_file);
    tracing::info!("退出标记：云端树缓存置为未完成（下次启动将全量重跑 BFS）");
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
            complete: false,
        };
        write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
        assert!(
            load_persisted_cloud_tree(&abs).is_none(),
            "complete=false 的缓存必须被拒绝，否则 startup 会拿残缺缓存触发文件同步"
        );
    }

    /// complete=true 但 tree 空 → load 应返回 None
    #[test]
    fn test_load_rejects_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let cache = CloudTreeCache {
            root_folder_id: Some("root".into()),
            tree: HashMap::new(),
            path_to_id: HashMap::new(),
            complete: true,
        };
        write_cache_raw(&abs, &serde_json::to_string_pretty(&cache).unwrap());
        assert!(
            load_persisted_cloud_tree(&abs).is_none(),
            "complete=true 但空 tree 不应被信任"
        );
    }

    /// complete=true 且 tree 非空 → load 应返回 Some
    #[test]
    fn test_load_accepts_complete_cache() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let mut tree = HashMap::new();
        tree.insert("学习".into(), sample_file());
        let cache = CloudTreeCache {
            root_folder_id: Some("root".into()),
            tree,
            path_to_id: HashMap::new(),
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

    /// persist_cloud_tree_internal 原子写：写入后文件存在且可被 load 正确读回，
    /// 且无残留 .tmp 文件。
    #[test]
    fn test_persist_internal_atomic_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().to_string_lossy().to_string();
        let mut tree = HashMap::new();
        tree.insert("学习".into(), sample_file());
        let mut p2i = HashMap::new();
        p2i.insert("学习".into(), "f1".into());
        persist_cloud_tree_internal(&abs, &tree, &p2i, &Some("root".into()), true).unwrap();

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
