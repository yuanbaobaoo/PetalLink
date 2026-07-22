//! 开机自启 —— 注册/注销 macOS LaunchAgent。
//!
//! 对齐 `legacy/lib/core/platform/launch_at_login_service.dart`。
//!
//! LaunchAgent plist 写入 `~/Library/LaunchAgents/<BUNDLE_IDENTIFIER>.plist`，
//! ProgramArguments 附加 `--hidden` 参数标识开机自启。
//!
//! # 立即生效
//! 写入/删除 plist 后调用 `launchctl bootstrap` / `launchctl bootout` 使其立即生效，
//! 无需用户注销或重启。

use std::path::PathBuf;
use std::process::Command;

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

/// 以 LaunchAgent 配置文件是否存在判断开机自启；不额外验证 bootstrap 状态。
///
/// 同时核对 launchd disabled 列表：用户在 系统设置 → 通用 → 登录项与扩展
/// 里关闭后，plist 文件仍在但服务已被 BTM 禁用，此时必须返回 false，
/// 保证 UI 开关与系统真实状态一致。
pub fn is_enabled() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false) && !is_disabled_by_system()
}

/// 系统（BTM / launchd disabled 列表）是否禁用了本服务。
/// 查询失败或列表中无本服务记录时按未禁用处理。
fn is_disabled_by_system() -> bool {
    let Some(uid) = current_uid() else {
        return false;
    };
    let output = Command::new("launchctl")
        .args(["print-disabled", &format!("gui/{uid}")])
        .output();
    let Ok(output) = output else { return false };
    parse_disabled_entry(&String::from_utf8_lossy(&output.stdout), BUNDLE_IDENTIFIER)
        .unwrap_or(false)
}

/// 解析 `launchctl print-disabled` 输出中指定 label 的禁用状态。
/// 行格式：`\t"com.example.foo" => enabled|disabled`。无记录返回 None。
fn parse_disabled_entry(output: &str, label: &str) -> Option<bool> {
    let quoted = format!("\"{label}\"");
    output.lines().find_map(|line| {
        let line = line.trim();
        if !line.contains(&quoted) {
            return None;
        }
        line.rsplit("=> ")
            .next()
            .map(|state| state.trim() == "disabled")
    })
}

/// 获取当前用户的 uid（供 launchctl bootstrap/bootout 用）。
fn current_uid() -> Option<u32> {
    let output = Command::new("id").arg("-u").output().ok()?;
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    s.parse().ok()
}

/// 解析可执行文件路径和 bundle 路径。
///
/// - 在 .app bundle 内运行时（release 打包）：返回 `(bundle_path, binary_inside_bundle)`，
///   plist ProgramArguments 用 `<bundle>/Contents/MacOS/<exec> --hidden`。
/// - 开发模式（`cargo tauri dev`，裸二进制）：返回 `(None, binary_path)`，
///   plist ProgramArguments 直接用 `<binary_path> --hidden`。
fn resolve_paths() -> (Option<PathBuf>, PathBuf) {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(EXECUTABLE_NAME));

    // .app bundle 路径：binary 往上三级（如 /Applications/PetalLink.app/Contents/MacOS/PetalLink）
    let bundle = exe
        .parent() // Contents/MacOS
        .and_then(|p| p.parent()) // Contents
        .and_then(|p| p.parent()) // .app
        .filter(|b| b.extension().map(|e| e == "app").unwrap_or(false));

    match bundle {
        Some(b) => {
            // bundle 内运行：ProgramArguments 写成 bundle 路径（LaunchAgent 用 open 或直接调 binary）
            let binary_in_bundle = PathBuf::from("Contents/MacOS").join(
                exe.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new(EXECUTABLE_NAME)),
            );
            (Some(b.to_path_buf()), binary_in_bundle)
        }
        None => {
            // 开发模式（裸二进制）：直接用可执行文件绝对路径
            (None, exe)
        }
    }
}

