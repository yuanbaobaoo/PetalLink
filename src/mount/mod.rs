//! Mount 模块 —— 本地镜像目录管理、文件监听、sha256 哈希。
//!
//! 对齐 `legacy/lib/mount/` 的模块划分。

pub mod file_hasher;
pub mod local_watcher;
pub mod manager;
pub mod skip;
