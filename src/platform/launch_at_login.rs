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
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    implementation::set_enabled(enabled)
}

/// 未实现登录启动集成的平台返回明确错误，避免 UI 误报成功。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn set_enabled(_enabled: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台暂不支持开机自启动",
    ))
}

/// macOS 清理可能与 LaunchAgent 重复的 Login Items；其他平台无需处理。
pub fn remove_from_login_items() {
    #[cfg(target_os = "macos")]
    implementation::remove_from_login_items();
}
