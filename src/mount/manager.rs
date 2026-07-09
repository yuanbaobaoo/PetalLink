//! 本地镜像目录管理器 —— 占位符 + 本地扫描 + Finder 灰标。
//!
//! 对齐 `legacy/lib/mount/mount_manager.dart`。
//!
//! # 占位符策略（v2, Files-On-Demand-lite）
//! - 占位文件使用**真实文件名**（无后缀），0 字节。
//! - 状态通过 xattr 3 个键追踪：com.hwcloud.fileId / com.hwcloud.state / com.hwcloud.size。
//! - Finder 灰标（label index 7）= 未下载；无标签 = 已下载。
//! - xattr 是数据源头（source of truth），Finder label 仅视觉反馈。
//! - 0 字节且非占位 → 拒绝删除（保护用户空文件如 .gitkeep）

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::mount::skip::should_skip;

// ===== xattr 键常量 =====
/// xattr 键：云端文件 ID
pub const XATTR_FILE_ID: &str = "com.hwcloud.fileId";
/// xattr 键：占位状态（placeholder / downloaded）
pub const XATTR_STATE: &str = "com.hwcloud.state";
/// xattr 键：文件大小
pub const XATTR_SIZE: &str = "com.hwcloud.size";

/// xattr 值常量
pub const STATE_PLACEHOLDER: &str = "placeholder";
pub const STATE_DOWNLOADED: &str = "downloaded";

/// 旧版占位符后缀（仅用于清理遗留文件）
pub const LEGACY_PLACEHOLDER_SUFFIX: &str = ".hwcloud_placeholder";

/// 本地文件条目（scanLocal 返回）
#[derive(Debug, Clone)]
pub struct LocalFileEntry {
    /// 绝对路径
    pub absolute_path: PathBuf,
    /// 相对挂载目录的路径
    pub relative_path: String,
    /// 文件大小（字节）
    pub size: u64,
    /// 修改时间（毫秒 epoch）
    pub mtime: i64,
    /// 是否文件夹
    pub is_folder: bool,
    /// 是否占位符（0 字节）
    pub is_placeholder: bool,
}

/// 本地镜像目录管理器。
pub struct MountManager {
    /// 挂载根目录（绝对路径）
    mount_dir: PathBuf,
}

impl MountManager {
    pub fn new(mount_dir: &Path) -> Self {
        Self {
            mount_dir: mount_dir.to_path_buf(),
        }
    }

    /// 获取挂载目录。
    pub fn mount_dir(&self) -> &Path {
        &self.mount_dir
    }

    /// 确保挂载目录存在（初始化时调用）。
    pub fn ensure_mount_dir(&self) -> AppResult<()> {
        if !self.mount_dir.exists() {
            std::fs::create_dir_all(&self.mount_dir)
                .map_err(|e| AppError::generic(format!("创建挂载目录失败：{e}")))?;
        }
        Ok(())
    }

    /// 确保文件夹存在（递归创建）。
    pub fn ensure_folder(&self, rel_path: &str) -> AppResult<PathBuf> {
        let full = crate::core::paths::safe_join_under(&self.mount_dir, rel_path, true)?;
        if !full.exists() {
            std::fs::create_dir_all(&full)
                .map_err(|e| AppError::generic(format!("创建目录失败：{e}")))?;
        }
        Ok(full)
    }

