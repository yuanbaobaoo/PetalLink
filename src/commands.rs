//! Tauri 命令层入口：领域命令模块声明与公开导出。
//!
//! 全局运行时（服务单例、引擎与 FUSE 会话生命周期）在 [`crate::runtime`]；
//! 此处全量再导出，保持 `commands::X` 既有引用路径不变。

/// 认证相关命令。
mod auth;
/// 配置读写命令。
mod config;
/// 云盘文件操作命令。
pub(crate) mod drive;
/// 目录递归同步命令。
mod folder_sync;
/// 本地空间释放与按需下载命令。
mod free_up;
/// 拖拽导入命令。
mod import_files;
/// 平台集成与应用维护命令。
pub(crate) mod platform;
/// 同步控制命令。
mod sync_control;
/// 同步状态查询命令。
mod sync_status;
/// 传输队列命令。
mod transfer;

pub use auth::*;
pub use config::*;
pub use drive::*;
pub use folder_sync::*;
pub use free_up::*;
pub use import_files::*;
pub use platform::*;
pub use sync_control::*;
pub use sync_status::*;
pub use transfer::*;

pub use crate::runtime::*;
