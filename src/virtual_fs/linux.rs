//! Linux FUSE 云盘 proxy。
//!
//! 用户访问 FUSE 挂载点；本实现把目录结构映射到私有 backing 目录。普通文件直接
//! 通过打开的真实文件句柄读写，带 PetalLink 占位 xattr 的文件在第一次读取或
//! 可能依赖旧内容的写打开前交给 [`Hydrator`]。命名空间写操作直接落到 backing，
//! 由既有 watcher 负责生成上传/删除任务。

use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::fs::{File, FileTimes, Metadata, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use fuser::{
    AccessFlags, BsdFileFlags, Config as FuseConfig, Errno, FileAttr, FileHandle, FileType,
    Filesystem, FopenFlags, Generation, INodeNo, InitFlags, KernelConfig, LockOwner, MountOption,
    OpenAccMode, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, SessionACL, TimeOrNow,
    WriteFlags,
};
use parking_lot::Mutex;
use tokio::runtime::Handle;
use tokio::sync::{Mutex as AsyncMutex, Notify, RwLock as AsyncRwLock};

use super::inode_table::InodeTable;

const ATTR_TTL: Duration = Duration::from_secs(1);
const GENERATION: Generation = Generation(1);
const XATTR_FILE_ID: &str = "com.hwcloud.fileId";
const XATTR_STATE: &str = "com.hwcloud.state";
const XATTR_SIZE: &str = "com.hwcloud.size";
const STATE_PLACEHOLDER: &[u8] = b"placeholder";
const STATE_DOWNLOADED: &[u8] = b"downloaded";

/// 既有下载服务收到的、与 Tauri IPC 无关的 hydration 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationRequest {
    /// backing 根目录下的稳定相对路径。
    pub relative_path: PathBuf,
    /// 下载最终必须落到的 backing 绝对路径。
    pub backing_path: PathBuf,
    /// 占位 xattr 中保存的云端文件 ID。
    pub file_id: String,
    /// FUSE 在下载前向应用报告的逻辑大小。
    pub logical_size: u64,
}

/// 把一个占位文件变为完整本地文件。
///
/// 实现方应复用 PetalLink 的下载队列、版本检查和原子替换逻辑。成功返回前，
/// `request.backing_path` 必须已是非 symlink 普通文件，且不再带 `placeholder` 状态。
#[async_trait]
pub trait Hydrator: Send + Sync + 'static {
    async fn hydrate(&self, request: HydrationRequest) -> io::Result<()>;
}

/// 与后台同步引擎协调 backing 路径修改。
///
/// 实现方必须把重叠的文件/目录子树视为互斥；`paths` 已按字节序排序并去重，rename
/// 会一次提交旧、新路径，避免调用方分两次取锁造成 ABBA 死锁。
#[async_trait]
pub trait MutationCoordinator: Send + Sync + 'static {
    async fn acquire_shared(&self, paths: Vec<PathBuf>) -> io::Result<Box<dyn PathLease>>;

    async fn acquire_exclusive(&self, paths: Vec<PathBuf>) -> io::Result<Box<dyn PathLease>>;

    /// 在 exclusive lease 内核验云端身份冲突，并为 legacy 源补齐稳定 fileId。
    ///
    /// 此钩子发生在 backing 原子 rename 之前；写在源 inode 上的 xattr 会随 rename
    /// 一起移动，不留下“路径已变但身份尚未补写”的崩溃窗口。
    async fn prepare_rename(
        &self,
        source: PathBuf,
        destination: PathBuf,
        is_directory: bool,
    ) -> io::Result<()>;

    /// backing rename 成功后的通知钩子，可更新易失路径提示/触发同步。
    ///
    /// 不得在这里把云端对齐的持久基线直接重键到新路径；远端 move 尚未提交，旧路径
    /// 基线仍需保留给 planner 的 rename 检测。
    async fn rename_observed(
        &self,
        source: PathBuf,
        destination: PathBuf,
        is_directory: bool,
    ) -> io::Result<()>;
}

/// 路径修改租约；析构即释放。
pub trait PathLease: Send + 'static {}

impl<T: Send + 'static> PathLease for T {}

/// 创建可写 FUSE 云盘会话所需的跨层参数。
pub struct VirtualDriveConfig {
    pub backing_root: PathBuf,
    pub mountpoint: PathBuf,
    pub runtime: Handle,
    pub hydrator: Arc<dyn Hydrator>,
    pub mutation_coordinator: Arc<dyn MutationCoordinator>,
}

impl std::fmt::Debug for VirtualDriveConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VirtualDriveConfig")
            .field("backing_root", &self.backing_root)
            .field("mountpoint", &self.mountpoint)
            .field("runtime", &self.runtime)
            .field("hydrator", &"<dyn Hydrator>")
            .field("mutation_coordinator", &"<dyn MutationCoordinator>")
            .finish()
    }
}

impl VirtualDriveConfig {
    pub fn new(
        backing_root: impl Into<PathBuf>,
        mountpoint: impl Into<PathBuf>,
        runtime: Handle,
        hydrator: Arc<dyn Hydrator>,
        mutation_coordinator: Arc<dyn MutationCoordinator>,
    ) -> Self {
        Self {
            backing_root: backing_root.into(),
            mountpoint: mountpoint.into(),
            runtime,
            hydrator,
            mutation_coordinator,
        }
    }
}

/// 旧的只读会话配置。保留此 API 供渐进迁移和只读回归测试使用。
pub struct ReadOnlyFuseConfig {
    pub backing_root: PathBuf,
    pub mountpoint: PathBuf,
    pub runtime: Handle,
    pub hydrator: Arc<dyn Hydrator>,
}

impl std::fmt::Debug for ReadOnlyFuseConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadOnlyFuseConfig")
            .field("backing_root", &self.backing_root)
            .field("mountpoint", &self.mountpoint)
            .field("runtime", &self.runtime)
            .field("hydrator", &"<dyn Hydrator>")
            .finish()
    }
}

impl ReadOnlyFuseConfig {
    pub fn new(
        backing_root: impl Into<PathBuf>,
        mountpoint: impl Into<PathBuf>,
        runtime: Handle,
        hydrator: Arc<dyn Hydrator>,
    ) -> Self {
        Self {
            backing_root: backing_root.into(),
            mountpoint: mountpoint.into(),
            runtime,
            hydrator,
        }
    }
}

/// 已挂载的后台 FUSE 会话。持有此值可保持挂载；调用 [`Self::unmount`] 可优雅卸载。
#[derive(Debug)]
pub struct FuseMountSession {
    mountpoint: PathBuf,
    session: Option<fuser::BackgroundSession>,
    cancellation: Arc<MountCancellation>,
}

impl FuseMountSession {
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// 卸载文件系统并等待 FUSE 请求线程退出；重复调用是幂等的。
    pub fn unmount(&mut self) -> io::Result<()> {
        self.cancellation.cancel();
        match self.session.take() {
            Some(session) => session.umount_and_join(),
            None => Ok(()),
        }
    }
}

impl Drop for FuseMountSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Debug, Default)]
struct MountCancellation {
    cancelled: AtomicBool,
    notify: Notify,
}

impl MountCancellation {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    async fn cancelled(&self) {
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        let notified = self.notify.notified();
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

/// 校验并规范化 backing 与挂载点。
///
/// 两者都必须是既有真实目录，挂载点必须为空，而且二者不能相同或互相包含。
/// 这样挂载不会遮住 backing，也不会让 FUSE 在遍历时递归进入自身。
pub fn validate_mount_layout(
    backing_root: impl AsRef<Path>,
    mountpoint: impl AsRef<Path>,
) -> io::Result<(PathBuf, PathBuf)> {
    let backing_root = checked_real_directory(backing_root.as_ref(), "backing")?;
    let mountpoint = checked_real_directory(mountpoint.as_ref(), "mountpoint")?;

    if backing_root.starts_with(&mountpoint) || mountpoint.starts_with(&backing_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "backing 与 FUSE 挂载点必须不同且互不包含",
        ));
    }

    let mut entries = std::fs::read_dir(&mountpoint)?;
    if entries.next().transpose()?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FUSE 挂载点必须是空目录",
        ));
    }

    Ok((backing_root, mountpoint))
}

/// 启动仅当前用户可访问的可写 FUSE 云盘会话。
///
/// 配置刻意不包含 `allow_other`，也不使用需要它的 `auto_unmount`。命名空间与内容
/// 修改直接落到 backing；正常关闭应显式调用 [`FuseMountSession::unmount`]。
pub fn mount_virtual_drive(config: VirtualDriveConfig) -> io::Result<FuseMountSession> {
    mount_filesystem(
        config.backing_root,
        config.mountpoint,
        config.runtime,
        config.hydrator,
        Some(config.mutation_coordinator),
        false,
    )
}

/// 启动旧版只读会话。新调用方应使用 [`mount_virtual_drive`]。
pub fn mount_read_only(config: ReadOnlyFuseConfig) -> io::Result<FuseMountSession> {
    mount_filesystem(
        config.backing_root,
        config.mountpoint,
        config.runtime,
        config.hydrator,
        None,
        true,
    )
}

fn mount_filesystem(
    backing_root: PathBuf,
    mountpoint: PathBuf,
    runtime: Handle,
    hydrator: Arc<dyn Hydrator>,
    mutation_coordinator: Option<Arc<dyn MutationCoordinator>>,
    read_only: bool,
) -> io::Result<FuseMountSession> {
    let (backing_root, mountpoint) = validate_mount_layout(&backing_root, &mountpoint)?;
    let cancellation = Arc::new(MountCancellation::default());
    let filesystem = VirtualDriveFs::new(
        backing_root,
        runtime,
        hydrator,
        mutation_coordinator,
        read_only,
        Arc::clone(&cancellation),
    );

    let mut fuse_config = FuseConfig::default();
    fuse_config.acl = SessionACL::Owner;
    fuse_config.n_threads = Some(4);
    fuse_config.clone_fd = true;
    fuse_config.mount_options = vec![
        MountOption::FSName("PetalLink".to_string()),
        MountOption::Subtype("petallink".to_string()),
        MountOption::NoDev,
        MountOption::NoSuid,
        MountOption::NoExec,
        MountOption::NoAtime,
        MountOption::DefaultPermissions,
    ];
    if read_only {
        fuse_config.mount_options.push(MountOption::RO);
    }

    let session = fuser::spawn_mount(filesystem, &mountpoint, &fuse_config)?;
    Ok(FuseMountSession {
        mountpoint,
        session: Some(session),
        cancellation,
    })
}

#[derive(Debug, Clone)]
struct PlaceholderMetadata {
    file_id: String,
    logical_size: u64,
}

/// 发起 FUSE 内容读取的 Linux 进程身份。
///
/// Linux 没有 Windows Cloud Files 那样的系统级“用户打开”和“后台缩略图”区分。
/// 因此只在占位文件首次读取前解析 `/proc`，识别明确的预览器与索引器；解析失败时
/// 必须 fail open，不能因为 `/proc` 竞态或权限限制阻断正常应用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProcessIdentity {
    comm: Option<String>,
    executable: Option<String>,
    command_line: Vec<String>,
}

impl ProcessIdentity {
    fn for_pid(pid: u32) -> Option<Self> {
        if pid == 0 {
            return None;
        }
        let proc_root = PathBuf::from(format!("/proc/{pid}"));
        let comm = std::fs::read(proc_root.join("comm"))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let executable = std::fs::read_link(proc_root.join("exe"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .filter(|value| !value.is_empty());
        let command_line = std::fs::read(proc_root.join("cmdline"))
            .ok()
            .map(|bytes| {
                bytes
                    .split(|byte| *byte == 0)
                    .filter(|part| !part.is_empty())
                    .map(|part| String::from_utf8_lossy(part).into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if comm.is_none() && executable.is_none() && command_line.is_empty() {
            None
        } else {
            Some(Self {
                comm,
                executable,
                command_line,
            })
        }
    }

    /// 返回应禁止触发整文件 hydration 的后台读取者标签。
    fn background_reader(&self) -> Option<&str> {
        const DEDICATED_BACKGROUND_READERS: &[&str] = &[
            // KDE：Dolphin 当前会在自身进程内解析 PDF/PPT 缩略图。
            "dolphin",
            "baloo_file",
            "baloo_file_extractor",
            "kio_thumbnail",
            // GNOME / Fedora / Ubuntu。
            "gnome-desktop-thumbnailer",
            "tracker-extract",
            "tracker-extract-3",
            "tracker-miner-fs",
            "tracker-miner-fs-3",
            "papers-thumbnailer",
            // Xfce / 通用桌面缩略图工具。
            "tumblerd",
            "evince-thumbnailer",
            "ffmpegthumbnailer",
            "gdk-pixbuf-thumbnailer",
            "totem-video-thumbnailer",
        ];

        let matches_dedicated = |candidate: &str| {
            let basename = Path::new(candidate)
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| candidate.into());
            DEDICATED_BACKGROUND_READERS
                .iter()
                .any(|known| basename.eq_ignore_ascii_case(known))
        };
        if let Some(label) = self
            .comm
            .as_deref()
            .filter(|candidate| matches_dedicated(candidate))
        {
            return Some(label);
        }
        if let Some(label) = self
            .executable
            .as_deref()
            .filter(|candidate| matches_dedicated(candidate))
        {
            return Some(label);
        }
        if let Some(label) = self
            .command_line
            .first()
            .map(String::as_str)
            .filter(|candidate| matches_dedicated(candidate))
        {
            return Some(label);
        }

        // Plasma 可能复用通用 kioworker，仅通过参数选择 thumbnail worker；普通
        // file worker 承担用户复制，绝不能一并拦截。
        let primary = self
            .executable
            .as_deref()
            .or(self.comm.as_deref())
            .or_else(|| self.command_line.first().map(String::as_str));
        let is_kio_worker = primary.is_some_and(|candidate| {
            Path::new(candidate)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("kioworker"))
        });
        let command_mentions_thumbnail = self.command_line.iter().any(|argument| {
            let argument = argument.to_ascii_lowercase();
            argument.contains("thumbnail") || argument.contains("thumbcreator")
        });
        if is_kio_worker && command_mentions_thumbnail {
            return primary;
        }

        // GNOME 49+ 会通过 bubblewrap 把原文件只读绑定到临时沙箱，再运行
        // papers-thumbnailer 等缩略图程序。FUSE 最先看到的调用者可能是 bwrap，
        // 而不是最终的 thumbnailer；只在参数明确包含 GNOME 缩略图临时路径或
        // *-thumbnailer 可执行文件时拦截，不能把普通 Flatpak/bwrap 访问一并拒绝。
        let is_bwrap = primary.is_some_and(|candidate| {
            Path::new(candidate)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("bwrap"))
        });
        let command_mentions_gnome_thumbnailer = self.command_line.iter().any(|argument| {
            let argument_lower = argument.to_ascii_lowercase();
            argument_lower.contains("gnome-desktop-thumbnailer")
                || argument_lower.contains("gnome-desktop-file-to-thumbnail")
                || Path::new(argument)
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with("-thumbnailer"))
        });
        if is_bwrap && command_mentions_gnome_thumbnailer {
            return primary;
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    file_type: u32,
}

#[derive(Debug)]
struct DirectoryEntry {
    inode: INodeNo,
    kind: FileType,
    name: OsString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenHandleKind {
    File,
    Directory,
}

struct OpenFileHandle {
    inode: INodeNo,
    file: Option<Arc<File>>,
    readable: bool,
    writable: bool,
    path_lease: Option<Box<dyn PathLease>>,
}

enum OpenHandle {
    File(OpenFileHandle),
    Directory { inode: INodeNo },
}

impl std::fmt::Debug for OpenFileHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenFileHandle")
            .field("inode", &self.inode)
            .field("file", &self.file)
            .field("readable", &self.readable)
            .field("writable", &self.writable)
            .field("path_lease", &self.path_lease.as_ref().map(|_| "<lease>"))
            .finish()
    }
}

impl std::fmt::Debug for OpenHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(opened) => formatter.debug_tuple("File").field(opened).finish(),
            Self::Directory { inode } => formatter
                .debug_struct("Directory")
                .field("inode", inode)
                .finish(),
        }
    }
}

