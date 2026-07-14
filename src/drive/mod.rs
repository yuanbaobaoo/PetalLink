//! Drive REST API 客户端 —— 华为云盘 API 封装。
//!
//! 对齐 `legacy/lib/drive/` 的模块划分。
//!
//! 增量同步已实现（`changes_api.rs`）：自动云端刷新优先走 `/drive/v1/changes` 增量路径
//! （cursor 持久化），失败/cursor 过期/连续达阈值自动回退全量 BFS。

/// 云盘容量与配额接口。
pub mod about_api;
/// 华为请求体所需的 ASCII JSON 编码。
pub mod ascii_json;
/// 云盘增量变更接口。
pub mod changes_api;
/// 带认证重放和结构化错误的 HTTP 客户端。
pub mod client;
/// 可恢复下载与安全安装接口。
pub mod download_api;
/// 云盘文件元数据及写操作接口。
pub mod files_api;
/// 云盘领域模型。
pub mod models;
/// 文件缩略图接口。
pub mod thumbnail_api;
/// 小文件、分片及断点续传上传接口。
pub mod upload_api;
