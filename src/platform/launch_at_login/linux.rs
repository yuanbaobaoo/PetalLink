//! Linux 开机自启 —— 写入 Freedesktop Autostart `.desktop` 文件。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::constants::{APP_NAME, BUNDLE_IDENTIFIER};

/// 用户级 Autostart 文件名；沿用 bundle id 避免与其他应用冲突。
fn desktop_entry_name() -> String {
    format!("{BUNDLE_IDENTIFIER}.desktop")
}

/// 按 XDG Base Directory 规则解析当前用户的 Autostart 目录。
fn autostart_dir() -> Option<PathBuf> {
    resolve_autostart_dir(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        dirs::home_dir().as_deref(),
    )
}

/// 使用显式环境输入解析路径，便于在不修改进程环境的情况下测试。
fn resolve_autostart_dir(xdg_config_home: Option<&OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    let config_home = xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|path| path.join(".config")))?;
    Some(config_home.join("autostart"))
}

/// 返回当前用户的 PetalLink Autostart 文件路径。
fn desktop_entry_path() -> Option<PathBuf> {
    Some(autostart_dir()?.join(desktop_entry_name()))
}

/// AppImage 使用原始镜像路径，系统包和开发构建使用当前可执行文件路径。
fn executable_path() -> std::io::Result<PathBuf> {
    if let Some(app_image) = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(app_image);
    }
    std::env::current_exe()
}

/// 按 Desktop Entry `Exec` 字段规则转义一个完整参数。
fn escape_exec_argument(value: &OsStr) -> std::io::Result<String> {
    let value = value.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "可执行文件路径不是有效 UTF-8",
        )
    })?;
    if value.contains(['\n', '\r', '\0']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "可执行文件路径包含无效控制字符",
        ));
    }

    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            // Desktop Entry 先解析通用 string 转义，再解析 Exec quoting。
            // 因此 literal backslash 需要四个反斜杠，其余 Exec 特殊字符需要两个。
            '\\' => escaped.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                escaped.push_str(r"\\");
                escaped.push(character);
            }
            '%' => escaped.push_str("%%"),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

/// 生成桌面环境登录后以隐藏模式启动应用的 Desktop Entry。
fn desktop_entry(executable: &Path) -> std::io::Result<String> {
    let exec = expected_exec_value(executable)?;
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={APP_NAME}\n\
         Comment=PetalLink background sync\n\
         Exec={exec}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    ))
}

/// 返回当前可执行文件对应的完整 `Exec` 值。
fn expected_exec_value(executable: &Path) -> std::io::Result<String> {
    Ok(format!(
        "{} --hidden",
        escape_exec_argument(executable.as_os_str())?
    ))
}

/// 仅当 Autostart 条目仍指向当前可执行文件时报告已启用。
pub(super) fn is_enabled() -> bool {
    let Some(path) = desktop_entry_path() else {
        return false;
    };
    let Ok(executable) = executable_path() else {
        return false;
    };
    std::fs::read_to_string(path)
        .map(|content| desktop_entry_is_enabled(&content, &executable))
        .unwrap_or(false)
}

/// 按主 Desktop Entry 组解析禁用标记，并核对类型与当前启动命令。
fn desktop_entry_is_enabled(content: &str, executable: &Path) -> bool {
    let Ok(expected_exec) = expected_exec_value(executable) else {
        return false;
    };
    let mut in_desktop_entry = false;
    let mut is_application = false;
    let mut exec_matches = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Hidden" if value.eq_ignore_ascii_case("true") => return false,
            "X-GNOME-Autostart-enabled" if value.eq_ignore_ascii_case("false") => return false,
            "Type" if value.eq_ignore_ascii_case("application") => is_application = true,
            "Exec" => exec_matches = value == expected_exec,
            _ => {}
        }
    }
    is_application && exec_matches
}

