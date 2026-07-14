//! 单分片提交、服务端确认偏移与断点会话查询。

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE};
use reqwest::StatusCode;
use serde_json::Value;

use crate::error::{AppError, AppResult, RequestSemantics};

use super::protocol::{
    complete_upload_file, is_remote_ambiguity, parse_confirmed_offset, remote_ambiguity,
    upload_response_error,
};
use super::{ChunkResult, ResumeSession, UploadApi};

impl UploadApi {
    /// PUT 单个分片。401 只刷新并重放一次，且 URL/body/Content-Range 完全不变。
    /// 对请求阶段不确定、5xx 或成功响应无法解析的情况，不按本地长度猜偏移，先查询
    /// 同一会话的服务端状态。
    pub(super) async fn put_chunk(
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

    /// 原样发送一次分片 PUT，不在此层自动重试。
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

    /// 通过同一会话查询收敛不确定写入，未推进时保留原错误。
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
    pub(super) async fn query_session_status(
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

    /// 发送零长度会话状态 PUT，不在此层自动重试。
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
}

/// 根据服务端确认偏移构造未完成结果。
fn incomplete_result(body: &Value, uploaded: u64) -> ChunkResult {
    ChunkResult {
        uploaded,
        is_final: false,
        final_file: None,
        process_time_ms: body.get("processTime").and_then(Value::as_u64),
    }
}
