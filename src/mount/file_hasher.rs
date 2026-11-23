//! 文件 SHA256 流式计算与内容指纹缓存。
//!
//! 同一个文件在多轮远端核验中会被反复散列；按「路径 + 大小 + 修改时间」缓存结果，
//! 只要文件内容可能变化（任一维度变化）就一定重新计算，不做基于时间的乐观复用。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// 单次读取缓冲区大小：1MB 在机械盘与网络盘上显著减少系统调用次数。
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
/// 缓存条目上限；超出后整体清空，避免长跑进程无限增长。
const HASH_CACHE_CAPACITY: usize = 512;

/// 内容指纹缓存键：路径、大小与修改时间共同确定一份内容。
#[derive(Clone, PartialEq, Eq, Hash)]
struct HashCacheKey {
    /// 被散列的文件绝对路径。
    path: PathBuf,
    /// 快照时刻的文件长度。
    size: u64,
    /// 快照时刻的修改时间。
    modified: SystemTime,
}

/// 进程内内容指纹缓存。
fn hash_cache() -> &'static Mutex<HashMap<HashCacheKey, String>> {
    static CACHE: OnceLock<Mutex<HashMap<HashCacheKey, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 流式计算文件 SHA256，避免把完整文件加载到内存。
///
/// 命中「路径 + 大小 + 修改时间」缓存时直接返回，否则放到阻塞线程池计算，
/// 不占用 async worker。
pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    // 先取内容快照特征；取不到说明文件已不可读。
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("待散列目标不是普通文件"));
    }
    let key = HashCacheKey {
        path: path.to_path_buf(),
        size: metadata.len(),
        modified: metadata.modified()?,
    };
    if let Some(cached) = hash_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Ok(cached);
    }
    // 大文件散列会长时间占用线程，交给阻塞池执行。
    let owned_path = path.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || hash_file_blocking(&owned_path))
        .await
        .map_err(|error| std::io::Error::other(format!("文件散列任务失败：{error}")))??;
    if let Ok(mut cache) = hash_cache().lock() {
        if cache.len() >= HASH_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(key, digest.clone());
    }
    Ok(digest)
}

/// 在阻塞线程中流式读取并计算 SHA256。
fn hash_file_blocking(path: &Path) -> std::io::Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
