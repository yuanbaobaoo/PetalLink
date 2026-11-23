//! About API —— 配额信息（GET /about?fields=*）。
//!
//! 对齐 `legacy/lib/drive/api/about_api.dart`。
//!
//! 注意：`fields=*` 是**强制**的，否则华为返回 400。

use std::sync::{Arc, OnceLock};

use crate::drive::client::{parse_json_response, DriveClient};
use crate::drive::models::DriveAbout;
use crate::error::{AppError, AppResult};

/// 配额快照复用窗口（毫秒）。
///
/// 批量上传时每个文件都查一次配额会让上传请求数翻倍；这里允许在极短窗口内
/// 复用同一份快照。服务端仍会在真正超限时拒绝写入，因此窗口只影响「提前判停」，
/// 不会让超配额的上传被误判成成功。
const QUOTA_CACHE_TTL_MS: i64 = 10_000;

/// 进程内配额快照缓存：`(获取时间, 配额)`。
static QUOTA_CACHE: OnceLock<parking_lot::Mutex<Option<(i64, DriveAbout)>>> = OnceLock::new();

/// 返回配额缓存容器。
fn quota_cache() -> &'static parking_lot::Mutex<Option<(i64, DriveAbout)>> {
    QUOTA_CACHE.get_or_init(|| parking_lot::Mutex::new(None))
}

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
        let about = self.cached_about().await?;
        if !about.can_fit(required_bytes) {
            return Err(AppError::quota_exceeded(
                required_bytes,
                about.remaining_space(),
            ));
        }
        Ok(())
    }

    /// 在短窗口内复用配额快照，避免批量上传时每个文件都多一次 GET /about。
    async fn cached_about(&self) -> AppResult<DriveAbout> {
        let now = chrono::Utc::now().timestamp_millis();
        if let Some((fetched_at, about)) = quota_cache().lock().as_ref() {
            if now.saturating_sub(*fetched_at) < QUOTA_CACHE_TTL_MS {
                return Ok(about.clone());
            }
        }
        // 查询失败不写缓存：下一次上传仍会重新确认配额。
        let about = self.get().await?;
        *quota_cache().lock() = Some((now, about.clone()));
        Ok(about)
    }
}