/// 启用时写入用户级 Autostart 文件，禁用时幂等删除。
pub(super) fn set_enabled(enabled: bool) -> std::io::Result<()> {
    let path = desktop_entry_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法获取 XDG Autostart 目录")
    })?;

    if !enabled {
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::info!(path = %path.display(), "Linux 开机自启已禁用"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        return Ok(());
    }

    let executable = executable_path()?;
    let content = desktop_entry(&executable)?;
    let directory = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Autostart 路径缺少父目录")
    })?;
    std::fs::create_dir_all(directory)?;

    // 同目录临时文件替换避免会话管理器读到半截 Desktop Entry。
    let temporary = path.with_extension(format!("desktop.tmp-{}", std::process::id()));
    std::fs::write(&temporary, content)?;
    std::fs::rename(&temporary, &path)?;
    tracing::info!(path = %path.display(), executable = %executable.display(), "Linux 开机自启已启用");
    Ok(())
}

#[cfg(test)]
/// Linux Autostart 路径和 Desktop Entry 转义合同测试。
mod tests {
    use super::{
        desktop_entry, desktop_entry_is_enabled, escape_exec_argument, resolve_autostart_dir,
    };
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    /// 显式 XDG_CONFIG_HOME 优先于 HOME，且最终落到 autostart 子目录。
    #[test]
    fn xdg_config_home_has_priority() {
        assert_eq!(
            resolve_autostart_dir(Some(OsStr::new("/tmp/xdg")), Some(Path::new("/home/test"))),
            Some(PathBuf::from("/tmp/xdg/autostart"))
        );
    }

    /// 相对 XDG_CONFIG_HOME 无效时按规范回退到 HOME/.config。
    #[test]
    fn relative_xdg_config_home_falls_back_to_home() {
        assert_eq!(
            resolve_autostart_dir(Some(OsStr::new("relative")), Some(Path::new("/home/test"))),
            Some(PathBuf::from("/home/test/.config/autostart"))
        );
    }

    /// Exec 参数必须保留空格并转义 Desktop Entry 的保留字符和百分号。
    #[test]
    fn exec_argument_is_quoted_and_escaped() {
        assert_eq!(
            escape_exec_argument(OsStr::new("/opt/Petal Link/$current%/PetalLink")).unwrap(),
            "\"/opt/Petal Link/\\\\$current%%/PetalLink\""
        );
    }

    /// 反斜杠和双引号必须同时经过 string 与 Exec 两层转义。
    #[test]
    fn exec_argument_uses_two_escape_layers() {
        assert_eq!(
            escape_exec_argument(OsStr::new("/opt/a\\b\"c")).unwrap(),
            "\"/opt/a\\\\\\\\b\\\\\"c\""
        );
    }

    /// 生成内容以隐藏模式启动且不打开终端。
    #[test]
    fn desktop_entry_launches_hidden() {
        let content = desktop_entry(Path::new("/usr/bin/PetalLink")).unwrap();
        assert!(content.contains("Exec=\"/usr/bin/PetalLink\" --hidden"));
        assert!(content.contains("Terminal=false"));
    }

    /// 标准 Hidden 与 GNOME 扩展禁用标记都应让设置页显示为关闭。
    #[test]
    fn disabled_desktop_entries_are_not_reported_enabled() {
        let executable = Path::new("/usr/bin/PetalLink");
        let enabled = "[Desktop Entry]\nType=Application\nExec=\"/usr/bin/PetalLink\" --hidden\n";
        assert!(desktop_entry_is_enabled(enabled, executable));
        assert!(!desktop_entry_is_enabled(
            &format!("{enabled}Hidden=true\n"),
            executable
        ));
        assert!(!desktop_entry_is_enabled(
            &format!("{enabled}X-GNOME-Autostart-enabled=false\n"),
            executable
        ));
        assert!(!desktop_entry_is_enabled(
            "[Desktop Entry]\nType=Application\nHidden=false\n",
            executable
        ));
    }

    /// AppImage 移动或条目被篡改后必须显示为未启用，促使用户重新打开开关。
    #[test]
    fn stale_executable_is_not_reported_enabled() {
        let content =
            "[Desktop Entry]\nType=Application\nExec=\"/old/PetalLink.AppImage\" --hidden\n";
        assert!(!desktop_entry_is_enabled(
            content,
            Path::new("/new/PetalLink.AppImage")
        ));
    }
}
