//! About API —— 配额信息（GET /about?fields=*）。
//!
//! 对齐 `legacy/lib/drive/api/about_api.dart`。
//!
//! 注意：`fields=*` 是**强制**的，否则华为返回 400。

use std::sync::Arc;


use crate::drive::client::DriveClient;
use crate::drive::models::DriveAbout;
use crate::error::{AppError, AppResult};

pub struct AboutApi {
    client: Arc<DriveClient>,
}

impl AboutApi {
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// 获取配额信息。对齐 dart `AboutApi.get()`。
    /// GET /about?fields=*
    pub async fn get(&self) -> AppResult<DriveAbout> {
        let resp = self.client.get("/about?fields=*").await?;
        if !resp.status().is_success() {
            return Err(crate::drive::client::handle_error_response(resp).await);
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 about 响应失败：{e}")))?;
        Ok(DriveAbout::from_json(&body))
    }

    /// 上传前配额校验（需求 §2.8 第三阶段）。
    /// 对齐 dart `AboutApi.ensureCapacity(int requiredBytes)`。
    pub async fn ensure_capacity(&self, required_bytes: i64) -> AppResult<()> {
        let about = self.get().await?;
        if !about.can_fit(required_bytes) {
            return Err(AppError::quota_exceeded(required_bytes, about.remaining_space()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::drive::models::DriveAbout;

    #[test]
    fn test_ensure_capacity_logic() {
        // can_fit 与 ensure_capacity 的核心逻辑测试
        let about = DriveAbout {
            user_capacity: 1000,
            used_space: 600,
            user_display_name: None,
        };
        assert!(about.can_fit(400)); // 剩 400，刚好够
        assert!(!about.can_fit(401)); // 不够
    }
}
