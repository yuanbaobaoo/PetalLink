//! Auth 模块 —— OAuth 2.0 授权码 + PKCE 流程、Token 存储、自动刷新、用户信息。
//!
//! 对齐 `legacy/lib/auth/` 的模块划分。

/// 授权令牌与用户信息模型。
pub mod models;
/// 本机 OAuth 回调服务。
pub mod oauth_server;
/// PKCE 随机参数生成。
pub mod pkce;
/// 访问令牌刷新与并发去重。
pub mod token_refresher;
/// 令牌加密持久化。
pub mod token_store;
/// 华为账号信息查询。
pub mod user_info_api;

/// 编排 OAuth 授权流程 + code 交换 + 用户信息获取。
pub mod service;
