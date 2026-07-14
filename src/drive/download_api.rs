//! Download API —— 版本校验后使用 Range 断点下载到 `.tmp`，完成后原子替换。
//!
//! `.tmp` 后缀是 load-bearing 的：watcher 和 scanner 会忽略它，下载中断时可以
//! 保留已落盘内容而不会被误判为本地新增文件。旁路元数据文件同样以 `.tmp` 结尾。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_RANGE, ETAG, IF_MATCH, RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::drive::client::{
    classify_transport_error, handle_error_response_with_metadata, DriveClient,
};
use crate::error::{AppError, AppResult, RequestSemantics};

/// 以临时文件和版本约束执行可恢复下载。
pub struct DownloadApi {
    client: Arc<DriveClient>,
    drive_base: String,
}

/// 下载进度回调（参数：已下载字节，总字节）。
pub type ProgressFn = Box<dyn Fn(u64, u64) + Send + Sync>;

/// 调度器已知的云端版本约束。
///
/// 提供约束时，API 会在写入前拒绝已经过期的任务，避免旧任务下载了一个新版本后
/// 仍按旧版本结算同步基线。未提供的字段不参与校验。
#[derive(Debug, Clone, Default)]
pub struct DownloadExpectation {
    pub edited_time_ms: Option<i64>,
    pub size: Option<u64>,
    pub content_hash: Option<String>,
    /// 已下载文件；仅当本地版本未变化时才允许替换。
    pub destination_snapshot: Option<LocalDestinationSnapshot>,
    /// 首次下载仅可写入空路径或当前云端文件未改动的占位符。
    pub placeholder_file_id: Option<String>,
}

/// 安装下载结果前必须保持不变的本地文件快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDestinationSnapshot {
    pub mtime_ms: i64,
    pub size: u64,
}

/// 与临时内容绑定的持久化断点身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ResumeMetadata {
    file_id: String,
    size: u64,
    revision: Option<String>,
    edited_time_ms: Option<i64>,
    etag: Option<String>,
    sha256: Option<String>,
    content_hash: Option<String>,
}

impl ResumeMetadata {
    /// 判断元数据是否含可阻止跨版本续传的稳定身份。
    fn has_stable_identity(&self) -> bool {
        self.revision.is_some()
            || self.edited_time_ms.is_some()
            || self.etag.is_some()
            || self.sha256.is_some()
            || self.content_hash.is_some()
    }
}

/// 从下载前云端元数据查询得到的版本快照。
#[derive(Debug, Clone)]
struct RemoteMetadata {
    resume: ResumeMetadata,
}

