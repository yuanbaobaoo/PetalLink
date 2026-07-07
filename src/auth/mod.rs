//! Auth 模块 —— OAuth 2.0 授权码 + PKCE 流程、Token 存储、自动刷新、用户信息。
//!
//! 对齐 `legacy/lib/auth/` 的模块划分。

pub mod models;
pub mod oauth_server;
pub mod pkce;
pub mod token_refresher;
pub mod token_store;
pub mod user_info_api;

/// 编排 OAuth 授权流程 + code 交换 + 用户信息获取。
pub mod service;