impl OpenHandle {
    fn inode(&self) -> INodeNo {
        match self {
            Self::File(opened) => opened.inode,
            Self::Directory { inode } => *inode,
        }
    }

    fn kind(&self) -> OpenHandleKind {
        match self {
            Self::File(_) => OpenHandleKind::File,
            Self::Directory { .. } => OpenHandleKind::Directory,
        }
    }
}

#[derive(Clone)]
struct VirtualDriveFs {
    backing_root: PathBuf,
    runtime: Handle,
    hydrator: Arc<dyn Hydrator>,
    mutation_coordinator: Option<Arc<dyn MutationCoordinator>>,
    read_only: bool,
    inodes: Arc<InodeTable>,
    next_handle: Arc<AtomicU64>,
    open_handles: Arc<Mutex<HashMap<FileHandle, OpenHandle>>>,
    hydration_locks: Arc<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>>,
    namespace_lock: Arc<AsyncRwLock<()>>,
    cancellation: Arc<MountCancellation>,
}

impl std::fmt::Debug for VirtualDriveFs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VirtualDriveFs")
            .field("backing_root", &self.backing_root)
            .field("read_only", &self.read_only)
            .field("inodes", &self.inodes)
            .finish_non_exhaustive()
    }
}

impl VirtualDriveFs {
    fn new(
        backing_root: PathBuf,
        runtime: Handle,
        hydrator: Arc<dyn Hydrator>,
        mutation_coordinator: Option<Arc<dyn MutationCoordinator>>,
        read_only: bool,
        cancellation: Arc<MountCancellation>,
    ) -> Self {
        Self {
            backing_root,
            runtime,
            hydrator,
            mutation_coordinator,
            read_only,
            inodes: Arc::new(InodeTable::new()),
            next_handle: Arc::new(AtomicU64::new(1)),
            open_handles: Arc::new(Mutex::new(HashMap::new())),
            hydration_locks: Arc::new(Mutex::new(HashMap::new())),
            namespace_lock: Arc::new(AsyncRwLock::new(())),
            cancellation,
        }
    }

    fn ensure_writable(&self) -> io::Result<()> {
        if self.read_only {
            Err(os_error(libc::EROFS))
        } else {
            Ok(())
        }
    }