    /// 为云端文件创建本地占位符（创建即打 Finder 灰标）。
    /// 对齐 dart `createPlaceholderIfNeeded`，但灰标改用直接写 com.apple.FinderInfo
    /// xattr（无 fork），故批量 BFS 也能「即建即标」，不像 dart 因 osascript fork 风暴而跳过。
    /// - 若文件已存在且 xattrState=downloaded → skip
    /// - 若文件已存在且 xattrState=placeholder → skip
    /// - 若文件已存在但无 xattr → skip（用户文件，永远不转为占位符）
    /// - 否则：确保父目录 → create 0 字节文件 → 写 3 个状态 xattr + Finder 灰标
    ///
    /// 全部阻塞 IO（exists 检查 + 父目录 + 建文件 + 4 xattr）合并进单次 spawn_blocking，
    /// 避免逐 xattr spawn_blocking 的调度开销（17K 文件：4→1 次/文件），且不阻塞 async runtime。
    pub async fn create_placeholder_if_needed(
        &self,
        file_name: &str,
        file_id: &str,
        size: i64,
    ) -> AppResult<()> {
        let local_path = crate::core::paths::safe_join_under(&self.mount_dir, file_name, false)?;
        let fp = local_path.to_string_lossy().to_string();
        let file_id = file_id.to_string();
        let size_str = size.to_string();

        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let local_path = Path::new(&fp);
            // 文件已存在 → 检查 xattr 状态决定行为
            if local_path.exists() {
                if let Ok(state) = get_xattr(local_path, XATTR_STATE) {
                    if state == STATE_DOWNLOADED || state == STATE_PLACEHOLDER {
                        return Ok(()); // 已有状态，跳过
                    }
                } else {
                    // 无 xattr → 用户文件，不覆盖
                    return Ok(());
                }
            }
            // 确保父目录存在
            if let Some(parent) = local_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // 创建 0 字节文件
            std::fs::write(local_path, [])
                .map_err(|e| AppError::generic(format!("创建占位符失败：{e}")))?;
            // 写 3 个状态 xattr + Finder 灰标（占位即打标，含批量 BFS）
            set_xattr_sync(&fp, XATTR_FILE_ID, &file_id)
                .map_err(|e| AppError::generic(format!("写 xattr fileId 失败：{e}")))?;
            set_xattr_sync(&fp, XATTR_STATE, STATE_PLACEHOLDER)
                .map_err(|e| AppError::generic(format!("写 xattr state 失败：{e}")))?;
            set_xattr_sync(&fp, XATTR_SIZE, &size_str)
                .map_err(|e| AppError::generic(format!("写 xattr size 失败：{e}")))?;
            let _ = set_finder_label_sync(local_path, true); // 灰标失败不阻断（仅 Finder 无灰标）
            Ok(())
        })
        .await
        .map_err(|e| AppError::generic(format!("占位创建线程异常：{e}")))?
    }

    /// 标记文件为已下载（更新 xattr + 清除灰标）。
    /// 对齐 dart `markDownloaded`。
    pub async fn mark_downloaded(&self, local_path: &Path) -> AppResult<()> {
        let fp = local_path.to_string_lossy().to_string();
        set_xattr_async(fp.clone(), XATTR_STATE, STATE_DOWNLOADED)
            .await
            .map_err(|e| AppError::generic(format!("更新 xattr 失败：{e}")))?;
        let _ = set_finder_label_async(fp, false).await; // 清除灰标
        Ok(())
    }

    /// 为已下载文件写入 fileId xattr。
    ///
    /// download_to_dest 先删占位再下载（新 inode），占位时的 `XATTR_FILE_ID` 随之丢失，
    /// reconcile_db_records 无法凭 xattr 自愈。下载完成后补写 fileId，使本地文件与
    /// 占位文件一样可被 xattr 识别（对齐 dart downloadOnDemand 不删文件、原地覆盖
    /// 从而保留 fileId xattr 的语义）。
    pub async fn set_file_id_xattr(&self, local_path: &Path, file_id: &str) -> AppResult<()> {
        let fp = local_path.to_string_lossy().to_string();
        let file_id = file_id.to_string();
        tokio::task::spawn_blocking(move || set_xattr_sync(&fp, XATTR_FILE_ID, &file_id))
            .await
            .map_err(|e| AppError::generic(format!("fileId xattr 线程异常：{e}")))?
            .map_err(|e| AppError::generic(format!("写 fileId xattr 失败：{e}")))
    }

    /// 下载前处理可能被用户修改过的占位文件（对齐 dart `backupModifiedPlaceholderIfNeeded`）。
    ///
    /// - 不存在 / 非 placeholder / 0 字节未修改 → 返回 None（调用方直接下载覆盖/删除）
    /// - state=placeholder 且 size>0（用户写入了内容）→ **改名**保留到
    ///   `<basename>.local-<YYYYMMDD-HHMMSS>.<ext>`（撞名加序号），清掉备份的占位 xattr
    ///   （避免被 sync 当成新占位），返回备份路径。下载再写到原路径。
    pub async fn backup_modified_placeholder_if_needed(
        &self,
        local_path: &Path,
    ) -> AppResult<Option<PathBuf>> {
        if !local_path.exists() {
            return Ok(None);
        }
        // 必须是占位（state=placeholder）才走备份逻辑
        let state = self.get_xattr_state(local_path).ok();
        if state.as_deref() != Some(STATE_PLACEHOLDER) {
            return Ok(None);
        }
        // 占位创建时 0 字节，size>0 即被用户写入了内容
        let meta = tokio::fs::metadata(local_path).await?;
        if meta.len() == 0 {
            return Ok(None);
        }
        // 改名保留：<base>.local-<stamp>.<ext>
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let dir = local_path.parent().unwrap_or_else(|| Path::new("."));
        let basename = local_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = local_path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut backup = dir.join(format!("{basename}.local-{stamp}{ext}"));
        let mut seq = 1;
        while backup.exists() {
            backup = dir.join(format!("{basename}.local-{stamp}.{seq}{ext}"));
            seq += 1;
        }
        tokio::fs::rename(local_path, &backup).await?;
        // 清掉备份的占位 xattr，避免被 sync 当新占位
        let _ = self.clear_placeholder_xattr(&backup).await;
        tracing::info!(
            "占位被修改过，已备份：{} → {}",
            local_path.display(),
            backup.display()
        );
        Ok(Some(backup))
    }

    /// 清除文件上的占位 xattr（fileId/state/size/FinderInfo）。
    ///
    /// 备份副本改名后调用：让副本被视为全新本地文件。否则副本保留原 fileId xattr，
    /// 下轮 reconcile 会用原 fileId 给副本建记录，planner 又判「云端已删除」把副本删掉
    /// ——副本保不住（修改冲突 / 删除冲突的副本都会丢）。清掉后副本下轮作为全新文件上传。
    pub async fn clear_placeholder_xattr(&self, local_path: &Path) -> AppResult<()> {
        let fp = local_path.to_string_lossy().to_string();
        tokio::task::spawn_blocking(move || {
            let p = std::path::Path::new(&fp);
            let _ = xattr::remove(p, XATTR_FILE_ID);
            let _ = xattr::remove(p, XATTR_STATE);
            let _ = xattr::remove(p, XATTR_SIZE);
            let _ = xattr::remove(p, "com.apple.FinderInfo");
        })
        .await
        .map_err(|e| AppError::generic(format!("清 xattr 线程异常：{e}")))?;
        Ok(())
    }

    /// 扫描挂载目录，返回全部非跳过文件的条目。
    /// 对齐 dart `scanLocal`。
    /// 大目录在独立线程池执行以避免阻塞 tokio runtime。
    pub async fn scan_local(&self, skip_patterns: &[String]) -> AppResult<Vec<LocalFileEntry>> {
        let mount = self.mount_dir.clone();
        // ★ 挂载目录为空时跳过扫描，返回空列表（避免误扫根目录或判断"本地无"误删云端）
        if mount.to_string_lossy().is_empty() {
            tracing::warn!("scan_local 跳过：挂载目录未配置");
            return Ok(Vec::new());
        }
        let patterns = skip_patterns.to_vec();
        // 用 spawn_blocking 避免阻塞 tokio worker（对齐 dart Isolate.run）
        tokio::task::spawn_blocking(move || scan_local_sync(&mount, &patterns))
            .await
            .map_err(|e| AppError::generic(format!("扫描线程异常：{e}")))?
    }

    /// 读取 xattr state 值
    fn get_xattr_state(&self, path: &Path) -> std::io::Result<String> {
        get_xattr(path, XATTR_STATE)
    }

    /// 删除本地文件（安全：0 字节文件若非占位符则拒绝删除，返回 ok 但跳过）。
    /// 对齐 dart `deleteLocal`。
    pub async fn delete_local(&self, local_path: &Path) -> AppResult<()> {
        crate::core::paths::relative_path_from_mount(&self.mount_dir, local_path)?;
        if !local_path.exists() {
            return Ok(());
        }
        let meta = tokio::fs::metadata(local_path)
            .await
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?;
        if meta.is_dir() {
            tokio::fs::remove_dir_all(local_path)
                .await
                .map_err(|e| AppError::generic(format!("删除目录失败：{e}")))?;
            return Ok(());
        }
        // 0 字节文件：必须是占位符才删；否则保留（用户文件如 .gitkeep）——返回 Ok 表示「已处理」
        if meta.len() == 0 {
            let is_pl = self
                .get_xattr_state(local_path)
                .ok()
                .map(|s| s == STATE_PLACEHOLDER)
                .unwrap_or(false);
            if !is_pl {
                tracing::debug!(path = %local_path.display(), "保留非占位 0 字节文件");
                return Ok(());
            }
        }
        tokio::fs::remove_file(local_path)
            .await
            .map_err(|e| AppError::generic(format!("删除文件失败：{e}")))?;
        // 清理旧版占位符
        let legacy = legacy_placeholder_path(local_path);
        if legacy.exists() {
            let _ = tokio::fs::remove_file(&legacy).await;
        }
        Ok(())
    }
}

