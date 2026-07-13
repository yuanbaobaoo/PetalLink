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

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION, RETRY_AFTER};
use reqwest::StatusCode;
use serde_json::Value;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

use crate::constants;
use crate::drive::client::DriveClient;
use crate::drive::models::DriveFile;
use crate::error::{parse_retry_after, AppError, AppResult, DriveTransportKind, RequestSemantics};

pub const SAFE_EXISTING_UPDATE_MAX_BYTES: u64 = 20 * 1024 * 1024;
const SMALL_LARGE_THRESHOLD: u64 = SAFE_EXISTING_UPDATE_MAX_BYTES;
/// 华为官方 SDK 的默认/最小分片，以及 REST 接口允许的单片上限。
const MIN_CHUNK_SIZE: u64 = 256 * 1024;
const DEFAULT_CHUNK_SIZE: u64 = 2 * 1024 * 1024;
const MAX_CHUNK_SIZE: u64 = 64 * 1024 * 1024;
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
}

pub type ProgressFn = Box<dyn Fn(f64) + Send + Sync>;
/// 断点续传进度回调：server_id, upload_id, 已上传字节偏移, session_url
/// session_url 为华为 resume 上传 Location 头返回的会话 URL（断点续传唯一 token）。
pub type ResumeProgressFn = Box<dyn Fn(&str, &str, u64, &str) + Send + Sync>;

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

impl UploadApi {
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

    /// 测试用：注入自定义 base URL（如 wiremock 地址）。
    #[cfg(test)]
    pub fn with_base_urls(
        client: Arc<DriveClient>,
        upload_base: String,
        _drive_base: String,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("构建 reqwest client 失败");
        Self {
            client,
            http,
            upload_base,
        }
    }

    /// 直接传入 token 进行 resume 分片上传（绕过 AuthService，供测试/调试用）。
    pub async fn upload_resume_with_token(
        &self,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        token: &str,
    ) -> AppResult<DriveFile> {
        let total_size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len();
        // 跳过 ensure_capacity_for（其内部依赖 AuthService，测试用 token 无法通过）
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let session = self
            .init_resume_session(&file_name, parent_id, total_size, token)
            .await?;
        self.upload_resume_session(file_path, session, token.to_string(), false, None, None)
            .await
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

    /// 更新云端已有文件（PATCH multipart/related，用于冲突解决）。
    /// PATCH 失败必须保留旧文件，并把错误返回给用户处理。
    pub async fn upload_update(
        &self,
        file_id: &str,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<DriveFile> {
        reject_unsafe_large_update(file_id, file_path)?;
        self.ensure_capacity_for(file_path).await?;
        let token = self.client.auth().ensure_valid_access_token().await?;
        self.upload_update_with_token(file_id, file_path, parent_id, on_progress, &token)
            .await
    }

    async fn upload_update_with_token(
        &self,
        file_id: &str,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
        token: &str,
    ) -> AppResult<DriveFile> {
        let size = reject_unsafe_large_update(file_id, file_path)?;
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let boundary = format!("hwcloud_{}", chrono::Utc::now().timestamp_micros());
        let metadata = build_metadata_json(&file_name, parent_id);
        let file_bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| AppError::generic(format!("读取文件失败：{e}")))?;
        let body = build_multipart_related(&boundary, metadata.as_bytes(), &file_bytes);
        let url = format!("{}/files/{file_id}?uploadType=multipart", self.upload_base);

        let (resp, auth_replayed) = self
            .send_multipart_with_auth_replay(reqwest::Method::PATCH, &url, &boundary, &body, token)
            .await?;
        if !resp.status().is_success() {
            tracing::warn!(
                file_id,
                status = resp.status().as_u16(),
                "PATCH 更新失败，保留云端旧文件"
            );
            return Err(crate::drive::client::handle_error_response_with_metadata(
                resp,
                RequestSemantics::Write,
                auth_replayed,
            )
            .await);
        }
        let json: Value = resp.json().await.map_err(|e| {
            crate::drive::client::response_decode_error(
                "PATCH 更新",
                RequestSemantics::Write,
                auth_replayed,
                &e.to_string(),
            )
        })?;
        if let Some(cb) = on_progress {
            cb(1.0);
        }
        complete_upload_file(&json, size, Some(&file_name)).ok_or_else(|| {
            remote_ambiguity(
                "PATCH 更新返回 2xx，但文件身份/名称/长度不完整或不匹配",
                auth_replayed,
            )
        })
    }

    /// 小文件 multipart/related 上传。
    pub async fn upload_small(
        &self,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<DriveFile> {
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let boundary = format!("hwcloud_{}", chrono::Utc::now().timestamp_micros());
        let metadata = build_metadata_json(&file_name, parent_id);
        let file_bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| AppError::generic(format!("读取文件失败：{e}")))?;
        let body = build_multipart_related(&boundary, metadata.as_bytes(), &file_bytes);
        let token = self.client.auth().ensure_valid_access_token().await?;
        let url = format!("{}/files?uploadType=multipart", self.upload_base);
        let (resp, auth_replayed) = self
            .send_multipart_with_auth_replay(reqwest::Method::POST, &url, &boundary, &body, &token)
            .await?;
        if !resp.status().is_success() {
            return Err(crate::drive::client::handle_error_response_with_metadata(
                resp,
                RequestSemantics::Write,
                auth_replayed,
            )
            .await);
        }
        if let Some(cb) = on_progress {
            cb(1.0);
        }
        let body_json: Value = resp.json().await.map_err(|e| {
            crate::drive::client::response_decode_error(
                "小文件上传",
                RequestSemantics::Write,
                auth_replayed,
                &e.to_string(),
            )
        })?;
        complete_upload_file(&body_json, file_bytes.len() as u64, Some(&file_name)).ok_or_else(
            || {
                remote_ambiguity(
                    "小文件上传返回 2xx，但文件身份/名称/长度不完整或不匹配",
                    auth_replayed,
                )
            },
        )
    }

    /// A 401 proves the request was rejected before the write was authorized, so replaying the
    /// identical method/URL/body once with a refreshed token is safe. Every other uncertain write
    /// error is returned without blind replay for TaskRunner verification.
    async fn send_multipart_with_auth_replay(
        &self,
        method: reqwest::Method,
        url: &str,
        boundary: &str,
        body: &[u8],
        token: &str,
    ) -> AppResult<(reqwest::Response, bool)> {
        let send = |token: &str| {
            self.http
                .request(method.clone(), url)
                .header(
                    CONTENT_TYPE,
                    format!("multipart/related; boundary={boundary}"),
                )
                .header(CONTENT_LENGTH, body.len().to_string())
                .bearer_auth(token)
                .body(body.to_vec())
        };
        let response = send(token).send().await.map_err(|error| {
            crate::drive::client::classify_transport_error(&error, RequestSemantics::Write, false)
        })?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok((response, false));
        }
        let refreshed = self.client.auth().refresher().refresh().await?;
        let response = send(&refreshed.access_token)
            .send()
            .await
            .map_err(|error| {
                crate::drive::client::classify_transport_error(
                    &error,
                    RequestSemantics::Write,
                    true,
                )
            })?;
        Ok((response, true))
    }

