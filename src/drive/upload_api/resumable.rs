//! 大文件断点续传的会话初始化、分片循环与最终确认。

use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, LOCATION};
use serde_json::Value;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult, RequestSemantics};

use super::protocol::{
    auth_already_replayed, build_metadata_json, is_remote_ambiguity, notify_resume_progress,
    remote_ambiguity, should_retry_chunk_locally, validated_chunk_size,
};
use super::{
    ChunkResult, ProgressFn, ResumeProgressFn, ResumeSession, UploadApi, CHUNK_RETRIES,
    FINAL_STATUS_MAX_POLLS, FINAL_STATUS_POLL_INTERVAL_SECS,
};

impl UploadApi {
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

    /// 提交一次非幂等会话初始化；响应不确定时不自动新建第二个会话。
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
}
