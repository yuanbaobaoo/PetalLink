//! 核心基础设施模块 —— 跨模块复用的配置、日志、缓存、路径工具。
//!
//! 对齐 `legacy/lib/core/` 的模块划分。

pub mod cache_paths;
pub mod config;
pub mod config_store;
pub mod env_loader;
pub mod logging;
pub mod net_guard;
pub mod paths;
