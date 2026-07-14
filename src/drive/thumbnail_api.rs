//! Thumbnail API —— 图片/视频缩略图。
//!
//! 对齐 `legacy/lib/drive/api/thumbnail_api.dart`。

use std::sync::Arc;

use crate::drive::client::DriveClient;
use crate::error::AppResult;

/// 获取云盘文件缩略图二进制内容。
pub struct ThumbnailApi {
    client: Arc<DriveClient>,
}

impl ThumbnailApi {
    /// 使用共享 Drive 客户端创建缩略图接口。
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// GET `/thumbnails/{fileId}?form=content` 获取缩略图二进制。
    pub async fn get(&self, file_id: &str) -> AppResult<Vec<u8>> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let url = format!(
            "{}/thumbnails/{file_id}?form=content",
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
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| crate::drive::client::classify_error(&e))?;
        Ok(bytes.to_vec())
    }
}
