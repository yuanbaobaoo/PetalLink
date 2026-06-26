//! Upload API —— 小文件 multipart/related + 大文件 5MB 分片断点续传 + 更新覆盖。
//!
//! 对齐 `legacy/lib/drive/api/upload_api.dart`。
//!
//! # 小文件（≤ 20MB）：multipart/related（Google Drive 风格）
//! # 大文件（> 20MB）：resume 分片（F-FILE-02）
//! # uploadUpdate：PATCH 覆盖已有文件（冲突解决），失败回退 delete+POST

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE};
use serde_json::Value;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

use crate::constants;
use crate::drive::client::DriveClient;
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

const SMALL_LARGE_THRESHOLD: u64 = 20 * 1024 * 1024;
const CHUNK_SIZE: u64 = 5 * 1024 * 1024;
const CHUNK_RETRIES: u32 = 3;

pub struct UploadApi {
    client: Arc<DriveClient>,
    http: reqwest::Client,
}

pub type ProgressFn = Box<dyn Fn(f64) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ResumeSession {
    pub server_id: String,
    pub upload_id: String,
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
        Self { client, http }
    }

    /// 路由：≤ 20MB → 小文件上传，否则分片续传。
    /// 对齐 dart `upload` dispatcher。
    pub async fn upload(
        &self, file_path: &std::path::Path, parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<DriveFile> {
        let size = file_path.metadata().map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len();
        if size <= SMALL_LARGE_THRESHOLD {
            self.upload_small(file_path, parent_id, on_progress).await
        } else {
            self.upload_resume(file_path, parent_id, None, on_progress).await
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
        let url = format!("{}/files/{file_id}?uploadType=multipart", constants::UPLOAD_API_BASE);

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
                let del_url = format!("{}/files/{file_id}", constants::DRIVE_API_BASE);
                let _ = self.http.delete(&del_url).bearer_auth(&token).send().await;
            }
        }
        // 回退 POST 新建
        self.upload(file_path, parent_id, on_progress).await
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
        let url = format!("{}/files?uploadType=multipart", constants::UPLOAD_API_BASE);
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
    ) -> AppResult<DriveFile> {
        let total_size = file_path.metadata().map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?.len();
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let token = self.client.auth().ensure_valid_access_token().await?;

        // 1. 初始化 resume 会话
        let session = match resume {
            Some(s) => s.clone(),
            None => self.init_resume_session(&file_name, parent_id, total_size, &token).await?,
        };

        // 2. 分片循环
        let mut file = File::open(file_path).await.map_err(|e| AppError::generic(format!("打开文件失败：{e}")))?;
        let mut offset: u64 = 0;
        let mut created_file_id: Option<String> = None;

        while offset < total_size {
            let chunk_len = std::cmp::min(CHUNK_SIZE, total_size - offset);
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
        }

        // 3. 尾部兜底：用抓到的 fileId 查询元数据。sid 是 resume 会话标识，未必等于 fileId，
        // 因此不能 GET /files/{sid}（可能 404）。没有 fileId 则抛出（等待下一轮云端轮询自动发现）。
        if let Some(fid) = created_file_id {
            let resp = self.http.get(format!("{}/files/{fid}", constants::DRIVE_API_BASE))
                .bearer_auth(&token).send().await.map_err(|e| AppError::generic(format!("兜底查询文件失败：{e}")))?;
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(f) = DriveFile::from_json(&body) { return Ok(f); }
                }
            }
        }
        Err(AppError::generic(
            "分片上传完成但未拿到最终文件元数据，请等待下一轮云端轮询自动发现"
        ))
    }

    async fn init_resume_session(
        &self, file_name: &str, parent_id: Option<&str>, total_size: u64, token: &str,
    ) -> AppResult<ResumeSession> {
        let metadata = build_metadata_json(file_name, parent_id);
        let resp = self.http.post(format!("{}/files?uploadType=resume", constants::UPLOAD_API_BASE))
            .header("X-Upload-Content-Length", total_size.to_string())
            .header(CONTENT_TYPE, "application/json").bearer_auth(token).body(metadata)
            .send().await.map_err(|e| AppError::generic(format!("初始化上传会话失败：{e}")))?;
        if !resp.status().is_success() { return Err(crate::drive::client::handle_error_response(resp).await); }
        let init: Value = resp.json().await.map_err(|e| AppError::generic(format!("解析上传会话响应失败：{e}")))?;
        let server_id = init.get("serverId").or_else(|| init.get("id")).and_then(Value::as_str)
            .ok_or_else(|| AppError::generic("上传会话响应缺少 serverId"))?.to_string();
        let upload_id = init.get("uploadId").and_then(Value::as_str).unwrap_or("").to_string();
        Ok(ResumeSession { server_id, upload_id })
    }

    /// PUT 单个分片。返回 ChunkResult：
    /// - 分片已接收 → `uploaded` 为华为返回的已上传偏移，`created_file_id` 可能已在响应中出现
    /// - 最终响应（body 含 id + fileName/size）→ `is_final=true, final_file=Some`
    async fn put_chunk(
        &self, session: &ResumeSession, token: &str, chunk: &[u8],
        offset: u64, chunk_len: u64, total_size: u64,
    ) -> AppResult<ChunkResult> {
        let url = format!("{}/files/{}?uploadId={}", constants::UPLOAD_API_BASE, session.server_id, session.upload_id);
        let end = offset + chunk_len - 1;
        let content_range = format!("bytes {offset}-{end}/{total_size}");
        let resp = self.http.put(&url).header(CONTENT_RANGE, &content_range)
            .header(CONTENT_LENGTH, chunk_len.to_string())
            .header(CONTENT_TYPE, "application/octet-stream").bearer_auth(token).body(chunk.to_vec())
            .send().await.map_err(|e| AppError::generic(format!("分片 PUT 失败：{e}")))?;
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
        assert_eq!(CHUNK_SIZE, 5 * 1024 * 1024);
        assert_eq!(CHUNK_RETRIES, 3);
    }
}
