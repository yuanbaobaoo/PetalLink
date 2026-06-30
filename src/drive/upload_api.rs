//! Upload API —— 小文件 multipart/related + 大文件分片断点续传 + 更新覆盖。
//!
//! 对齐 `legacy/lib/drive/api/upload_api.dart`。
//!
//! # 小文件（≤ 20MB）：multipart/related（Google Drive 风格）
//! # 大文件（> 20MB）：resume 分片（F-FILE-02）
//! # uploadUpdate：PATCH 覆盖已有文件（冲突解决），失败回退 delete+POST
//!
//! ## 断点续传流程（Google Drive 风格）
//! 1. POST 初始化会话 → 从 `Location` 响应头获取 session URI
//! 2. PUT 分片到 session URI（`Content-Range: bytes X-Y/Total`）
//! 3. 中间分片返回 `{"size": <uploaded_bytes>}`，最后一片返回完整文件元数据
//!
//! 华为 API 变更后，init 响应 body 仅含 `{"sliceSize":...}`，不含 serverId/uploadId。
//! 必须从 `Location` 头提取会话 URL 才能继续分片上传。

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION};
use serde_json::Value;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

use crate::constants;
use crate::drive::client::DriveClient;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

const SMALL_LARGE_THRESHOLD: u64 = 20 * 1024 * 1024;
/// 默认分片大小（华为 API sliceSize 通常返回 10MB，此处为兜底值）
const DEFAULT_CHUNK_SIZE: u64 = 5 * 1024 * 1024;
const CHUNK_RETRIES: u32 = 3;
/// 分片全部发完后的最终状态查询轮询次数（华为服务端异步合并，立即查询常得 308）
const FINAL_STATUS_MAX_POLLS: u32 = 5;
/// 每次最终状态查询的间隔（秒）
const FINAL_STATUS_POLL_INTERVAL_SECS: u64 = 3;

pub struct UploadApi {
    client: Arc<DriveClient>,
    http: reqwest::Client,
    /// Upload API base URL（默认 `UPLOAD_API_BASE`；测试可注入 wiremock 地址）。
    upload_base: String,
    /// Drive API base URL（默认 `DRIVE_API_BASE`；测试可注入 wiremock 地址，用于兜底查询）。
    drive_base: String,
}

pub type ProgressFn = Box<dyn Fn(f64) + Send + Sync>;
/// 断点续传进度回调：server_id, upload_id, 已上传字节偏移
pub type ResumeProgressFn = Box<dyn Fn(&str, &str, u64) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ResumeSession {
    pub server_id: String,
    pub upload_id: String,
    /// 华为 API 变更后：init 响应 `Location` 头给出的会话 URL。
    /// 非空时 `put_chunk` 直接 PUT 到此 URL，不再用 serverId/uploadId 拼接。
    pub session_url: String,
    /// API 建议的分片大小（来自 init 响应 body `sliceSize`），0 表示用默认值。
    pub chunk_size: u64,
}

/// put_chunk 返回：已上传字节偏移 + 可选（兜底查询用的 createdFileId）
struct ChunkResult {
    uploaded: u64,
    created_file_id: Option<String>,
    /// 是否为最终响应（含完整文件元数据）
    is_final: bool,
    final_file: Option<DriveFile>,
}

