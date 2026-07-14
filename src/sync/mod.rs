//! 同步引擎 —— 3-way diff、冲突处理、并发执行、云端树 BFS。
//!
//! 对齐 `legacy/lib/sync/` 的模块划分。

/// 维护可信云端目录树与检查点。
pub mod cloud_tree;
/// 处理本地与云端并发修改冲突。
pub mod conflict;
/// 编排同步周期、状态发布与生命周期。
pub mod engine;
/// 执行规划动作并桥接持久传输任务。
pub mod executor;
/// 恢复已验证的本地路径移动。
pub mod path_recovery;
/// 根据三方快照生成同步动作。
pub mod planner;
/// 将传输错误归类为可执行的恢复决策。
pub mod retry_policy;
/// 判断本地文件是否稳定且可安全传输。
pub mod stability;
/// 定义同步动作与运行时状态模型。
pub mod state;
/// 聚合持久化与运行时同步状态。
pub mod status_aggregator;
/// 读写本地文件状态快照。
pub mod sync_state_store;
/// 驱动持久传输任务的准入、执行与恢复。
pub mod task_runner;
/// 定义传输生命周期的持久化枚举与转换。
pub mod transfer_state;