// ===== 平台相关实现（xattr + osascript） =====

/// 同步读 xattr 值。
#[cfg(target_os = "macos")]
fn get_xattr(path: &Path, key: &str) -> std::io::Result<String> {
    let bytes = xattr::get(path, key)?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "xattr not found"))?;
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(not(target_os = "macos"))]
fn get_xattr(_path: &Path, _key: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "xattr not available",
    ))
}

/// 异步写 xattr（在 tokio 线程池执行）。
async fn set_xattr_async(path: String, key: &str, value: &str) -> std::io::Result<()> {
    let path_clone = path;
    let key = key.to_string();
    let value = value.to_string();
    tokio::task::spawn_blocking(move || set_xattr_sync(&path_clone, &key, &value))
        .await
        .map_err(std::io::Error::other)?
}

#[cfg(target_os = "macos")]
fn set_xattr_sync(path: &str, key: &str, value: &str) -> std::io::Result<()> {
    xattr::set(Path::new(path), key, value.as_bytes())
}

#[cfg(not(target_os = "macos"))]
fn set_xattr_sync(_path: &str, _key: &str, _value: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "xattr not available",
    ))
}

/// Finder 灰标 xattr 键
#[cfg(target_os = "macos")]
const FINDER_INFO_XATTR: &str = "com.apple.FinderInfo";
/// FinderInfo byte[9] 的灰标值（label index 7 = 灰；实测 byte[9]=0x02，
/// 与 osascript `set label index to 7` 结果一致，kMDItemFSLabel=1）。
#[cfg(target_os = "macos")]
const GRAY_LABEL_BYTE: u8 = 0x02;