impl UploadApi {
    pub fn new(client: Arc<DriveClient>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            client,
            http,
            upload_base: constants::UPLOAD_API_BASE.to_string(),
            drive_base: constants::DRIVE_API_BASE.to_string(),
        }
    }

    /// 测试用：注入自定义 base URL（如 wiremock 地址）。
    #[cfg(test)]
    pub fn with_base_urls(
        client: Arc<DriveClient>,
        upload_base: String,
        drive_base: String,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("构建 reqwest client 失败");
        Self { client, http, upload_base, drive_base }
    }

    /// 直接传入 token 进行 resume 分片上传（绕过 AuthService，供测试/调试用）。
    pub async fn upload_resume_with_token(
        &self, file_path: &std::path::Path, parent_id: Option<&str>, token: &str,
    ) -> AppResult<DriveFile> {
        let total_size = file_path.metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len();
        // 跳过 ensure_capacity_for（其内部依赖 AuthService，测试用 token 无法通过）
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();

        let session = self.init_resume_session(&file_name, parent_id, total_size, token).await?;
        let chunk_size = if session.chunk_size > 0 { session.chunk_size } else { DEFAULT_CHUNK_SIZE };

        let mut file = File::open(file_path).await
            .map_err(|e| AppError::generic(format!("打开文件失败：{e}")))?;
        let mut offset: u64 = 0;
        let mut created_file_id: Option<String> = None;

        while offset < total_size {
            let chunk_len = std::cmp::min(chunk_size, total_size - offset);
            file.seek(SeekFrom::Start(offset)).await
                .map_err(|e| AppError::generic(format!("文件定位失败：{e}")))?;
            let mut chunk = vec![0u8; chunk_len as usize];
            file.read_exact(&mut chunk).await
                .map_err(|e| AppError::generic(format!("读取分片失败：{e}")))?;

            let mut last_err: Option<AppError> = None;
            let mut chunk_result: Option<ChunkResult> = None;
            for attempt in 1..=CHUNK_RETRIES {
                match self.put_chunk(&session, token, &chunk, offset, chunk_len, total_size).await {
                    Ok(r) => { chunk_result = Some(r); break; }
                    Err(e) => {
                        last_err = Some(e);
                        tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                    }
                }
            }
            let cr = chunk_result.ok_or_else(|| last_err.unwrap_or_else(|| AppError::generic("分片上传失败")))?;

            if let Some(ref fid) = cr.created_file_id { created_file_id = Some(fid.clone()); }
            if cr.is_final {
                if let Some(f) = cr.final_file { return Ok(f); }
            }
            offset = if cr.uploaded > offset && cr.uploaded <= total_size {
                cr.uploaded
            } else {
                offset + chunk_len
            };
        }

        if let Some(fid) = created_file_id {
            let resp = self.http.get(format!("{}/files/{fid}", self.drive_base))
                .bearer_auth(token).send().await
                .map_err(|e| AppError::generic(format!("兜底查询文件失败：{e}")))?;
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(f) = DriveFile::from_json(&body) { return Ok(f); }
                }
            }
        }

        // 所有分片已发送但未拿到文件元数据 → 轮询查询上传状态（华为异步合并，详见 query_final_status）。
        if !session.session_url.is_empty() {
            tracing::info!(session_url = %session.session_url, "所有分片已发送，查询上传状态...");
            return self.query_final_status(&session.session_url, token, total_size).await;
        }

        Err(AppError::generic("分片上传完成但未拿到最终文件元数据，请等待下一轮云端轮询自动发现"))
    }

    /// 路由：≤ 20MB → 小文件上传，否则分片续传。
    /// `on_resume_progress`：分片续传进度回调（serverId, uploadId, offset），供断点续传持久化。
    pub async fn upload(
        &self, file_path: &std::path::Path, parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
        on_resume_progress: Option<&ResumeProgressFn>,
    ) -> AppResult<DriveFile> {
        let size = file_path.metadata().map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len();
        if size <= SMALL_LARGE_THRESHOLD {
            self.upload_small(file_path, parent_id, on_progress).await
        } else {
            self.upload_resume(file_path, parent_id, None, on_progress, on_resume_progress).await
        }
    }

    /// 更新云端已有文件（PATCH multipart/related，用于冲突解决）。
    /// 对齐 dart `uploadUpdate`：失败则 delete 旧文件 + POST 新建。
    pub async fn upload_update(
        &self, file_id: &str, file_path: &std::path::Path, parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<DriveFile> {
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let boundary = format!("hwcloud_{}", chrono::Utc::now().timestamp_micros());
        let metadata = build_metadata_json(&file_name, parent_id);
        let file_bytes = tokio::fs::read(file_path).await.map_err(|e| AppError::generic(format!("读取文件失败：{e}")))?;
        let body = build_multipart_related(&boundary, metadata.as_bytes(), &file_bytes);
        let token = self.client.auth().ensure_valid_access_token().await?;
        let url = format!("{}/files/{file_id}?uploadType=multipart", self.upload_base);

        // 尝试 PATCH
        let resp = self.http.request(reqwest::Method::PATCH, &url)
            .header(CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
            .header(CONTENT_LENGTH, body.len().to_string())
            .bearer_auth(&token).body(body).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let json: Value = r.json().await.map_err(|e| AppError::generic(format!("解析 PATCH 响应失败：{e}")))?;
                return DriveFile::from_json(&json).ok_or_else(|| AppError::generic("PATCH 响应异常"));
            }
            _ => {
                tracing::warn!("PATCH 更新失败（fileId={file_id}），回退为 delete + POST");
                // 删除旧文件
                let del_url = format!("{}/files/{file_id}", self.drive_base);
                let _ = self.http.delete(&del_url).bearer_auth(&token).send().await;
            }
        }
        // 回退 POST 新建
        self.upload(file_path, parent_id, on_progress, None).await
    }

    /// 小文件 multipart/related 上传。
    pub async fn upload_small(
        &self, file_path: &std::path::Path, parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<DriveFile> {
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let boundary = format!("hwcloud_{}", chrono::Utc::now().timestamp_micros());
        let metadata = build_metadata_json(&file_name, parent_id);
        let file_bytes = tokio::fs::read(file_path).await.map_err(|e| AppError::generic(format!("读取文件失败：{e}")))?;
        let body = build_multipart_related(&boundary, metadata.as_bytes(), &file_bytes);
        let token = self.client.auth().ensure_valid_access_token().await?;
        let url = format!("{}/files?uploadType=multipart", self.upload_base);
        let resp = self.http.post(&url)
            .header(CONTENT_TYPE, format!("multipart/related; boundary={boundary}"))
            .header(CONTENT_LENGTH, body.len().to_string()).bearer_auth(token).body(body)
            .send().await.map_err(|e| AppError::generic(format!("上传请求失败：{e}")))?;
        if !resp.status().is_success() { return Err(crate::drive::client::handle_error_response(resp).await); }
        if let Some(cb) = on_progress { cb(1.0); }
        let body_json: Value = resp.json().await.map_err(|e| AppError::generic(format!("解析上传响应失败：{e}")))?;
        DriveFile::from_json(&body_json).ok_or_else(|| AppError::generic("上传响应异常"))
    }

    /// 大文件 resume 分片上传。
    pub async fn upload_resume(
        &self, file_path: &std::path::Path, parent_id: Option<&str>,
        resume: Option<&ResumeSession>, on_progress: Option<&ProgressFn>,
        on_resume_progress: Option<&ResumeProgressFn>,
    ) -> AppResult<DriveFile> {
        let total_size = file_path.metadata().map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len();
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let token = self.client.auth().ensure_valid_access_token().await?;

        // 1. 初始化 resume 会话
        let session = match resume {
            Some(s) => s.clone(),
            None => match self.init_resume_session(&file_name, parent_id, total_size, &token).await {
                Ok(s) => {
                    // 通知调用方持久化会话信息
                    if let Some(cb) = on_resume_progress {
                        cb(&s.server_id, &s.upload_id, 0);
                    }
                    s
                }
                Err(e) => {
                    let msg = e.to_string();
                    // resume 会话初始化失败 → 回退小文件上传（>20MB 时也会失败，但至少给出清晰错误）
                    tracing::warn!(size = total_size, error = %msg, "resume 会话初始化失败，回退小文件上传");
                    return self.upload_small(file_path, parent_id, on_progress).await;
                }
            },
        };

        // 使用 API 返回的 sliceSize，否则用默认 5MB
        let chunk_size = if session.chunk_size > 0 { session.chunk_size } else { DEFAULT_CHUNK_SIZE };

        // 2. 分片循环
        let mut file = File::open(file_path).await.map_err(|e| AppError::generic(format!("打开文件失败：{e}")))?;
        let mut offset: u64 = 0;
        let mut created_file_id: Option<String> = None;

        while offset < total_size {
            let chunk_len = std::cmp::min(chunk_size, total_size - offset);
            file.seek(SeekFrom::Start(offset)).await.map_err(|e| AppError::generic(format!("文件定位失败：{e}")))?;
            let mut chunk = vec![0u8; chunk_len as usize];
            // 用 read_exact 安全：chunk_len 保证 ≤ 剩余字节
            file.read_exact(&mut chunk).await.map_err(|e| AppError::generic(format!("读取分片失败：{e}")))?;

            // 3 次重试
            let mut last_err: Option<AppError> = None;
            let mut chunk_result: Option<ChunkResult> = None;
            for attempt in 1..=CHUNK_RETRIES {
                match self.put_chunk(&session, &token, &chunk, offset, chunk_len, total_size).await {
                    Ok(r) => {
                        chunk_result = Some(r);
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                    }
                }
            }
            let cr = chunk_result.ok_or_else(|| last_err.unwrap_or_else(|| AppError::generic("分片上传失败")))?;

            // 捕获兜底查询用的 createdFileId
            if let Some(ref fid) = cr.created_file_id { created_file_id = Some(fid.clone()); }

            // 若为最终响应 → 直接返回文件
            if cr.is_final {
                if let Some(f) = cr.final_file {
                    return Ok(f);
                }
            }

            // offset 防御性校验（防服务端回滚或越界）
            if cr.uploaded > offset && cr.uploaded <= total_size {
                offset = cr.uploaded;
            } else {
                offset += chunk_len;
            }
            if let Some(cb) = on_progress {
                cb(offset as f64 / total_size as f64);
            }
            // 通知调用方持久化进度（断点续传）
            if let Some(cb) = on_resume_progress {
                cb(&session.server_id, &session.upload_id, offset);
            }
        }

        // 3. 尾部兜底：用抓到的 fileId 查询元数据。sid 是 resume 会话标识，未必等于 fileId，
        // 因此不能 GET /files/{sid}（可能 404）。没有 fileId 则查询上传状态。
        if let Some(fid) = created_file_id {
            let resp = self.http.get(format!("{}/files/{fid}", self.drive_base))
                .bearer_auth(&token).send().await.map_err(|e| AppError::generic(format!("兜底查询文件失败：{e}")))?;
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(f) = DriveFile::from_json(&body) { return Ok(f); }
                }
            }
        }

        // 所有分片已发送但未拿到文件元数据 → 轮询查询上传状态（华为异步合并，详见 query_final_status）。
        if !session.session_url.is_empty() {
            tracing::info!(session_url = %session.session_url, "所有分片已发送，查询上传状态...");
            return self.query_final_status(&session.session_url, &token, total_size).await;
        }

        Err(AppError::generic(
            "分片上传完成但未拿到最终文件元数据，请等待下一轮云端轮询自动发现"
        ))
    }

    async fn init_resume_session(
        &self, file_name: &str, parent_id: Option<&str>, total_size: u64, token: &str,
    ) -> AppResult<ResumeSession> {
        let metadata = build_metadata_json(file_name, parent_id);
        let resp = self.http.post(format!("{}/files?uploadType=resume", self.upload_base))
            .header("X-Upload-Content-Length", total_size.to_string())
            .header(CONTENT_TYPE, "application/json").bearer_auth(token).body(metadata)
            .send().await.map_err(|e| AppError::generic(format!("初始化上传会话失败：{e}")))?;
        if !resp.status().is_success() { return Err(crate::drive::client::handle_error_response(resp).await); }
        let status_code = resp.status().as_u16();

        // ★ 关键：从 Location 响应头获取会话 URL（Google Drive 风格断点续传）。
        // 华为 API 变更后 body 仅含 {"sliceSize":...}，不含 serverId/uploadId，
        // 后续分片 PUT 必须直接用 Location 头返回的 URL。
        let session_url = resp.headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_default();

        let init: Value = resp.json().await.map_err(|e| AppError::generic(format!("解析上传会话响应失败：{e}")))?;
        tracing::info!(status = status_code, has_location = !session_url.is_empty(), response = %init, file = %file_name, size = total_size, "resume 会话初始化响应");

        // 解析 body 中的标识字段（旧 API 兼容）
        let server_id = init.get("serverId")
            .or_else(|| init.get("id"))
            .or_else(|| init.get("fileId"))
            .and_then(Value::as_str)
            .map(String::from);

        let upload_id = init.get("uploadId").and_then(Value::as_str).unwrap_or("").to_string();

        // API 建议的分片大小（新 API 仅返回 sliceSize，如 10485760 = 10MB）
        let chunk_size = init.get("sliceSize")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // 有 session_url（Location 头）→ 后续 PUT 直接用它；没有 → 用 serverId/uploadId 拼接
        let server_id = match server_id {
            Some(sid) => sid,
            None if session_url.is_empty() => {
                let keys: Vec<&str> = init.as_object().map(|o| o.keys().map(|s| s.as_str()).collect()).unwrap_or_default();
                tracing::error!(response = %init, keys = ?keys, "上传会话响应缺少 serverId 且无 Location 头");
                return Err(AppError::generic(format!("上传会话响应缺少 serverId（可用字段: {keys:?}）")));
            }
            None => {
                // 有 Location 头但没有 body serverId → 用空字符串（后续 PUT 走 session_url）
                String::new()
            }
        };

        Ok(ResumeSession { server_id, upload_id, session_url, chunk_size })
    }

    /// PUT 单个分片。返回 ChunkResult：
    /// - 分片已接收 → `uploaded` 为华为返回的已上传偏移，`created_file_id` 可能已在响应中出现
    /// - 最终响应（body 含 id + fileName/size）→ `is_final=true, final_file=Some`
    ///
    /// URL 优先级：
    /// 1. session_url（Location 头返回的会话 URL）—— 华为 API 变更后必须走此路径
    /// 2. server_id + upload_id 拼接（旧 API 兼容）
    ///
    /// HTTP 状态码处理（Google Drive 断点续传协议）：
    /// - 200/201 → 可能为最终响应（含文件元数据）或中间响应（`{"size":...}`）
    /// - 308 Resume Incomplete → 中间响应，body 含 `rangeList` 标识已接收的字节范围
    async fn put_chunk(
        &self, session: &ResumeSession, token: &str, chunk: &[u8],
        offset: u64, chunk_len: u64, total_size: u64,
    ) -> AppResult<ChunkResult> {
        let url = if !session.session_url.is_empty() {
            // 华为 API 变更后：直接用 init 响应 Location 头返回的会话 URL
            session.session_url.clone()
        } else if session.server_id.is_empty() {
            format!("{}/files?uploadId={}", self.upload_base, session.upload_id)
        } else {
            format!("{}/files/{}?uploadId={}", self.upload_base, session.server_id, session.upload_id)
        };
        let end = offset + chunk_len - 1;
        let content_range = format!("bytes {offset}-{end}/{total_size}");
        let resp = self.http.put(&url).header(CONTENT_RANGE, &content_range)
            .header(CONTENT_LENGTH, chunk_len.to_string())
            .header(CONTENT_TYPE, "application/octet-stream").bearer_auth(token).body(chunk.to_vec())
            .send().await.map_err(|e| AppError::generic(format!("分片 PUT 失败：{e}")))?;

        let status = resp.status().as_u16();

        // ★ HTTP 308 Resume Incomplete：Google Drive 风格断点续传中间响应。
        // 华为 API 返回 body 含 rangeList（如 ["0-10485759"]），标识已确认接收的字节范围。
        // 客户端应从 rangeList 末尾 + 1 继续下一分片，而非重试同一分片。
        if status == 308 {
            let body: Value = resp.json().await
                .map_err(|e| AppError::generic(format!("解析 308 分片响应失败：{e}")))?;
            let uploaded = parse_uploaded_from_range_list(&body, offset + chunk_len);
            tracing::debug!(offset, chunk_len, uploaded, range_list = ?body.get("rangeList"), "分片已接收 (308)");
            return Ok(ChunkResult { uploaded, created_file_id: None, is_final: false, final_file: None });
        }

        if !resp.status().is_success() { return Err(crate::drive::client::handle_error_response(resp).await); }
        let body: Value = resp.json().await.map_err(|e| AppError::generic(format!("解析分片响应失败：{e}")))?;

        // 华为返回：中间片返回 {"size": <已上传字节数>}，最后一片返回完整文件元数据
        let created_file_id = body.get("id").or_else(|| body.get("fileId")).and_then(Value::as_str).map(String::from);

        // 判断是否为最终响应（含文件元数据，有 id 且 fileName 或 size）
        if created_file_id.is_some() && (body.get("fileName").is_some() || body.get("size").is_some()) {
            if let Some(drive_file) = DriveFile::from_json(&body) {
                return Ok(ChunkResult {
                    uploaded: total_size,
                    created_file_id: Some(drive_file.id.clone()),
                    is_final: true,
                    final_file: Some(drive_file),
                });
            }
        }

        // 中间分片：获取华为返回的已上传偏移，防御性校验在外层
        let uploaded = body.get("size").and_then(|v| v.as_u64()).unwrap_or(offset + chunk_len);
        Ok(ChunkResult { uploaded, created_file_id, is_final: false, final_file: None })
    }

    async fn ensure_capacity_for(&self, file_path: &std::path::Path) -> AppResult<()> {
        let size = file_path.metadata().map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len() as i64;
        crate::drive::about_api::AboutApi::new(self.client.clone()).ensure_capacity(size).await
    }

    /// 所有分片发完后的最终状态查询（轮询重试）。
    ///
    /// 华为云盘大文件上传的最终合并是**异步**的：所有分片网络层已发出，
    /// 但服务端合并确认有延迟，立即查询常返回 308（Resume Incomplete）。
    /// 本方法循环查询（默认 5 次，间隔 3 秒），期间复用同一 session，**不重传数据**：
    /// - 2xx：服务端合并完成，返回文件元数据
    /// - 308：服务端仍在合并，等待后重试
    /// - 其他/请求失败：记录后继续（不立即放弃，给服务端更多时间）
    ///
    /// 全部轮询后仍无元数据 → Err（保持兜底语义，靠下轮云端轮询发现）
    async fn query_final_status(
        &self,
        session_url: &str,
        token: &str,
        total_size: u64,
    ) -> AppResult<DriveFile> {
        for attempt in 1..=FINAL_STATUS_MAX_POLLS {
            let status_resp = self
                .http
                .put(session_url)
                .header(CONTENT_RANGE, format!("bytes */{total_size}"))
                .header(CONTENT_LENGTH, "0")
                .bearer_auth(token)
                .send()
                .await;
            match status_resp {
                Ok(r) if r.status().is_success() => {
                    // 2xx：服务端合并完成
                    if let Ok(body) = r.json::<Value>().await {
                        tracing::info!(response = %body, "上传状态查询成功（第 {attempt} 次）");
                        if let Some(f) = DriveFile::from_json(&body) {
                            return Ok(f);
                        }
                    }
                    // 2xx 但无元数据：罕见，继续重试
                    tracing::warn!("上传状态查询 2xx 但无文件元数据，继续重试（第 {attempt} 次）");
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    let body = r.text().await.unwrap_or_default();
                    // 308 = Resume Incomplete：服务端仍在异步合并，正常现象，等待重试
                    if attempt < FINAL_STATUS_MAX_POLLS {
                        tracing::warn!(
                            attempt, max = FINAL_STATUS_MAX_POLLS, status, body = %body,
                            "上传状态查询返回非 2xx（服务端异步合并中），等待重试"
                        );
                    } else {
                        tracing::warn!(status, body = %body, "上传状态查询返回非 2xx（已到最大重试）");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "上传状态查询请求失败，继续重试");
                }
            }
            // 未成功且未到最后一次 → 等待后重试
            if attempt < FINAL_STATUS_MAX_POLLS {
                tokio::time::sleep(Duration::from_secs(FINAL_STATUS_POLL_INTERVAL_SECS)).await;
            }
        }
        Err(AppError::generic(
            "分片上传完成但未拿到最终文件元数据，请等待下一轮云端轮询自动发现",
        ))
    }
}

