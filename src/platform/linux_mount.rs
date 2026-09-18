//! Linux 挂载能力探测与挂载身份管理。
//!
//! 从 `core::config_store` 迁出：FUSE 挂载点能力探测、/proc/*/mountinfo 解析、
//! PetalLink 自有挂载的身份核验与安全卸载。非 Linux 平台由同名 stub 保持行为不变。

#[cfg(target_os = "linux")]
use std::fs::{self, OpenOptions};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use crate::core::config::AppConfig;
#[cfg(target_os = "linux")]
use crate::error::AppError;
use crate::error::AppResult;

/// Linux 上确认同步目录可创建、可写且支持 PetalLink 所需扩展属性。
///
/// 该探测在选择新目录和安装同步引擎前执行，不挂在普通配置保存上，避免用户只修改
/// 托盘等设置时产生文件系统副作用。macOS 保持既有目录行为，由原生同步路径处理。
#[cfg(target_os = "linux")]
pub(crate) fn validate_configured_mount_dir_access(config: &AppConfig) -> AppResult<()> {
    if !config.mount_configured {
        return Ok(());
    }
    let dir = config.expanded_mount_dir();
    if dir.exists() && !dir.is_dir() {
        return Err(AppError::config(format!(
            "同步目录不是文件夹：{}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::config(format!("同步目录创建失败：{}：{e}", dir.display())))?;
    let probe = dir.join(format!(
        "{}xattr-probe-{}-{:016x}",
        crate::constants::INTERNAL_FILE_PREFIX,
        std::process::id(),
        rand::random::<u64>()
    ));
    fs::write(&probe, b"ok")
        .map_err(|e| AppError::config(format!("同步目录不可写：{}：{e}", dir.display())))?;

    const PROBE_KEY: &str = "com.hwcloud.capabilityProbe";
    const PROBE_VALUE: &[u8] = b"petallink";
    let capability_result = (|| {
        crate::platform::xattr::set(&probe, PROBE_KEY, PROBE_VALUE).map_err(|e| {
            AppError::config(format!(
                "同步目录不支持扩展属性，无法安全保存占位文件状态：{}：{e}",
                dir.display()
            ))
        })?;
        let actual = crate::platform::xattr::get(&probe, PROBE_KEY).map_err(|e| {
            AppError::config(format!("同步目录扩展属性不可读：{}：{e}", dir.display()))
        })?;
        if actual.as_deref() != Some(PROBE_VALUE) {
            return Err(AppError::config(format!(
                "同步目录扩展属性校验失败：{}",
                dir.display()
            )));
        }
        crate::platform::xattr::remove(&probe, PROBE_KEY).map_err(|e| {
            AppError::config(format!("同步目录扩展属性不可删除：{}：{e}", dir.display()))
        })
    })();

    let cleanup_result = fs::remove_file(&probe).map_err(|e| {
        AppError::config(format!(
            "同步目录写入探测清理失败：{}：{e}",
            probe.display()
        ))
    });
    capability_result.and(cleanup_result)
}

/// 非 Linux 平台暂不改变稳定版本的挂载目录准入语义。
#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_configured_mount_dir_access(_config: &AppConfig) -> AppResult<()> {
    Ok(())
}

/// Linux 上确认 FUSE 挂载点可安全创建、当前用户可写、为空且尚未被挂载。
///
/// 这是显式能力探测，不会从普通 `load`/`save` 路径调用。初版按需云盘仍使用
/// `mount_dir` 作为 writable+xattr backing，由 [`validate_configured_mount_dir_access`]
/// 独立探测。
#[cfg(target_os = "linux")]
pub(crate) fn validate_virtual_mount_dir_access(config: &AppConfig) -> AppResult<()> {
    if !config.virtual_drive_enabled {
        return Ok(());
    }
    config.validate()?;
    validate_fuse_runtime()?;

    let dir = config.expanded_virtual_mount_dir();
    // 必须先查 mountinfo、再碰挂载点 inode。FUSE daemon 异常退出后，
    // exists/is_dir/canonicalize 都可能直接返回 ENOTCONN，若顺序反过来就永远走不到
    // stale mount 恢复逻辑。
    if is_current_mountpoint(&dir)? && !recover_disconnected_petallink_mount(&dir)? {
        return Err(AppError::config(format!(
            "按需云盘挂载点已被其他文件系统占用：{}",
            dir.display()
        )));
    }
    if dir.exists() && !dir.is_dir() {
        return Err(AppError::config(format!(
            "按需云盘挂载点不是文件夹：{}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::config(format!("按需云盘挂载点创建失败：{}：{e}", dir.display())))?;
    let canonical_dir = fs::canonicalize(&dir).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点无法解析为安全路径：{}：{e}",
            dir.display()
        ))
    })?;
    let canonical_backing = fs::canonicalize(config.expanded_mount_dir()).map_err(|e| {
        AppError::config(format!(
            "物理 backing 目录不存在或无法访问：{}：{e}",
            config.expanded_mount_dir().display()
        ))
    })?;

    validate_resolved_virtual_path(&canonical_backing, &canonical_dir)?;
    // 配置路径可能经过符号链接。解析后的真实路径再查一次，仍只允许恢复身份明确、
    // 当前用户持有且已断开的 PetalLink 挂载。
    if canonical_dir != dir
        && is_current_mountpoint(&canonical_dir)?
        && !recover_disconnected_petallink_mount(&canonical_dir)?
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点已被其他文件系统占用：{}",
            canonical_dir.display()
        )));
    }

    let mut entries = fs::read_dir(&canonical_dir).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点不可读取：{}：{e}",
            canonical_dir.display()
        ))
    })?;
    if entries
        .next()
        .transpose()
        .map_err(|e| {
            AppError::config(format!(
                "按需云盘挂载点读取失败：{}：{e}",
                canonical_dir.display()
            ))
        })?
        .is_some()
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点必须为空：{}",
            canonical_dir.display()
        )));
    }

    let probe = canonical_dir.join(format!(
        "{}fuse-write-probe-{}-{:016x}",
        crate::constants::INTERNAL_FILE_PREFIX,
        std::process::id(),
        rand::random::<u64>()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| {
            AppError::config(format!(
                "当前用户不可写按需云盘挂载点：{}：{e}",
                canonical_dir.display()
            ))
        })?;
    fs::remove_file(&probe).map_err(|e| {
        AppError::config(format!(
            "按需云盘挂载点写入探测清理失败：{}：{e}",
            probe.display()
        ))
    })
}