/// 设置/清除 Finder 灰色标签：直接读写 com.apple.FinderInfo xattr，无 fork。
/// - gray=true：byte[9]=0x02（灰标）
/// - gray=false：byte[9]=0x00（清除；若整块全 0 则删 xattr，对齐 osascript label 0）
///
/// 用直接 xattr 写而非 osascript，避免批量 17K 文件 fork 进程风暴，
/// 使占位文件创建时即可打标（含批量 BFS）。读改写保留其它 FinderInfo 字段。
#[cfg(target_os = "macos")]
fn set_finder_label_sync(path: &Path, gray: bool) -> std::io::Result<()> {
    let mut buf = xattr::get(path, FINDER_INFO_XATTR)
        .ok()
        .flatten()
        .unwrap_or_default();
    if buf.len() < 32 {
        buf.resize(32, 0);
    }
    buf[9] = if gray { GRAY_LABEL_BYTE } else { 0x00 };
    if !gray && buf.iter().all(|&b| b == 0) {
        let _ = xattr::remove(path, FINDER_INFO_XATTR);
    } else {
        xattr::set(path, FINDER_INFO_XATTR, &buf)?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn set_finder_label_sync(_path: &Path, _gray: bool) -> std::io::Result<()> {
    Ok(())
}

/// 异步设置/清除 Finder 灰标（spawn_blocking，避免阻塞 tokio runtime）。
async fn set_finder_label_async(path: String, gray: bool) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || set_finder_label_sync(Path::new(&path), gray))
        .await
        .map_err(std::io::Error::other)?
}

// ===== 同步扫描 =====

/// 同步扫描目录（在 spawn_blocking 中执行）。
fn scan_local_sync(mount_dir: &Path, skip_patterns: &[String]) -> AppResult<Vec<LocalFileEntry>> {
    let mut entries = Vec::new();
    scan_recursive(mount_dir, mount_dir, skip_patterns, &mut entries)
        .map_err(|e| AppError::generic(format!("扫描目录失败：{e}")))?;
    Ok(entries)
}