/// 从 308 Resume Incomplete 响应的 rangeList 解析已上传偏移。
///
/// rangeList 格式：`["0-10485759"]`（可能多个范围）。
/// 返回值 = 最后一个范围的 end + 1（即下一字节偏移），解析失败回退到 fallback。
fn parse_uploaded_from_range_list(body: &Value, fallback: u64) -> u64 {
    body.get("rangeList")
        .and_then(|v| v.as_array())
        .and_then(|ranges| ranges.last())
        .and_then(|r| r.as_str())
        .and_then(|s| {
            // 格式："0-10485759" → 取 `-` 右侧 + 1
            s.split('-').nth(1)
                .and_then(|end_str| end_str.parse::<u64>().ok())
                .map(|end| end + 1)
        })
        .unwrap_or(fallback)
}

/// 构造 metadata JSON（multipart 路径用普通 JSON，容忍 UTF-8，不需 asciiJsonEncode）。
fn build_metadata_json(file_name: &str, parent_id: Option<&str>) -> String {
    let mut meta = serde_json::Map::new();
    meta.insert("fileName".into(), Value::String(file_name.to_string()));
    if let Some(pid) = parent_id { if !pid.is_empty() { meta.insert("parentFolder".into(), Value::Array(vec![Value::String(pid.to_string())])); } }
    Value::Object(meta).to_string()
}

