//! Drive REST API 客户端 —— 华为云盘 API 封装。
//!
//! 对齐 `legacy/lib/drive/` 的模块划分。
//!
//! # TODO（阶段二：增量同步优化）
//! 当前自动云端→本地同步走 `sync::engine::run_auto_cloud_refresh` 的全量 BFS（定时重拉整棵云端树）。
//! 后续若接入华为 `/drive/v1/changes?cursor=...` 增量接口，建议新增 `changes_api.rs`
//! （仿 `about_api.rs`，用 `DriveClient::get`），在 `run_auto_cloud_refresh_impl` 内改为
//! 「有持久化 cursor → 增量拉取变更；无 cursor/失效 → 回退全量 BFS」。定时任务框架已为此预留。

pub mod about_api;
pub mod ascii_json;
pub mod client;
pub mod download_api;
pub mod files_api;
pub mod models;
pub mod thumbnail_api;
pub mod upload_api;