impl DownloadApi {
    /// 使用共享 Drive 客户端创建下载接口。
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self {
            client,
            drive_base: crate::constants::DRIVE_API_BASE.to_string(),
        }
    }

    /// 下载文件到 `dest_path`。保留原接口；版本由 API 每次从云端读取并校验。
    pub async fn download(
        &self,
        file_id: &str,
        dest_path: &Path,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<()> {
        self.download_with_expectation(file_id, dest_path, None, on_progress)
            .await
    }

    /// 带调度器版本约束的断点下载。
    pub async fn download_with_expectation(
        &self,
        file_id: &str,
        dest_path: &Path,
        expectation: Option<&DownloadExpectation>,
        on_progress: Option<&ProgressFn>,
    ) -> AppResult<()> {
        if file_id.is_empty() {
            return Err(AppError::generic("下载 file_id 不能为空"));
        }
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| AppError::generic(format!("创建下载目录失败：{error}")))?;
        }

        let remote = match self.fetch_remote_metadata(file_id).await {
            Ok(remote) => remote,
            Err(error) => {
                cleanup_if_permanent(dest_path, &error);
                return Err(error);
            }
        };
        if let Some(expectation) = expectation {
            if !matches_expectation(&remote.resume, expectation) {
                discard_resume_artifacts(dest_path);
                return Err(AppError::generic(
                    "云端文件版本已变化，当前下载任务已过期，请重新规划同步",
                ));
            }
        }

        let tmp = tmp_path(dest_path);
        let mut offset = self
            .validated_resume_offset(dest_path, &remote.resume)
            .await?;
        write_resume_metadata(dest_path, &remote.resume).await?;

        // 上次响应已经写完，但在最终核验或 rename 前断网/崩溃：不重复下载。
        if tmp.exists() && offset == remote.resume.size {
            return self
                .verify_and_install(file_id, dest_path, &remote.resume, expectation)
                .await;
        }

        // 空文件没有内容请求也可以安全落盘。
        if remote.resume.size == 0 {
            let file = File::create(&tmp)
                .await
                .map_err(|error| AppError::generic(format!("创建临时文件失败：{error}")))?;
            file.sync_all()
                .await
                .map_err(|error| AppError::generic(format!("同步临时文件失败：{error}")))?;
            return self
                .verify_and_install(file_id, dest_path, &remote.resume, expectation)
                .await;
        }

        // Range 不匹配或 416 时只允许在本次调用中安全回退一次到 offset=0。
        let mut restarted_from_zero = offset == 0;
        loop {
            let (response, auth_replayed) = match self
                .send_content_request(file_id, offset, remote.resume.etag.as_deref())
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    cleanup_if_permanent(dest_path, &error);
                    return Err(error);
                }
            };

            if response.status() == StatusCode::RANGE_NOT_SATISFIABLE
                && offset > 0
                && !restarted_from_zero
            {
                discard_resume_artifacts(dest_path);
                write_resume_metadata(dest_path, &remote.resume).await?;
                offset = 0;
                restarted_from_zero = true;
                continue;
            }
            if !response.status().is_success() {
                let error = handle_error_response_with_metadata(
                    response,
                    RequestSemantics::Read,
                    auth_replayed,
                )
                .await;
                cleanup_if_permanent(dest_path, &error);
                return Err(error);
            }

            let write_offset = match validated_response_offset(
                &response,
                offset,
                remote.resume.size,
            ) {
                Ok(write_offset) => write_offset,
                Err(message) if offset > 0 && !restarted_from_zero => {
                    tracing::warn!(requested_offset = offset, %message, "Range 响应不可信，从 0 重启");
                    discard_resume_artifacts(dest_path);
                    write_resume_metadata(dest_path, &remote.resume).await?;
                    offset = 0;
                    restarted_from_zero = true;
                    continue;
                }
                Err(message) => {
                    discard_resume_artifacts(dest_path);
                    return Err(AppError::generic(message));
                }
            };

            let mut file = if write_offset == 0 {
                File::create(&tmp)
                    .await
                    .map_err(|error| AppError::generic(format!("创建临时文件失败：{error}")))?
            } else {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&tmp)
                    .await
                    .map_err(|error| AppError::generic(format!("打开临时文件失败：{error}")))?
            };

            let mut received = write_offset;
            if let Some(callback) = on_progress {
                callback(received, remote.resume.size);
            }
            let mut stream = response.bytes_stream();
            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        let _ = file.flush().await;
                        let _ = file.sync_data().await;
                        return Err(classify_transport_error(
                            &error,
                            RequestSemantics::Read,
                            auth_replayed,
                        ));
                    }
                };
                file.write_all(&chunk)
                    .await
                    .map_err(|error| AppError::generic(format!("写入临时文件失败：{error}")))?;
                received = received.saturating_add(chunk.len() as u64);
                if let Some(callback) = on_progress {
                    callback(received, remote.resume.size);
                }
            }
            file.flush()
                .await
                .map_err(|error| AppError::generic(format!("刷新临时文件失败：{error}")))?;
            file.sync_all()
                .await
                .map_err(|error| AppError::generic(format!("同步临时文件失败：{error}")))?;
            drop(file);

            let actual_size = tokio::fs::metadata(&tmp)
                .await
                .map_err(|error| AppError::generic(format!("读取临时文件长度失败：{error}")))?
                .len();
            if actual_size != remote.resume.size {
                if actual_size > remote.resume.size {
                    discard_resume_artifacts(dest_path);
                    return Err(AppError::generic(format!(
                        "下载长度异常：期望 {} 字节，实际 {actual_size} 字节",
                        remote.resume.size
                    )));
                }
                // 某些代理会干净地提前结束响应；保留部分文件，下一次继续 Range。
                return Err(AppError::drive_network(Some(&format!(
                    "下载响应提前结束：期望 {} 字节，已接收 {actual_size} 字节",
                    remote.resume.size
                ))));
            }

            return self
                .verify_and_install(file_id, dest_path, &remote.resume, expectation)
                .await;
        }
    }

    /// 获取并严格校验下载所需的云端版本元数据。
    async fn fetch_remote_metadata(&self, file_id: &str) -> AppResult<RemoteMetadata> {
        let encoded_id = crate::drive::files_api::urlencoding(file_id);
        let response = self
            .client
            .get(&format!("/files/{encoded_id}?fields=*"))
            .await?;
        let header_etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body: Value = response.json().await.map_err(|error| {
            AppError::drive_transport(
                crate::error::DriveTransportKind::Decode,
                RequestSemantics::Read,
                false,
                Some(&error.to_string()),
            )
        })?;

        let returned_id = body
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AppError::generic("下载元数据缺少有效 id"))?;
        if returned_id != file_id {
            return Err(AppError::generic("下载元数据 id 与请求不一致"));
        }
        let size = parse_u64(body.get("size"))
            .ok_or_else(|| AppError::generic("下载元数据缺少有效 size"))?;
        let revision = scalar_string(body.get("contentVersion"))
            .or_else(|| scalar_string(body.get("version")));
        let edited_time_ms = body
            .get("editedTime")
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp_millis());
        let sha256 = ["sha256", "fileSha256"]
            .iter()
            .find_map(|field| nonempty_string(body.get(*field)));
        let content_hash = ["contentHash", "hash", "md5", "md5Checksum"]
            .iter()
            .find_map(|field| nonempty_string(body.get(*field)));
        let etag = header_etag.or_else(|| nonempty_string(body.get("etag")));

        Ok(RemoteMetadata {
            resume: ResumeMetadata {
                file_id: file_id.to_string(),
                size,
                revision,
                edited_time_ms,
                etag,
                sha256,
                content_hash,
            },
        })
    }

    /// 仅在断点身份与当前云端版本一致时返回可续传偏移。
    async fn validated_resume_offset(
        &self,
        dest_path: &Path,
        current: &ResumeMetadata,
    ) -> AppResult<u64> {
        let tmp = tmp_path(dest_path);
        if !tmp.exists() {
            remove_resume_metadata(dest_path);
            return Ok(0);
        }

        let stored = read_resume_metadata(dest_path).await;
        if stored.as_ref() != Some(current) || !current.has_stable_identity() {
            discard_resume_artifacts(dest_path);
            return Ok(0);
        }
        let length = tokio::fs::metadata(&tmp)
            .await
            .map_err(|error| AppError::generic(format!("读取断点文件长度失败：{error}")))?
            .len();
        if length > current.size {
            discard_resume_artifacts(dest_path);
            return Ok(0);
        }
        Ok(length)
    }

    /// 发送内容请求；遇到 401 时刷新认证并原样重放一次。
    async fn send_content_request(
        &self,
        file_id: &str,
        offset: u64,
        etag: Option<&str>,
    ) -> AppResult<(reqwest::Response, bool)> {
        let token = self.client.auth().ensure_valid_access_token().await?;
        let response = self
            .build_content_request(file_id, offset, etag, &token)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, RequestSemantics::Read, false))?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok((response, false));
        }

        // 只刷新一次，并对同一个 URL 和同一个 Range 原样重放。
        let refreshed = self.client.auth().refresher().refresh().await?;
        let response = self
            .build_content_request(file_id, offset, etag, &refreshed.access_token)
            .send()
            .await
            .map_err(|error| classify_transport_error(&error, RequestSemantics::Read, true))?;
        Ok((response, true))
    }

    /// 构造带可选 Range 与版本条件的已认证内容请求。
    fn build_content_request(
        &self,
        file_id: &str,
        offset: u64,
        etag: Option<&str>,
        token: &str,
    ) -> reqwest::RequestBuilder {
        let encoded_id = crate::drive::files_api::urlencoding(file_id);
        let mut request = self
            .client
            .raw_http()
            .get(format!(
                "{}/files/{encoded_id}?form=content",
                self.drive_base
            ))
            .bearer_auth(token);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        if let Some(etag) = etag {
            request = request.header(IF_MATCH, etag);
        }
        request
    }

    /// 复核长度、哈希及两端版本后原子安装临时文件。
    async fn verify_and_install(
        &self,
        file_id: &str,
        dest_path: &Path,
        downloaded_version: &ResumeMetadata,
        expectation: Option<&DownloadExpectation>,
    ) -> AppResult<()> {
        let tmp = tmp_path(dest_path);
        let actual_size = tokio::fs::metadata(&tmp)
            .await
            .map_err(|error| AppError::generic(format!("读取临时文件失败：{error}")))?
            .len();
        if actual_size != downloaded_version.size {
            if actual_size > downloaded_version.size {
                discard_resume_artifacts(dest_path);
            }
            return Err(AppError::drive_network(Some("断点文件尚未下载完整")));
        }

        if let Some(expected_sha256) = downloaded_version.sha256.as_deref() {
            let actual_sha256 = sha256_file(&tmp).await?;
            if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
                discard_resume_artifacts(dest_path);
                return Err(AppError::generic("下载文件 sha256 校验失败"));
            }
        }

        // 内容读取结束后再取一次元数据，防止无 ETag 时把两个云端版本混为一次成功。
        let current = match self.fetch_remote_metadata(file_id).await {
            Ok(current) => current,
            Err(error) => {
                cleanup_if_permanent(dest_path, &error);
                return Err(error);
            }
        };
        if current.resume != *downloaded_version {
            discard_resume_artifacts(dest_path);
            return Err(AppError::generic(
                "下载期间云端文件发生变化，已丢弃旧断点并等待重新下载",
            ));
        }

        verify_local_destination(dest_path, expectation)?;

        // POSIX rename 在同一文件系统内原子替换旧目标；失败时保留 .tmp 供重试。
        tokio::fs::rename(&tmp, dest_path)
            .await
            .map_err(|error| AppError::generic(format!("安装下载文件失败：{error}")))?;
        remove_resume_metadata(dest_path);
        Ok(())
    }
}