    /// 大文件 resume 分片上传。
    pub async fn upload_resume(
        &self,
        file_path: &std::path::Path,
        parent_id: Option<&str>,
        resume: Option<&ResumeSession>,
        on_progress: Option<&ProgressFn>,
        on_resume_progress: Option<&ResumeProgressFn>,
    ) -> AppResult<DriveFile> {
        let total_size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len();
        self.ensure_capacity_for(file_path).await?;
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let mut token = self.client.auth().ensure_valid_access_token().await?;

        // 1. 初始化或恢复 resume 会话。已有会话必须先向服务端查询确认偏移。
        let (session, verify_server_offset) = match resume {
            Some(s) => (s.clone(), true),
            None => {
                let initialized = match self
                    .init_resume_session(&file_name, parent_id, total_size, &token)
                    .await
                {
                    Err(error) if error.drive_status() == Some(401) => {
                        let refreshed = self.client.auth().refresher().refresh().await?;
                        token = refreshed.access_token;
                        self.init_resume_session(&file_name, parent_id, total_size, &token)
                            .await
                    }
                    result => result,
                };
                match initialized {
                    Ok(s) => {
                        // 通知调用方持久化会话信息（含 session_url，断点续传必需）
                        if let Some(cb) = on_resume_progress {
                            cb(&s.server_id, &s.upload_id, 0, &s.session_url);
                        }
                        (s, false)
                    }
                    Err(e) => {
                        // 初始化 POST 的响应可能已丢失，绝不能回退为另一个 create。
                        tracing::warn!(size = total_size, error = %e, "resume 会话初始化失败，保留结构化错误并停止新建重放");
                        return Err(e);
                    }
                }
            }
        };
        self.upload_resume_session(
            file_path,
            session,
            token,
            verify_server_offset,
            on_progress,
            on_resume_progress,
        )
        .await
    }