    async fn namespace_read(&self) -> io::Result<tokio::sync::RwLockReadGuard<'_, ()>> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(os_error(libc::EINTR)),
            guard = self.namespace_lock.read() => Ok(guard),
        }
    }

    async fn namespace_write(&self) -> io::Result<tokio::sync::RwLockWriteGuard<'_, ()>> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(os_error(libc::EINTR)),
            guard = self.namespace_lock.write() => Ok(guard),
        }
    }

    async fn acquire_path_lease(
        &self,
        mut paths: Vec<PathBuf>,
        exclusive: bool,
    ) -> io::Result<Box<dyn PathLease>> {
        if exclusive {
            self.ensure_writable()?;
        }
        for path in &paths {
            ensure_visible_relative_path(path)?;
        }
        paths.sort_by(|left, right| {
            left.as_os_str()
                .as_bytes()
                .cmp(right.as_os_str().as_bytes())
        });
        paths.dedup();
        let coordinator = self
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| os_error(libc::EROFS))?;
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(os_error(libc::EINTR)),
            lease = async {
                if exclusive {
                    coordinator.acquire_exclusive(paths).await
                } else {
                    coordinator.acquire_shared(paths).await
                }
            } => lease,
        }
    }

    async fn acquire_mutation_lease(&self, paths: Vec<PathBuf>) -> io::Result<Box<dyn PathLease>> {
        self.acquire_path_lease(paths, true).await
    }

    fn relative_for_inode(&self, inode: INodeNo) -> io::Result<PathBuf> {
        self.inodes
            .path(inode)
            .ok_or_else(|| os_error(libc::ENOENT))
    }

    fn backing_path(&self, relative_path: &Path) -> PathBuf {
        self.backing_root.join(relative_path)
    }

    fn entry_for_relative(&self, relative_path: &Path) -> io::Result<FileAttr> {
        ensure_visible_relative_path(relative_path)?;
        let path = self.backing_path(relative_path);
        let metadata = symlink_metadata_beneath(&self.backing_root, relative_path)?;
        ensure_supported_type(&metadata)?;
        let placeholder = if metadata.is_file() {
            read_placeholder_metadata(&path)?
        } else {
            None
        };
        let inode = self.inodes.get_or_insert(relative_path);
        Ok(file_attr(inode, &metadata, placeholder.as_ref()))
    }

    fn child_relative(&self, parent: INodeNo, name: &OsStr) -> io::Result<PathBuf> {
        validate_child_name(name)?;
        let mut relative_path = self.relative_for_inode(parent)?;
        relative_path.push(name);
        ensure_safe_relative_path(&relative_path)?;
        Ok(relative_path)
    }

    fn visible_child_relative(&self, parent: INodeNo, name: &OsStr) -> io::Result<PathBuf> {
        let relative_path = self.child_relative(parent, name)?;
        ensure_visible_relative_path(&relative_path)?;
        Ok(relative_path)
    }

    fn list_directory(&self, inode: INodeNo) -> io::Result<Vec<DirectoryEntry>> {
        let relative_path = self.relative_for_inode(inode)?;
        let path = self.backing_path(&relative_path);
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }

        let parent_relative = relative_path.parent().unwrap_or(Path::new(""));
        let parent_inode = self.inodes.get_or_insert(parent_relative);
        let mut entries = vec![
            DirectoryEntry {
                inode,
                kind: FileType::Directory,
                name: OsString::from("."),
            },
            DirectoryEntry {
                inode: parent_inode,
                kind: FileType::Directory,
                name: OsString::from(".."),
            },
        ];

        let mut children = Vec::new();
        for result in std::fs::read_dir(path)? {
            let entry = result?;
            let name = entry.file_name();
            if name.to_str().is_none() {
                // 同步数据库当前使用 UTF-8 String 路径；隐藏既有非 UTF-8 项，避免
                // to_string_lossy 碰撞导致错误上传或删除。
                continue;
            }
            validate_child_name(&name)?;
            if is_hidden_name(&name) {
                continue;
            }
            let file_type = entry.file_type()?;
            let kind = if file_type.is_dir() {
                FileType::Directory
            } else if file_type.is_file() {
                FileType::RegularFile
            } else {
                // symlink、设备、socket 等均不暴露给云盘视图。
                continue;
            };
            let child_relative = relative_path.join(&name);
            let child_inode = self.inodes.get_or_insert(&child_relative);
            children.push(DirectoryEntry {
                inode: child_inode,
                kind,
                name,
            });
        }
        children.sort_by(|left, right| {
            left.name
                .as_os_str()
                .as_bytes()
                .cmp(right.name.as_os_str().as_bytes())
        });
        entries.extend(children);
        Ok(entries)
    }

    #[cfg(test)]
    async fn open_inode(&self, inode: INodeNo, flags: OpenFlags) -> io::Result<FileHandle> {
        self.open_inode_for_caller(inode, flags, None).await
    }

    /// 打开普通文件，并在占位文件产生句柄前应用后台预览保护。
    ///
    /// 必须在 FUSE `open` 回调中同步取得调用者身份再传入这里：KIO 缩略图 worker
    /// 可能很快退出或复用，若等异步 `read` 任务运行后才读取 `/proc`，会因竞态而
    /// fail-open，继而把整个目录的占位文件加入下载队列。
    async fn open_inode_for_caller(
        &self,
        inode: INodeNo,
        flags: OpenFlags,
        hydration_caller: Option<(u32, &ProcessIdentity)>,
    ) -> io::Result<FileHandle> {
        let _namespace_guard = self.namespace_read().await?;
        let relative_path = self.relative_for_inode(inode)?;
        let path = self.backing_path(&relative_path);
        let readable = flags.acc_mode() != OpenAccMode::O_WRONLY;
        let writable = flags.acc_mode() != OpenAccMode::O_RDONLY;
        let truncate = flags.0 & libc::O_TRUNC != 0;
        if writable {
            self.ensure_writable()?;
        } else if truncate {
            return Err(os_error(libc::EINVAL));
        }

        // 先用 shared 覆盖 lstat/xattr 与可能的 hydration。writable placeholder 不能
        // 在 exclusive 下下载，因为 TaskRunner 自身需要同路径 shared activity。
        let shared_lease = if self.mutation_coordinator.is_some() {
            Some(
                self.acquire_path_lease(vec![relative_path.clone()], false)
                    .await?,
            )
        } else {
            None
        };
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if metadata.is_dir() {
            return Err(os_error(libc::EISDIR));
        }
        if flags.0 & libc::O_DIRECTORY != 0 {
            return Err(os_error(libc::ENOTDIR));
        }
        let initial_placeholder = read_placeholder_metadata(&path)?.is_some();
        if initial_placeholder && !writable {
            if let Some((pid, caller)) = hydration_caller {
                if let Some(process) = caller.background_reader() {
                    tracing::info!(
                        pid,
                        process,
                        relative_path = %relative_path.display(),
                        "已在打开阶段阻止后台预览或索引访问占位文件"
                    );
                    return Err(os_error(libc::EOPNOTSUPP));
                }
            }
        }
        if initial_placeholder && writable && !truncate {
            self.ensure_hydrated(&relative_path, &path).await?;
        }
        let path_lease = if writable {
            let expected_identity = path_identity_beneath(&self.backing_root, &relative_path)?;
            drop(shared_lease);
            let exclusive = self
                .acquire_mutation_lease(vec![relative_path.clone()])
                .await?;
            verify_path_identity(&self.backing_root, &relative_path, expected_identity)?;
            Some(exclusive)
        } else {
            shared_lease
        };
        let placeholder = read_placeholder_metadata(&path)?.is_some();
        if writable && !truncate && placeholder {
            // shared→exclusive 的极短切换窗口中状态改变；不能把新占位当 0 字节内容。
            return Err(os_error(libc::EBUSY));
        }

        // 纯只读打开占位文件不触发网络；首次 read 时下载并把真实 fd 写回句柄。
        let file = if placeholder && !writable {
            drop(open_read_only_no_follow(&path)?);
            None
        } else {
            let open_flags = if placeholder && truncate {
                OpenFlags(flags.0 & !libc::O_TRUNC)
            } else {
                flags
            };
            let file = Arc::new(open_existing_no_follow(&path, open_flags)?);
            if placeholder && truncate {
                convert_placeholder_to_truncated(&path, &file)?;
            }
            Some(file)
        };

        let handle_lease = if writable || (placeholder && file.is_none()) {
            path_lease
        } else {
            // 真实 fd 已固定 inode；只读访问不再需要阻塞后台的排他 free-up/删除。
            None
        };
        self.allocate_file_handle(inode, file, readable, writable, handle_lease)
    }

    fn open_directory(&self, inode: INodeNo, flags: OpenFlags) -> io::Result<FileHandle> {
        if flags.acc_mode() != OpenAccMode::O_RDONLY || flags.0 & libc::O_TRUNC != 0 {
            return Err(os_error(libc::EISDIR));
        }
        let relative_path = self.relative_for_inode(inode)?;
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }
        self.allocate_directory_handle(inode)
    }

    fn next_file_handle(&self) -> io::Result<FileHandle> {
        let raw_handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        if raw_handle == 0 {
            return Err(io::Error::other("FUSE 文件句柄空间耗尽"));
        }
        Ok(FileHandle(raw_handle))
    }

    fn allocate_file_handle(
        &self,
        inode: INodeNo,
        file: Option<Arc<File>>,
        readable: bool,
        writable: bool,
        path_lease: Option<Box<dyn PathLease>>,
    ) -> io::Result<FileHandle> {
        let handle = self.next_file_handle()?;
        self.open_handles.lock().insert(
            handle,
            OpenHandle::File(OpenFileHandle {
                inode,
                file,
                readable,
                writable,
                path_lease,
            }),
        );
        Ok(handle)
    }

    fn allocate_directory_handle(&self, inode: INodeNo) -> io::Result<FileHandle> {
        let handle = self.next_file_handle()?;
        self.open_handles
            .lock()
            .insert(handle, OpenHandle::Directory { inode });
        Ok(handle)
    }

    fn release_handle(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        expected_kind: OpenHandleKind,
    ) -> io::Result<()> {
        let mut handles = self.open_handles.lock();
        match handles.get(&handle) {
            Some(opened) if opened.inode() == inode && opened.kind() == expected_kind => {
                handles.remove(&handle);
                Ok(())
            }
            Some(_) | None => Err(os_error(libc::EBADF)),
        }
    }

    fn validate_handle(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        expected_kind: OpenHandleKind,
    ) -> io::Result<()> {
        match self.open_handles.lock().get(&handle) {
            Some(opened) if opened.inode() == inode && opened.kind() == expected_kind => Ok(()),
            Some(_) | None => Err(os_error(libc::EBADF)),
        }
    }

    fn has_writable_handle(&self, inode: INodeNo) -> bool {
        self.open_handles.lock().values().any(
            |opened| matches!(opened, OpenHandle::File(file) if file.inode == inode && file.writable),
        )
    }

    fn has_pending_read_handle(&self, inode: INodeNo) -> bool {
        self.open_handles.lock().values().any(
            |opened| matches!(opened, OpenHandle::File(file) if file.inode == inode && file.file.is_none()),
        )
    }

    /// 指定句柄是否仍等待占位内容落地；用于避免普通文件每次 read 都访问 `/proc`。
    fn handle_needs_hydration(&self, inode: INodeNo, handle: FileHandle) -> io::Result<bool> {
        match self.open_handles.lock().get(&handle) {
            Some(OpenHandle::File(opened)) if opened.inode == inode => Ok(opened.file.is_none()),
            Some(OpenHandle::Directory { .. }) | Some(OpenHandle::File(_)) | None => {
                Err(os_error(libc::EBADF))
            }
        }
    }

    async fn opened_file(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        require_readable: bool,
        require_writable: bool,
        hydration_caller: Option<(u32, &ProcessIdentity)>,
    ) -> io::Result<Arc<File>> {
        let pending = {
            let handles = self.open_handles.lock();
            match handles.get(&handle) {
                Some(OpenHandle::File(opened)) if opened.inode == inode => {
                    if require_readable && !opened.readable {
                        return Err(os_error(libc::EBADF));
                    }
                    if require_writable && !opened.writable {
                        return Err(os_error(libc::EBADF));
                    }
                    if let Some(file) = &opened.file {
                        return Ok(Arc::clone(file));
                    }
                    true
                }
                Some(OpenHandle::Directory { .. }) | None => return Err(os_error(libc::EBADF)),
                Some(OpenHandle::File(_)) => return Err(os_error(libc::EBADF)),
            }
        };
        debug_assert!(pending);

        let _namespace_guard = self.namespace_read().await?;
        // 另一请求可能已在等待 namespace lock 时完成 hydration。
        {
            let handles = self.open_handles.lock();
            match handles.get(&handle) {
                Some(OpenHandle::File(opened)) if opened.inode == inode => {
                    if let Some(file) = &opened.file {
                        return Ok(Arc::clone(file));
                    }
                }
                _ => return Err(os_error(libc::EBADF)),
            }
        }

        let relative_path = self.relative_for_inode(inode)?;
        let path = self.backing_path(&relative_path);
        if read_placeholder_metadata(&path)?.is_some() {
            if let Some((pid, caller)) = hydration_caller {
                if let Some(process) = caller.background_reader() {
                    tracing::info!(
                        pid,
                        process,
                        relative_path = %relative_path.display(),
                        "已阻止后台预览或索引触发占位文件整文件下载"
                    );
                    return Err(os_error(libc::EOPNOTSUPP));
                }
            }
        }
        self.ensure_hydrated(&relative_path, &path).await?;
        let file = Arc::new(open_read_only_no_follow(&path)?);
        let metadata = file.metadata()?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_file() {
            return Err(os_error(libc::EISDIR));
        }

        let mut handles = self.open_handles.lock();
        match handles.get_mut(&handle) {
            Some(OpenHandle::File(opened)) if opened.inode == inode && opened.file.is_none() => {
                opened.file = Some(Arc::clone(&file));
                opened.path_lease.take();
                Ok(file)
            }
            Some(OpenHandle::File(opened)) if opened.inode == inode => opened
                .file
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| os_error(libc::EBADF)),
            _ => Err(os_error(libc::EBADF)),
        }
    }

    #[cfg(test)]
    async fn read_inode(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        size: u32,
    ) -> io::Result<Vec<u8>> {
        self.read_inode_for_caller(inode, handle, offset, size, None)
            .await
    }

    /// 按 FUSE 请求进程身份读取文件；仅占位文件首次 read 会使用该身份做预览保护。
    async fn read_inode_for_caller(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        size: u32,
        hydration_caller: Option<(u32, &ProcessIdentity)>,
    ) -> io::Result<Vec<u8>> {
        if size == 0 {
            self.validate_handle(inode, handle, OpenHandleKind::File)?;
            return Ok(Vec::new());
        }
        let file = self
            .opened_file(inode, handle, true, false, hydration_caller)
            .await?;
        read_at(&file, offset, size)
    }

    async fn write_inode(
        &self,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
    ) -> io::Result<u32> {
        self.ensure_writable()?;
        let file = self.opened_file(inode, handle, false, true, None).await?;
        write_at(&file, offset, data)
    }

    async fn ensure_hydrated(&self, relative_path: &Path, path: &Path) -> io::Result<()> {
        let Some(_) = read_placeholder_metadata(path)? else {
            return Ok(());
        };

        let hydration_lock = self
            .hydration_locks
            .lock()
            .entry(relative_path.to_path_buf())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let _guard = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Err(os_error(libc::EINTR)),
            guard = hydration_lock.lock() => guard,
        };

        // 并发读者等待首个下载完成后必须重新检查，避免重复提交同一下载任务。
        let Some(placeholder) = read_placeholder_metadata(path)? else {
            return Ok(());
        };
        let request = HydrationRequest {
            relative_path: relative_path.to_path_buf(),
            backing_path: path.to_path_buf(),
            file_id: placeholder.file_id,
            logical_size: placeholder.logical_size,
        };

        let hydrator = Arc::clone(&self.hydrator);
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Err(os_error(libc::EINTR)),
            result = hydrator.hydrate(request) => result?,
        }

        let metadata = symlink_metadata_beneath(&self.backing_root, relative_path)?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_file() {
            return Err(io::Error::other("按需下载结果不是普通文件"));
        }
        if read_placeholder_metadata(path)?.is_some() {
            return Err(io::Error::other(
                "按需下载返回成功，但 backing 文件仍是占位状态",
            ));
        }
        Ok(())
    }

    async fn prepare_open_handles_before_unlink(
        &self,
        inode: INodeNo,
        relative_path: &Path,
        path: &Path,
    ) -> io::Result<()> {
        let has_pending_handle = self.open_handles.lock().values().any(
            |opened| matches!(opened, OpenHandle::File(file) if file.inode == inode && file.file.is_none()),
        );
        if !has_pending_handle {
            return Ok(());
        }

        self.ensure_hydrated(relative_path, path).await?;
        let file = Arc::new(open_read_only_no_follow(path)?);
        let mut handles = self.open_handles.lock();
        for opened in handles.values_mut() {
            if let OpenHandle::File(opened) = opened {
                if opened.inode == inode && opened.file.is_none() {
                    opened.file = Some(Arc::clone(&file));
                    opened.path_lease.take();
                }
            }
        }
        Ok(())
    }

    async fn create_file(
        &self,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        raw_flags: i32,
    ) -> io::Result<(FileAttr, FileHandle)> {
        self.ensure_writable()?;
        if mode & libc::S_IFMT != 0 && mode & libc::S_IFMT != libc::S_IFREG {
            return Err(os_error(libc::EPERM));
        }
        let _namespace_guard = self.namespace_write().await?;
        let relative_path = self.visible_child_relative(parent, name)?;
        self.ensure_directory_inode(parent)?;
        let mutation_lease = self
            .acquire_mutation_lease(vec![relative_path.clone()])
            .await?;
        self.ensure_directory_inode(parent)?;
        let path = self.backing_path(&relative_path);
        let permissions = mode & !umask & 0o7777;
        let creation = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(permissions)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path);
        let created = creation?;
        drop(created);

        let flags = OpenFlags(raw_flags & !(libc::O_CREAT | libc::O_EXCL | libc::O_TRUNC));
        let file = match open_existing_no_follow(&path, flags) {
            Ok(file) => Arc::new(file),
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
        };
        let inode = self.inodes.get_or_insert(&relative_path);
        let readable = flags.acc_mode() != OpenAccMode::O_WRONLY;
        let writable = flags.acc_mode() != OpenAccMode::O_RDONLY;
        let handle_lease = writable.then_some(mutation_lease);
        let handle =
            self.allocate_file_handle(inode, Some(file), readable, writable, handle_lease)?;
        let attr = self.entry_for_relative(&relative_path)?;
        Ok((attr, handle))
    }

    async fn create_directory(
        &self,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
    ) -> io::Result<FileAttr> {
        self.ensure_writable()?;
        let _namespace_guard = self.namespace_write().await?;
        let relative_path = self.visible_child_relative(parent, name)?;
        self.ensure_directory_inode(parent)?;
        let _mutation_lease = self
            .acquire_mutation_lease(vec![relative_path.clone()])
            .await?;
        self.ensure_directory_inode(parent)?;
        let path = self.backing_path(&relative_path);
        std::fs::DirBuilder::new()
            .mode(mode & !umask & 0o7777)
            .create(&path)?;
        self.entry_for_relative(&relative_path)
    }

    fn ensure_directory_inode(&self, inode: INodeNo) -> io::Result<PathBuf> {
        let relative_path = self.relative_for_inode(inode)?;
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }
        Ok(relative_path)
    }

    async fn unlink_file(&self, parent: INodeNo, name: &OsStr) -> io::Result<()> {
        self.ensure_writable()?;
        let _namespace_guard = self.namespace_write().await?;
        let relative_path = self.visible_child_relative(parent, name)?;
        self.ensure_directory_inode(parent)?;
        let path = self.backing_path(&relative_path);
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if metadata.is_dir() {
            return Err(os_error(libc::EISDIR));
        }
        let inode = self.inodes.get_or_insert(&relative_path);
        if self.has_writable_handle(inode) {
            return Err(os_error(libc::EBUSY));
        }
        let shared_lease = self
            .acquire_path_lease(vec![relative_path.clone()], false)
            .await?;
        self.prepare_open_handles_before_unlink(inode, &relative_path, &path)
            .await?;
        let expected_identity = path_identity_beneath(&self.backing_root, &relative_path)?;
        drop(shared_lease);
        let _mutation_lease = self
            .acquire_mutation_lease(vec![relative_path.clone()])
            .await?;
        verify_path_identity(&self.backing_root, &relative_path, expected_identity)?;
        std::fs::remove_file(&path)?;
        self.inodes.remove_subtree(&relative_path);
        Ok(())
    }

    async fn remove_directory(&self, parent: INodeNo, name: &OsStr) -> io::Result<()> {
        self.ensure_writable()?;
        let _namespace_guard = self.namespace_write().await?;
        let relative_path = self.visible_child_relative(parent, name)?;
        self.ensure_directory_inode(parent)?;
        let shared_lease = self
            .acquire_path_lease(vec![relative_path.clone()], false)
            .await?;
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }
        let expected_identity = file_identity(&metadata);
        drop(shared_lease);
        let _mutation_lease = self
            .acquire_mutation_lease(vec![relative_path.clone()])
            .await?;
        verify_path_identity(&self.backing_root, &relative_path, expected_identity)?;
        std::fs::remove_dir(self.backing_path(&relative_path))?;
        self.inodes.remove_subtree(&relative_path);
        Ok(())
    }

    async fn rename_entry(
        &self,
        parent: INodeNo,
        name: &OsStr,
        new_parent: INodeNo,
        new_name: &OsStr,
        flags: RenameFlags,
    ) -> io::Result<()> {
        self.ensure_writable()?;
        if flags.contains(RenameFlags::RENAME_EXCHANGE)
            || flags.contains(RenameFlags::RENAME_WHITEOUT)
            || flags.bits() & !RenameFlags::RENAME_NOREPLACE.bits() != 0
        {
            return Err(os_error(libc::EOPNOTSUPP));
        }

        let _namespace_guard = self.namespace_write().await?;
        let source = self.visible_child_relative(parent, name)?;
        let destination = self.visible_child_relative(new_parent, new_name)?;
        self.ensure_directory_inode(parent)?;
        self.ensure_directory_inode(new_parent)?;
        if source == destination {
            return Ok(());
        }
        let shared_lease = self
            .acquire_path_lease(vec![source.clone(), destination.clone()], false)
            .await?;
        let source_metadata = symlink_metadata_beneath(&self.backing_root, &source)?;
        ensure_supported_type(&source_metadata)?;
        if source_metadata.is_dir() && destination.starts_with(&source) {
            return Err(os_error(libc::EINVAL));
        }
        let source_inode = self.inodes.get_or_insert(&source);
        if self.has_writable_handle(source_inode) {
            return Err(os_error(libc::EBUSY));
        }
        let source_path = self.backing_path(&source);
        // 仅已打开且尚未水合的句柄需要在 rename 前固定真实 fd；未打开的单文件或
        // 整个目录树中的 placeholder 可原样离线移动，目标路径凭 fileId xattr 水合。
        self.promote_pending_handles_in_subtree(&source).await?;
        let source_metadata = symlink_metadata_beneath(&self.backing_root, &source)?;
        ensure_supported_type(&source_metadata)?;

        let destination_path = self.backing_path(&destination);
        let destination_metadata = match std::fs::symlink_metadata(&destination_path) {
            Ok(metadata) => {
                ensure_supported_type(&metadata)?;
                if source_metadata.is_dir() != metadata.is_dir() {
                    return Err(os_error(if source_metadata.is_dir() {
                        libc::ENOTDIR
                    } else {
                        libc::EISDIR
                    }));
                }
                // 云端目录具有稳定 folderId；在完整的远端目标替换事务落地前，
                // 不把 POSIX 的“覆盖空目录”扩大为可能覆盖另一云端目录身份。
                if source_metadata.is_dir() {
                    return Err(os_error(libc::EEXIST));
                }
                let source_owner = crate::platform::xattr::get(&source_path, XATTR_FILE_ID)?;
                if let Some(source_owner) = source_owner {
                    let destination_owner =
                        crate::platform::xattr::get(&destination_path, XATTR_FILE_ID)?;
                    if destination_owner.as_deref() != Some(source_owner.as_slice()) {
                        // 云端文件 A 覆盖另一身份 B 需要“删除 B + 移动 A”的远端事务；
                        // 普通编辑器临时文件没有 fileId，仍可原子替换目标完成保存。
                        return Err(os_error(libc::EEXIST));
                    }
                }
                if metadata.is_dir()
                    && std::fs::read_dir(&destination_path)?
                        .next()
                        .transpose()?
                        .is_some()
                {
                    return Err(os_error(libc::ENOTEMPTY));
                }
                let destination_inode = self.inodes.get_or_insert(&destination);
                if self.has_writable_handle(destination_inode) {
                    return Err(os_error(libc::EBUSY));
                }
                self.prepare_open_handles_before_unlink(
                    destination_inode,
                    &destination,
                    &destination_path,
                )
                .await?;
                Some(symlink_metadata_beneath(&self.backing_root, &destination)?)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let expected_source = file_identity(&source_metadata);
        let expected_destination = destination_metadata.as_ref().map(file_identity);
        drop(shared_lease);
        let mutation_lease = self
            .acquire_mutation_lease(vec![source.clone(), destination.clone()])
            .await?;
        verify_path_identity(&self.backing_root, &source, expected_source)?;
        verify_optional_path_identity(&self.backing_root, &destination, expected_destination)?;
        if destination_metadata.as_ref().is_some_and(Metadata::is_dir)
            && std::fs::read_dir(&destination_path)?
                .next()
                .transpose()?
                .is_some()
        {
            return Err(os_error(libc::ENOTEMPTY));
        }

        let coordinator = self
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| os_error(libc::EROFS))?;
        coordinator
            .prepare_rename(
                source.clone(),
                destination.clone(),
                source_metadata.is_dir(),
            )
            .await?;
        let no_replace = flags.contains(RenameFlags::RENAME_NOREPLACE);
        rename_beneath(&source_path, &destination_path, no_replace)?;
        self.inodes.rename_subtree(&source, &destination);
        drop(mutation_lease);

        if let Err(error) = coordinator
            .rename_observed(
                source.clone(),
                destination.clone(),
                source_metadata.is_dir(),
            )
            .await
        {
            // backing rename 已原子提交；通知失败不能用第二次 rename 伪造回滚，
            // 否则崩溃窗口会破坏 POSIX 原子性。watcher/下次扫描仍会收敛。
            tracing::warn!(
                source = %source.display(),
                destination = %destination.display(),
                %error,
                "FUSE rename 后同步通知失败"
            );
        }
        Ok(())
    }

    async fn promote_pending_handles_in_subtree(&self, relative_root: &Path) -> io::Result<()> {
        let mut pending_inodes: Vec<INodeNo> = self
            .open_handles
            .lock()
            .values()
            .filter_map(|opened| match opened {
                OpenHandle::File(file) if file.file.is_none() => Some(file.inode),
                _ => None,
            })
            .collect();
        pending_inodes.sort_by_key(|inode| inode.0);
        pending_inodes.dedup();

        for inode in pending_inodes {
            let Some(relative_path) = self.inodes.path(inode) else {
                continue;
            };
            if relative_path == relative_root || relative_path.starts_with(relative_root) {
                let path = self.backing_path(&relative_path);
                self.prepare_open_handles_before_unlink(inode, &relative_path, &path)
                    .await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn set_attributes(
        &self,
        inode: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        handle: Option<FileHandle>,
        flags: Option<BsdFileFlags>,
    ) -> io::Result<FileAttr> {
        self.ensure_writable()?;
        if flags.is_some() {
            return Err(os_error(libc::EOPNOTSUPP));
        }
        let _namespace_guard = self.namespace_write().await?;

        let relative_path = self.relative_for_inode(inode).ok();
        let path = relative_path
            .as_ref()
            .map(|relative_path| self.backing_path(relative_path));
        if let Some(relative_path) = &relative_path {
            ensure_visible_relative_path(relative_path)?;
            let metadata = symlink_metadata_beneath(&self.backing_root, relative_path)?;
            ensure_supported_type(&metadata)?;
        } else if handle.is_none() {
            return Err(os_error(libc::ENOENT));
        }

        let (mut opened, handle_has_exclusive_lease) = if let Some(handle) = handle {
            let handles = self.open_handles.lock();
            match handles.get(&handle) {
                Some(OpenHandle::File(file)) if file.inode == inode => {
                    if size.is_some() && !file.writable {
                        return Err(os_error(libc::EBADF));
                    }
                    (
                        file.file.as_ref().map(Arc::clone),
                        file.writable && file.path_lease.is_some(),
                    )
                }
                Some(OpenHandle::Directory {
                    inode: opened_inode,
                }) if *opened_inode == inode => (None, false),
                _ => return Err(os_error(libc::EBADF)),
            }
        } else {
            (None, false)
        };

        let has_changes = mode.is_some()
            || uid.is_some()
            || gid.is_some()
            || size.is_some()
            || atime.is_some()
            || mtime.is_some();
        let _operation_lease = if has_changes && !handle_has_exclusive_lease {
            let relative_path = relative_path
                .as_ref()
                .ok_or_else(|| os_error(libc::ENOENT))?;
            let path = path.as_ref().ok_or_else(|| os_error(libc::ENOENT))?;
            let shared_lease = self
                .acquire_path_lease(vec![relative_path.clone()], false)
                .await?;
            if self.has_pending_read_handle(inode) {
                self.prepare_open_handles_before_unlink(inode, relative_path, path)
                    .await?;
            }
            if size.is_some_and(|new_size| new_size > 0)
                && read_placeholder_metadata(path)?.is_some()
            {
                self.ensure_hydrated(relative_path, path).await?;
            }
            let expected_identity = path_identity_beneath(&self.backing_root, relative_path)?;
            drop(shared_lease);
            let lease = self
                .acquire_mutation_lease(vec![relative_path.clone()])
                .await?;
            verify_path_identity(&self.backing_root, relative_path, expected_identity)?;
            Some(lease)
        } else {
            None
        };

        if let Some(new_size) = size {
            let Some(relative_path) = relative_path.as_ref() else {
                let file = opened.as_ref().ok_or_else(|| os_error(libc::EBADF))?;
                file.set_len(new_size)?;
                return Ok(file_attr(inode, &file.metadata()?, None));
            };
            let path = path.as_ref().expect("相对路径存在时 backing 路径必须存在");
            let metadata = symlink_metadata_beneath(&self.backing_root, relative_path)?;
            if metadata.is_dir() {
                return Err(os_error(libc::EISDIR));
            }
            if read_placeholder_metadata(path)?.is_some() {
                if new_size == 0 {
                    let replacement = Arc::new(open_read_write_no_follow(path)?);
                    convert_placeholder_to_truncated(path, &replacement)?;
                    opened = Some(replacement);
                } else {
                    debug_assert!(read_placeholder_metadata(path)?.is_none());
                }
            }
            if opened.is_none() {
                opened = Some(Arc::new(open_read_write_no_follow(path)?));
            }
            opened
                .as_ref()
                .ok_or_else(|| os_error(libc::EBADF))?
                .set_len(new_size)?;
        }

        if mode.is_some() || uid.is_some() || gid.is_some() || atime.is_some() || mtime.is_some() {
            if opened.is_none() {
                let path = path.as_ref().ok_or_else(|| os_error(libc::ENOENT))?;
                opened = Some(Arc::new(open_metadata_no_follow(path)?));
            }
            let file = opened.as_ref().ok_or_else(|| os_error(libc::EBADF))?;
            if let Some(mode) = mode {
                file.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))?;
            }
            if uid.is_some() || gid.is_some() {
                change_file_owner(file, uid, gid)?;
            }
            if atime.is_some() || mtime.is_some() {
                let mut times = FileTimes::new();
                if let Some(atime) = atime {
                    times = times.set_accessed(resolve_time(atime));
                }
                if let Some(mtime) = mtime {
                    times = times.set_modified(resolve_time(mtime));
                }
                file.set_times(times)?;
            }
        }

        if let Some(relative_path) = relative_path {
            self.entry_for_relative(&relative_path)
        } else {
            let file = opened.as_ref().ok_or_else(|| os_error(libc::EBADF))?;
            Ok(file_attr(inode, &file.metadata()?, None))
        }
    }

    fn sync_handle(&self, inode: INodeNo, handle: FileHandle, data_only: bool) -> io::Result<()> {
        let file = {
            let handles = self.open_handles.lock();
            match handles.get(&handle) {
                Some(OpenHandle::File(opened)) if opened.inode == inode => {
                    opened.file.as_ref().map(Arc::clone)
                }
                _ => return Err(os_error(libc::EBADF)),
            }
        };
        match file {
            Some(file) if data_only => file.sync_data(),
            Some(file) => file.sync_all(),
            // 尚未水合的只读占位句柄没有本地脏数据，不应因为 fsync 触发下载。
            None => Ok(()),
        }
    }

    fn attributes_for_inode(
        &self,
        inode: INodeNo,
        handle: Option<FileHandle>,
    ) -> io::Result<FileAttr> {
        if let Some(handle) = handle {
            let handles = self.open_handles.lock();
            return match handles.get(&handle) {
                Some(OpenHandle::File(opened)) if opened.inode == inode => {
                    if let Some(file) = &opened.file {
                        Ok(file_attr(inode, &file.metadata()?, None))
                    } else {
                        drop(handles);
                        let relative_path = self.relative_for_inode(inode)?;
                        self.entry_for_relative(&relative_path)
                    }
                }
                Some(OpenHandle::Directory {
                    inode: opened_inode,
                }) if *opened_inode == inode => {
                    drop(handles);
                    let relative_path = self.relative_for_inode(inode)?;
                    self.entry_for_relative(&relative_path)
                }
                _ => Err(os_error(libc::EBADF)),
            };
        }
        let relative_path = self.relative_for_inode(inode)?;
        self.entry_for_relative(&relative_path)
    }

    fn check_access(&self, inode: INodeNo, mask: AccessFlags) -> io::Result<()> {
        if mask.contains(AccessFlags::W_OK) && self.read_only {
            return Err(os_error(libc::EROFS));
        }
        let relative_path = self.relative_for_inode(inode)?;
        let metadata = symlink_metadata_beneath(&self.backing_root, &relative_path)?;
        ensure_supported_type(&metadata)?;
        if mask.contains(AccessFlags::X_OK) && !metadata.is_dir() {
            return Err(os_error(libc::EACCES));
        }
        Ok(())
    }

    fn filesystem_stats(&self) -> io::Result<FileSystemStats> {
        statvfs(&self.backing_root)
    }
}

impl Filesystem for VirtualDriveFs {
    fn init(&mut self, _request: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // backing 会被同步引擎直接原子替换（下载、释放空间、远端更新）。禁用普通文件
        // 的内核页缓存后，每次新 open 都会重新经过 FUSE，避免读到旧内容或绕过 hydration。
        // 新内核可同时允许 direct-I/O 文件被 mmap；旧内核不支持此能力时仍可正常 read。
        if let Err(unsupported) = config.add_capabilities(InitFlags::FUSE_DIRECT_IO_ALLOW_MMAP) {
            tracing::debug!(
                ?unsupported,
                "内核不支持 direct-I/O mmap；普通读写可用，依赖 mmap 的应用可能受限"
            );
        }
        Ok(())
    }

    fn lookup(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let result = self
            .visible_child_relative(parent, name)
            .and_then(|relative_path| self.entry_for_relative(&relative_path));
        match result {
            Ok(attr) => reply.entry(&ATTR_TTL, &attr, GENERATION),
            Err(error) => reply.error(error.into()),
        }
    }

    fn getattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: Option<FileHandle>,
        reply: ReplyAttr,
    ) {
        let result = self.attributes_for_inode(inode, handle);
        match result {
            Ok(attr) => reply.attr(&ATTR_TTL, &attr),
            Err(error) => reply.error(error.into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn setattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        handle: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let filesystem = self.clone();
        self.runtime.spawn(async move {
            match filesystem
                .set_attributes(inode, mode, uid, gid, size, atime, mtime, handle, flags)
                .await
            {
                Ok(attr) => reply.attr(&ATTR_TTL, &attr),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn mknod(
        &self,
        _request: &Request,
        _parent: INodeNo,
        _name: &OsStr,
        _mode: u32,
        _umask: u32,
        _rdev: u32,
        reply: ReplyEntry,
    ) {
        reply.error(if self.read_only {
            Errno::EROFS
        } else {
            Errno::EPERM
        });
    }

    fn mkdir(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let filesystem = self.clone();
        let name = name.to_os_string();
        self.runtime.spawn(async move {
            match filesystem
                .create_directory(parent, &name, mode, umask)
                .await
            {
                Ok(attr) => reply.entry(&ATTR_TTL, &attr, GENERATION),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn unlink(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let filesystem = self.clone();
        let name = name.to_os_string();
        self.runtime.spawn(async move {
            match filesystem.unlink_file(parent, &name).await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn rmdir(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let filesystem = self.clone();
        let name = name.to_os_string();
        self.runtime.spawn(async move {
            match filesystem.remove_directory(parent, &name).await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn symlink(
        &self,
        _request: &Request,
        _parent: INodeNo,
        _link_name: &OsStr,
        _target: &Path,
        reply: ReplyEntry,
    ) {
        reply.error(if self.read_only {
            Errno::EROFS
        } else {
            Errno::EPERM
        });
    }

    fn rename(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        new_parent: INodeNo,
        new_name: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let filesystem = self.clone();
        let name = name.to_os_string();
        let new_name = new_name.to_os_string();
        self.runtime.spawn(async move {
            match filesystem
                .rename_entry(parent, &name, new_parent, &new_name, flags)
                .await
            {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn link(
        &self,
        _request: &Request,
        _inode: INodeNo,
        _new_parent: INodeNo,
        _new_name: &OsStr,
        reply: ReplyEntry,
    ) {
        reply.error(if self.read_only {
            Errno::EROFS
        } else {
            Errno::EPERM
        });
    }

    fn readdir(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let entries = match self
            .validate_handle(inode, handle, OpenHandleKind::Directory)
            .and_then(|()| self.list_directory(inode))
        {
            Ok(entries) => entries,
            Err(error) => {
                reply.error(error.into());
                return;
            }
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        for (index, entry) in entries.into_iter().enumerate().skip(start) {
            if reply.add(entry.inode, (index + 1) as u64, entry.kind, entry.name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, request: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        // `/proc` 身份必须在同步 FUSE 回调期间快照；不要推迟到 Tokio 任务中。
        let pid = request.pid();
        let caller = ProcessIdentity::for_pid(pid);
        let filesystem = self.clone();
        self.runtime.spawn(async move {
            match filesystem
                .open_inode_for_caller(inode, flags, caller.as_ref().map(|caller| (pid, caller)))
                .await
            {
                Ok(handle) => reply.opened(handle, FopenFlags::FOPEN_DIRECT_IO),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn opendir(&self, _request: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        match self.open_directory(inode, flags) {
            Ok(handle) => reply.opened(handle, FopenFlags::empty()),
            Err(error) => reply.error(error.into()),
        }
    }

    fn read(
        &self,
        request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        // ReplyData 可跨线程持有；网络 hydration 完全在 Tokio 任务中运行，FUSE
        // session worker 在调度后立即返回，不会因 4 个并发云文件读而全部饿死。
        let pid = request.pid();
        let caller = self
            .handle_needs_hydration(inode, handle)
            .ok()
            .filter(|needs_hydration| *needs_hydration)
            .and_then(|_| ProcessIdentity::for_pid(pid));
        let filesystem = self.clone();
        self.runtime.spawn(async move {
            match filesystem
                .read_inode_for_caller(
                    inode,
                    handle,
                    offset,
                    size,
                    caller.as_ref().map(|caller| (pid, caller)),
                )
                .await
            {
                Ok(data) => reply.data(&data),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn write(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let filesystem = self.clone();
        let data = data.to_vec();
        self.runtime.spawn(async move {
            match filesystem.write_inode(inode, handle, offset, &data).await {
                Ok(written) => reply.written(written),
                Err(error) => reply.error(error.into()),
            }
        });
    }

    fn flush(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        match self.validate_handle(inode, handle, OpenHandleKind::File) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error.into()),
        }
    }

    fn fsync(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let filesystem = self.clone();
        self.runtime.spawn(async move {
            let sync = tokio::task::spawn_blocking(move || {
                filesystem.sync_handle(inode, handle, datasync)
            })
            .await;
            match sync {
                Ok(Ok(())) => reply.ok(),
                Ok(Err(error)) => reply.error(error.into()),
                Err(error) => {
                    reply.error(io::Error::other(format!("fsync 任务异常结束：{error}")).into())
                }
            }
        });
    }

    fn release(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.release_handle(inode, handle, OpenHandleKind::File) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error.into()),
        }
    }

    fn releasedir(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        match self.release_handle(inode, handle, OpenHandleKind::Directory) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error.into()),
        }
    }

    fn access(&self, _request: &Request, inode: INodeNo, mask: AccessFlags, reply: ReplyEmpty) {
        match self.check_access(inode, mask) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(error.into()),
        }
    }

    fn statfs(&self, _request: &Request, _inode: INodeNo, reply: ReplyStatfs) {
        match self.filesystem_stats() {
            Ok(stats) => reply.statfs(
                stats.blocks,
                stats.blocks_free,
                stats.blocks_available,
                stats.files,
                stats.files_free,
                stats.block_size,
                stats.name_length,
                stats.fragment_size,
            ),
            Err(error) => reply.error(error.into()),
        }
    }

    fn create(
        &self,
        _request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let filesystem = self.clone();
        let name = name.to_os_string();
        self.runtime.spawn(async move {
            match filesystem
                .create_file(parent, &name, mode, umask, flags)
                .await
            {
                Ok((attr, handle)) => reply.created(
                    &ATTR_TTL,
                    &attr,
                    GENERATION,
                    handle,
                    FopenFlags::FOPEN_DIRECT_IO,
                ),
                Err(error) => reply.error(error.into()),
            }
        });
    }
}

fn checked_real_directory(path: &Path, label: &str) -> io::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{label} 目录不可用（{}）：{error}", path.display()),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} 目录不能是符号链接"),
        ));
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} 必须是目录"),
        ));
    }
    std::fs::canonicalize(path)
}

fn ensure_safe_relative_path(path: &Path) -> io::Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "FUSE backing 路径不是安全相对路径",
        ));
    }
    Ok(())
}

/// 逐层 `lstat`，确保 backing 相对路径的任意组件都不是 symlink。
///
/// 叶节点随后仍使用 `O_NOFOLLOW` 打开；逐层校验负责阻止普通情况下通过中间目录
/// symlink 逃出 backing。完全消除同 UID 恶意进程制造的竞态需后续改为 fd-relative
/// `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)`。
fn symlink_metadata_beneath(root: &Path, relative_path: &Path) -> io::Result<Metadata> {
    ensure_safe_relative_path(relative_path)?;
    let mut current = root.to_path_buf();
    let mut metadata = std::fs::symlink_metadata(&current)?;
    ensure_supported_type(&metadata)?;

    let components: Vec<_> = relative_path.components().collect();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            continue;
        };
        if !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }
        current.push(name);
        metadata = std::fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            return Err(os_error(libc::ELOOP));
        }
        if index + 1 != components.len() && !metadata.is_dir() {
            return Err(os_error(libc::ENOTDIR));
        }
    }
    Ok(metadata)
}

fn file_identity(metadata: &Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        file_type: metadata.mode() & libc::S_IFMT,
    }
}

fn path_identity_beneath(root: &Path, relative_path: &Path) -> io::Result<FileIdentity> {
    let metadata = symlink_metadata_beneath(root, relative_path)?;
    ensure_supported_type(&metadata)?;
    Ok(file_identity(&metadata))
}

fn verify_path_identity(
    root: &Path,
    relative_path: &Path,
    expected: FileIdentity,
) -> io::Result<()> {
    let actual = path_identity_beneath(root, relative_path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(os_error(libc::EBUSY))
    }
}

fn verify_optional_path_identity(
    root: &Path,
    relative_path: &Path,
    expected: Option<FileIdentity>,
) -> io::Result<()> {
    match expected {
        Some(expected) => verify_path_identity(root, relative_path, expected),
        None => match symlink_metadata_beneath(root, relative_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(os_error(libc::EBUSY)),
            Err(error) => Err(error),
        },
    }
}

fn validate_child_name(name: &OsStr) -> io::Result<()> {
    if name.to_str().is_none() {
        return Err(os_error(libc::EILSEQ));
    }
    if name.is_empty()
        || name == OsStr::new(".")
        || name == OsStr::new("..")
        || name.as_bytes().contains(&b'/')
        || name.as_bytes().contains(&0)
    {
        return Err(os_error(libc::EINVAL));
    }
    Ok(())
}

fn is_hidden_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.starts_with(crate::constants::INTERNAL_FILE_PREFIX.as_bytes())
        || bytes.ends_with(b".hwcloud_placeholder")
        || bytes.ends_with(b".download-meta.tmp")
        || bytes.ends_with(b".download-meta-write.tmp")
}

fn ensure_visible_relative_path(relative_path: &Path) -> io::Result<()> {
    ensure_safe_relative_path(relative_path)?;
    if relative_path
        .components()
        .any(|component| matches!(component, Component::Normal(name) if is_hidden_name(name)))
    {
        // 隐藏项对 lookup/getattr 表现为不存在，也避免绕过 readdir 直接访问。
        return Err(os_error(libc::ENOENT));
    }
    Ok(())
}

fn ensure_supported_type(metadata: &Metadata) -> io::Result<()> {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(os_error(libc::ELOOP));
    }
    if !file_type.is_file() && !file_type.is_dir() {
        return Err(os_error(libc::EACCES));
    }
    Ok(())
}

fn read_placeholder_metadata(path: &Path) -> io::Result<Option<PlaceholderMetadata>> {
    let state = crate::platform::xattr::get(path, XATTR_STATE)?;
    if state.as_deref() != Some(STATE_PLACEHOLDER) {
        return Ok(None);
    }

    let file_id = required_xattr_text(path, XATTR_FILE_ID)?;
    let logical_size = required_xattr_text(path, XATTR_SIZE)?
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "占位文件 size xattr 无效"))?;
    Ok(Some(PlaceholderMetadata {
        file_id,
        logical_size,
    }))
}

fn required_xattr_text(path: &Path, key: &str) -> io::Result<String> {
    let value = crate::platform::xattr::get(path, key)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("占位文件缺少 {key} xattr"),
        )
    })?;
    let value = String::from_utf8(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "占位 xattr 不是 UTF-8"))?;
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("占位文件 {key} xattr 为空"),
        ));
    }
    Ok(value)
}