/// 安装前确认本地目标仍为空缺、原快照或本文件的未改占位符。
fn verify_local_destination(
    dest_path: &Path,
    expectation: Option<&DownloadExpectation>,
) -> AppResult<()> {
    let Some(expectation) = expectation else {
        return Ok(());
    };
    if let Some(snapshot) = expectation.destination_snapshot.as_ref() {
        let metadata = std::fs::symlink_metadata(dest_path)
            .map_err(|error| AppError::generic(format!("安装下载结果前读取原文件失败：{error}")))?;
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64);
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != snapshot.size
            || mtime_ms != Some(snapshot.mtime_ms)
        {
            return Err(AppError::generic(
                "下载期间本地目标已被修改，已保留用户内容和下载临时文件",
            ));
        }
        return Ok(());
    }
    if let Some(file_id) = expectation.placeholder_file_id.as_deref() {
        let metadata = match std::fs::symlink_metadata(dest_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(AppError::generic(format!(
                    "安装下载结果前读取目标路径失败：{error}"
                )))
            }
        };
        let owner = xattr::get(dest_path, crate::mount::manager::XATTR_FILE_ID)
            .map_err(|error| AppError::generic(format!("读取下载占位身份失败：{error}")))?
            .map(String::from_utf8)
            .transpose()
            .map_err(|_| AppError::generic("下载占位 fileId 标记损坏"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != 0
            || !crate::mount::manager::is_placeholder_file(dest_path)
            || owner.as_deref() != Some(file_id)
        {
            return Err(AppError::generic(
                "下载期间目标路径出现用户内容，已拒绝覆盖并保留下载临时文件",
            ));
        }
    }
    Ok(())
}