    /// 使用一个已初始化的会话上传。恢复会话时，`start_offset` 只是本地提示；真正起点
    /// 必须由同一 session URL 的状态查询确认，避免断网窗口把未收到的分片误算成功。
    async fn upload_resume_session(
        &self,
        file_path: &std::path::Path,
        mut session: ResumeSession,
        mut token: String,
        verify_server_offset: bool,
        on_progress: Option<&ProgressFn>,
        on_resume_progress: Option<&ResumeProgressFn>,
    ) -> AppResult<DriveFile> {
        let total_size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len();
        let mut file = File::open(file_path)
            .await
            .map_err(|e| AppError::generic(format!("打开文件失败：{e}")))?;

        let mut offset = if verify_server_offset {
            let observed = self
                .query_session_status(&mut session, &mut token, total_size)
                .await?;
            if let Some(file) = observed.final_file {
                return Ok(file);
            }
            observed.uploaded
        } else {
            0
        };

        if offset > total_size {
            return Err(remote_ambiguity(
                &format!("服务端断点偏移 {offset} 超过本地文件长度 {total_size}"),
                false,
            ));
        }
        notify_resume_progress(
            &session,
            offset,
            total_size,
            on_progress,
            on_resume_progress,
        );

        let mut final_status_polls = 0;
        loop {
            if offset < total_size {
                final_status_polls = 0;
                let chunk_size = validated_chunk_size(session.chunk_size)?;
                let chunk_len = std::cmp::min(chunk_size, total_size - offset);
                file.seek(SeekFrom::Start(offset))
                    .await
                    .map_err(|e| AppError::generic(format!("文件定位失败：{e}")))?;
                let mut chunk = vec![0u8; chunk_len as usize];
                file.read_exact(&mut chunk)
                    .await
                    .map_err(|e| AppError::generic(format!("读取分片失败：{e}")))?;

                let mut last_err: Option<AppError> = None;
                let mut chunk_result: Option<ChunkResult> = None;
                for attempt in 1..=CHUNK_RETRIES {
                    match self
                        .put_chunk(
                            &mut session,
                            &mut token,
                            &chunk,
                            offset,
                            chunk_len,
                            total_size,
                        )
                        .await
                    {
                        Ok(result) => {
                            chunk_result = Some(result);
                            break;
                        }
                        Err(error) => {
                            // 308/状态查询可能已轮换 Location；即使随后解析失败，也先持久化
                            // 当前会话 URL，避免恢复时回到已失效的旧地址。
                            notify_resume_progress(
                                &session,
                                offset,
                                total_size,
                                on_progress,
                                on_resume_progress,
                            );
                            let retry_locally =
                                attempt < CHUNK_RETRIES && should_retry_chunk_locally(&error);
                            last_err = Some(error);
                            if retry_locally {
                                tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                            } else {
                                break;
                            }
                        }
                    }
                }
                let result = chunk_result
                    .ok_or_else(|| last_err.unwrap_or_else(|| AppError::generic("分片上传失败")))?;
                if let Some(file) = result.final_file {
                    return Ok(file);
                }
                if result.uploaded > total_size {
                    return Err(remote_ambiguity(
                        &format!(
                            "服务端确认偏移 {} 超过本地文件长度 {total_size}",
                            result.uploaded
                        ),
                        false,
                    ));
                }
                if result.uploaded == offset {
                    return Err(remote_ambiguity(
                        "服务端状态查询未确认当前分片，停止本地偏移推进",
                        false,
                    ));
                }
                offset = result.uploaded;
                notify_resume_progress(
                    &session,
                    offset,
                    total_size,
                    on_progress,
                    on_resume_progress,
                );
                continue;
            }

            // 数据范围已全部确认，但只有最终 200 + 完整 File 才能结算完成。
            final_status_polls += 1;
            match self
                .query_session_status(&mut session, &mut token, total_size)
                .await
            {
                Ok(result) => {
                    if let Some(file) = result.final_file {
                        return Ok(file);
                    }
                    if result.uploaded < total_size {
                        offset = result.uploaded;
                        notify_resume_progress(
                            &session,
                            offset,
                            total_size,
                            on_progress,
                            on_resume_progress,
                        );
                        continue;
                    }
                    if final_status_polls < FINAL_STATUS_MAX_POLLS {
                        let wait_ms = result
                            .process_time_ms
                            .unwrap_or(FINAL_STATUS_POLL_INTERVAL_SECS * 1_000)
                            .clamp(250, FINAL_STATUS_POLL_INTERVAL_SECS * 1_000);
                        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                        continue;
                    }
                }
                Err(error) if is_remote_ambiguity(&error) => {
                    notify_resume_progress(
                        &session,
                        offset,
                        total_size,
                        on_progress,
                        on_resume_progress,
                    );
                    return Err(error);
                }
                Err(error) if auth_already_replayed(&error) => return Err(error),
                Err(error) if final_status_polls < FINAL_STATUS_MAX_POLLS => {
                    notify_resume_progress(
                        &session,
                        offset,
                        total_size,
                        on_progress,
                        on_resume_progress,
                    );
                    tracing::warn!(%error, final_status_polls, "最终上传状态查询失败，继续查询同一会话");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                Err(error) => {
                    notify_resume_progress(
                        &session,
                        offset,
                        total_size,
                        on_progress,
                        on_resume_progress,
                    );
                    tracing::warn!(%error, "最终上传状态仍不确定，交由任务层远端核验");
                    return Err(remote_ambiguity(
                        &format!("最终上传状态查询失败：{error}"),
                        auth_already_replayed(&error),
                    ));
                }
            }

            return Err(remote_ambiguity(
                "所有字节已由服务端确认，但未返回最终文件元数据",
                false,
            ));
        }
    }

    async fn init_resume_session(
        &self,
        file_name: &str,
        parent_id: Option<&str>,
        total_size: u64,
        token: &str,
    ) -> AppResult<ResumeSession> {
        let metadata = build_metadata_json(file_name, parent_id);
        let resp = self
            .http
            .post(format!("{}/files?uploadType=resume", self.upload_base))
            .header("X-Upload-Content-Length", total_size.to_string())
            .header(CONTENT_TYPE, "application/json")
            .bearer_auth(token)
            .body(metadata)
            .send()
            .await
            .map_err(|e| {
                crate::drive::client::classify_transport_error(&e, RequestSemantics::Write, false)
            })?;
        if !resp.status().is_success() {
            return Err(crate::drive::client::handle_error_response_with_metadata(
                resp,
                RequestSemantics::Write,
                false,
            )
            .await);
        }
        let status_code = resp.status().as_u16();

        // ★ 关键：从 Location 响应头获取会话 URL（Google Drive 风格断点续传）。
        // 华为 API 变更后 body 仅含 {"sliceSize":...}，不含 serverId/uploadId，
        // 后续分片 PUT 必须直接用 Location 头返回的 URL。
        let session_url = resp
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_default();

        let init: Value = resp.json().await.map_err(|e| {
            crate::drive::client::response_decode_error(
                "上传会话初始化",
                RequestSemantics::Write,
                false,
                &e.to_string(),
            )
        })?;
        tracing::info!(status = status_code, has_location = !session_url.is_empty(), response = %init, file = %file_name, size = total_size, "resume 会话初始化响应");

        // 解析 body 中的标识字段（旧 API 兼容）
        let server_id = init
            .get("serverId")
            .or_else(|| init.get("id"))
            .or_else(|| init.get("fileId"))
            .and_then(Value::as_str)
            .map(String::from);

        let upload_id = init
            .get("uploadId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // API 建议的分片大小（新 API 仅返回 sliceSize，如 10485760 = 10MB）
        let chunk_size = init.get("sliceSize").and_then(|v| v.as_u64()).unwrap_or(0);

        // 有 session_url（Location 头）→ 后续 PUT 直接用它；没有 → 用 serverId/uploadId 拼接
        let server_id = match server_id {
            Some(sid) => sid,
            None if session_url.is_empty() => {
                let keys: Vec<&str> = init
                    .as_object()
                    .map(|o| o.keys().map(|s| s.as_str()).collect())
                    .unwrap_or_default();
                tracing::error!(response = %init, keys = ?keys, "上传会话响应缺少 serverId 且无 Location 头");
                return Err(remote_ambiguity(
                    &format!("上传会话响应缺少 Location/serverId（可用字段: {keys:?}）"),
                    false,
                ));
            }
            None => {
                // 有 Location 头但没有 body serverId → 用空字符串（后续 PUT 走 session_url）
                String::new()
            }
        };

        Ok(ResumeSession {
            server_id,
            upload_id,
            session_url,
            chunk_size,
            start_offset: 0,
        })
    }

    /// PUT 单个分片。401 只刷新并重放一次，且 URL/body/Content-Range 完全不变。
    /// 对请求阶段不确定、5xx 或成功响应无法解析的情况，不按本地长度猜偏移，先查询
    /// 同一会话的服务端状态。
    async fn put_chunk(
        &self,
        session: &mut ResumeSession,
        token: &mut String,
        chunk: &[u8],
        offset: u64,
        chunk_len: u64,
        total_size: u64,
    ) -> AppResult<ChunkResult> {
        let Some(chunk_end_exclusive) = offset.checked_add(chunk_len) else {
            return Err(AppError::generic("上传分片边界溢出"));
        };
        if chunk_len == 0 || chunk.len() as u64 != chunk_len || chunk_end_exclusive > total_size {
            return Err(AppError::generic("非法上传分片边界"));
        }
        let url = self.session_request_url(session)?;
        let end = offset + chunk_len - 1;
        let content_range = format!("bytes {offset}-{end}/{total_size}");
        let mut auth_replayed = false;
        let mut response = match self
            .send_chunk_request(&url, token, &content_range, chunk)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let original = crate::drive::client::classify_transport_error(
                    &error,
                    RequestSemantics::Write,
                    false,
                );
                return self
                    .reconcile_uncertain_chunk(session, token, total_size, offset, original)
                    .await;
            }
        };

        if response.status() == StatusCode::UNAUTHORIZED {
            let refreshed = self.client.auth().refresher().refresh().await?;
            *token = refreshed.access_token;
            auth_replayed = true;
            response = match self
                .send_chunk_request(&url, token, &content_range, chunk)
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let original = crate::drive::client::classify_transport_error(
                        &error,
                        RequestSemantics::Write,
                        true,
                    );
                    return self
                        .reconcile_uncertain_chunk(session, token, total_size, offset, original)
                        .await;
                }
            };
        }

        self.update_session_location(session, &response)?;
        let status = response.status();
        if status.as_u16() == 308 {
            let body = response.json::<Value>().await.map_err(|error| {
                remote_ambiguity(&format!("308 分片响应无法解析：{error}"), auth_replayed)
            })?;
            self.update_session_chunk_size(session, &body)?;
            let uploaded = parse_confirmed_offset(&body, total_size)?;
            if uploaded <= offset {
                return Err(remote_ambiguity(
                    &format!("308 未确认当前分片：本地起点 {offset}，服务端确认偏移 {uploaded}"),
                    auth_replayed,
                ));
            }
            tracing::debug!(offset, chunk_len, uploaded, range_list = ?body.get("rangeList"), "分片已由服务端确认 (308)");
            return Ok(incomplete_result(&body, uploaded));
        }

        if !status.is_success() {
            let should_query = status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT;
            let original =
                upload_response_error(response, RequestSemantics::Write, auth_replayed, true).await;
            if should_query {
                return self
                    .reconcile_uncertain_chunk(session, token, total_size, offset, original)
                    .await;
            }
            return Err(original);
        }

        let body = match response.json::<Value>().await {
            Ok(body) => body,
            Err(error) => {
                let original =
                    remote_ambiguity(&format!("分片成功响应无法解析：{error}"), auth_replayed);
                return self
                    .reconcile_uncertain_chunk(session, token, total_size, offset, original)
                    .await;
            }
        };
        if let Some(file) = complete_upload_file(&body, total_size, None) {
            return Ok(ChunkResult {
                uploaded: total_size,
                is_final: true,
                final_file: Some(file),
                process_time_ms: None,
            });
        }

        // 兼容旧接口的中间 2xx `size`，但只信任服务端显式数值，绝不本地相加。
        if let Some(uploaded) = body.get("size").and_then(Value::as_u64) {
            if uploaded <= total_size && uploaded > offset {
                return Ok(ChunkResult {
                    uploaded,
                    is_final: false,
                    final_file: None,
                    process_time_ms: body.get("processTime").and_then(Value::as_u64),
                });
            }
        }

        let original = remote_ambiguity(
            "分片返回 2xx，但既无完整 File 也无有效服务端确认偏移",
            auth_replayed,
        );
        self.reconcile_uncertain_chunk(session, token, total_size, offset, original)
            .await
    }

    async fn send_chunk_request(
        &self,
        url: &str,
        token: &str,
        content_range: &str,
        chunk: &[u8],
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.http
            .put(url)
            .header(CONTENT_RANGE, content_range)
            .header(CONTENT_LENGTH, chunk.len().to_string())
            .header(CONTENT_TYPE, "application/octet-stream")
            .bearer_auth(token)
            .body(chunk.to_vec())
            .send()
            .await
    }

    async fn reconcile_uncertain_chunk(
        &self,
        session: &mut ResumeSession,
        token: &mut String,
        total_size: u64,
        previous_offset: u64,
        original: AppError,
    ) -> AppResult<ChunkResult> {
        match self.query_session_status(session, token, total_size).await {
            Ok(result) if result.is_final || result.uploaded != previous_offset => Ok(result),
            Ok(_) => Err(original),
            Err(query_error) if is_remote_ambiguity(&query_error) => Err(query_error),
            Err(query_error) => {
                tracing::warn!(%query_error, "分片结果不确定且会话查询失败，保留原始写入歧义");
                Err(original)
            }
        }
    }

    /// 对同一 session 发零长度状态查询；查询本身是只读语义，401 也只重放一次。
    async fn query_session_status(
        &self,
        session: &mut ResumeSession,
        token: &mut String,
        total_size: u64,
    ) -> AppResult<ChunkResult> {
        let url = self.session_request_url(session)?;
        let content_range = format!("bytes */{total_size}");
        let mut auth_replayed = false;
        let mut response = self
            .send_status_request(&url, token, &content_range)
            .await
            .map_err(|error| {
                crate::drive::client::classify_transport_error(
                    &error,
                    RequestSemantics::Read,
                    false,
                )
            })?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let refreshed = self.client.auth().refresher().refresh().await?;
            *token = refreshed.access_token;
            auth_replayed = true;
            response = self
                .send_status_request(&url, token, &content_range)
                .await
                .map_err(|error| {
                    crate::drive::client::classify_transport_error(
                        &error,
                        RequestSemantics::Read,
                        true,
                    )
                })?;
        }

        self.update_session_location(session, &response)?;
        let status = response.status();
        if status.as_u16() == 308 {
            let body = response.json::<Value>().await.map_err(|error| {
                remote_ambiguity(
                    &format!("上传状态 308 响应无法解析：{error}"),
                    auth_replayed,
                )
            })?;
            self.update_session_chunk_size(session, &body)?;
            let uploaded = parse_confirmed_offset(&body, total_size)?;
            return Ok(incomplete_result(&body, uploaded));
        }

        if !status.is_success() {
            return Err(upload_response_error(
                response,
                RequestSemantics::Read,
                auth_replayed,
                true,
            )
            .await);
        }

        let body = response.json::<Value>().await.map_err(|error| {
            remote_ambiguity(&format!("上传状态成功响应无法解析：{error}"), auth_replayed)
        })?;
        if let Some(file) = complete_upload_file(&body, total_size, None) {
            return Ok(ChunkResult {
                uploaded: total_size,
                is_final: true,
                final_file: Some(file),
                process_time_ms: None,
            });
        }
        if let Some(uploaded) = body.get("size").and_then(Value::as_u64) {
            if uploaded <= total_size {
                return Ok(ChunkResult {
                    uploaded,
                    is_final: false,
                    final_file: None,
                    process_time_ms: body.get("processTime").and_then(Value::as_u64),
                });
            }
        }
        Err(remote_ambiguity(
            "上传状态返回 2xx，但缺少完整 File 或有效服务端确认偏移",
            auth_replayed,
        ))
    }

    async fn send_status_request(
        &self,
        url: &str,
        token: &str,
        content_range: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.http
            .put(url)
            .header(CONTENT_RANGE, content_range)
            .header(CONTENT_LENGTH, "0")
            .bearer_auth(token)
            .body(Vec::<u8>::new())
            .send()
            .await
    }

    fn session_request_url(&self, session: &ResumeSession) -> AppResult<String> {
        if !session.session_url.is_empty() {
            return Ok(session.session_url.clone());
        }
        if !session.server_id.is_empty() && !session.upload_id.is_empty() {
            return Ok(format!(
                "{}/files/{}?uploadId={}",
                self.upload_base, session.server_id, session.upload_id
            ));
        }
        Err(remote_ambiguity(
            "断点上传会话缺少 Location，也缺少完整 serverId/uploadId",
            false,
        ))
    }

    fn update_session_location(
        &self,
        session: &mut ResumeSession,
        response: &reqwest::Response,
    ) -> AppResult<()> {
        let Some(location) = response.headers().get(LOCATION) else {
            return Ok(());
        };
        let location = location.to_str().map_err(|error| {
            remote_ambiguity(&format!("上传响应 Location 不是有效文本：{error}"), false)
        })?;
        if location.trim().is_empty() {
            return Err(remote_ambiguity("上传响应返回空 Location", false));
        }
        session.session_url = location.to_string();
        Ok(())
    }

    fn update_session_chunk_size(
        &self,
        session: &mut ResumeSession,
        body: &Value,
    ) -> AppResult<()> {
        if let Some(slice_size) = body.get("sliceSize").and_then(Value::as_u64) {
            validated_chunk_size(slice_size)?;
            session.chunk_size = slice_size;
        }
        Ok(())
    }

    async fn ensure_capacity_for(&self, file_path: &std::path::Path) -> AppResult<()> {
        let size = file_path
            .metadata()
            .map_err(|e| AppError::generic(format!("读取文件元数据失败：{e}")))?
            .len() as i64;
        crate::drive::about_api::AboutApi::new(self.client.clone())
            .ensure_capacity(size)
            .await
    }
}

