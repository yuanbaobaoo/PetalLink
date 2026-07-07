//! Download API —— 流式下载到 .tmp 再原子 rename。
//!
//! 对齐 `legacy/lib/drive/api/download_api.dart`。
//!
//! 关键：`.tmp` 后缀是 load-bearing 的——watcher 和 scanner 全链路跳过 .tmp，
//! 避免下载中的不完整文件被当「新增文件」误上传。

use std::path::{Path, PathBuf};

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::drive::client::DriveClient;
use crate::error::{AppError, AppResult};

pub struct DownloadApi {
    client: Arc<DriveClient>,
}

/// 下载进度回调（参数：已下载字节，总字节）
pub type ProgressFn = Box<dyn Fn(u64, u64) + Send + Sync>;

impl DownloadApi {
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// 下载文件到 dest_path。
    /// 对齐 dart `DownloadApi.download({fileId, destPath, onProgress?})`。
    ///
    /// 流程：流式写 `<dest>.tmp` → 完成后 rename 到 dest。
    pub async fn download(
        &self,
        file_id: &str,
        dest_path: &Path,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<()> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let url = format!(
            "{}/files/{file_id}?form=content",
            crate::constants::DRIVE_API_BASE
        );
        let resp = self
            .client
            .raw_http()
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| crate::drive::client::classify_error(&e))?;
        if !resp.status().is_success() {
            return Err(crate::drive::client::handle_error_response(resp).await);
        }

        let total = resp.content_length().unwrap_or(0);

        // 流式写 .tmp
        let tmp_path = tmp_path(dest_path);
        // 已存在的 .tmp 先删除（避免残留）
        if tmp_path.exists() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        // 确保父目录存在
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        let mut file = File::create(&tmp_path)
            .await
            .map_err(|e| AppError::generic(format!("创建临时文件失败：{e}")))?;

        let mut stream = resp.bytes_stream();
        let mut received: u64 = 0;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| {
                // 下载失败时清理 .tmp 残留（对齐 dart catch 块 tmpPath.delete）
                let _ = std::fs::remove_file(&tmp_path);
                AppError::drive_network(Some(&e.to_string()))
            })?;
            file.write_all(&chunk)
                .await
                .map_err(|e| AppError::generic(format!("写入失败：{e}")))?;
            received += chunk.len() as u64;
            if let Some(cb) = on_progress {
                cb(received, total);
            }
        }
        file.flush().await.ok();
        drop(file);

        // 原子 rename 到目标路径
        if dest_path.exists() {
            let _ = std::fs::remove_file(dest_path);
        }
        tokio::fs::rename(&tmp_path, dest_path).await.map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            AppError::generic(format!("重命名失败：{e}"))
        })?;

        Ok(())
    }
}

/// 构造 .tmp 临时文件路径。
/// 对齐 dart：destPath 加 `.tmp` 后缀。
pub fn tmp_path(dest: &Path) -> PathBuf {
    // 将 .tmp 加到完整路径后（含扩展名）
    let mut s = dest.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_tmp_path_appends_suffix() {
        let dest = PathBuf::from("/tmp/report.txt");
        let tmp = tmp_path(&dest);
        assert_eq!(tmp, PathBuf::from("/tmp/report.txt.tmp"));
    }

    #[test]
    fn test_tmp_path_multiple_extensions() {
        // .tar.gz 也应在最后加 .tmp（对齐 dart：加到完整路径后）
        let dest = PathBuf::from("/tmp/archive.tar.gz");
        let tmp = tmp_path(&dest);
        assert_eq!(tmp, PathBuf::from("/tmp/archive.tar.gz.tmp"));
    }
}
