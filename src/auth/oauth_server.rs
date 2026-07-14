//! OAuth 本地回调 HTTP Server（需求 F-AUTH-02 / F-AUTH-06）。
//!
//! 对齐 `legacy/lib/auth/oauth_server.dart`。
//!
//! 绑定 127.0.0.1:port（不监听 0.0.0.0，满足安全要求）。
//! 监听 GET {callbackPath}，解析 code/state/error/sub_error。
//! 单次使用：拿到 code 后自动关闭。

use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::constants;
use crate::error::{AppError, AppResult};

/// OAuth 回调结果
#[derive(Debug, Clone, Serialize, Default)]
pub struct OauthCallbackResult {
    pub code: Option<String>,
    pub state: Option<String>,
    /// 华为 error 码（如 '1101'）
    pub error: Option<String>,
    /// 华为 error_description（如 'invalid scope'）
    #[serde(rename = "errorDescription")]
    pub error_description: Option<String>,
    /// 华为 sub_error（如 '20042' 表示 scope 未授权）
    #[serde(rename = "subError")]
    pub sub_error: Option<String>,
}

impl OauthCallbackResult {
    /// 是否成功（有 code 且无 error）
    pub fn is_success(&self) -> bool {
        self.code.is_some() && self.error.is_none()
    }
}

/// 本地 OAuth 回调服务器。
///
/// 使用 tokio TcpListener 监听 127.0.0.1，手工解析 HTTP 请求行（足够覆盖 OAuth 回调）。
pub struct OauthServer {
    /// 停止句柄（发送信号让监听任务退出）
    stop_handle: OauthServerStopHandle,
    /// 监听任务句柄
    listen_task: Option<JoinHandle<()>>,
    /// 回调结果接收端
    result_rx: Option<oneshot::Receiver<OauthCallbackResult>>,
}

/// 可跨任务触发回调监听停止的句柄。
#[derive(Clone)]
pub struct OauthServerStopHandle {
    stop_tx: watch::Sender<bool>,
}

impl OauthServerStopHandle {
    /// 通知 OAuth server 停止等待回调。
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

impl OauthServer {
    /// 启动监听。重复调用会返回错误。
    pub async fn start(port: u16) -> AppResult<Self> {
        let addr = format!("{}:{}", constants::LOOPBACK_HOST, port);
        tracing::info!(addr = %addr, "启动 OAuth 回调监听");
        // 仅绑定 loopback IPv4
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| AppError::generic(format!("绑定回调端口失败：{e}")))?;

        let (result_tx, result_rx) = oneshot::channel::<OauthCallbackResult>();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let stop_handle = OauthServerStopHandle { stop_tx };

        let listen_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    changed = stop_rx.changed() => {
                        if changed.is_ok() && *stop_rx.borrow() {
                            break;
                        }
                    }
                    accept = listener.accept() => {
                        match accept {
                            Ok((mut stream, _)) => {
                                let result = handle_request(&mut stream).await;
                                // 回写响应页
                                let html = build_response_page(&result);
                                let _ = write_response(&mut stream, &html).await;
                                // 完成回调
                                let _ = result_tx.send(result.clone());
                                // 单次使用：拿到结果后停止监听
                                break;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "OAuth 回调 accept 失败");
                            }
                        }
                    }
                }
            }
            tracing::info!("OAuth 回调 server 已关闭");
        });

        Ok(Self {
            stop_handle,
            listen_task: Some(listen_task),
            result_rx: Some(result_rx),
        })
    }

    /// 获取可克隆停止句柄，供取消授权从外部关闭监听。
    pub fn stop_handle(&self) -> OauthServerStopHandle {
        self.stop_handle.clone()
    }

    /// 等待授权码（带超时）。超时返回 [`AppError::auth_timeout`]。
    /// 对齐 dart `OauthServer.waitForCallback`。
    pub async fn wait_for_callback(mut self) -> AppResult<OauthCallbackResult> {
        let result_rx = self
            .result_rx
            .take()
            .ok_or_else(|| AppError::generic("OauthServer 未启动"))?;
        match timeout(
            Duration::from_secs(constants::OAUTH_TIMEOUT_SECS),
            result_rx,
        )
        .await
        {
            Ok(Ok(result)) => {
                self.stop().await;
                Ok(result)
            }
            Ok(Err(_)) => {
                self.stop().await;
                Err(AppError::generic("OAuth 回调通道关闭"))
            }
            Err(_) => {
                self.stop().await;
                tracing::warn!("OAuth 回调等待超时");
                Err(AppError::auth_timeout())
            }
        }
    }

    /// 关闭 server，释放端口。
    pub async fn stop(mut self) {
        self.stop_handle.stop();
        if let Some(handle) = self.listen_task.take() {
            let _ = handle.await;
        }
    }
}

/// 解析 HTTP 请求，提取回调参数。
async fn handle_request(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> OauthCallbackResult {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]);

    // 解析请求行：GET /oauth/callback?code=xxx&state=yyy HTTP/1.1
    let request_line = request.lines().next().unwrap_or("");
    let path = request_line.split_whitespace().nth(1).unwrap_or("");

    if !path.starts_with(constants::CALLBACK_PATH) {
        return OauthCallbackResult {
            error: Some("无效回调路径".to_string()),
            ..Default::default()
        };
    }

    // 提取 query string
    let query = path.split('?').nth(1).unwrap_or("");
    parse_query(query)
}

/// 解析 query string 为回调结果。
fn parse_query(query: &str) -> OauthCallbackResult {
    let mut result = OauthCallbackResult::default();
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = url_decode(kv.next().unwrap_or(""));
        let value = url_decode(kv.next().unwrap_or(""));
        match key.as_str() {
            "code" => result.code = Some(value),
            "state" => result.state = Some(value),
            "error" => result.error = Some(value),
            "error_description" => result.error_description = Some(value),
            "sub_error" => result.sub_error = Some(value),
            _ => {}
        }
    }
    result
}

/// URL 解码（百分号编码 + '+' 当空格，对齐 form-urlencoded / dart Uri.splitQueryString）。
fn url_decode(s: &str) -> String {
    // form-urlencoded 中 '+' 表示空格（percent-decoding 不处理 '+'，需先替换）
    let with_spaces = s.replace('+', " ");
    percent_encoding::percent_decode_str(&with_spaces)
        .decode_utf8_lossy()
        .to_string()
}

/// 写入带长度和关闭连接头的 UTF-8 HTML 成功响应。
async fn write_response(
    stream: &mut (impl tokio::io::AsyncWrite + Unpin),
    html: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// 构建回写浏览器的友好页面。对齐 dart `_buildResponsePage`。
fn build_response_page(result: &OauthCallbackResult) -> String {
    if result.is_success() {
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>授权成功</title>
<style>body{font-family:-apple-system,sans-serif;text-align:center;margin-top:80px;color:#333}
h1{color:#1a7f37}</style></head>
<body><h1>✅ 授权成功</h1>
<p>已成功登录华为云盘，现在可以关闭此页面并返回 App。</p></body></html>"#
            .to_string()
    } else {
        let reason = result.error.as_deref().unwrap_or("未知错误");
        format!(
            r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>授权失败</title>
<style>body{{font-family:-apple-system,sans-serif;text-align:center;margin-top:80px;color:#333}}
h1{{color:#d73a49}}</style></head>
<body><h1>❌ 授权失败</h1>
<p>{reason}</p>
<p>请返回 App 重新登录。</p></body></html>"#
        )
    }
}
