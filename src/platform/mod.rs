//! 桌面平台集成 —— 系统托盘 / 激活策略 / 开机自启 / 扩展属性 / 系统关机。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift` 的双层架构。
//!
//! # 核心不变量
//! - LSUIElement=true 在 Info.plist（Tauri 2 模板需配置）
//! - 系统托盘 = NSStatusItem，菜单含「显示主窗口」「退出 PetalLink」
//! - activationPolicy 在 .regular ↔ .accessory 间切换（UI 层显隐）
//! - --hidden 参数区分开机自启 vs 手动打开（Cmd+Q/Dock Quit/关闭按钮仅隐藏）
//! - 仅「退出 PetalLink」真正终止进程

/// 应用激活策略与退出拦截。
pub mod activation;
/// macOS 登录项注册。
pub mod launch_at_login;
/// 使用系统默认应用打开 URL 或本地路径。
pub mod opener;
/// 真实退出时的同步收束。
pub mod shutdown;
/// 系统托盘菜单与状态刷新。
pub mod tray;
/// 跨平台扩展属性键映射与读写。
pub mod xattr;
