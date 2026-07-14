//! Mount 模块 —— 本地镜像目录管理、文件监听、sha256 哈希。
//!
//! 对齐 `legacy/lib/mount/` 的模块划分。

/// 带元数据缓存的文件哈希计算。
pub mod file_hasher;
/// 本地文件系统变更监听与防抖。
pub mod local_watcher;
/// 镜像目录、占位符 与 xattr 管理。
pub mod manager;
/// 内部文件及用户模式的跳过规则。
pub mod skip;
