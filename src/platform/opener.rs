//! 系统打开器 —— 以参数方式调用桌面环境默认应用，避免经过 shell。

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "linux")]
const APPIMAGE_ENV_VARS: &[&str] = &[
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "GDK_PIXBUF_MODULEDIR",
    "GDK_PIXBUF_MODULE_FILE",
    "GIO_EXTRA_MODULES",
    "GSETTINGS_SCHEMA_DIR",
    "GTK_PATH",
    "LD_LIBRARY_PATH",
    "OWD",
];

/// AppImage 会把自身的动态库目录放到环境变量和 `PATH` 前部。如果直接继承这些变量，
/// `xdg-open` 启动的系统 `gio` 可能加载 AppImage 内的 Ubuntu GLib，进而在 Fedora 等
/// 发行版上寻找不存在的 `gio-launch-desktop` 路径。
#[cfg(target_os = "linux")]
fn configure_linux_opener_environment(
    command: &mut Command,
    is_appimage: bool,
    appdir: Option<&Path>,
    path: Option<&OsStr>,
) {
    if !is_appimage {
        return;
    }

    for variable in APPIMAGE_ENV_VARS {
        command.env_remove(variable);
    }

    let Some(path) = path else {
        return;
    };
    let entries = std::env::split_paths(&path)
        .filter(|entry| appdir.is_none_or(|root| !entry.starts_with(root)));
    if let Ok(path) = std::env::join_paths(entries) {
        command.env("PATH", path);
    }
}

/// 启动系统打开器并回收子进程。
///
/// `open`、`xdg-open` 只负责把目标交给桌面环境，正常情况下会很快退出。等待它们
/// 结束既能取得真实退出状态，也能避免在长期运行的托盘进程下留下僵尸子进程。
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_opener<I, S>(program: &str, args: I) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(target_os = "linux")]
    {
        let appdir = std::env::var_os("APPDIR");
        let is_appimage = std::env::var_os("APPIMAGE").is_some() || appdir.is_some();
        let path = std::env::var_os("PATH");
        configure_linux_opener_environment(
            &mut command,
            is_appimage,
            appdir.as_deref().map(Path::new),
            path.as_deref(),
        );
    }

    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} 退出状态异常：{status}"
        )))
    }
}

/// 使用系统默认应用打开 URL 或本地路径。
#[cfg(target_os = "macos")]
pub fn open(target: &str) -> io::Result<()> {
    run_opener("open", [OsStr::new(target)])
}

/// Linux 通过 Freedesktop `xdg-open` 交给当前桌面环境处理。
#[cfg(target_os = "linux")]
pub fn open(target: &str) -> io::Result<()> {
    run_opener("xdg-open", [OsStr::new(target)])
}

/// 尚未适配系统打开器的平台返回明确错误。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn open(_target: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "当前平台暂不支持系统打开器",
    ))
}

/// 使用系统文件管理器打开本地路径，保留平台原生路径参数。
#[cfg(target_os = "macos")]
pub fn open_path(path: &Path) -> io::Result<()> {
    run_opener("open", [path.as_os_str()])
}

/// Linux 通过 `xdg-open` 把绝对路径交给当前桌面文件管理器。
#[cfg(target_os = "linux")]
pub fn open_path(path: &Path) -> io::Result<()> {
    run_opener("xdg-open", [path.as_os_str()])
}

/// 尚未适配文件管理器的平台返回明确错误。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn open_path(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "当前平台暂不支持系统文件管理器",
    ))
}

/// 在系统文件管理器中显示本地项。
#[cfg(target_os = "macos")]
pub fn reveal_path(path: &Path) -> io::Result<()> {
    run_opener("open", [OsStr::new("-R"), path.as_os_str()])
}

/// Linux 桌面没有统一的“选中此文件”协议，退化为打开所在目录。
#[cfg(target_os = "linux")]
pub fn reveal_path(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "待显示路径缺少所在目录"))?;
    run_opener("xdg-open", [parent.as_os_str()])
}

/// 尚未适配文件管理器的平台返回明确错误。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn reveal_path(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "当前平台暂不支持在文件管理器中显示本地项",
    ))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn opener_success_status_is_accepted() {
        run_opener("true", [OsStr::new("ignored")]).expect("true should succeed");
    }

    #[test]
    fn opener_nonzero_status_is_reported() {
        let error = run_opener("false", [OsStr::new("ignored")]).expect_err("false should fail");
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn appimage_opener_uses_system_path_and_glib_environment() {
        let appdir = Path::new("/tmp/PetalLink.AppDir");
        let path =
            std::env::join_paths([appdir.join("usr/bin"), std::path::PathBuf::from("/usr/bin")])
                .unwrap();
        let mut command = Command::new("env");
        command.env("LD_LIBRARY_PATH", appdir.join("usr/lib"));
        command.env("GIO_EXTRA_MODULES", appdir.join("usr/lib/gio/modules"));
        configure_linux_opener_environment(
            &mut command,
            true,
            Some(appdir),
            Some(path.as_os_str()),
        );

        let output = command.output().unwrap();
        assert!(output.status.success());
        let environment = String::from_utf8(output.stdout).unwrap();
        assert!(!environment.contains("LD_LIBRARY_PATH="));
        assert!(!environment.contains("GIO_EXTRA_MODULES="));
        assert!(environment.lines().any(|line| line == "PATH=/usr/bin"));
    }
}
