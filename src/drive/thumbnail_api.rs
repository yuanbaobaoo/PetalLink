//! Thumbnail API —— 图片/视频缩略图。
//!
//! 对齐 `legacy/lib/drive/api/thumbnail_api.dart`。

use std::sync::Arc;

use base64::Engine;
use reqwest::header::CONTENT_TYPE;

use crate::drive::client::DriveClient;
use crate::error::{AppError, AppResult};

/// 获取云盘文件缩略图并生成可供 WebView 使用的 data URL。
pub struct ThumbnailApi {
    client: Arc<DriveClient>,
}

impl ThumbnailApi {
    /// 使用共享 Drive 客户端创建缩略图接口。
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// 获取缩略图并编码为保留真实 MIME 的 data URL。
    ///
    /// 请求复用 `DriveClient` 的 401 刷新重放；未知二进制格式拒绝传给 WebView。
    pub async fn get_data_url(&self, file_id: &str) -> AppResult<String> {
        let path = format!("/thumbnails/{file_id}?form=content");
        let resp = self.client.get(&path).await?;
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| crate::drive::client::classify_error(&e))?;
        if bytes.is_empty() {
            return Err(AppError::generic("缩略图响应为空"));
        }
        let media_type = thumbnail_media_type(content_type.as_deref(), &bytes)?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(format!("data:{media_type};base64,{encoded}"))
    }
}

/// 解析服务端 MIME，并在通用二进制响应时按文件签名识别图片格式。
fn thumbnail_media_type(content_type: Option<&str>, bytes: &[u8]) -> AppResult<String> {
    if let Some(media_type) = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| value.starts_with("image/"))
    {
        return Ok(media_type);
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok("image/png".to_string());
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg".to_string());
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok("image/gif".to_string());
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Ok("image/webp".to_string());
    }
    Err(AppError::generic("缩略图响应不是支持的图片格式"))
}

#[cfg(test)]
mod tests {
    use super::thumbnail_media_type;

    /// 服务端返回通用二进制时必须按 JPEG 文件签名恢复真实 MIME。
    #[test]
    fn detects_jpeg_when_content_type_is_generic() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0];

        assert_eq!(
            thumbnail_media_type(Some("application/octet-stream"), &bytes).unwrap(),
            "image/jpeg"
        );
    }

    /// 明确的图片 Content-Type 必须去掉参数并规范化大小写。
    #[test]
    fn preserves_explicit_image_content_type() {
        assert_eq!(
            thumbnail_media_type(Some("IMAGE/PNG; charset=binary"), b"payload").unwrap(),
            "image/png"
        );
    }

    /// 非图片响应必须被拒绝，避免把错误页编码成不可显示的 data URL。
    #[test]
    fn rejects_unknown_binary_payload() {
        assert!(thumbnail_media_type(Some("text/html"), b"<html>").is_err());
    }
}
