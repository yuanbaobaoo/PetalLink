//! 应用入口 —— PetalLink Tauri 客户端
//!
//! 启动顺序（对齐 `legacy/lib/main.dart`）：
//! 1. 初始化日志系统（越早越好，便于观察后续初始化失败）
//! 2. 加载 .env 配置（client_secret 等敏感凭据；文件不存在时静默跳过）
//! 3. 全局异常捕获（§3.4：不崩溃）
//! 4. 启动 Tauri 应用
//!
//! # Panic 兜底
//! 通过 `std::panic::set_hook` 捕获 panic，记录日志后不崩溃（对齐 dart 的 PlatformDispatcher.onError）。

// 桌面端二进制入口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 全局 panic 钩子：记录后不崩溃（§3.4）
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("Box<dyn Any>");
        tracing::error!(location = %location, payload = %payload, "未捕获 panic");
        // ★ 写入 crash 标记文件，下次启动时检测并提示用户
        if let Ok(support) = petal_link_lib::core::config_store::support_dir() {
            let marker = support.join("last_crash.marker");
            let content = format!(
                "time={}\nlocation={}\npayload={}\n",
                chrono::Utc::now().to_rfc3339(),
                location,
                payload
            );
            let _ = std::fs::write(&marker, content);
        }
        default_hook(info);
    }));

    petal_link_lib::run()
}