fn file_attr(
    inode: INodeNo,
    metadata: &Metadata,
    placeholder: Option<&PlaceholderMetadata>,
) -> FileAttr {
    let kind = if metadata.is_dir() {
        FileType::Directory
    } else {
        FileType::RegularFile
    };
    FileAttr {
        ino: inode,
        size: placeholder
            .map(|placeholder| placeholder.logical_size)
            .unwrap_or_else(|| metadata.len()),
        blocks: if placeholder.is_some() {
            0
        } else {
            metadata.blocks()
        },
        atime: unix_time(metadata.atime(), metadata.atime_nsec()),
        mtime: unix_time(metadata.mtime(), metadata.mtime_nsec()),
        ctime: unix_time(metadata.ctime(), metadata.ctime_nsec()),
        crtime: UNIX_EPOCH,
        kind,
        perm: (metadata.mode() & 0o7777) as u16,
        nlink: metadata.nlink().min(u32::MAX as u64) as u32,
        uid: metadata.uid(),
        gid: metadata.gid(),
        rdev: metadata.rdev().min(u32::MAX as u64) as u32,
        blksize: metadata.blksize().clamp(512, u32::MAX as u64) as u32,
        flags: 0,
    }
}

fn unix_time(seconds: i64, nanoseconds: i64) -> SystemTime {
    let total_nanoseconds = i128::from(seconds) * 1_000_000_000_i128 + i128::from(nanoseconds);
    if total_nanoseconds >= 0 {
        UNIX_EPOCH
            .checked_add(Duration::from_nanos(
                total_nanoseconds.min(u64::MAX as i128) as u64,
            ))
            .unwrap_or(UNIX_EPOCH)
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_nanos(
                (-total_nanoseconds).min(u64::MAX as i128) as u64,
            ))
            .unwrap_or(UNIX_EPOCH)
    }
}

