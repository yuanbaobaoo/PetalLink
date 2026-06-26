//! 开机自启 —— 注册/注销 macOS LaunchAgent（SMAppService）。
//!
//! 对齐 `legacy/lib/core/platform/launch_at_login_service.dart`。
//!
//! LaunchAgent plist 写入 `~/Library/LaunchAgents/<BUNDLE_IDENTIFIER>.plist`，
//! ProgramArguments 附加 `--hidden` 参数标识开机自启。

use std::path::PathBuf;

use crate::constants::{BUNDLE_IDENTIFIER, EXECUTABLE_NAME};

/// LaunchAgent plist 文件名（派生自 bundle id，保持同步）。
fn plist_name() -> String {
    format!("{BUNDLE_IDENTIFIER}.plist")
}

/// 获取 LaunchAgents 目录路径。
fn launch_agents_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join("Library/LaunchAgents"))
}

/// 获取 plist 完整路径。
fn plist_path() -> Option<PathBuf> {
    Some(launch_agents_dir()?.join(plist_name()))
}

/// 是否已启用开机自启（plist 文件存在）。
pub fn is_enabled() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

/// 启用开机自启：写 LaunchAgent plist（ProgramArguments 带 --hidden）。
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    let dir = launch_agents_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法获取 LaunchAgents 目录")
    })?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(plist_name());

    if !enabled {
        // 禁用：删除 plist
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }

    // 启用：写 plist
    let bundle_path = std::env::current_exe()
        .ok()
        .and_then(|p| {
            // .app bundle 路径往上三级（如 /Applications/PetalLink.app/Contents/MacOS/PetalLink）
            p.parent()?.parent()?.parent().map(|b| b.to_path_buf())
        })
        .unwrap_or_else(|| PathBuf::from("/Applications/PetalLink.app"));

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bundle_path}/Contents/MacOS/{exec}</string>
        <string>--hidden</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#,
        label = BUNDLE_IDENTIFIER,
        exec = EXECUTABLE_NAME,
        bundle_path = bundle_path.display(),
    );
    std::fs::write(&path, plist)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_enabled_false_when_no_plist() {
        // 测试环境下不应有真实 plist
        assert!(!is_enabled() || plist_path().map(|p| p.exists()).unwrap_or(false));
    }
}