/// 判断远端版本是否满足调度器提供的全部约束。
fn matches_expectation(remote: &ResumeMetadata, expectation: &DownloadExpectation) -> bool {
    expectation
        .edited_time_ms
        .map_or(true, |expected| remote.edited_time_ms == Some(expected))
        && expectation
            .size
            .map_or(true, |expected| remote.size == expected)
        && expectation
            .content_hash
            .as_deref()
            .map_or(true, |expected| {
                remote
                    .sha256
                    .as_deref()
                    .or(remote.content_hash.as_deref())
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
            })
}

/// 返回写入起点。`200` 表示服务端忽略 Range，调用方必须截断后从 0 写。
fn validated_response_offset(
    response: &reqwest::Response,
    requested_offset: u64,
    expected_total: u64,
) -> Result<u64, String> {
    match response.status() {
        StatusCode::OK => Ok(0),
        StatusCode::PARTIAL_CONTENT => {
            let value = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Range 响应缺少 Content-Range".to_string())?;
            let (start, end, total) = parse_content_range(value)
                .ok_or_else(|| "Range 响应的 Content-Range 无效".to_string())?;
            if start != requested_offset || total != expected_total || end < start || end >= total {
                return Err(format!(
                    "Range 响应不匹配：请求 {requested_offset}，响应 {start}-{end}/{total}"
                ));
            }
            Ok(start)
        }
        status => Err(format!("下载返回了不支持的成功状态码：{status}")),
    }
}

