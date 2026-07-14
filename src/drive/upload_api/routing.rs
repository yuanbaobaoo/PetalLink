//! 上传客户端构造、大小路由与配额预检。

use std::sync::Arc;
use std::time::Duration;

use crate::constants;
use crate::drive::client::DriveClient;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

use super::{ProgressFn, ResumeProgressFn, UploadApi, SMALL_LARGE_THRESHOLD};

impl UploadApi {
    /// 创建禁用自动重定向且带上传超时的客户端。
    pub fn new(client: Arc<DriveClient>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            client,
            http,
            upload_base: constants::UPLOAD_API_BASE.to_string(),
        }
    }

    /// 路由：≤ 20MB → 小文件上传，否则分片续传。
    /// `on_resume_progress`：分片续传进度回调（serverId, uploadId, offset, session_url），供断点续传持久化。
    pub async fn upload(
        &self,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
        on_resume_progress: Option<&ResumeProgressFn>,
    ) -> AppResult<DriveFile> {
        let size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len();
        if size <= SMALL_LARGE_THRESHOLD {
            self.upload_small(file_path, parent_id, on_progress).await
        } else {
            self.upload_resume(file_path, parent_id, None, on_progress, on_resume_progress)
                .await
        }
    }

    /// 读取本地长度并查询云盘剩余配额；任一步失败都拒绝上传。
    pub(super) async fn ensure_capacity_for(&self, file_path: &std::path::Path) -> AppResult<()> {
        let size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len() as i64;
        crate::drive::about_api::AboutApi::new(self.client.clone())
            .ensure_capacity(size)
            .await
    }
}