fn incomplete_result(body: &Value, uploaded: u64) -> ChunkResult {
    ChunkResult {
        uploaded,
        is_final: false,
        final_file: None,
        process_time_ms: body.get("processTime").and_then(Value::as_u64),
    }
}

fn reject_unsafe_large_update(file_id: &str, file_path: &std::path::Path) -> AppResult<u64> {
    let size = file_path
        .metadata()
        .map_err(|error| AppError::generic(format!("读取文件元数据失败：{error}")))?
        .len();
    if size > SMALL_LARGE_THRESHOLD {
        return Err(AppError::generic(format!(
            "现有云端文件更新大小超过 20 MiB，当前不支持安全的 resumable update；已保留原文件，禁止回退为新建（fileId={file_id}）"
        )));
    }
    Ok(size)
}

fn complete_upload_file(
    body: &Value,
    expected_size: u64,
    expected_name: Option<&str>,
) -> Option<DriveFile> {
    let file = DriveFile::from_json(body)?;
    let size_matches = file.size >= 0 && file.size as u64 == expected_size;
    let name_matches = !file.name.trim().is_empty()
        && expected_name.map_or(true, |expected| file.name.as_str() == expected);
    (!file.id.trim().is_empty() && size_matches && name_matches).then_some(file)
}

/// 只接受从 0 开始、连续、无重叠且不越界的已接收范围。空数组明确表示 0；
/// 缺字段、hole、重叠或格式异常都不能回退成本地 `offset + chunk_len`。
fn parse_confirmed_offset(body: &Value, total_size: u64) -> AppResult<u64> {
    let ranges = body
        .get("rangeList")
        .and_then(Value::as_array)
        .ok_or_else(|| remote_ambiguity("308 响应缺少 rangeList", false))?;
    if ranges.is_empty() {
        return Ok(0);
    }

    let mut expected_start = 0_u64;
    for range in ranges {
        let raw = range
            .as_str()
            .ok_or_else(|| remote_ambiguity("rangeList 含非字符串元素", false))?;
        let (start, end) = raw
            .split_once('-')
            .ok_or_else(|| remote_ambiguity(&format!("非法上传范围：{raw}"), false))?;
        if end.contains('-') {
            return Err(remote_ambiguity(&format!("非法上传范围：{raw}"), false));
        }
        let start = start
            .parse::<u64>()
            .map_err(|_| remote_ambiguity(&format!("非法上传范围起点：{raw}"), false))?;
        let end = end
            .parse::<u64>()
            .map_err(|_| remote_ambiguity(&format!("非法上传范围终点：{raw}"), false))?;
        if start != expected_start || end < start || end >= total_size {
            return Err(remote_ambiguity(
                &format!(
                    "上传范围不连续或越界：{raw}，期望起点 {expected_start}，总长度 {total_size}"
                ),
                false,
            ));
        }
        expected_start = end
            .checked_add(1)
            .ok_or_else(|| remote_ambiguity("上传范围终点溢出", false))?;
    }
    Ok(expected_start)
}