/// 解析 `bytes start-end/total` 响应范围。
fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

/// 从无符号整数或十进制字符串读取字节数。
fn parse_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

/// 将非空字符串或数值标量转换为版本字符串。
fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

/// 克隆可选的非空 JSON 字符串。
fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// 流式计算文件 SHA-256；打开或读取失败直接返回错误。
async fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)
        .await
        .map_err(|error| AppError::generic(format!("打开临时文件校验失败：{error}")))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| AppError::generic(format!("读取临时文件校验失败：{error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 尝试读取断点元数据；缺失、读失败或损坏均视为不可续传。
async fn read_resume_metadata(dest: &Path) -> Option<ResumeMetadata> {
    let bytes = tokio::fs::read(resume_metadata_path(dest)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 先同步暂存文件再原子提交断点元数据。
async fn write_resume_metadata(dest: &Path, metadata: &ResumeMetadata) -> AppResult<()> {
    let bytes = serde_json::to_vec(metadata)
        .map_err(|error| AppError::generic(format!("序列化下载断点失败：{error}")))?;
    let staging = resume_metadata_staging_path(dest);
    let target = resume_metadata_path(dest);
    let mut file = File::create(&staging)
        .await
        .map_err(|error| AppError::generic(format!("创建下载断点元数据失败：{error}")))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| AppError::generic(format!("写入下载断点元数据失败：{error}")))?;
    file.flush()
        .await
        .map_err(|error| AppError::generic(format!("刷新下载断点元数据失败：{error}")))?;
    file.sync_all()
        .await
        .map_err(|error| AppError::generic(format!("同步下载断点元数据失败：{error}")))?;
    drop(file);
    tokio::fs::rename(&staging, &target)
        .await
        .map_err(|error| AppError::generic(format!("提交下载断点元数据失败：{error}")))?;
    Ok(())
}

/// 仅对判定为永久失败的错误清除断点，暂态失败保留现场。
fn cleanup_if_permanent(dest: &Path, error: &AppError) {
    let should_keep = match error {
        AppError::DriveApi {
            status_code: None, ..
        }
        | AppError::Token { .. }
        | AppError::Auth { .. }
        | AppError::Generic { .. } => true,
        AppError::DriveApi {
            status_code: Some(status),
            ..
        } => matches!(*status, 401 | 408 | 409 | 425 | 429 | 500..=599),
        AppError::Config { .. } | AppError::QuotaExceeded { .. } => false,
    };
    if !should_keep {
        discard_resume_artifacts(dest);
    }
}

/// 构造 `.tmp` 临时文件路径：将后缀加到完整目标路径之后。
pub fn tmp_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".tmp")
}

/// 构造断点元数据路径。它同样以 `.tmp` 结尾，确保不会被扫描器上传。
pub fn resume_metadata_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".download-meta.tmp")
}

/// 构造断点元数据写入阶段使用的暂存路径。
fn resume_metadata_staging_path(dest: &Path) -> PathBuf {
    append_suffix(dest, ".download-meta-write.tmp")
}

/// 在完整路径字节串末尾追加内部后缀。
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

/// 尽力删除已提交及写入中的断点元数据。
fn remove_resume_metadata(dest: &Path) {
    let _ = std::fs::remove_file(resume_metadata_path(dest));
    let _ = std::fs::remove_file(resume_metadata_staging_path(dest));
}

/// 永久失败、取消或显式重启时由任务层调用；网络失败不要调用。
pub fn discard_resume_artifacts(dest: &Path) {
    let _ = std::fs::remove_file(tmp_path(dest));
    remove_resume_metadata(dest);
}