fn scan_recursive(
    base: &Path,
    current: &Path,
    skip_patterns: &[String],
    out: &mut Vec<LocalFileEntry>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // 跳过内部文件
        if should_skip(&name_str, skip_patterns) {
            continue;
        }

        let abs = entry.path();
        let rel = abs
            .strip_prefix(base)
            .unwrap_or(&abs)
            .to_string_lossy()
            .to_string();
        let meta = entry.metadata()?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        if file_type.is_dir() {
            out.push(LocalFileEntry {
                absolute_path: abs.clone(),
                relative_path: rel,
                size: 0,
                mtime,
                is_folder: true,
                is_placeholder: false,
            });
            // 递归进入子目录
            scan_recursive(base, &abs, skip_patterns, out)?;
        } else if file_type.is_file() {
            let size = meta.len();
            // 占位符判断用 xattr state，而非 0 字节（用户空文件如 .gitkeep 不是占位符）
            let is_placeholder = size == 0 && is_placeholder_file(&abs);
            out.push(LocalFileEntry {
                absolute_path: abs,
                relative_path: rel,
                size,
                mtime,
                is_folder: false,
                is_placeholder,
            });
        }
    }
    Ok(())
}

/// 旧版占位符文件路径
fn legacy_placeholder_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(LEGACY_PLACEHOLDER_SUFFIX);
    PathBuf::from(s)
}

/// 通过 xattr 判断是否为占位符（state=placeholder）。
pub fn is_placeholder_file(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        xattr::get(path, XATTR_STATE)
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b) == STATE_PLACEHOLDER)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_mount() -> MountManager {
        let dir = tempdir().unwrap().keep();
        std::fs::create_dir_all(&dir).unwrap();
        MountManager::new(&dir)
    }

    #[test]
    fn test_set_finder_label_gray_byte9() {
        // 锁住 FinderInfo 灰标编码：byte[9]=0x02；清除后整块全 0 则删 xattr。
        #[cfg(target_os = "macos")]
        {
            let dir = tempdir().unwrap();
            let f = dir.path().join("t.txt");
            std::fs::write(&f, b"x").unwrap();
            // 打灰标
            set_finder_label_sync(&f, true).unwrap();
            let buf = xattr::get(&f, FINDER_INFO_XATTR).unwrap().unwrap();
            assert_eq!(buf.len(), 32);
            assert_eq!(buf[9], GRAY_LABEL_BYTE);
            // 清除 → 整块全 0 → xattr 被删除
            set_finder_label_sync(&f, false).unwrap();
            assert!(xattr::get(&f, FINDER_INFO_XATTR).unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn test_ensure_folder_creates() {
        let m = test_mount();
        let path = m.ensure_folder("sub/deep").unwrap();
        assert!(path.exists());
        assert!(path.is_dir());
    }

    #[tokio::test]
    async fn test_scan_local_skips_internal_files() {
        let m = test_mount();
        // 创建普通文件 + 内部文件（应跳过）
        tokio::fs::write(m.mount_dir().join("normal.txt"), b"hello")
            .await
            .unwrap();
        tokio::fs::write(m.mount_dir().join(".hwcloud_cache.json"), b"{}")
            .await
            .unwrap();
        tokio::fs::write(m.mount_dir().join("temp.tmp"), b"temp")
            .await
            .unwrap();

        let entries = m.scan_local(&[]).await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.relative_path.as_str()).collect();
        assert!(names.contains(&"normal.txt"));
        assert!(!names.contains(&".hwcloud_cache.json"));
        assert!(!names.contains(&"temp.tmp"));
    }

    #[tokio::test]
    async fn test_scan_local_folders_and_files() {
        let m = test_mount();
        tokio::fs::create_dir(m.mount_dir().join("sub"))
            .await
            .unwrap();
        tokio::fs::write(m.mount_dir().join("sub/file.txt"), b"data")
            .await
            .unwrap();
        tokio::fs::write(m.mount_dir().join("root.txt"), b"root")
            .await
            .unwrap();

        let entries = m.scan_local(&[]).await.unwrap();
        assert!(entries.len() >= 3);
        // 文件夹 is_folder=true
        let sub = entries.iter().find(|e| e.relative_path == "sub").unwrap();
        assert!(sub.is_folder);
        assert_eq!(sub.size, 0);
        // 文件 is_placeholder 基于 size
        let f = entries
            .iter()
            .find(|e| e.relative_path == "root.txt")
            .unwrap();
        assert!(!f.is_folder);
        assert!(!f.is_placeholder); // 4 字节，非占位
    }

    #[test]
    fn test_placeholder_0_byte_is_placeholder() {
        // 0 字节文件判定为占位符
        let e = LocalFileEntry {
            absolute_path: PathBuf::from("/tmp/f"),
            relative_path: "f".into(),
            size: 0,
            mtime: 0,
            is_folder: false,
            is_placeholder: true,
        };
        assert!(e.is_placeholder);
    }
}