/// 构造 multipart/related body（对齐 dart `_buildMultipartRelated`）。
fn build_multipart_related(boundary: &str, metadata: &[u8], file_bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_json_utf8_preserved() {
        let meta = build_metadata_json("报告.txt", Some("parent-1"));
        assert!(meta.contains("报告.txt"));
        assert!(meta.contains("parent-1"));
    }

    #[test]
    fn test_metadata_json_root_no_parent() {
        let meta = build_metadata_json("file.txt", None);
        assert!(!meta.contains("parentFolder"));
    }

    #[test]
    fn test_multipart_related_structure() {
        let boundary = "test-boundary";
        let body = build_multipart_related(boundary, br#"{"fileName":"f.txt"}"#, b"file-content");
        let body_str = String::from_utf8_lossy(&body);
        assert_eq!(body_str.matches("--test-boundary").count(), 3);
        assert!(body_str.contains("application/json; charset=UTF-8"));
        assert!(body_str.contains(r#"{"fileName":"f.txt"}"#));
        assert!(body_str.contains("application/octet-stream"));
        assert!(body_str.contains("file-content"));
        assert!(body_str.ends_with("--test-boundary--\r\n"));
    }

    #[test]
    fn test_thresholds() {
        assert_eq!(SMALL_LARGE_THRESHOLD, 20 * 1024 * 1024);
        assert_eq!(DEFAULT_CHUNK_SIZE, 5 * 1024 * 1024);
        assert_eq!(CHUNK_RETRIES, 3);
    }

    // ── wiremock 集成测试：验证 Location 头捕获与分片 PUT 流程 ──

    /// 构造一个用于测试的 UploadApi（指向 mock server，绕过真实 auth）。
    fn test_api(base_url: &str) -> UploadApi {
        let auth = Arc::new(crate::auth::service::AuthService::new());
        let client = Arc::new(DriveClient::with_base_url(auth, base_url.to_string()));
        UploadApi::with_base_urls(client, base_url.to_string(), base_url.to_string())
    }

    /// 验证：init_resume_session 从 Location 响应头提取 session_url，
    /// 并从 body 提取 sliceSize（华为 API 变更后仅返回 sliceSize）。
    #[tokio::test]
    async fn test_init_resume_session_captures_location_header() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();

        // Mock: POST /files?uploadType=resume → 200 + Location 头 + body 仅含 sliceSize
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/files"))
            .and(wiremock::matchers::query_param("uploadType", "resume"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .append_header("Location", format!("{base}/upload/drive/v1/files?uploadId=mock-session-1"))
                    .set_body_json(serde_json::json!({"sliceSize": 10485760}))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = api.init_resume_session("large_file.bin", None, 30_000_000, "fake-token")
            .await
            .expect("init_resume_session 应成功");

        // ★ 核心断言：session_url 从 Location 头捕获
        assert!(!session.session_url.is_empty(), "session_url 应从 Location 头捕获");
        assert!(session.session_url.contains("uploadId=mock-session-1"),
            "session_url 应包含 Location 头中的 uploadId");

        // sliceSize 从 body 提取
        assert_eq!(session.chunk_size, 10485760, "sliceSize 应从 body 提取");

        // body 无 serverId/id/fileId → server_id 为空（后续 PUT 走 session_url）
        assert!(session.server_id.is_empty(),
            "body 仅含 sliceSize 时 server_id 应为空");
    }

    /// 验证：init_resume_session 在没有 Location 头但有 serverId 时回退到旧逻辑。
    #[tokio::test]
    async fn test_init_resume_session_falls_back_to_server_id() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();

        // Mock: POST → 200 含 serverId/uploadId，无 Location 头（旧 API）
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/files"))
            .and(wiremock::matchers::query_param("uploadType", "resume"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "serverId": "old-server-123",
                        "uploadId": "old-upload-456",
                        "sliceSize": 5242880
                    }))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = api.init_resume_session("file.bin", None, 10_000_000, "fake-token")
            .await
            .expect("旧 API 格式也应成功");

        // 无 Location 头 → session_url 为空
        assert!(session.session_url.is_empty(), "无 Location 头时 session_url 应为空");
        // serverId/uploadId 从 body 提取
        assert_eq!(session.server_id, "old-server-123");
        assert_eq!(session.upload_id, "old-upload-456");
        assert_eq!(session.chunk_size, 5242880);
    }

    /// 验证：put_chunk 在有 session_url 时优先使用 session_url 而非拼接。
    #[tokio::test]
    async fn test_put_chunk_uses_session_url() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();
        let session_url = format!("{base}/upload/drive/v1/files?uploadId=mock-session-put");

        // Mock: PUT 到 session URL → 200 + 中间分片响应
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "mock-session-put"))
            .and(wiremock::matchers::header("Content-Range", "bytes 0-4999999/20000000"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"size": 5000000}))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url: session_url.clone(),  // ← 走 session_url 路径
            chunk_size: 5_000_000,
        };

        let chunk = vec![0u8; 5_000_000];
        let result = api.put_chunk(&session, "fake-token", &chunk, 0, 5_000_000, 20_000_000)
            .await
            .expect("put_chunk 应成功");

        assert_eq!(result.uploaded, 5_000_000);
        assert!(!result.is_final);
    }

    /// 验证：put_chunk 在无 session_url 时使用 server_id/upload_id 拼接（旧 API 兼容）。
    #[tokio::test]
    async fn test_put_chunk_falls_back_to_server_id_url() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();

        // Mock: PUT 到 /files/{serverId}?uploadId={uploadId} 路径
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/files/old-sid"))
            .and(wiremock::matchers::query_param("uploadId", "old-uid"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"size": 3000000}))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = ResumeSession {
            server_id: "old-sid".to_string(),
            upload_id: "old-uid".to_string(),
            session_url: String::new(),  // ← 无 session_url，走旧路径
            chunk_size: 3_000_000,
        };

        let chunk = vec![0u8; 3_000_000];
        let result = api.put_chunk(&session, "fake-token", &chunk, 0, 3_000_000, 10_000_000)
            .await
            .expect("旧 URL 拼接 put_chunk 应成功");

        assert_eq!(result.uploaded, 3_000_000);
    }

    /// 验证：put_chunk 最终分片返回完整文件元数据（is_final=true）。
    #[tokio::test]
    async fn test_put_chunk_final_returns_drive_file() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();

        // Mock: PUT 最终分片 → 200 + 完整文件 JSON
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/files"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "id": "final-file-id",
                        "fileName": "uploaded.bin",
                        "mimeType": "application/octet-stream",
                        "size": 22000000
                    }))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url: format!("{base}/files"),
            chunk_size: 0,
        };

        let chunk = vec![0u8; 2_000_000];
        let result = api.put_chunk(&session, "fake-token", &chunk, 20_000_000, 2_000_000, 22_000_000)
            .await
            .expect("最终分片 put_chunk 应成功");

        assert!(result.is_final, "最后一片应为 final");
        assert!(result.final_file.is_some());
        let f = result.final_file.unwrap();
        assert_eq!(f.id, "final-file-id");
        assert_eq!(f.name, "uploaded.bin");
        assert_eq!(f.size, 22000000);
    }

    /// 单元测试：parse_uploaded_from_range_list 解析 rangeList。
    #[test]
    fn test_parse_uploaded_from_range_list() {
        // 标准格式：单范围
        let body = serde_json::json!({"sliceSize": 10485760, "rangeList": ["0-10485759"], "processTime": 8000});
        assert_eq!(parse_uploaded_from_range_list(&body, 0), 10485760);

        // 多范围 → 取最后一个
        let body = serde_json::json!({"rangeList": ["0-10485759", "10485760-20971519"]});
        assert_eq!(parse_uploaded_from_range_list(&body, 0), 20971520);

        // 空 rangeList → fallback
        let body = serde_json::json!({"rangeList": []});
        assert_eq!(parse_uploaded_from_range_list(&body, 999), 999);

        // 无 rangeList → fallback
        let body = serde_json::json!({"size": 5000000});
        assert_eq!(parse_uploaded_from_range_list(&body, 5000000), 5000000);

        // 非字符串元素 → fallback
        let body = serde_json::json!({"rangeList": [123]});
        assert_eq!(parse_uploaded_from_range_list(&body, 777), 777);
    }

    /// 验证：put_chunk 正确处理 HTTP 308 Resume Incomplete 响应（华为真实行为）。
    /// 308 表示分片已接收但上传未完成，body 含 rangeList 标识已确认的字节范围。
    #[tokio::test]
    async fn test_put_chunk_handles_308_resume_incomplete() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();
        let session_url = format!("{base}/upload/drive/v1/files?uploadId=mock-308");

        // Mock: PUT 分片 → 308 + rangeList（华为真实响应格式）
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "mock-308"))
            .respond_with(
                wiremock::ResponseTemplate::new(308)
                    .set_body_json(serde_json::json!({
                        "sliceSize": 10485760,
                        "rangeList": ["0-10485759"],
                        "processTime": 8000
                    }))
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url,
            chunk_size: 10_485_760,
        };

        // 发送第一个分片 bytes 0-10485759/30000000
        let chunk = vec![0u8; 10_485_760];
        let result = api.put_chunk(&session, "fake-token", &chunk, 0, 10_485_760, 30_000_000)
            .await
            .expect("308 应被视为成功响应");

        // ★ 308 应返回 is_final=false，uploaded 应从 rangeList 解析（10485760）
        assert!(!result.is_final, "308 应非最终响应");
        assert!(result.final_file.is_none());
        assert_eq!(result.uploaded, 10485760, "uploaded 应从 rangeList 解析: 10485759+1");
    }

    /// 端到端测试：>20MB 文件通过 resume 分片上传，模拟真实 308 流程。
    /// 流程：about 配额校验 → init（Location 头）→ PUT 片1 (308) → PUT 片2 (308) → PUT 片3 (200 final)
    #[tokio::test]
    async fn test_upload_resume_over_20mb_end_to_end() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();
        let session_url = format!("{base}/upload/drive/v1/files?uploadId=e2e-session");
        let chunk_sz = 10_485_760u64; // 10MB sliceSize

        // 0. Mock about 配额接口（ensure_capacity_for 调用）
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/about"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "storageQuota": {
                            "userCapacity": "107374182400",
                            "usedSpace": "0"
                        },
                        "user": { "displayName": "测试用户" }
                    }))
            )
            .mount(&server)
            .await;

        // 1. Mock init resume 会话（Location 头 + sliceSize）
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/files"))
            .and(wiremock::matchers::query_param("uploadType", "resume"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .append_header("Location", &session_url)
                    .set_body_json(serde_json::json!({"sliceSize": chunk_sz}))
            )
            .mount(&server)
            .await;

        // 2. Mock PUT 分片 1: 308 + rangeList ["0-10485759"]
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header("Content-Range", format!("bytes 0-{}/22000000", chunk_sz - 1).as_str()))
            .respond_with(
                wiremock::ResponseTemplate::new(308)
                    .set_body_json(serde_json::json!({
                        "sliceSize": chunk_sz,
                        "rangeList": [format!("0-{}", chunk_sz - 1)],
                        "processTime": 8000
                    }))
            )
            .mount(&server)
            .await;

        // 3. Mock PUT 分片 2: 308 + rangeList ["0-20971519"]（累计）
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header("Content-Range", format!("bytes {}-{}/22000000", chunk_sz, chunk_sz * 2 - 1).as_str()))
            .respond_with(
                wiremock::ResponseTemplate::new(308)
                    .set_body_json(serde_json::json!({
                        "sliceSize": chunk_sz,
                        "rangeList": [format!("0-{}", chunk_sz * 2 - 1)],
                        "processTime": 7000
                    }))
            )
            .mount(&server)
            .await;

        // 4. Mock PUT 分片 3（最终）: 200 + 完整文件元数据
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header("Content-Range", format!("bytes {}-21999999/22000000", chunk_sz * 2).as_str()))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({
                        "id": "e2e-file-id",
                        "fileName": "e2e_large_file.bin",
                        "mimeType": "application/octet-stream",
                        "size": 22000000
                    }))
            )
            .mount(&server)
            .await;

        // 5. 创建 sparse 22MB 文件（不实际占用磁盘空间）
        let tmpdir = tempfile::tempdir().expect("创建临时目录失败");
        let file_path = tmpdir.path().join("e2e_large_file.bin");
        {
            let f = std::fs::File::create(&file_path).expect("创建文件失败");
            f.set_len(22_000_000).expect("set_len 失败"); // 22MB sparse
        }

        let api = test_api(&base);
        let token = "e2e-fake-token".to_string();

        // 手动走 upload_resume 流程（避免 AuthService 依赖）
        let total_size = 22_000_000u64;
        let session = api.init_resume_session("e2e_large_file.bin", None, total_size, &token)
            .await
            .expect("init 应成功");

        // ★ 验证 Location 头被捕获
        assert!(!session.session_url.is_empty(), "Location 头应被捕获");
        assert_eq!(session.session_url, session_url);
        assert_eq!(session.chunk_size, chunk_sz);

        let chunk_size = if session.chunk_size > 0 { session.chunk_size } else { DEFAULT_CHUNK_SIZE };
        assert_eq!(chunk_size, 10485760);

        // 模拟分片循环（3 片：10MB + 10MB + 2MB）
        let mut offset = 0u64;
        let mut final_file: Option<DriveFile> = None;
        // 跟踪每片的预期 308/200 状态
        let expected_statuses = vec![(308, false), (308, false), (200, true)];
        for (i, (expected_status, expected_final)) in expected_statuses.iter().enumerate() {
            let cl = std::cmp::min(chunk_size, total_size - offset);
            let chunk = vec![0u8; cl as usize];
            let result = api.put_chunk(&session, &token, &chunk, offset, cl, total_size)
                .await
                .expect(&format!("分片 {i} put_chunk 应成功"));

            assert_eq!(result.is_final, *expected_final,
                "分片 {i}: is_final 应为 {expected_final}（预期 HTTP {expected_status}）");

            if result.is_final {
                final_file = result.final_file;
                break;
            }
            // 从 308 rangeList 或 200 size 更新 offset
            let next_offset = if result.uploaded > offset && result.uploaded <= total_size {
                result.uploaded
            } else {
                offset + cl
            };
            assert!(next_offset > offset, "分片 {i}: offset 应前进");
            offset = next_offset;
        }

        // ★ 端到端成功：拿到了最终文件元数据
        assert!(final_file.is_some(), "应拿到最终文件元数据");
        let f = final_file.unwrap();
        assert_eq!(f.id, "e2e-file-id");
        assert_eq!(f.name, "e2e_large_file.bin");
        assert_eq!(f.size, 22000000);
    }
}
