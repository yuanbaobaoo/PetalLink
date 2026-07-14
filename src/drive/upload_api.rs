//! Upload API —— 小文件 multipart/related + 大文件分片断点续传 + 更新覆盖。
//!
//! 对齐 `legacy/lib/drive/api/upload_api.dart`。
//!
//! # 小文件（≤ 20MB）：multipart/related（Google Drive 风格）
//! # 大文件（> 20MB）：resume 分片（F-FILE-02）
//! # uploadUpdate：PATCH 覆盖已有文件（冲突解决），失败时保留旧文件并返回错误
//!
//! ## 断点续传流程（华为 resume 合同）
//! 1. POST 初始化会话 → 从 `Location` 响应头获取 session URI
//! 2. PUT 分片到 session URI（`Content-Range: bytes X-Y/Total`）
//! 3. 308/状态查询的 `rangeList` 是唯一可持久化的确认偏移
//! 4. 只有最终 200 + 完整文件元数据才算完成
//!
//! 华为 API 变更后，init 响应 body 仅含 `{"sliceSize":...}`，不含 serverId/uploadId。
//! 必须从 `Location` 头提取会话 URL 才能继续分片上传。

/// 分片请求、确认偏移与会话状态查询。
mod chunk;
/// 小文件上传与已有文件安全覆盖。
mod multipart;
/// 断点会话协议解析与错误分类。
mod protocol;
/// 断点续传循环与会话初始化。
mod resumable;
/// 按文件大小选择上传路径并执行容量预检。
mod routing;

use std::sync::Arc;

use crate::drive::client::DriveClient;
use crate::drive::models::DriveFile;

/// 当前可安全覆盖已有云端文件的最大本地文件大小。
pub const SAFE_EXISTING_UPDATE_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// 小文件 multipart 与大文件断点续传的路由阈值。
const SMALL_LARGE_THRESHOLD: u64 = SAFE_EXISTING_UPDATE_MAX_BYTES;
/// 华为官方 SDK 允许的最小分片大小。
const MIN_CHUNK_SIZE: u64 = 256 * 1024;
/// 服务端未建议分片大小时使用的默认值。
const DEFAULT_CHUNK_SIZE: u64 = 2 * 1024 * 1024;
/// REST 接口允许的单片大小上限。
const MAX_CHUNK_SIZE: u64 = 64 * 1024 * 1024;
/// 明确未提交的连接失败允许的单分片本地尝试次数。
const CHUNK_RETRIES: u32 = 3;
/// 分片全部发完后的最终状态查询轮询次数（华为服务端异步合并，立即查询常得 308）
const FINAL_STATUS_MAX_POLLS: u32 = 5;
/// 每次最终状态查询的间隔（秒）
const FINAL_STATUS_POLL_INTERVAL_SECS: u64 = 3;

/// 协调小文件、覆盖更新与断点续传的上传接口。
pub struct UploadApi {
    client: Arc<DriveClient>,
    http: reqwest::Client,
    /// Upload API base URL（默认 `UPLOAD_API_BASE`）。
    upload_base: String,
}

/// 接收 `0.0..=1.0` 上传比例的进度回调。
pub type ProgressFn = Box<dyn Fn(f64) + Send + Sync>;
/// 断点续传进度回调：server_id, upload_id, 已上传字节偏移, session_url
/// session_url 为华为 resume 上传 Location 头返回的会话 URL（断点续传唯一 token）。
pub type ResumeProgressFn = Box<dyn Fn(&str, &str, u64, &str) + Send + Sync>;

/// 可持久化并向服务端重新核验的断点上传会话。
#[derive(Debug, Clone)]
pub struct ResumeSession {
    pub server_id: String,
    pub upload_id: String,
    /// 华为 API 变更后：init 响应 `Location` 头给出的会话 URL。
    /// 非空时 `put_chunk` 直接 PUT 到此 URL，不再用 serverId/uploadId 拼接。
    pub session_url: String,
    /// API 建议的分片大小（来自 init 响应 body `sliceSize`），0 表示用默认值。
    pub chunk_size: u64,
    /// 本地持久化的续传偏移提示。恢复时不会直接信任该值，而是先查询同一会话的
    /// `rangeList`；新建会话（init_resume_session 构造）时为 0。
    pub start_offset: u64,
}

/// 单次分片请求或会话状态查询的服务端确认结果。
struct ChunkResult {
    /// 仅来自服务端 `rangeList`/`size` 的确认偏移；禁止用本地分片长度推算。
    uploaded: u64,
    /// 是否为最终响应（含完整文件元数据）
    is_final: bool,
    final_file: Option<DriveFile>,
    /// 服务端建议在再次查询前等待的毫秒数。
    process_time_ms: Option<u64>,
}
