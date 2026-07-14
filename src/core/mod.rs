//! 核心基础设施模块 —— 跨模块复用的配置、日志、缓存、路径工具。
//!
//! 对齐 `legacy/lib/core/` 的模块划分。

/// 同步缓存路径与旧文件迁移。
pub mod cache_paths;
/// 应用配置模型。
pub mod config;
/// 配置文件持久化。
pub mod config_store;
/// 开发环境凭据加载。
pub mod env_loader;
/// 日志收集与文件保留。
pub mod logging;
/// 网络状态探测与发布。
pub mod net_guard;
/// 通用路径安全工具。
pub mod paths;
