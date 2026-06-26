//! 同步引擎 —— 3-way diff、冲突处理、并发执行、云端树 BFS。
//!
//! 对齐 `legacy/lib/sync/` 的模块划分。

pub mod cloud_tree;
pub mod conflict;
pub mod engine;
pub mod executor;
pub mod planner;
pub mod stability;
pub mod state;
pub mod sync_state_store;
