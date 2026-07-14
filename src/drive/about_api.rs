//! About API —— 配额信息（GET /about?fields=*）。
//!
//! 对齐 `legacy/lib/drive/api/about_api.dart`。
//!
//! 注意：`fields=*` 是**强制**的，否则华为返回 400。

use std::sync::Arc;

use crate::drive::client::{parse_json_response, DriveClient};
use crate::drive::models::DriveAbout;
use crate::error::{AppError, AppResult};

/// 查询云盘配额并在上传前执行容量预检。
pub struct AboutApi {
    client: Arc<DriveClient>,
}

impl AboutApi {
    /// 使用共享 Drive 客户端创建配额接口。
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// GET `/about?fields=*` 获取配额信息；对齐 dart `AboutApi.get()`。
    pub async fn get(&self) -> AppResult<DriveAbout> {
        let resp = self.client.get("/about?fields=*").await?;
        let body: serde_json::Value = parse_json_response(resp, "about").await?;
        Ok(DriveAbout::from_json(&body))
    }

    /// 上传前配额校验（需求 §2.8 第三阶段）。
    /// 对齐 dart `AboutApi.ensureCapacity(int requiredBytes)`。
    pub async fn ensure_capacity(&self, required_bytes: i64) -> AppResult<()> {
        let about = self.get().await?;
        if !about.can_fit(required_bytes) {
            return Err(AppError::quota_exceeded(
                required_bytes,
                about.remaining_space(),
            ));
        }
        Ok(())
    }
}