/// 提前给出可操作的 FUSE 依赖错误，避免保存配置并重启后才静默挂载失败。
#[cfg(all(target_os = "linux", not(test)))]
fn validate_fuse_runtime() -> AppResult<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/fuse")
        .map_err(|error| {
            AppError::config(format!(
                "当前用户无法访问 /dev/fuse：{error}。请安装并启用 fuse3，重新登录后再试"
            ))
        })?;

    let candidates: Vec<PathBuf> = std::env::var_os("FUSERMOUNT_PATH")
        .map(PathBuf::from)
        .map(|helper| vec![helper])
        .unwrap_or_else(|| {
            [
                "fusermount3",
                "fusermount",
                "/bin/fusermount3",
                "/bin/fusermount",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect()
        });
    if candidates
        .iter()
        .any(|helper| Command::new(helper).arg("-h").output().is_ok())
    {
        return Ok(());
    }
    Err(AppError::config(
        "未找到 fusermount3；请安装发行版的 fuse3 软件包".to_string(),
    ))
}

/// 单元测试不依赖宿主是否把 `/dev/fuse` 映射进测试容器；真实挂载由 ignored smoke 覆盖。
#[cfg(all(target_os = "linux", test))]
fn validate_fuse_runtime() -> AppResult<()> {
    Ok(())
}

/// 非 Linux 平台不启用 FUSE 能力探测，保持原有配置行为。
#[cfg(not(target_os = "linux"))]
pub(crate) fn validate_virtual_mount_dir_access(_config: &AppConfig) -> AppResult<()> {
    Ok(())
}

/// 显式探测初版按需云盘所需的 backing 与 FUSE 挂载点能力。
pub(crate) fn validate_virtual_drive_capabilities(config: &AppConfig) -> AppResult<()> {
    validate_configured_mount_dir_access(config)?;
    validate_virtual_mount_dir_access(config)
}

