//! 文件 SHA256 流式计算。

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// 流式计算文件 SHA256，避免把完整文件加载到内存。
pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}