fn open_read_only_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

fn open_read_write_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

fn open_metadata_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

fn open_existing_no_follow(path: &Path, flags: OpenFlags) -> io::Result<File> {
    let readable = flags.acc_mode() != OpenAccMode::O_WRONLY;
    let writable = flags.acc_mode() != OpenAccMode::O_RDONLY;
    if flags.0 & libc::O_TRUNC != 0 && !writable {
        return Err(os_error(libc::EINVAL));
    }
    let retained_flags = flags.0 & (libc::O_SYNC | libc::O_DSYNC);
    OpenOptions::new()
        .read(readable)
        .write(writable)
        .truncate(flags.0 & libc::O_TRUNC != 0)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | retained_flags)
        .open(path)
}

fn read_at(file: &File, offset: u64, size: u32) -> io::Result<Vec<u8>> {
    let mut buffer = vec![0_u8; size as usize];
    let mut filled = 0;
    while filled < buffer.len() {
        let current_offset = offset
            .checked_add(filled as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "读取偏移溢出"))?;
        match file.read_at(&mut buffer[filled..], current_offset) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

fn write_at(file: &File, offset: u64, data: &[u8]) -> io::Result<u32> {
    let mut written = 0_usize;
    while written < data.len() {
        let current_offset = offset
            .checked_add(written as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "写入偏移溢出"))?;
        match file.write_at(&data[written..], current_offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "FUSE backing 短写",
                ));
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    u32::try_from(written)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "单次写入超过 FUSE 上限"))
}