/// 使用解析后的真实路径再次防御符号链接绕过配置层的路径隔离。
#[cfg(target_os = "linux")]
fn validate_resolved_virtual_path(
    canonical_backing: &Path,
    canonical_virtual: &Path,
) -> AppResult<()> {
    if canonical_backing == canonical_virtual
        || canonical_backing.starts_with(canonical_virtual)
        || canonical_virtual.starts_with(canonical_backing)
    {
        return Err(AppError::config(format!(
            "按需云盘挂载点必须与物理 backing 目录不同且互不包含：{} ↔ {}",
            canonical_backing.display(),
            canonical_virtual.display()
        )));
    }
    if canonical_virtual == Path::new("/") {
        return Err(AppError::config(
            "不能把系统根目录作为按需云盘挂载目录".to_string(),
        ));
    }
    if let Some(home) = dirs::home_dir() {
        let resolved_home = fs::canonicalize(&home).unwrap_or(home);
        if canonical_virtual == resolved_home {
            return Err(AppError::config(
                "不能把用户 Home 目录作为按需云盘挂载目录".to_string(),
            ));
        }
    }
    if let Some(data_dir) = dirs::data_dir() {
        let resolved_data = fs::canonicalize(&data_dir).unwrap_or(data_dir);
        if canonical_virtual.starts_with(resolved_data) {
            return Err(AppError::config(
                "不能把 Application Support 目录作为按需云盘挂载目录".to_string(),
            ));
        }
    }
    Ok(())
}

/// 判断路径是否正好是当前 mount namespace 中的挂载点。
#[cfg(target_os = "linux")]
fn is_current_mountpoint(path: &Path) -> AppResult<bool> {
    Ok(current_mount_record(path)?.is_some())
}

/// `/proc/self/mountinfo` 中与目标路径匹配的最小安全身份。
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct MountRecord {
    mount_id: u64,
    fs_type: String,
    source: String,
    super_options: String,
}

/// 查找目标路径的挂载记录，并解析 ` - ` 分隔后的文件系统身份。
#[cfg(target_os = "linux")]
fn current_mount_record(path: &Path) -> AppResult<Option<MountRecord>> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|e| AppError::config(format!("无法读取当前挂载信息：{e}")))?;
    Ok(parse_mount_record(&mountinfo, path))
}

/// 核对目标是否仍是当前 mount namespace 中可访问的 PetalLink FUSE 挂载。
#[cfg(target_os = "linux")]
pub(crate) fn is_active_petallink_mount(path: &Path) -> AppResult<bool> {
    let Some(record) = current_mount_record(path)? else {
        return Ok(false);
    };
    if record.fs_type != "fuse.petallink"
        || (record.source != "PetalLink" && record.source != "petallink")
    {
        return Ok(false);
    }
    match fs::read_dir(path) {
        Ok(_) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::ENOTCONN) => Ok(false),
        Err(error) => Err(AppError::config(format!(
            "PetalLink 挂载点当前不可访问：{}：{error}",
            path.display()
        ))),
    }
}

#[cfg(target_os = "linux")]
fn parse_mount_record(mountinfo: &str, path: &Path) -> Option<MountRecord> {
    mountinfo.lines().find_map(|line| {
        let (left, right) = line.split_once(" - ")?;
        let mut left_fields = left.split_whitespace();
        let mount_id = left_fields.next()?.parse().ok()?;
        let mountpoint = left_fields.nth(3)?;
        if decode_mountinfo_path(mountpoint) != path {
            return None;
        }
        let mut fields = right.split_whitespace();
        Some(MountRecord {
            mount_id,
            fs_type: fields.next()?.to_string(),
            source: fields.next()?.to_string(),
            super_options: fields.next().unwrap_or_default().to_string(),
        })
    })
}

#[cfg(target_os = "linux")]
fn is_petallink_mount_record(record: &MountRecord) -> bool {
    record.fs_type == "fuse.petallink"
        && (record.source == "PetalLink" || record.source == "petallink")
}

#[cfg(target_os = "linux")]
fn mount_owner_uid(record: &MountRecord) -> Option<u32> {
    record
        .super_options
        .split(',')
        .find_map(|option| option.strip_prefix("user_id="))
        .and_then(|uid| uid.parse::<u32>().ok())
}

