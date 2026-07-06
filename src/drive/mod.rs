//! Drive REST API 客户端 —— 华为云盘 API 封装。
//!
//! 对齐 `legacy/lib/drive/` 的模块划分。
//!
//! 增量同步已实现（`changes_api.rs`）：自动云端刷新优先走 `/drive/v1/changes` 增量路径
//! （cursor 持久化），失败/cursor 过期/连续达阈值自动回退全量 BFS。

pub mod about_api;
pub mod changes_api;
pub mod ascii_json;
pub mod client;
pub mod download_api;
pub mod files_api;
pub mod models;
pub mod thumbnail_api;
pub mod upload_api;
