//! 开机自启 —— 按桌面平台注册用户登录启动项。

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as implementation;
#[cfg(target_os = "macos")]
use macos as implementation;

/// 查询当前用户的 PetalLink 登录启动项是否存在。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn is_enabled() -> bool {
    implementation::is_enabled()
}

/// 未实现登录启动集成的平台默认返回关闭。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn is_enabled() -> bool {
    false
}

/// 启用或禁用当前用户的 PetalLink 登录启动项。
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn set_enabled(enabled: bool, display_name: &str) -> std::io::Result<()> {
    implementation::set_enabled(enabled, display_name)
}

/// 未实现登录启动集成的平台返回明确错误，避免 UI 误报成功。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn set_enabled(_enabled: bool, _display_name: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台暂不支持开机自启动",
    ))
}

/// macOS 清理可能与 LaunchAgent 重复的 Login Items；其他平台无需处理。
pub fn remove_from_login_items(display_name: &str) {
    #[cfg(target_os = "macos")]
    implementation::remove_from_login_items(display_name);
    // Linux 等平台不维护独立 Login Items 列表，参数仅为跨平台签名一致。
    #[cfg(not(target_os = "macos"))]
    let _ = display_name;
}
