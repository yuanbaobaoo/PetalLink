//! macOS 原生集成 —— 系统托盘 / 激活策略 / 开机自启 / 系统关机。
//!
//! 对齐 `legacy/macos/Runner/AppDelegate.swift` 的双层架构。
//!
//! # 核心不变量
//! - LSUIElement=true 在 Info.plist（Tauri 2 模板需配置）
//! - 系统托盘 = NSStatusItem，菜单含「显示主窗口」「退出 PetalLink」
//! - activationPolicy 在 .regular ↔ .accessory 间切换（UI 层显隐）
//! - --hidden 参数区分开机自启 vs 手动打开（Cmd+Q/Dock Quit/关闭按钮仅隐藏）
//! - 仅「退出 PetalLink」真正终止进程

pub mod activation;
pub mod launch_at_login;
pub mod shutdown;
pub mod tray;
