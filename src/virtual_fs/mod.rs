//! Linux 按需云盘的用户态文件系统边界。
//!
//! 目录与普通文件来自私有 backing 目录，读取占位文件或基于旧内容编辑时通过
//! [`Hydrator`] 触发既有下载服务；用户写操作直接落到 backing，再由同步 watcher
//! 上传。此模块不依赖 Tauri command，便于后续把挂载会话放进独立后台进程。

#[cfg(target_os = "linux")]
mod inode_table;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    mount_read_only, mount_virtual_drive, validate_mount_layout, FuseMountSession,
    HydrationRequest, Hydrator, MutationCoordinator, PathLease, ReadOnlyFuseConfig,
    VirtualDriveConfig,
};
