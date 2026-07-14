//! Multipart 小文件上传与已有文件的受限覆盖更新。

use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::StatusCode;
use serde_json::Value;

use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult, RequestSemantics};

use super::protocol::{build_metadata_json, complete_upload_file, remote_ambiguity};
use super::{ProgressFn, UploadApi, SMALL_LARGE_THRESHOLD};

impl UploadApi {
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

    /// 使用给定 token 覆盖小文件，并严格核验返回文件的名称和长度。
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

    /// 401 表示写请求尚未授权，可用刷新后的 token 原样重放一次。
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
}

/// 拒绝会迫使已有 fileId 退化为新建的大文件覆盖请求。
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
