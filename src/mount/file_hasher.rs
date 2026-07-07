//! 文件 SHA256 哈希（带 mtime+size 缓存）。
//!
//! 对齐 `legacy/lib/mount/file_hasher.dart`。
//!
//! 流式计算（不整文件加载到内存）。
//! 缓存：key=绝对路径 → {mtime_ms, size, sha256}，若 mtime+size 未变则返回缓存。

use std::collections::HashMap;
use std::path::Path;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

/// 哈希缓存条目
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CacheEntry {
    mtime_ms: i64,
    size: u64,
    sha256: String,
}

/// 文件哈希器（带 mtime+size 缓存）。
/// 对齐 dart `FileHasher`。
#[allow(dead_code)]
pub struct FileHasher {
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl FileHasher {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// 计算文件 SHA256（hex 小写）。
    /// 若 mtime+size 与缓存一致则返回缓存（不重算）。
    #[allow(dead_code)]
    pub async fn hash_file(&self, path: &Path) -> std::io::Result<String> {
        let meta = tokio::fs::metadata(path).await?;
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let size = meta.len();

        // 缓存命中检查
        let key = path.to_string_lossy().to_string();
        if let Some(entry) = self.cache.lock().get(&key) {
            if entry.mtime_ms == mtime_ms && entry.size == size {
                return Ok(entry.sha256.clone());
            }
        }

        // 流式计算 SHA256
        let mut file = File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024]; // 64KB 缓冲
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let result = hasher.finalize();
        let sha256 = hex::encode(result);

        // 更新缓存
        self.cache.lock().insert(
            key,
            CacheEntry {
                mtime_ms,
                size,
                sha256: sha256.clone(),
            },
        );
        Ok(sha256)
    }

    /// 计算文件指定区间的 SHA256（用于 resume 校验，可选偏移读取）。
    #[allow(dead_code)]
    pub async fn hash_range(&self, path: &Path, offset: u64, len: u64) -> std::io::Result<String> {
        let mut file = File::open(path).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        let mut hasher = Sha256::new();
        let mut remaining = len;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let to_read = std::cmp::min(remaining, buf.len() as u64);
            let n = file.read(&mut buf[..to_read as usize]).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            remaining -= n as u64;
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// 失效某文件的缓存（文件被修改/删除后调用）。
    #[allow(dead_code)]
    pub fn invalidate(&self, path: &Path) {
        let key = path.to_string_lossy().to_string();
        self.cache.lock().remove(&key);
    }

    /// 清空全部缓存。
    #[allow(dead_code)]
    pub fn clear(&self) {
        self.cache.lock().clear();
    }

    /// 计算字符串的 SHA256（非文件场景）。
    #[allow(dead_code)]
    pub fn sha256_of_string(s: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(s.as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl Default for FileHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_hash_file_returns_hex() {
        let dir = tempdir().unwrap().keep();
        let path = dir.join("test.txt");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let hasher = FileHasher::new();
        let hash = hasher.hash_file(&path).await.unwrap();
        // "hello world" 的 SHA256
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn test_hash_cache_hit() {
        let dir = tempdir().unwrap().keep();
        let path = dir.join("test.txt");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        let hasher = FileHasher::new();
        let hash1 = hasher.hash_file(&path).await.unwrap();
        let hash2 = hasher.hash_file(&path).await.unwrap();
        // 缓存命中应返回相同结果
        assert_eq!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_hash_cache_invalidated_on_change() {
        let dir = tempdir().unwrap().keep();
        let path = dir.join("test.txt");
        tokio::fs::write(&path, b"content1").await.unwrap();

        let hasher = FileHasher::new();
        let hash1 = hasher.hash_file(&path).await.unwrap();

        // 修改文件 + 等待足够时间确保 mtime 变化
        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::fs::write(&path, b"content2-different-enough")
            .await
            .unwrap();

        let hash2 = hasher.hash_file(&path).await.unwrap();
        assert_ne!(hash1, hash2, "内容变更后哈希应不同");
    }

    #[test]
    fn test_sha256_of_string() {
        let hash = FileHasher::sha256_of_string("hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_invalidate_clears_cache() {
        let hasher = FileHasher::new();
        hasher.cache.lock().insert(
            "/fake/path".to_string(),
            CacheEntry {
                mtime_ms: 0,
                size: 0,
                sha256: "abc".to_string(),
            },
        );
        hasher.invalidate(Path::new("/fake/path"));
        assert!(hasher.cache.lock().is_empty());
    }
}