fn validated_chunk_size(chunk_size: u64) -> AppResult<u64> {
    let chunk_size = if chunk_size == 0 {
        DEFAULT_CHUNK_SIZE
    } else {
        chunk_size
    };
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
        return Err(remote_ambiguity(
            &format!(
                "服务端分片大小 {chunk_size} 不在允许范围 {MIN_CHUNK_SIZE}..={MAX_CHUNK_SIZE}"
            ),
            false,
        ));
    }
    Ok(chunk_size)
}

fn notify_resume_progress(
    session: &ResumeSession,
    offset: u64,
    total_size: u64,
    on_progress: Option<&ProgressFn>,
    on_resume_progress: Option<&ResumeProgressFn>,
) {
    if let Some(callback) = on_progress {
        let ratio = if total_size == 0 {
            1.0
        } else {
            offset as f64 / total_size as f64
        };
        callback(ratio.clamp(0.0, 1.0));
    }
    if let Some(callback) = on_resume_progress {
        callback(
            &session.server_id,
            &session.upload_id,
            offset,
            &session.session_url,
        );
    }
}

fn remote_ambiguity(cause: &str, auth_already_replayed: bool) -> AppError {
    AppError::drive_transport_with_submission(
        DriveTransportKind::Decode,
        true,
        auth_already_replayed,
        Some(cause),
    )
}