fn convert_placeholder_to_truncated(path: &Path, file: &File) -> io::Result<()> {
    // downloaded 是提交标记，必须最后写。若 truncate 失败，文件仍明确是占位符；
    // 若状态写入失败，open 直接失败且后续首次读仍会安全地重新 hydration。
    file.set_len(0)?;
    crate::platform::xattr::set(path, XATTR_STATE, STATE_DOWNLOADED)?;
    Ok(())
}

fn rename_beneath(source: &Path, destination: &Path, no_replace: bool) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "源路径包含 NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "目标路径包含 NUL"))?;
    let flags = if no_replace {
        libc::RENAME_NOREPLACE
    } else {
        0
    };
    // SAFETY: 两个 CString 都有效且 NUL 结尾；路径组件已在调用方逐层 lstat，
    // renameat2 只执行同一 backing 中的原子命名空间操作。
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            flags,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn resolve_time(time: TimeOrNow) -> SystemTime {
    match time {
        TimeOrNow::SpecificTime(time) => time,
        TimeOrNow::Now => SystemTime::now(),
    }
}

fn change_file_owner(file: &File, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let uid = uid.unwrap_or(u32::MAX);
    let gid = gid.unwrap_or(u32::MAX);
    // SAFETY: fd 来自仍由 Arc<File> 持有的有效打开文件；-1（u32::MAX）表示该字段不变。
    if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSystemStats {
    blocks: u64,
    blocks_free: u64,
    blocks_available: u64,
    files: u64,
    files_free: u64,
    block_size: u32,
    name_length: u32,
    fragment_size: u32,
}

fn statvfs(path: &Path) -> io::Result<FileSystemStats> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径包含 NUL"))?;
    let mut raw = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` 是有效且以 NUL 结尾的 C 字符串；`raw` 指向可写的 statvfs 空间，
    // libc 返回成功后才读取初始化结果。
    if unsafe { libc::statvfs(path.as_ptr(), raw.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 上面的 statvfs 已返回成功并初始化整个结构体。
    let raw = unsafe { raw.assume_init() };
    Ok(FileSystemStats {
        blocks: raw.f_blocks,
        blocks_free: raw.f_bfree,
        blocks_available: raw.f_bavail,
        files: raw.f_files,
        files_free: raw.f_ffree,
        block_size: raw.f_bsize.clamp(1, u32::MAX as libc::c_ulong) as u32,
        name_length: raw.f_namemax.clamp(1, u32::MAX as libc::c_ulong) as u32,
        fragment_size: raw.f_frsize.clamp(1, u32::MAX as libc::c_ulong) as u32,
    })
}

fn os_error(errno: i32) -> io::Error {
    io::Error::from_raw_os_error(errno)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // These tests mount real FUSE filesystems. Running them concurrently can make
    // an earlier release race with the next mutation in the intentionally strict
    // test coordinator, so serialize only the host-level smoke tests.
    #[cfg(target_os = "linux")]
    static REAL_FUSE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[derive(Debug)]
    struct CopyHydrator {
        data: Vec<u8>,
        calls: AtomicUsize,
    }

    impl CopyHydrator {
        fn new(data: impl Into<Vec<u8>>) -> Self {
            Self {
                data: data.into(),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Hydrator for CopyHydrator {
        async fn hydrate(&self, request: HydrationRequest) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let temporary = request.backing_path.with_extension("hydrating-test");
            std::fs::write(&temporary, &self.data)?;
            crate::platform::xattr::set(&temporary, XATTR_FILE_ID, request.file_id.as_bytes())?;
            crate::platform::xattr::set(&temporary, XATTR_STATE, b"downloaded")?;
            std::fs::rename(temporary, request.backing_path)?;
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct RecordingCoordinator {
        active: Arc<AtomicUsize>,
        exclusive: Arc<AtomicBool>,
        acquisitions: Mutex<Vec<Vec<PathBuf>>>,
    }

    #[derive(Debug)]
    struct RecordingLease {
        active: Arc<AtomicUsize>,
        exclusive: Arc<AtomicBool>,
        is_exclusive: bool,
    }

    impl Drop for RecordingLease {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.is_exclusive {
                self.exclusive.store(false, Ordering::SeqCst);
            }
        }
    }

    #[async_trait]
    impl MutationCoordinator for RecordingCoordinator {
        async fn acquire_shared(&self, paths: Vec<PathBuf>) -> io::Result<Box<dyn PathLease>> {
            if self.exclusive.load(Ordering::SeqCst) {
                return Err(os_error(libc::EBUSY));
            }
            self.active.fetch_add(1, Ordering::SeqCst);
            if self.exclusive.load(Ordering::SeqCst) {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Err(os_error(libc::EBUSY));
            }
            self.record(paths, false)
        }

        async fn acquire_exclusive(&self, paths: Vec<PathBuf>) -> io::Result<Box<dyn PathLease>> {
            if self.active.load(Ordering::SeqCst) != 0
                || self
                    .exclusive
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            {
                return Err(os_error(libc::EBUSY));
            }
            self.active.fetch_add(1, Ordering::SeqCst);
            self.record(paths, true)
        }

        async fn prepare_rename(
            &self,
            _source: PathBuf,
            _destination: PathBuf,
            _is_directory: bool,
        ) -> io::Result<()> {
            Ok(())
        }

        async fn rename_observed(
            &self,
            _source: PathBuf,
            _destination: PathBuf,
            _is_directory: bool,
        ) -> io::Result<()> {
            Ok(())
        }
    }

    impl RecordingCoordinator {
        fn record(
            &self,
            paths: Vec<PathBuf>,
            is_exclusive: bool,
        ) -> io::Result<Box<dyn PathLease>> {
            self.acquisitions.lock().push(paths);
            Ok(Box::new(RecordingLease {
                active: Arc::clone(&self.active),
                exclusive: Arc::clone(&self.exclusive),
                is_exclusive,
            }))
        }
    }

    fn write_placeholder(path: &Path, file_id: &str, logical_size: u64) {
        File::create(path).expect("创建测试占位文件失败");
        crate::platform::xattr::set(path, XATTR_FILE_ID, file_id.as_bytes())
            .expect("写 fileId xattr 失败");
        crate::platform::xattr::set(path, XATTR_STATE, STATE_PLACEHOLDER)
            .expect("写 state xattr 失败");
        crate::platform::xattr::set(path, XATTR_SIZE, logical_size.to_string().as_bytes())
            .expect("写 size xattr 失败");
    }

    fn test_filesystem(
        backing_root: &Path,
        hydrator: Arc<dyn Hydrator>,
    ) -> (
        tokio::runtime::Runtime,
        VirtualDriveFs,
        Arc<RecordingCoordinator>,
    ) {
        let runtime = tokio::runtime::Runtime::new().expect("创建 Tokio runtime 失败");
        let coordinator = Arc::new(RecordingCoordinator::default());
        let filesystem = VirtualDriveFs::new(
            backing_root.to_path_buf(),
            runtime.handle().clone(),
            hydrator,
            Some(coordinator.clone()),
            false,
            Arc::new(MountCancellation::default()),
        );
        (runtime, filesystem, coordinator)
    }

    fn test_read_only_filesystem(
        backing_root: &Path,
        hydrator: Arc<dyn Hydrator>,
    ) -> (tokio::runtime::Runtime, VirtualDriveFs) {
        let runtime = tokio::runtime::Runtime::new().expect("创建 Tokio runtime 失败");
        let filesystem = VirtualDriveFs::new(
            backing_root.to_path_buf(),
            runtime.handle().clone(),
            hydrator,
            None,
            true,
            Arc::new(MountCancellation::default()),
        );
        (runtime, filesystem)
    }

    #[test]
    fn layout_requires_distinct_non_nested_empty_directories() {
        let backing = tempfile::tempdir().expect("创建 backing 失败");
        let mountpoint = tempfile::tempdir().expect("创建 mountpoint 失败");
        let (actual_backing, actual_mountpoint) =
            validate_mount_layout(backing.path(), mountpoint.path()).expect("合法布局被拒绝");
        assert_eq!(
            actual_backing,
            std::fs::canonicalize(backing.path()).unwrap()
        );
        assert_eq!(
            actual_mountpoint,
            std::fs::canonicalize(mountpoint.path()).unwrap()
        );

        let same = validate_mount_layout(backing.path(), backing.path()).unwrap_err();
        assert_eq!(same.kind(), io::ErrorKind::InvalidInput);

        let nested_mount = backing.path().join("mount");
        std::fs::create_dir(&nested_mount).unwrap();
        let nested = validate_mount_layout(backing.path(), &nested_mount).unwrap_err();
        assert_eq!(nested.kind(), io::ErrorKind::InvalidInput);

        std::fs::write(mountpoint.path().join("occupied"), b"x").unwrap();
        let occupied = validate_mount_layout(backing.path(), mountpoint.path()).unwrap_err();
        assert_eq!(occupied.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn placeholder_attr_uses_logical_size_without_hydrating() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("report.pdf");
        write_placeholder(&path, "cloud-id", 4_294_967_296);
        let hydrator = Arc::new(CopyHydrator::new(b"not used".to_vec()));
        let (_runtime, filesystem, _coordinator) =
            test_filesystem(backing.path(), hydrator.clone());

        let attr = filesystem
            .entry_for_relative(Path::new("report.pdf"))
            .unwrap();
        assert_eq!(attr.size, 4_294_967_296);
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn first_read_hydrates_then_reads_and_second_read_reuses_content() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("paper.txt");
        write_placeholder(&path, "cloud-paper", 11);
        let hydrator = Arc::new(CopyHydrator::new(b"hello world".to_vec()));
        let (runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator.clone());
        let attr = filesystem
            .entry_for_relative(Path::new("paper.txt"))
            .unwrap();
        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDONLY)))
            .unwrap();
        assert_eq!(
            _coordinator.active.load(Ordering::SeqCst),
            1,
            "未水合只读句柄必须暂持 shared lease"
        );

        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 6, 5))
                .unwrap(),
            b"world"
        );
        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 0, 5))
                .unwrap(),
            b"hello"
        );
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            _coordinator.active.load(Ordering::SeqCst),
            0,
            "真实 fd 安装后应立即释放 shared lease"
        );
        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
    }

    #[test]
    fn background_reader_classification_is_narrow_and_fail_open() {
        let dolphin = ProcessIdentity {
            comm: Some("dolphin".to_string()),
            executable: Some("dolphin".to_string()),
            command_line: vec!["/usr/bin/dolphin".to_string(), "/mnt/cloud".to_string()],
        };
        assert_eq!(dolphin.background_reader(), Some("dolphin"));

        let tracker = ProcessIdentity {
            executable: Some("tracker-extract-3".to_string()),
            ..Default::default()
        };
        assert_eq!(tracker.background_reader(), Some("tracker-extract-3"));

        let papers_thumbnailer = ProcessIdentity {
            comm: Some("papers-thumbnai".to_string()),
            executable: Some("papers-thumbnailer".to_string()),
            command_line: vec![
                "papers-thumbnailer".to_string(),
                "-s".to_string(),
                "256".to_string(),
            ],
        };
        assert_eq!(
            papers_thumbnailer.background_reader(),
            Some("papers-thumbnailer")
        );

        let gnome_thumbnail_sandbox = ProcessIdentity {
            comm: Some("bwrap".to_string()),
            executable: Some("bwrap".to_string()),
            command_line: vec![
                "bwrap".to_string(),
                "--ro-bind".to_string(),
                "/mnt/cloud/book.pdf".to_string(),
                "/tmp/gnome-desktop-file-to-thumbnail.pdf".to_string(),
                "papers-thumbnailer".to_string(),
                "-s".to_string(),
                "256".to_string(),
            ],
        };
        assert_eq!(gnome_thumbnail_sandbox.background_reader(), Some("bwrap"));

        let ordinary_sandbox = ProcessIdentity {
            comm: Some("bwrap".to_string()),
            executable: Some("bwrap".to_string()),
            command_line: vec![
                "bwrap".to_string(),
                "--ro-bind".to_string(),
                "/mnt/cloud/book.pdf".to_string(),
                "/document.pdf".to_string(),
                "papers".to_string(),
                "/document.pdf".to_string(),
            ],
        };
        assert_eq!(ordinary_sandbox.background_reader(), None);

        let thumbnail_worker = ProcessIdentity {
            executable: Some("kioworker".to_string()),
            command_line: vec![
                "/usr/lib/kf6/kioworker".to_string(),
                "/usr/lib/qt6/plugins/kf6/kio/thumbnail.so".to_string(),
                "thumbnail".to_string(),
                "local:/run/user/1000/dolphin.kioworker.socket".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(thumbnail_worker.background_reader(), Some("kioworker"));

        let copy_worker = ProcessIdentity {
            executable: Some("kioworker".to_string()),
            command_line: vec![
                "/usr/lib/qt6/plugins/kf6/kio/kioworker".to_string(),
                "file".to_string(),
                "local:/run/user/1000/dolphin".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(copy_worker.background_reader(), None);

        for interactive in ["wpspdf", "okular", "msedge", "cat"] {
            let identity = ProcessIdentity {
                executable: Some(interactive.to_string()),
                ..Default::default()
            };
            assert_eq!(identity.background_reader(), None, "{interactive}");
        }
        assert_eq!(ProcessIdentity::default().background_reader(), None);
    }

    #[test]
    fn background_preview_open_is_rejected_before_handle_allocation() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("previewed.pdf");
        write_placeholder(&path, "cloud-preview", 11);
        let hydrator = Arc::new(CopyHydrator::new(b"hello world".to_vec()));
        let (runtime, filesystem, coordinator) = test_filesystem(backing.path(), hydrator.clone());
        let attr = filesystem
            .entry_for_relative(Path::new("previewed.pdf"))
            .unwrap();
        let thumbnail_worker = ProcessIdentity {
            comm: Some("kioworker".to_string()),
            executable: Some("kioworker".to_string()),
            command_line: vec![
                "/usr/lib/kf6/kioworker".to_string(),
                "/usr/lib/qt6/plugins/kf6/kio/thumbnail.so".to_string(),
                "thumbnail".to_string(),
            ],
        };

        let error = runtime
            .block_on(filesystem.open_inode_for_caller(
                attr.ino,
                OpenFlags(libc::O_RDONLY),
                Some((4242, &thumbnail_worker)),
            ))
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EOPNOTSUPP));
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 0);
        assert!(filesystem.open_handles.lock().is_empty());
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 0);
        assert!(read_placeholder_metadata(&path).unwrap().is_some());
    }

    #[test]
    fn background_preview_read_does_not_hydrate_but_interactive_read_still_does() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("previewed.pdf");
        write_placeholder(&path, "cloud-preview", 11);
        let hydrator = Arc::new(CopyHydrator::new(b"hello world".to_vec()));
        let (runtime, filesystem, coordinator) = test_filesystem(backing.path(), hydrator.clone());
        let attr = filesystem
            .entry_for_relative(Path::new("previewed.pdf"))
            .unwrap();
        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDONLY)))
            .unwrap();
        let dolphin = ProcessIdentity {
            comm: Some("dolphin".to_string()),
            executable: Some("dolphin".to_string()),
            command_line: vec!["/usr/bin/dolphin".to_string()],
        };

        let error = runtime
            .block_on(filesystem.read_inode_for_caller(
                attr.ino,
                handle,
                0,
                5,
                Some((4242, &dolphin)),
            ))
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EOPNOTSUPP));
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 0);
        assert!(read_placeholder_metadata(&path).unwrap().is_some());
        assert_eq!(
            coordinator.active.load(Ordering::SeqCst),
            1,
            "拒绝后台预览后句柄仍需持有 shared lease 等待真正读取"
        );

        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 6, 5))
                .unwrap(),
            b"world"
        );
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 0);
        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
    }

    #[test]
    fn writable_open_hydrates_old_content_and_holds_exclusive_lease_until_release() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("editable.txt");
        write_placeholder(&path, "cloud-editable", 11);
        let hydrator = Arc::new(CopyHydrator::new(b"hello world".to_vec()));
        let (runtime, filesystem, coordinator) = test_filesystem(backing.path(), hydrator.clone());
        let attr = filesystem
            .entry_for_relative(Path::new("editable.txt"))
            .unwrap();

        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDWR)))
            .unwrap();
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime
                .block_on(filesystem.write_inode(attr.ino, handle, 6, b"FUSE!"))
                .unwrap(),
            5
        );
        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 0, 11))
                .unwrap(),
            b"hello FUSE!"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"hello FUSE!");

        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn truncate_open_skips_hydration_and_converts_placeholder_state() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("replace.txt");
        write_placeholder(&path, "cloud-replace", 1024);
        let hydrator = Arc::new(CopyHydrator::new(b"must not download".to_vec()));
        let (runtime, filesystem, coordinator) = test_filesystem(backing.path(), hydrator.clone());
        let attr = filesystem
            .entry_for_relative(Path::new("replace.txt"))
            .unwrap();

        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_WRONLY | libc::O_TRUNC)))
            .unwrap();
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert_eq!(
            crate::platform::xattr::get(&path, XATTR_STATE).unwrap(),
            Some(STATE_DOWNLOADED.to_vec())
        );
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 1);
        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
        assert_eq!(coordinator.active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn opened_file_reads_fixed_inode_after_backing_path_is_replaced() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("versioned.txt");
        std::fs::write(&path, b"old version").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (runtime, filesystem, coordinator) = test_filesystem(backing.path(), hydrator);
        let attr = filesystem
            .entry_for_relative(Path::new("versioned.txt"))
            .unwrap();
        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDONLY)))
            .unwrap();
        assert_eq!(
            coordinator.active.load(Ordering::SeqCst),
            0,
            "普通只读文件在固定 fd 后不长期持 lease"
        );

        let replacement = backing.path().join("new-version");
        std::fs::write(&replacement, b"new version").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 0, 64))
                .unwrap(),
            b"old version"
        );
        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
    }

    #[test]
    fn writable_open_blocks_unlink_and_rename_until_release() {
        let backing = tempfile::tempdir().unwrap();
        std::fs::write(backing.path().join("busy.txt"), b"content").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);
        let attr = filesystem
            .entry_for_relative(Path::new("busy.txt"))
            .unwrap();
        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDWR)))
            .unwrap();

        let unlink_error = runtime
            .block_on(filesystem.unlink_file(INodeNo::ROOT, OsStr::new("busy.txt")))
            .unwrap_err();
        assert_eq!(unlink_error.raw_os_error(), Some(libc::EBUSY));
        let rename_error = runtime
            .block_on(filesystem.rename_entry(
                INodeNo::ROOT,
                OsStr::new("busy.txt"),
                INodeNo::ROOT,
                OsStr::new("moved.txt"),
                RenameFlags::empty(),
            ))
            .unwrap_err();
        assert_eq!(rename_error.raw_os_error(), Some(libc::EBUSY));

        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();
        runtime
            .block_on(filesystem.rename_entry(
                INodeNo::ROOT,
                OsStr::new("busy.txt"),
                INodeNo::ROOT,
                OsStr::new("moved.txt"),
                RenameFlags::empty(),
            ))
            .unwrap();
    }

    #[test]
    fn directory_rename_never_replaces_an_existing_directory_identity() {
        let backing = tempfile::tempdir().unwrap();
        std::fs::create_dir(backing.path().join("source")).unwrap();
        std::fs::create_dir(backing.path().join("destination")).unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);

        let error = runtime
            .block_on(filesystem.rename_entry(
                INodeNo::ROOT,
                OsStr::new("source"),
                INodeNo::ROOT,
                OsStr::new("destination"),
                RenameFlags::empty(),
            ))
            .unwrap_err();

        assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
        assert!(backing.path().join("source").is_dir());
        assert!(backing.path().join("destination").is_dir());
    }

    #[test]
    fn cloud_file_rename_does_not_replace_a_different_existing_identity() {
        let backing = tempfile::tempdir().unwrap();
        let source = backing.path().join("source.txt");
        let destination = backing.path().join("destination.txt");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&destination, b"destination").unwrap();
        crate::platform::xattr::set(&source, XATTR_FILE_ID, b"cloud-a").unwrap();
        crate::platform::xattr::set(&destination, XATTR_FILE_ID, b"cloud-b").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);

        let error = runtime
            .block_on(filesystem.rename_entry(
                INodeNo::ROOT,
                OsStr::new("source.txt"),
                INodeNo::ROOT,
                OsStr::new("destination.txt"),
                RenameFlags::empty(),
            ))
            .unwrap_err();

        assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(std::fs::read(source).unwrap(), b"source");
        assert_eq!(std::fs::read(destination).unwrap(), b"destination");
    }

    #[test]
    fn read_only_compatibility_mount_rejects_write_open() {
        let backing = tempfile::tempdir().unwrap();
        std::fs::write(backing.path().join("plain.txt"), b"0123456789").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (runtime, filesystem) = test_read_only_filesystem(backing.path(), hydrator);
        let attr = filesystem
            .entry_for_relative(Path::new("plain.txt"))
            .unwrap();
        let handle = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDONLY)))
            .unwrap();
        assert_eq!(
            runtime
                .block_on(filesystem.read_inode(attr.ino, handle, 3, 4))
                .unwrap(),
            b"3456"
        );
        filesystem
            .release_handle(attr.ino, handle, OpenHandleKind::File)
            .unwrap();

        let error = runtime
            .block_on(filesystem.open_inode(attr.ino, OpenFlags(libc::O_RDWR)))
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EROFS));
    }

    #[test]
    fn inode_survives_backing_atomic_replacement() {
        let backing = tempfile::tempdir().unwrap();
        let path = backing.path().join("stable.txt");
        std::fs::write(&path, b"before").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (_runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);
        let before = filesystem
            .entry_for_relative(Path::new("stable.txt"))
            .unwrap();

        let replacement = backing.path().join("replacement");
        std::fs::write(&replacement, b"after").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let after = filesystem
            .entry_for_relative(Path::new("stable.txt"))
            .unwrap();

        assert_eq!(before.ino, after.ino);
        assert_ne!(before.size, after.size);
    }

    #[test]
    fn symlink_is_not_exposed_or_opened() {
        let backing = tempfile::tempdir().unwrap();
        std::fs::write(backing.path().join("target"), b"secret").unwrap();
        std::os::unix::fs::symlink("target", backing.path().join("link")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("nested-secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), backing.path().join("linked-dir")).unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (_runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);

        let error = filesystem
            .entry_for_relative(Path::new("link"))
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        let names: Vec<_> = filesystem
            .list_directory(INodeNo::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(!names.contains(&OsString::from("link")));
        assert!(!names.contains(&OsString::from("linked-dir")));

        let nested_error = filesystem
            .entry_for_relative(Path::new("linked-dir/nested-secret"))
            .unwrap_err();
        assert_eq!(nested_error.raw_os_error(), Some(libc::ELOOP));
    }

    #[test]
    fn private_entries_are_hidden_but_dotfiles_and_tmp_files_remain_visible() {
        let backing = tempfile::tempdir().unwrap();
        std::fs::write(backing.path().join(".hwcloud_syncstate.json"), b"private").unwrap();
        std::fs::write(backing.path().join("old.hwcloud_placeholder"), b"private").unwrap();
        std::fs::write(backing.path().join(".env"), b"user dotfile").unwrap();
        std::fs::write(backing.path().join("editor-save.tmp"), b"user temp").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (_runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);

        let names: Vec<_> = filesystem
            .list_directory(INodeNo::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(!names.contains(&OsString::from(".hwcloud_syncstate.json")));
        assert!(!names.contains(&OsString::from("old.hwcloud_placeholder")));
        assert!(names.contains(&OsString::from(".env")));
        assert!(names.contains(&OsString::from("editor-save.tmp")));

        let hidden_lookup = filesystem
            .entry_for_relative(Path::new(".hwcloud_syncstate.json"))
            .unwrap_err();
        assert_eq!(hidden_lookup.raw_os_error(), Some(libc::ENOENT));
        assert!(filesystem
            .entry_for_relative(Path::new("editor-save.tmp"))
            .is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_names_are_not_exposed_or_accepted() {
        use std::os::unix::ffi::OsStringExt;

        let backing = tempfile::tempdir().unwrap();
        let invalid = OsString::from_vec(vec![b'b', b'a', b'd', 0xff]);
        std::fs::write(backing.path().join(&invalid), b"unsafe for DB").unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (_runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);
        let names: Vec<_> = filesystem
            .list_directory(INodeNo::ROOT)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(!names.contains(&invalid));
        assert_eq!(
            filesystem
                .visible_child_relative(INodeNo::ROOT, &invalid)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EILSEQ)
        );
    }

    #[test]
    fn statfs_reflects_backing_filesystem() {
        let backing = tempfile::tempdir().unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let (_runtime, filesystem, _coordinator) = test_filesystem(backing.path(), hydrator);
        let stats = filesystem.filesystem_stats().unwrap();
        assert!(stats.block_size > 0);
        assert!(stats.blocks > 0);
    }

    /// 手工/CI 环境提供可用 `/dev/fuse` 与 `fusermount3` 时运行：
    /// `cargo test virtual_fs::linux::tests::real_fuse_read_smoke -- --ignored`
    #[test]
    #[ignore = "需要主机提供可用的 /dev/fuse 与 fusermount3"]
    fn real_fuse_read_smoke() {
        let _serial = REAL_FUSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backing = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();
        std::fs::write(backing.path().join("hello.txt"), b"hello from FUSE").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let hydrator = Arc::new(CopyHydrator::new(Vec::new()));
        let mut session = mount_read_only(ReadOnlyFuseConfig::new(
            backing.path(),
            mountpoint.path(),
            runtime.handle().clone(),
            hydrator,
        ))
        .expect("FUSE 挂载失败");

        assert_eq!(
            std::fs::read(mountpoint.path().join("hello.txt")).unwrap(),
            b"hello from FUSE"
        );
        let write_error = std::fs::write(mountpoint.path().join("new.txt"), b"denied").unwrap_err();
        assert_eq!(write_error.raw_os_error(), Some(libc::EROFS));
        session.unmount().expect("FUSE 卸载失败");
    }

    /// 真实 FUSE 请求必须携带可用于抑制缩略图水合的调用者 PID。
    #[test]
    #[ignore = "需要主机提供可用的 /dev/fuse、fusermount3 与 coreutils head"]
    fn real_fuse_background_preview_does_not_hydrate() {
        let _serial = REAL_FUSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backing = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();
        let helper_dir = tempfile::tempdir().unwrap();
        let path = backing.path().join("preview.pdf");
        write_placeholder(&path, "preview-id", 11);
        let head = [Path::new("/usr/bin/head"), Path::new("/bin/head")]
            .into_iter()
            .find(|candidate| candidate.is_file())
            .expect("找不到 coreutils head");
        let fake_dolphin = helper_dir.path().join("dolphin");
        std::os::unix::fs::symlink(head, &fake_dolphin).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let hydrator = Arc::new(CopyHydrator::new(b"hello world".to_vec()));
        let mut session = mount_read_only(ReadOnlyFuseConfig::new(
            backing.path(),
            mountpoint.path(),
            runtime.handle().clone(),
            hydrator.clone(),
        ))
        .expect("FUSE 挂载失败");

        let output = std::process::Command::new(&fake_dolphin)
            .arg("-c")
            .arg("5")
            .arg(mountpoint.path().join("preview.pdf"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "后台预览读取不应取得占位内容");
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 0);
        assert!(read_placeholder_metadata(&path).unwrap().is_some());

        assert_eq!(
            std::fs::read(mountpoint.path().join("preview.pdf")).unwrap(),
            b"hello world"
        );
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);
        session.unmount().expect("FUSE 卸载失败");
    }

    /// 可写真实 FUSE 冒烟：
    /// `cargo test virtual_fs::linux::tests::real_fuse_write_smoke -- --ignored --exact`
    #[test]
    #[ignore = "需要主机提供可用的 /dev/fuse 与 fusermount3"]
    fn real_fuse_write_smoke() {
        let _serial = REAL_FUSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backing = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();
        std::fs::create_dir(backing.path().join("cloud-folder")).unwrap();
        write_placeholder(
            &backing.path().join("cloud-folder").join("remote.txt"),
            "remote-id",
            11,
        );
        std::fs::write(backing.path().join(".hwcloud_internal"), b"hidden").unwrap();
        std::fs::write(backing.path().join("normal.tmp"), b"visible").unwrap();
        std::fs::write(backing.path().join("cache-cycle.txt"), b"old content").unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let hydrator = Arc::new(CopyHydrator::new(b"hello cloud".to_vec()));
        let coordinator = Arc::new(RecordingCoordinator::default());
        let mut session = mount_virtual_drive(VirtualDriveConfig::new(
            backing.path(),
            mountpoint.path(),
            runtime.handle().clone(),
            hydrator.clone(),
            coordinator,
        ))
        .expect("可写 FUSE 挂载失败");
        assert!(
            crate::core::config_store::is_active_petallink_mount(mountpoint.path())
                .expect("读取挂载状态失败"),
            "运行状态查询必须识别真实 PetalLink FUSE 挂载"
        );

        let visible_names: Vec<_> = std::fs::read_dir(mountpoint.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(!visible_names.contains(&OsString::from(".hwcloud_internal")));
        assert!(visible_names.contains(&OsString::from("normal.tmp")));

        // 同步引擎会直接在 backing 原子替换下载文件或占位符。即使旧内容已经经由
        // FUSE 读进内核页缓存，新 open 也必须重新经过 FUSE 并触发 hydration。
        let cached_path = mountpoint.path().join("cache-cycle.txt");
        assert_eq!(std::fs::read(&cached_path).unwrap(), b"old content");
        let replacement = backing.path().join(".cache-cycle.placeholder");
        write_placeholder(&replacement, "cache-cycle-id", 11);
        std::fs::rename(&replacement, backing.path().join("cache-cycle.txt")).unwrap();
        assert_eq!(std::fs::read(&cached_path).unwrap(), b"hello cloud");
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);

        let local_dir = mountpoint.path().join("documents");
        std::fs::create_dir(&local_dir).unwrap();
        let draft = local_dir.join("draft.txt");
        std::fs::write(&draft, b"hello").unwrap();
        let mut append = OpenOptions::new().append(true).open(&draft).unwrap();
        append.write_all(b" world").unwrap();
        append.sync_all().unwrap();
        drop(append);
        assert_eq!(std::fs::read(&draft).unwrap(), b"hello world");

        let published = local_dir.join("published.txt");
        std::fs::rename(&draft, &published).unwrap();
        assert!(
            backing.path().join("documents/published.txt").is_file(),
            "rename 必须落到 backing 普通文件"
        );
        let published_metadata = std::fs::symlink_metadata(&published);
        assert!(
            published_metadata
                .as_ref()
                .map(|metadata| metadata.is_file())
                .unwrap_or(false),
            "rename 后 FUSE getattr 必须仍报告普通文件：{published_metadata:?}"
        );
        let mut opened = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&published)
            .unwrap();
        opened.seek(SeekFrom::Start(6)).unwrap();
        opened.write_all(b"FUSE!").unwrap();
        opened.seek(SeekFrom::Start(0)).unwrap();
        let mut edited = Vec::new();
        opened.read_to_end(&mut edited).unwrap();
        assert_eq!(edited, b"hello FUSE!");
        drop(opened);
        std::fs::remove_file(&published).unwrap();
        std::fs::remove_dir(&local_dir).unwrap();

        // 含 placeholder 的目录可离线原子移动；首次读取目标才水合。
        let moved_cloud = mountpoint.path().join("moved-cloud");
        std::fs::rename(mountpoint.path().join("cloud-folder"), &moved_cloud).unwrap();
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(moved_cloud.join("remote.txt")).unwrap(),
            b"hello cloud"
        );
        assert_eq!(hydrator.calls.load(Ordering::SeqCst), 2);

        let symlink_error =
            std::os::unix::fs::symlink("normal.tmp", mountpoint.path().join("link")).unwrap_err();
        assert_eq!(symlink_error.raw_os_error(), Some(libc::EPERM));
        std::fs::remove_file(moved_cloud.join("remote.txt")).unwrap();
        std::fs::remove_dir(&moved_cloud).unwrap();
        session.unmount().expect("可写 FUSE 卸载失败");
        assert!(
            !crate::core::config_store::is_active_petallink_mount(mountpoint.path())
                .expect("读取卸载状态失败"),
            "卸载后不能继续报告按需云盘可用"
        );
    }

    /// 不访问华为账号的真实 FUSE 压力/故障回归：
    ///
    /// `cargo test virtual_fs::linux::tests::real_fuse_stress_and_fault_regression -- --ignored --exact`
    #[test]
    #[ignore = "需要主机提供可用的 /dev/fuse 与 fusermount3"]
    fn real_fuse_stress_and_fault_regression() {
        let _serial = REAL_FUSE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backing = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();

        let cloud_tree = backing.path().join("cloud-tree/level-one/level-two");
        std::fs::create_dir_all(&cloud_tree).unwrap();
        std::fs::write(
            backing.path().join("cloud-tree/local-sibling.txt"),
            b"already local",
        )
        .unwrap();
        write_placeholder(
            &cloud_tree.join("remote-placeholder.txt"),
            "nested-placeholder-id",
            14,
        );

        let cache_path = backing.path().join("cache-version.txt");
        std::fs::write(&cache_path, b"version one").unwrap();

        let collision_source = backing.path().join("cloud-source.txt");
        let collision_destination = backing.path().join("cloud-destination.txt");
        std::fs::write(&collision_source, b"source must survive").unwrap();
        std::fs::write(&collision_destination, b"destination must survive").unwrap();
        crate::platform::xattr::set(&collision_source, XATTR_FILE_ID, b"cloud-source-id").unwrap();
        crate::platform::xattr::set(
            &collision_destination,
            XATTR_FILE_ID,
            b"cloud-destination-id",
        )
        .unwrap();

        std::fs::create_dir(backing.path().join("cloud-directory-a")).unwrap();
        std::fs::create_dir(backing.path().join("cloud-directory-b")).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let hydration_payload = b"hydrated data".to_vec();
        let hydrator = Arc::new(CopyHydrator::new(hydration_payload.clone()));
        let coordinator = Arc::new(RecordingCoordinator::default());
        let mut session = mount_virtual_drive(VirtualDriveConfig::new(
            backing.path(),
            mountpoint.path(),
            runtime.handle().clone(),
            hydrator.clone(),
            coordinator,
        ))
        .expect("压力测试 FUSE 挂载失败");

        // 模拟编辑器常见的“写临时文件 -> fsync -> 原子替换 -> 重新打开 -> 删除”。
        // 固定复用同一组名称，比每轮换名字更容易暴露 dentry/page-cache 陈旧问题。
        let stress_dir = mountpoint.path().join("stress");
        std::fs::create_dir(&stress_dir).unwrap();
        let live_path = stress_dir.join("document.bin");
        let temporary_path = stress_dir.join(".document.bin.save");
        for iteration in 0_u32..128 {
            let old_payload = format!("old-{iteration:03}").into_bytes();
            std::fs::write(&live_path, &old_payload).unwrap();
            assert_eq!(std::fs::read(&live_path).unwrap(), old_payload);

            let replacement_payload = format!("replacement-{iteration:03}").into_bytes();
            let mut temporary = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
                .unwrap();
            temporary.write_all(&replacement_payload).unwrap();
            temporary.sync_all().unwrap();
            drop(temporary);
            std::fs::rename(&temporary_path, &live_path).unwrap();

            assert_eq!(
                std::fs::read(&live_path).unwrap(),
                replacement_payload,
                "第 {iteration} 轮原子替换后读到了旧内容"
            );
            assert_eq!(
                std::fs::read(backing.path().join("stress/document.bin")).unwrap(),
                replacement_payload,
                "第 {iteration} 轮写入没有落到 backing"
            );
            std::fs::remove_file(&live_path).unwrap();
            assert!(
                !backing.path().join("stress/document.bin").exists(),
                "第 {iteration} 轮删除没有落到 backing"
            );
        }
        std::fs::remove_dir(&stress_dir).unwrap();

        // 整树 rename 不应提前下载嵌套 placeholder；目标路径首次读取才 hydration。
        let hydration_calls_before_tree_move = hydrator.calls.load(Ordering::SeqCst);
        let moved_tree = mountpoint.path().join("renamed-cloud-tree");
        std::fs::rename(mountpoint.path().join("cloud-tree"), &moved_tree).unwrap();
        assert!(!mountpoint.path().join("cloud-tree").exists());
        assert_eq!(
            hydrator.calls.load(Ordering::SeqCst),
            hydration_calls_before_tree_move,
            "目录整树 rename 不应下载 placeholder"
        );
        assert_eq!(
            std::fs::read(moved_tree.join("local-sibling.txt")).unwrap(),
            b"already local"
        );
        assert_eq!(
            std::fs::read(moved_tree.join("level-one/level-two/remote-placeholder.txt")).unwrap(),
            hydration_payload
        );
        assert_eq!(
            hydrator.calls.load(Ordering::SeqCst),
            hydration_calls_before_tree_move + 1,
            "改名后首次读必须且只能 hydration 一次"
        );

        // 已打开句柄保持旧 inode 是 POSIX 语义；后台原子替换后，新 open 必须看到新版本。
        let mounted_cache_path = mountpoint.path().join("cache-version.txt");
        let mut old_handle = File::open(&mounted_cache_path).unwrap();
        let mut old_bytes = Vec::new();
        old_handle.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, b"version one");

        let new_version = backing.path().join(".cache-version.new");
        std::fs::write(&new_version, b"version two").unwrap();
        std::fs::rename(&new_version, &cache_path).unwrap();
        assert_eq!(
            std::fs::read(&mounted_cache_path).unwrap(),
            b"version two",
            "后台原子替换后，新 open 不能命中旧页缓存"
        );
        old_handle.seek(SeekFrom::Start(0)).unwrap();
        old_bytes.clear();
        old_handle.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(
            old_bytes, b"version one",
            "替换前已打开的句柄应继续固定旧 inode"
        );
        drop(old_handle);

        // “释放空间”会把完整 backing 原子替换回 placeholder。此前内容即使读入缓存，
        // 新 open 也必须重新经过 FUSE 并触发 hydration。
        let released_placeholder = backing.path().join(".cache-version.placeholder");
        write_placeholder(
            &released_placeholder,
            "released-cache-version-id",
            hydration_payload.len() as u64,
        );
        std::fs::rename(&released_placeholder, &cache_path).unwrap();
        let calls_before_release_read = hydrator.calls.load(Ordering::SeqCst);
        assert_eq!(
            std::fs::read(&mounted_cache_path).unwrap(),
            hydration_payload,
            "释放空间后，新 open 不能返回释放前的缓存内容"
        );
        assert_eq!(
            hydrator.calls.load(Ordering::SeqCst),
            calls_before_release_read + 1
        );

        // 不同云端 fileId 的目标不能被 POSIX rename 静默覆盖。
        let collision_error = std::fs::rename(
            mountpoint.path().join("cloud-source.txt"),
            mountpoint.path().join("cloud-destination.txt"),
        )
        .unwrap_err();
        assert_eq!(collision_error.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(
            std::fs::read(&collision_source).unwrap(),
            b"source must survive"
        );
        assert_eq!(
            std::fs::read(&collision_destination).unwrap(),
            b"destination must survive"
        );
        assert_eq!(
            crate::platform::xattr::get(&collision_source, XATTR_FILE_ID).unwrap(),
            Some(b"cloud-source-id".to_vec())
        );
        assert_eq!(
            crate::platform::xattr::get(&collision_destination, XATTR_FILE_ID).unwrap(),
            Some(b"cloud-destination-id".to_vec())
        );

        // 目录替换需要远端事务支持；MVP 必须拒绝，不能覆盖另一棵目录树。
        let directory_collision = std::fs::rename(
            mountpoint.path().join("cloud-directory-a"),
            mountpoint.path().join("cloud-directory-b"),
        )
        .unwrap_err();
        assert_eq!(directory_collision.raw_os_error(), Some(libc::EEXIST));
        assert!(backing.path().join("cloud-directory-a").is_dir());
        assert!(backing.path().join("cloud-directory-b").is_dir());

        session.unmount().expect("压力测试 FUSE 卸载失败");
    }
}