/// 从 macOS Login Items（系统设置 > 通用 > 登录项）移除 PetalLink。
///
/// 我们通过 LaunchAgent 管理开机自启，如用户此前手动添加到 Login Items 或
/// macOS 还原会话时自动添加，会造成「LaunchAgent 带 --hidden」+「Login Items 不带 --hidden」
/// 双重启动：Login Items 触发 single-instance 回调 → 无条件 show() 顶出窗口。
///
/// 此函数通过 osascript 调用 System Events 删除 Login Items 中的 PetalLink。
/// 如果 System Events 未授权或 PetalLink 不在 Login Items 中，静默忽略。
pub fn remove_from_login_items() {
    let script = format!(
        "tell application \"System Events\" to delete every login item whose name is \"{name}\"",
        name = crate::constants::APP_NAME,
    );
    match Command::new("osascript").arg("-e").arg(&script).output() {
        Ok(output) if output.status.success() => {
            tracing::info!("已从 Login Items 移除 {}", crate::constants::APP_NAME);
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // 常见无权限错误：System Events 未授权辅助功能
            if !stderr.trim().is_empty() {
                tracing::debug!(stderr = %stderr.trim(), "移除 Login Items 失败（可能未授权或不存在）");
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "osascript 调用失败，Login Items 清理跳过");
        }
    }
}

/// 启用/禁用开机自启：写/删 LaunchAgent plist + launchctl bootstrap/bootout。
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    let dir = launch_agents_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法获取 LaunchAgents 目录")
    })?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(plist_name());
    let uid = current_uid().unwrap_or(501); // 兜底默认 uid

    if !enabled {
        // 禁用：先 bootout 卸载，再删 plist
        if path.exists() {
            let label = BUNDLE_IDENTIFIER;
            let domain = format!("gui/{uid}/{label}");
            let _ = Command::new("launchctl")
                .args(["bootout", &domain])
                .output();
            std::fs::remove_file(&path)?;
            tracing::info!(label, "开机自启已禁用，LaunchAgent plist 已移除");
        }
        // 同时清理 Login Items（如果存在）
        remove_from_login_items();
        return Ok(());
    }

    // 启用：先清除 Login Items 中的重复项（防止双重启动 - 见 remove_from_login_items 文档）
    remove_from_login_items();

    // 启用：先卸载旧实例（避免重复 bootstrap 报错），再写 plist，最后 bootstrap
    let label = BUNDLE_IDENTIFIER;
    let domain = format!("gui/{uid}/{label}");
    if path.exists() {
        // 旧 plist 存在 → 先 bootout 再覆盖
        let _ = Command::new("launchctl")
            .args(["bootout", &domain])
            .output();
    }

    let (bundle, binary) = resolve_paths();

    let plist = if let Some(bundle_path) = bundle {
        // .app bundle 模式：用 open 命令启动（LaunchAgent 推荐方式）
        format!(
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
        )
    } else {
        // 开发模式（裸二进制）：直接用可执行文件路径
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>--hidden</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin</string>
    </dict>
</dict>
</plist>"#,
            label = BUNDLE_IDENTIFIER,
            binary = binary.display(),
        )
    };
    std::fs::write(&path, plist)?;

    // bootstrap 使 plist 立即生效（无需注销/重启）
    let bootstrap_result = Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{uid}"), &path.to_string_lossy()])
        .output();
    match &bootstrap_result {
        Ok(output) if output.status.success() => {
            tracing::info!(label, "开机自启已启用，LaunchAgent 已 bootstrap");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // bootstrap 报 "already bootstrapped" 也视为成功（旧实例未清理干净）
            if stderr.contains("already bootstrapped") || stderr.contains("service already loaded")
            {
                tracing::info!(label, "开机自启已启用（LaunchAgent 此前已 bootstrap）");
            } else {
                tracing::warn!(label, stderr = %stderr.trim(), "bootstrap 返回非零，plist 已写入但可能未立即生效（下次登录生效）");
            }
        }
        Err(e) => {
            tracing::warn!(label, error = %e, "bootstrap 命令执行失败，plist 已写入但可能未立即生效（下次登录生效）");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_disabled_entry;

    const SAMPLE: &str = r#"
	disabled services = {
		"com.docker.helper" => enabled
		"com.apple.ManagedClientAgent.enrollagent" => disabled
		"io.github.yuanbaobaoo.PetalLink" => disabled
		"homebrew.mxcl.ollama" => enabled
	}
"#;

    /// 系统设置里被关闭的服务应解析为禁用。
    #[test]
    fn parse_disabled_entry_detects_disabled() {
        assert_eq!(
            parse_disabled_entry(SAMPLE, "io.github.yuanbaobaoo.PetalLink"),
            Some(true)
        );
        assert_eq!(
            parse_disabled_entry(SAMPLE, "com.docker.helper"),
            Some(false)
        );
        // 无记录 → None（按未禁用处理）
        assert_eq!(parse_disabled_entry(SAMPLE, "com.example.missing"), None);
    }
}