/// 只清理由当前用户持有、身份明确且已经断开的 PetalLink FUSE 挂载。
///
/// 活跃挂载、其他 FUSE 类型或所有权不匹配一律不碰，避免把宽泛的自动清理变成
/// 任意卸载能力。崩溃后的典型访问错误是 `ENOTCONN`。
#[cfg(target_os = "linux")]
fn recover_disconnected_petallink_mount(path: &Path) -> AppResult<bool> {
    cleanup_verified_petallink_mount(path, true)
}

/// 正常关闭时的兜底卸载。
///
/// 调用方只可传入当前进程已经持有的 [`crate::virtual_fs::FuseMountSession`] 挂载点。
/// 即便如此仍重新核对 mount id、`fuse.petallink` 身份和当前 uid；路径若已被别的
/// 文件系统替换则拒绝操作。
#[cfg(target_os = "linux")]
pub(crate) fn detach_owned_petallink_mount(path: &Path) -> AppResult<bool> {
    cleanup_verified_petallink_mount(path, false)
}

#[cfg(target_os = "linux")]
fn cleanup_verified_petallink_mount(path: &Path, require_disconnected: bool) -> AppResult<bool> {
    let Some(record) = current_mount_record(path)? else {
        return Ok(false);
    };
    if !is_petallink_mount_record(&record) {
        return Ok(false);
    }
    if require_disconnected {
        match fs::read_dir(path) {
            Err(error) if error.raw_os_error() == Some(libc::ENOTCONN) => {}
            Ok(_) => return Ok(false),
            Err(error) => {
                return Err(AppError::config(format!(
                    "无法确认 PetalLink 挂载是否已经断开：{}：{error}",
                    path.display()
                )));
            }
        }
    }

    let current_uid = unsafe { libc::geteuid() };
    if mount_owner_uid(&record) != Some(current_uid) {
        return Err(AppError::config(format!(
            "检测到 PetalLink 挂载，但其所有者不是当前用户，拒绝自动卸载：{}",
            path.display()
        )));
    }

    let mut last_error = None;
    let configured_helper = std::env::var_os("FUSERMOUNT_PATH").map(PathBuf::from);
    let candidates: Vec<PathBuf> =
        configured_helper
            .map(|helper| vec![helper])
            .unwrap_or_else(|| {
                [
                    "fusermount3",
                    "fusermount",
                    "/bin/fusermount3",
                    "/bin/fusermount",
                ]
                .into_iter()
                .map(PathBuf::from)
                .collect()
            });
    for helper in candidates {
        // 防止检查后路径被换挂：mount id 和完整 PetalLink 身份必须仍与首次快照一致。
        let Some(current) = current_mount_record(path)? else {
            return Ok(true);
        };
        if current.mount_id != record.mount_id
            || !is_petallink_mount_record(&current)
            || mount_owner_uid(&current) != Some(current_uid)
        {
            return Err(AppError::config(format!(
                "挂载点身份在自动卸载前发生变化，拒绝继续操作：{}",
                path.display()
            )));
        }

        match Command::new(&helper)
            .args(["-u", "-z", "--"])
            .arg(path)
            .output()
        {
            Ok(output) if output.status.success() => {
                for _ in 0..10 {
                    match current_mount_record(path)? {
                        None => {
                            tracing::warn!(
                                mountpoint = %path.display(),
                                helper = %helper.display(),
                                disconnected_only = require_disconnected,
                                "已清理 PetalLink FUSE 挂载"
                            );
                            return Ok(true);
                        }
                        Some(current) if current.mount_id == record.mount_id => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Some(_) => {
                            return Err(AppError::config(format!(
                                "卸载期间挂载点被另一个文件系统替换：{}",
                                path.display()
                            )));
                        }
                    }
                }
                last_error = Some("卸载命令成功返回，但挂载记录仍然存在".to_string());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                last_error = Some(format!(
                    "{} 退出状态 {}：{}",
                    helper.display(),
                    output.status,
                    stderr.trim()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                last_error = Some(format!("无法执行 {}：{error}", helper.display()));
            }
        }
    }
    Err(AppError::config(format!(
        "无法清理 PetalLink 挂载 {}：{}",
        path.display(),
        last_error.unwrap_or_else(|| "未找到 fusermount3，请安装 fuse3".to_string())
    )))
}

/// `/proc/*/mountinfo` 使用反斜线加三位八进制数转义空格、制表符、换行和反斜线。
#[cfg(target_os = "linux")]
fn decode_mountinfo_path(encoded: &str) -> PathBuf {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..=index + 3].iter().all(u8::is_ascii_digit)
            && bytes[index + 1..=index + 3].iter().all(|b| *b < b'8')
        {
            let value = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    PathBuf::from(OsString::from_vec(decoded))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_creates_empty_writable_directories() {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig {
            mount_dir: temp.path().join("backing").to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: temp.path().join("drive").to_string_lossy().into_owned(),
            ..AppConfig::default()
        };

        validate_virtual_drive_capabilities(&config).unwrap();
        assert!(config.expanded_mount_dir().is_dir());
        assert!(config.expanded_virtual_mount_dir().is_dir());
        assert_eq!(
            fs::read_dir(config.expanded_virtual_mount_dir())
                .unwrap()
                .count(),
            0
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_rejects_nonempty_mountpoint() {
        let temp = tempfile::tempdir().unwrap();
        let backing = temp.path().join("backing");
        let virtual_mount = temp.path().join("drive");
        fs::create_dir_all(&virtual_mount).unwrap();
        fs::write(virtual_mount.join("existing.txt"), b"keep me").unwrap();
        let config = AppConfig {
            mount_dir: backing.to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: virtual_mount.to_string_lossy().into_owned(),
            ..AppConfig::default()
        };

        let error = validate_virtual_drive_capabilities(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("必须为空"), "{error}");
        assert_eq!(
            fs::read(virtual_mount.join("existing.txt")).unwrap(),
            b"keep me"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_capability_check_rejects_existing_mount() {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig {
            mount_dir: temp.path().join("backing").to_string_lossy().into_owned(),
            mount_configured: true,
            virtual_drive_enabled: true,
            virtual_mount_dir: "/proc".to_string(),
            ..AppConfig::default()
        };

        let error = validate_virtual_drive_capabilities(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("已被其他文件系统占用"), "{error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_decoder_handles_escaped_paths() {
        assert_eq!(
            decode_mountinfo_path("/tmp/PetalLink\\040Drive\\134cache"),
            PathBuf::from("/tmp/PetalLink Drive\\cache")
        );
        let record = parse_mount_record(
            "42 31 0:77 / /tmp/PetalLink\\040Drive rw,nosuid,nodev - \
             fuse.petallink PetalLink rw,user_id=1000,group_id=1000\n",
            Path::new("/tmp/PetalLink Drive"),
        )
        .unwrap();
        assert_eq!(record.mount_id, 42);
        assert_eq!(record.fs_type, "fuse.petallink");
        assert_eq!(record.source, "PetalLink");
        assert_eq!(record.super_options, "rw,user_id=1000,group_id=1000");
        assert!(is_petallink_mount_record(&record));
        assert_eq!(mount_owner_uid(&record), Some(1000));
        assert!(is_current_mountpoint(Path::new("/")).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_cleanup_identity_rejects_other_filesystems_and_owners() {
        let other_fuse = parse_mount_record(
            "51 31 0:88 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.sshfs remote rw,user_id=1000,group_id=1000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(!is_petallink_mount_record(&other_fuse));

        let wrong_source = parse_mount_record(
            "52 31 0:89 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.petallink NotPetalLink rw,user_id=1000,group_id=1000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(!is_petallink_mount_record(&wrong_source));

        let other_owner = parse_mount_record(
            "53 31 0:90 / /mnt/huawei_cloud rw,nosuid,nodev - \
             fuse.petallink PetalLink rw,user_id=2000,group_id=2000\n",
            Path::new("/mnt/huawei_cloud"),
        )
        .unwrap();
        assert!(is_petallink_mount_record(&other_owner));
        assert_eq!(mount_owner_uid(&other_owner), Some(2000));

        assert!(
            parse_mount_record(
                "54 31 0:91 / /mnt/huawei_cloud-old rw - \
                 fuse.petallink PetalLink rw,user_id=1000\n",
                Path::new("/mnt/huawei_cloud"),
            )
            .is_none(),
            "挂载点必须精确匹配，不能按路径前缀误判"
        );
    }
}