fn is_remote_ambiguity(error: &AppError) -> bool {
    match error {
        AppError::DriveApi {
            request_may_have_reached_server: true,
            transport_kind: Some(_),
            ..
        } => true,
        AppError::DriveApi {
            error_code: Some(error_code),
            ..
        } => error_code == "upload_session_expired",
        _ => false,
    }
}

/// 只在请求明确未到服务端的连接失败上做短暂本地重试。401 已在 `put_chunk` 内完成
/// 唯一一次刷新重放；任何可能已提交的写入都必须交回持久化恢复策略。
fn should_retry_chunk_locally(error: &AppError) -> bool {
    matches!(
        error,
        AppError::DriveApi {
            status_code: None,
            transport_kind: Some(DriveTransportKind::Connect),
            request_may_have_reached_server: false,
            ..
        }
    )
}

fn auth_already_replayed(error: &AppError) -> bool {
    matches!(
        error,
        AppError::DriveApi {
            auth_already_replayed: true,
            ..
        }
    )
}

async fn upload_response_error(
    response: reqwest::Response,
    semantics: RequestSemantics,
    auth_already_replayed: bool,
    session_sensitive: bool,
) -> AppError {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    let body = response.text().await.unwrap_or_default();
    let upper = body.to_ascii_uppercase();
    if session_sensitive
        && (status == 404
            || status == 410
            || upper.contains("CONTENT_NOT_FOUND")
            || upper.contains("UPLOAD_ID_NOT_FOUND"))
    {
        return AppError::drive_upload_session_expired(status, auth_already_replayed);
    }
    AppError::drive_from_response(status, &body, retry_after, semantics, auth_already_replayed)
}

/// 构造 metadata JSON（multipart 路径用普通 JSON，容忍 UTF-8，不需 asciiJsonEncode）。
fn build_metadata_json(file_name: &str, parent_id: Option<&str>) -> String {
    let mut meta = serde_json::Map::new();
    meta.insert("fileName".into(), Value::String(file_name.to_string()));
    if let Some(pid) = parent_id {
        if !pid.is_empty() {
            meta.insert(
                "parentFolder".into(),
                Value::Array(vec![Value::String(pid.to_string())]),
            );
        }
    }
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
        assert_eq!(DEFAULT_CHUNK_SIZE, 2 * 1024 * 1024);
        assert_eq!(MIN_CHUNK_SIZE, 256 * 1024);
        assert_eq!(MAX_CHUNK_SIZE, 64 * 1024 * 1024);
        assert_eq!(CHUNK_RETRIES, 3);
    }

    /// PATCH 覆盖失败必须保留云端旧文件，不能再 delete + POST。
    #[tokio::test]
    async fn test_upload_update_patch_failure_preserves_existing_file() {
        let server = wiremock::MockServer::start().await;
        let base = server.uri();

        wiremock::Mock::given(wiremock::matchers::method("PATCH"))
            .and(wiremock::matchers::path("/files/old-file"))
            .and(wiremock::matchers::query_param("uploadType", "multipart"))
            .respond_with(
                wiremock::ResponseTemplate::new(500)
                    .set_body_json(serde_json::json!({"error": "patch failed"})),
            )
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("DELETE"))
            .and(wiremock::matchers::path("/files/old-file"))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/files"))
            .and(wiremock::matchers::query_param("uploadType", "multipart"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "replacement-file",
                    "fileName": "replacement.txt",
                    "size": 11
                })),
            )
            .mount(&server)
            .await;

        let tmpdir = tempfile::tempdir().expect("创建临时目录失败");
        let file_path = tmpdir.path().join("replacement.txt");
        tokio::fs::write(&file_path, b"replacement")
            .await
            .expect("写测试文件失败");

        let api = test_api(&base);
        let result = api
            .upload_update_with_token("old-file", &file_path, None, None, "fake-token")
            .await;

        assert!(
            result.is_err(),
            "PATCH 失败应返回错误，而不是静默重建新文件"
        );
        let requests = server
            .received_requests()
            .await
            .expect("mock server 应记录请求");
        let delete_count = requests
            .iter()
            .filter(|r| r.method.as_str() == "DELETE" && r.url.path() == "/files/old-file")
            .count();
        let post_count = requests
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/files")
            .count();
        assert_eq!(delete_count, 0, "PATCH 失败不得删除旧云端文件");
        assert_eq!(post_count, 0, "PATCH 失败不得回退 POST 新建文件");
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
                    .append_header(
                        "Location",
                        format!("{base}/upload/drive/v1/files?uploadId=mock-session-1"),
                    )
                    .set_body_json(serde_json::json!({"sliceSize": 10485760})),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = api
            .init_resume_session("large_file.bin", None, 30_000_000, "fake-token")
            .await
            .expect("init_resume_session 应成功");

        // ★ 核心断言：session_url 从 Location 头捕获
        assert!(
            !session.session_url.is_empty(),
            "session_url 应从 Location 头捕获"
        );
        assert!(
            session.session_url.contains("uploadId=mock-session-1"),
            "session_url 应包含 Location 头中的 uploadId"
        );

        // sliceSize 从 body 提取
        assert_eq!(session.chunk_size, 10485760, "sliceSize 应从 body 提取");

        // body 无 serverId/id/fileId → server_id 为空（后续 PUT 走 session_url）
        assert!(
            session.server_id.is_empty(),
            "body 仅含 sliceSize 时 server_id 应为空"
        );
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
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "serverId": "old-server-123",
                    "uploadId": "old-upload-456",
                    "sliceSize": 5242880
                })),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let session = api
            .init_resume_session("file.bin", None, 10_000_000, "fake-token")
            .await
            .expect("旧 API 格式也应成功");

        // 无 Location 头 → session_url 为空
        assert!(
            session.session_url.is_empty(),
            "无 Location 头时 session_url 应为空"
        );
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
            .and(wiremock::matchers::query_param(
                "uploadId",
                "mock-session-put",
            ))
            .and(wiremock::matchers::header(
                "Content-Range",
                "bytes 0-4999999/20000000",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"size": 5000000})),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let mut session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url: session_url.clone(), // ← 走 session_url 路径
            chunk_size: 5_000_000,
            start_offset: 0,
        };

        let chunk = vec![0u8; 5_000_000];
        let mut token = "fake-token".to_string();
        let result = api
            .put_chunk(&mut session, &mut token, &chunk, 0, 5_000_000, 20_000_000)
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
                    .set_body_json(serde_json::json!({"size": 3000000})),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let mut session = ResumeSession {
            server_id: "old-sid".to_string(),
            upload_id: "old-uid".to_string(),
            session_url: String::new(), // ← 无 session_url，走旧路径
            chunk_size: 3_000_000,
            start_offset: 0,
        };

        let chunk = vec![0u8; 3_000_000];
        let mut token = "fake-token".to_string();
        let result = api
            .put_chunk(&mut session, &mut token, &chunk, 0, 3_000_000, 10_000_000)
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
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "final-file-id",
                    "fileName": "uploaded.bin",
                    "mimeType": "application/octet-stream",
                    "size": 22000000
                })),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let mut session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url: format!("{base}/files"),
            chunk_size: 0,
            start_offset: 0,
        };

        let chunk = vec![0u8; 2_000_000];
        let mut token = "fake-token".to_string();
        let result = api
            .put_chunk(
                &mut session,
                &mut token,
                &chunk,
                20_000_000,
                2_000_000,
                22_000_000,
            )
            .await
            .expect("最终分片 put_chunk 应成功");

        assert!(result.is_final, "最后一片应为 final");
        assert!(result.final_file.is_some());
        let f = result.final_file.unwrap();
        assert_eq!(f.id, "final-file-id");
        assert_eq!(f.name, "uploaded.bin");
        assert_eq!(f.size, 22000000);
    }

    /// 单元测试：严格解析连续 rangeList，不再使用本地 fallback。
    #[test]
    fn test_parse_confirmed_offset() {
        // 标准格式：单范围
        let body = serde_json::json!({"sliceSize": 10485760, "rangeList": ["0-10485759"], "processTime": 8000});
        assert_eq!(parse_confirmed_offset(&body, 30_000_000).unwrap(), 10485760);

        // 多范围必须连续
        let body = serde_json::json!({"rangeList": ["0-10485759", "10485760-20971519"]});
        assert_eq!(parse_confirmed_offset(&body, 30_000_000).unwrap(), 20971520);

        // 空 rangeList 明确表示服务端尚未确认字节。
        let body = serde_json::json!({"rangeList": []});
        assert_eq!(parse_confirmed_offset(&body, 30_000_000).unwrap(), 0);

        // 缺失或非法范围必须失败，禁止本地猜偏移。
        let body = serde_json::json!({"size": 5000000});
        assert!(parse_confirmed_offset(&body, 30_000_000).is_err());

        let body = serde_json::json!({"rangeList": [123]});
        assert!(parse_confirmed_offset(&body, 30_000_000).is_err());

        let body = serde_json::json!({"rangeList": ["0-9", "20-29"]});
        assert!(parse_confirmed_offset(&body, 30_000_000).is_err());
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
                wiremock::ResponseTemplate::new(308).set_body_json(serde_json::json!({
                    "sliceSize": 10485760,
                    "rangeList": ["0-10485759"],
                    "processTime": 8000
                })),
            )
            .mount(&server)
            .await;

        let api = test_api(&base);
        let mut session = ResumeSession {
            server_id: String::new(),
            upload_id: String::new(),
            session_url,
            chunk_size: 10_485_760,
            start_offset: 0,
        };

        // 发送第一个分片 bytes 0-10485759/30000000
        let chunk = vec![0u8; 10_485_760];
        let mut token = "fake-token".to_string();
        let result = api
            .put_chunk(&mut session, &mut token, &chunk, 0, 10_485_760, 30_000_000)
            .await
            .expect("308 应被视为成功响应");

        // ★ 308 应返回 is_final=false，uploaded 应从 rangeList 解析（10485760）
        assert!(!result.is_final, "308 应非最终响应");
        assert!(result.final_file.is_none());
        assert_eq!(
            result.uploaded, 10485760,
            "uploaded 应从 rangeList 解析: 10485759+1"
        );
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
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "storageQuota": {
                        "userCapacity": "107374182400",
                        "usedSpace": "0"
                    },
                    "user": { "displayName": "测试用户" }
                })),
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
                    .set_body_json(serde_json::json!({"sliceSize": chunk_sz})),
            )
            .mount(&server)
            .await;

        // 2. Mock PUT 分片 1: 308 + rangeList ["0-10485759"]
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header(
                "Content-Range",
                format!("bytes 0-{}/22000000", chunk_sz - 1).as_str(),
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(308).set_body_json(serde_json::json!({
                    "sliceSize": chunk_sz,
                    "rangeList": [format!("0-{}", chunk_sz - 1)],
                    "processTime": 8000
                })),
            )
            .mount(&server)
            .await;

        // 3. Mock PUT 分片 2: 308 + rangeList ["0-20971519"]（累计）
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header(
                "Content-Range",
                format!("bytes {}-{}/22000000", chunk_sz, chunk_sz * 2 - 1).as_str(),
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(308).set_body_json(serde_json::json!({
                    "sliceSize": chunk_sz,
                    "rangeList": [format!("0-{}", chunk_sz * 2 - 1)],
                    "processTime": 7000
                })),
            )
            .mount(&server)
            .await;

        // 4. Mock PUT 分片 3（最终）: 200 + 完整文件元数据
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .and(wiremock::matchers::path("/upload/drive/v1/files"))
            .and(wiremock::matchers::query_param("uploadId", "e2e-session"))
            .and(wiremock::matchers::header(
                "Content-Range",
                format!("bytes {}-21999999/22000000", chunk_sz * 2).as_str(),
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "e2e-file-id",
                    "fileName": "e2e_large_file.bin",
                    "mimeType": "application/octet-stream",
                    "size": 22000000
                })),
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
        let mut token = "e2e-fake-token".to_string();

        // 手动走 upload_resume 流程（避免 AuthService 依赖）
        let total_size = 22_000_000u64;
        let mut session = api
            .init_resume_session("e2e_large_file.bin", None, total_size, &token)
            .await
            .expect("init 应成功");

        // ★ 验证 Location 头被捕获
        assert!(!session.session_url.is_empty(), "Location 头应被捕获");
        assert_eq!(session.session_url, session_url);
        assert_eq!(session.chunk_size, chunk_sz);

        let chunk_size = if session.chunk_size > 0 {
            session.chunk_size
        } else {
            DEFAULT_CHUNK_SIZE
        };
        assert_eq!(chunk_size, 10485760);

        // 模拟分片循环（3 片：10MB + 10MB + 2MB）
        let mut offset = 0u64;
        let mut final_file: Option<DriveFile> = None;
        // 跟踪每片的预期 308/200 状态
        let expected_statuses = [(308, false), (308, false), (200, true)];
        for (i, (expected_status, expected_final)) in expected_statuses.iter().enumerate() {
            let cl = std::cmp::min(chunk_size, total_size - offset);
            let chunk = vec![0u8; cl as usize];
            let result = api
                .put_chunk(&mut session, &mut token, &chunk, offset, cl, total_size)
                .await
                .unwrap_or_else(|_| panic!("分片 {i} put_chunk 应成功"));

            assert_eq!(
                result.is_final, *expected_final,
                "分片 {i}: is_final 应为 {expected_final}（预期 HTTP {expected_status}）"
            );

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
